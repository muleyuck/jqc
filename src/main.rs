mod args;
mod color;
mod jq;
mod jsonc;
mod patch;

use std::io::{self, Read, Write};
use std::process::{ExitCode, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::anyhow;
use is_terminal::IsTerminal;

use args::{Command, Run};

const HELP: &str = "\
jqc - jq for JSONC

Usage: jqc [jq options] [filter] [files...]
       jqc --edit [jq options] <filter> [file]
       jqc --in-place [jq options] <filter> <files...>
       jqc fmt [--in-place] [file]

jqc converts JSONC (JSON with comments) to JSON and runs jq on it, so jq's
options work as they do in jq.

jqc's own options:
  --edit       Run the filter as an edit and print the input with the
               result written back, keeping comments and formatting
  --in-place   Like --edit, but write the result back to each file
  --help       Show this help (jq's own help: jqc -h)
  --version    Show the versions of jqc and jq

Subcommands:
  fmt          Validate JSONC and print it with its comments

jqc needs jq on PATH: https://jqlang.org/download/
";

/// An error that ends jqc with `status`, as jq uses them: 2 for a usage
/// problem or system error, 5 for input jqc can't read. `error` is `None`
/// when jq already reported the error itself.
struct Failure {
    status: u8,
    error: Option<anyhow::Error>,
}

fn fail(status: u8) -> impl FnOnce(anyhow::Error) -> Failure {
    move |error| Failure {
        status,
        error: Some(error),
    }
}

/// jq failed and printed its own message; jqc exits with jq's status.
fn jq_failed(status: ExitStatus) -> Failure {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return Failure {
                status: (128 + signal) as u8,
                error: None,
            };
        }
    }
    Failure {
        status: status.code().unwrap_or(1) as u8,
        error: None,
    }
}

