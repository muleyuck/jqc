mod args;
mod color;
mod edit;
mod edit_detect;
mod jaq;
mod jq;
mod jsonc;

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
       jqc fmt [--in-place] [file]

jqc converts JSONC (JSON with comments) to JSON and runs jq on it, so jq's
options work as they do in jq. Edit expressions such as '.a = 1' and
'del(.a)' print the edited JSONC with its comments instead.

jqc's own options:
  --in-place   Write an edit expression's result back to the file
  --help       Show this help (jq's own help: jqc -h)
  --version    Show the versions of jqc and jq

Subcommands:
  fmt          Validate JSONC and print it with its comments

jqc needs jq on PATH: https://jqlang.org/download/
";

/// An error that ends jqc with `status`, as jq uses them: 2 for a usage
/// problem or system error, 5 for input jqc can't read.
struct Failure {
    status: u8,
    error: anyhow::Error,
}

fn fail(status: u8) -> impl FnOnce(anyhow::Error) -> Failure {
    move |error| Failure { status, error }
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
            eprintln!("Error: {error:?}");
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
            let form = match &run.filter {
                Some(filter) => edit_detect::detect(filter).map_err(fail(5))?,
                None => None,
            };
            match form {
                Some(form) => run_edit_form(form, &run),
                None if run.in_place => Err(fail(2)(anyhow!(
                    "--in-place requires an edit expression (e.g. '.a = 1' or 'del(.a)'), not a read-only filter"
                ))),
                None => run_filter(run),
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
        print_colored(&text);
    } else {
        println!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

fn run_edit_form(form: edit_detect::EditForm<'_>, run: &Run) -> Result<ExitCode, Failure> {
    let file = match run.files.as_slice() {
        [] => None,
        [file] => Some(file.as_str()),
        _ => return Err(fail(2)(anyhow!("an edit expression takes one input file"))),
    };
    reject_jq_options(run).map_err(fail(2))?;
    if run.in_place && file.is_none() {
        return Err(fail(2)(anyhow!("--in-place requires a file argument")));
    }
    let text = read_input(file).map_err(fail(2))?;
    let filter = run
        .filter
        .as_deref()
        .expect("edit forms are detected from the filter");
    let result = match form {
        edit_detect::EditForm::Assign { lhs } => edit::apply_assign(&text, lhs, filter),
        edit_detect::EditForm::Del { path } => edit::del(&text, path),
    }
    .map_err(fail(5))?;
    if run.in_place {
        write_output(&result, file).map_err(fail(2))?;
    } else if resolve_color(run.color) {
        print_colored(&result);
    } else {
        println!("{result}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Edit expressions run without jq, so jq's options would be silently
/// ignored. Only the filter and the color options are accepted.
fn reject_jq_options(run: &Run) -> anyhow::Result<()> {
    // `-c` is accepted and ignored: edit output is never compacted (#68).
    let is_accepted = |a: &str| {
        args::COLOR_OPTIONS.contains(&a)
            || a == "--compact-output"
            || (a.len() > 1
                && a.starts_with('-')
                && !a.starts_with("--")
                && a[1..].chars().all(|c| matches!(c, 'c' | 'C' | 'M')))
    };
    let mut filter_skipped = false;
    let mut rejected = Vec::new();
    for a in &run.jq_args {
        if !filter_skipped && Some(a) == run.filter.as_ref() {
            filter_skipped = true;
        } else if a != "--" && !is_accepted(a) {
            rejected.push(a.as_str());
        }
    }
    if rejected.is_empty() {
        return Ok(());
    }
    let short_i = |a: &&str| a.starts_with('-') && !a.starts_with("--") && a.contains('i');
    let hint = if rejected.iter().any(short_i) {
        " (use --in-place to write the file)"
    } else {
        ""
    };
    Err(anyhow!(
        "jq options are not supported with edit expressions yet: {}{hint}",
        rejected.join(" ")
    ))
}

fn run_filter(mut run: Run) -> Result<ExitCode, Failure> {
    // jq reads --slurpfile files itself, so it gets converted copies.
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
    thread::spawn(move || {
        feed(&mut stdin, &files, &feeder_failures);
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
fn feed(jq: &mut impl Write, files: &[String], failures: &Mutex<Failures>) {
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

fn print_colored(text: &str) {
    let palette = color::Palette::from_env();
    println!("{}", color::colorize_jsonc(text, &palette));
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