fn main() -> ExitCode {
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect();
    let args = match args {
        Ok(args) => args,
        Err(arg) => {
            eprintln!("Error: argument is not valid UTF-8: {arg:?}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(code) => code,
        Err(Failure { status, error }) => {
            if let Some(error) = error {
                eprintln!("Error: {error:?}");
            }
            ExitCode::from(status)
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, Failure> {
    match args::parse(args).map_err(fail(2))? {
        Command::Help => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Version => {
            println!("jqc {}", env!("CARGO_PKG_VERSION"));
            println!(
                "{}",
                jq::version().unwrap_or_else(|| "jq not found".to_string())
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Fmt {
            file,
            in_place,
            color,
        } => run_fmt(file.as_deref(), in_place, color),
        Command::Run(run) => {
            if run.edit || run.in_place {
                run_edit(run)
            } else {
                run_filter(run)
            }
        }
    }
}

fn run_fmt(file: Option<&str>, in_place: bool, color: Option<bool>) -> Result<ExitCode, Failure> {
    if in_place && file.is_none() {
        return Err(fail(2)(anyhow!("--in-place requires a file argument")));
    }
    let text = read_input(file).map_err(fail(2))?;
    jsonc_parser::parse_to_serde_value::<serde_json::Value>(&text, &Default::default())
        .map_err(|e| fail(5)(anyhow!("Failed to parse JSONC: {e}")))?;
    if in_place {
        write_output(&text, file).map_err(fail(2))?;
    } else if resolve_color(color) {
        print_colored(&text)?;
    } else {
        print_out(&text)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// jq reads --slurpfile files itself, so it gets converted copies. The
/// copies live as long as the returned files.
fn convert_slurpfiles(run: &mut Run) -> Result<Vec<tempfile::NamedTempFile>, Failure> {
    let mut slurped = Vec::new();
    for &i in &run.slurpfiles {
        let path = run.jq_args[i].clone();
        let text = read_input(Some(&path)).map_err(fail(2))?;
        let json = jsonc::convert(&text, &path).map_err(fail(2))?.join("\n");
        let mut copy = tempfile::NamedTempFile::new().map_err(|e| fail(2)(e.into()))?;
        copy.write_all(json.as_bytes())
            .map_err(|e| fail(2)(e.into()))?;
        run.jq_args[i] = copy.path().to_string_lossy().into_owned();
        slurped.push(copy);
    }
    Ok(slurped)
}

/// jq options that change how jq reads its input or writes its output.
/// Edit mode reads one document and writes it back in its own format.
const EDIT_REJECTED_OPTIONS: [&str; 18] = [
    "--debug-trace",
    "--debug-dump-disasm",
    "--build-configuration",
    "--run-tests",
    "--null-input",
    "--raw-input",
    "--slurp",
    "--stream",
    "--stream-errors",
    "--seq",
    "--compact-output",
    "--raw-output",
    "--raw-output0",
    "--join-output",
    "--ascii-output",
    "--sort-keys",
    "--tab",
    "--indent",
];
const EDIT_REJECTED_SHORT: [char; 10] = ['n', 'R', 's', 'c', 'r', 'j', 'a', 'S', 'h', 'V'];

fn check_edit_options(run: &Run) -> anyhow::Result<()> {
    for option in &run.options {
        let rejected = if option.starts_with("--") {
            EDIT_REJECTED_OPTIONS.contains(&option.as_str())
                || option.starts_with("--debug-trace")
                || option.starts_with("--debug-dump-disasm")
        } else {
            option[1..]
                .chars()
                .any(|c| EDIT_REJECTED_SHORT.contains(&c))
        };
        if rejected {
            return Err(anyhow!(
                "{option} cannot be used with --edit: edits keep the file's own format"
            ));
        }
    }
    Ok(())
}

fn run_edit(mut run: Run) -> Result<ExitCode, Failure> {
    check_edit_options(&run).map_err(fail(2))?;
    if run.in_place && (run.files.is_empty() || run.files.iter().any(|f| f == "-")) {
        return Err(fail(2)(anyhow!("--in-place requires a file argument")));
    }
    if !run.in_place && run.files.len() > 1 {
        return Err(fail(2)(anyhow!("--edit takes one input file")));
    }
    let _slurped = convert_slurpfiles(&mut run)?;
    // jqc colors the edited document itself; jq's color codes would break
    // reading its result. jq's -M wins over -C wherever -C stands.
    let mut jq_args = vec!["-c".to_string(), "-M".to_string()];
    jq_args.extend(run.jq_args.iter().cloned());
    let inputs: Vec<Option<&str>> = if run.files.is_empty() {
        vec![None]
    } else {
        run.files
            .iter()
            .map(|f| Some(f.as_str()).filter(|f| *f != "-"))
            .collect()
    };
    for file in inputs {
        let text = read_input(file).map_err(fail(2))?;
        let edited = edit_document(&jq_args, &text, file.unwrap_or("<stdin>"))?;
        if run.in_place {
            write_output(&edited, file).map_err(fail(2))?;
        } else if resolve_color(run.color) {
            print_colored(&edited)?;
        } else {
            print_out(&edited)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// One document: jq computes the edited value from the converted JSON, and
/// only the difference is written back into `text`.
fn edit_document(jq_args: &[String], text: &str, name: &str) -> Result<String, Failure> {
    let values = jsonc::convert(text, name).map_err(fail(5))?;
    let [converted] = values.as_slice() else {
        return Err(fail(5)(anyhow!(
            "an edit needs exactly one value in {name}, found {}",
            values.len()
        )));
    };
    // jq canonicalizes number spellings (`1e2` becomes `1E+2`), so the
    // original value is compared in the spelling jq gives it back. jq prints
    // NaN as null, which would make an edit to null look like no change, so
    // NaN is mapped to a string no result can equal (`patch::NAN_MARK`).
    const CANONICALIZE: &str = r#"def w: if type == "object" then map_values(w) elif type == "array" then map(w) elif type == "number" and isnan then "\u0000jqc:NaN" else . end; w"#;
    let (status, canonical) =
        jq::output(&["-c".to_string(), CANONICALIZE.to_string()], converted).map_err(fail(2))?;
    if !status.success() {
        return Err(jq_failed(status));
    }
    let canonical = jsonc::convert(&canonical, "jq's output").map_err(fail(5))?;
    let [source] = canonical.as_slice() else {
        return Err(fail(5)(anyhow!("jq did not return one value for {name}")));
    };
    let (status, stdout) = jq::output(jq_args, converted).map_err(fail(2))?;
    if !status.success() {
        return Err(jq_failed(status));
    }
    let results = jsonc::convert(&stdout, "jq's output").map_err(fail(5))?;
    let [result] = results.as_slice() else {
        return Err(fail(5)(anyhow!(
            "the edit produced {} results; it must produce exactly one",
            results.len()
        )));
    };
    patch::write_back(text, source, result).map_err(fail(5))
}

fn run_filter(mut run: Run) -> Result<ExitCode, Failure> {
    let slurped = convert_slurpfiles(&mut run)?;

    if run.raw_input {
        // -R reads text, not JSON, so jq reads the inputs itself, with each
        // file back where it was (it matters relative to --args).
        let mut jq_args = run.jq_args;
        for (file, position) in run.files.into_iter().zip(run.file_positions).rev() {
            jq_args.insert(position, file);
        }
        let mut child = jq::spawn(&jq_args, Stdio::inherit()).map_err(fail(2))?;
        let status = child.wait().map_err(|e| fail(2)(e.into()))?;
        return Ok(exit_code(status));
    }

    let mut child = jq::spawn(&run.jq_args, Stdio::piped()).map_err(fail(2))?;
    let mut stdin = child.stdin.take().expect("jq's stdin is piped");
    let failures = Arc::new(Mutex::new(Failures::default()));
    let feeder_failures = Arc::clone(&failures);
    let files = run.files;
    let seq = run.options.iter().any(|o| o == "--seq");
    thread::spawn(move || {
        feed(&mut stdin, &files, seq, &feeder_failures);
        // Closing stdin after recording the failures lets jq finish first.
        drop(stdin);
    });
    // Wait for jq only: input jq never reads (`-n 1`) must not keep jqc
    // running.
    let status = child.wait().map_err(|e| fail(2)(e.into()))?;
    drop(slurped);
    // jq exits 2 for a usage problem and 3 for a filter that doesn't
    // compile, before it opens any input, so the inputs are not reported.
    if matches!(status.code(), Some(2 | 3)) {
        return Ok(exit_code(status));
    }
    let Failures { read, parse } = std::mem::take(&mut *failures.lock().unwrap());
    if let Some(read) = &read {
        eprintln!("Error: {read:?}");
    }
    // jq reads a token it rejects where the broken value was, so its own
    // status already says whether the filter hit the error (5, jq's status
    // for input it can't parse) or caught or never read it. jqc only adds
    // where the input really broke.
    if let Some(parse) = parse.filter(|_| status.code() == Some(5)) {
        eprintln!("Error: {parse:?}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal().is_some() {
            return Ok(exit_code(status));
        }
    }
    // Like jq, an unopenable file gives 2 over jq's own status.
    if read.is_some() {
        return Ok(ExitCode::from(2));
    }
    Ok(exit_code(status))
}

/// What went wrong while feeding jq. Kept as the failures happen, because
/// jq may stop reading before the feeder finishes.
#[derive(Default)]
struct Failures {
    /// The first file that couldn't be read.
    read: Option<anyhow::Error>,
    /// The input that couldn't be parsed; feeding stops there.
    parse: Option<anyhow::Error>,
}

/// Writes the inputs to jq as JSON, one file after another. Stops quietly
/// when jq stops reading.
fn feed(jq: &mut impl Write, files: &[String], seq: bool, failures: &Mutex<Failures>) {
    let inputs: Vec<Option<&str>> = if files.is_empty() {
        vec![None]
    } else {
        files.iter().map(|f| Some(f.as_str())).collect()
    };
    for file in inputs {
        // Like jq, `-` names stdin.
        let file = file.filter(|f| *f != "-");
        // Like jq, an unreadable file is reported and the remaining files
        // are still read.
        let text = match read_input(file) {
            Ok(text) => text,
            Err(e) => {
                failures.lock().unwrap().read.get_or_insert(e);
                continue;
            }
        };
        if seq {
            if jq.write_all(seq_records(&text).as_bytes()).is_err() {
                return;
            }
            continue;
        }
        let prefix = jsonc::convert_prefix(&text, file.unwrap_or("<stdin>"));
        let mut json: String = prefix.values.iter().map(|v| format!("{v}\n")).collect();
        if let Some(e) = prefix.error {
            // After the values before the broken one, a token jq always
            // rejects: jq fails where the broken value was only if it reads
            // that far (not under -n unless the filter asks for input), as
            // it would on the original input. The original rest isn't enough:
            // jq accepts some things jsonc-parser rejects.
            json.push_str("]\n");
            failures.lock().unwrap().parse = Some(e);
            let _ = jq.write_all(json.as_bytes());
            return;
        }
        if jq.write_all(json.as_bytes()).is_err() {
            return;
        }
    }
}

/// jq --seq reads values that each start with a record separator (RS) and
/// skips, with a warning, what it can't read. Each RS-separated part is
/// converted as JSONC; a part that doesn't convert, and any text before the
/// first RS, goes to jq as written, so jq skips what it would skip.
fn seq_records(text: &str) -> String {
    let mut parts = text.split('\x1e');
    let mut out = parts.next().unwrap_or("").to_string();
    for part in parts {
        out.push('\x1e');
        let prefix = jsonc::convert_prefix(part, "<seq>");
        out.push_str(&prefix.values.join("\n"));
        if prefix.error.is_some() {
            // jq skips the broken remainder, as written, with a warning.
            if !prefix.values.is_empty() {
                out.push('\n');
            }
            out.push_str(&part[prefix.rest..]);
        } else if part.len() > part.trim_end().len() || jsonc::ends_with_comment(part) {
            // jq reads a value as cut off only when the record's text
            // ends exactly at the value, so end it with a newline
            // whenever anything (whitespace or a comment) followed it.
            out.push('\n');
        }
    }
    out
}

fn exit_code(status: ExitStatus) -> ExitCode {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return ExitCode::from((128 + signal) as u8);
        }
    }
    ExitCode::from(status.code().unwrap_or(1) as u8)
}

/// Print `text` and a newline. When the reader has closed the pipe
/// (`| head`), end quietly with 141, as a process killed by SIGPIPE does
/// (and as jq does in filter mode).
fn print_out(text: &str) -> Result<(), Failure> {
    let mut out = io::stdout().lock();
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Err(Failure {
            status: 141,
            error: None,
        }),
        Err(e) => Err(fail(2)(anyhow!("Failed to write to stdout: {e}"))),
    }
}

fn print_colored(text: &str) -> Result<(), Failure> {
    let palette = color::Palette::from_env();
    print_out(&color::colorize_jsonc(text, &palette))
}

/// `color` is `-C` (`Some(true)`) or `-M` (`Some(false)`).
fn resolve_color(color: Option<bool>) -> bool {
    if let Some(color) = color {
        return color;
    }
    // NO_COLOR spec: https://no-color.org/
    if std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty()) {
        return false;
    }
    io::stdout().is_terminal()
}

/// Write `content` to `file` in-place (atomic via temp file), or to stdout if `file` is None.
fn write_output(content: &str, file: Option<&str>) -> anyhow::Result<()> {
    let Some(path) = file else {
        println!("{content}");
        return Ok(());
    };
    let path = std::path::Path::new(path);
    // Write to a temp file in the same directory, then rename for atomicity
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| anyhow!("Failed to create temp file: {e}"))?;
    tmp.write_all(content.as_bytes())
        .map_err(|e| anyhow!("Failed to write temp file: {e}"))?;
    tmp.persist(path)
        .map_err(|e| anyhow!("Failed to replace '{}': {e}", path.display()))?;
    Ok(())
}

fn read_input(file: Option<&str>) -> anyhow::Result<String> {
    match file {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|e| anyhow!("Failed to read '{path}': {e}"))
        }
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow!("Failed to read stdin: {e}"))?;
            Ok(buf)
        }
    }
}
