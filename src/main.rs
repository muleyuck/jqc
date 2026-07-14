mod color;
mod edit;
mod edit_detect;
mod jaq;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use is_terminal::IsTerminal;
use std::io::{self, Read, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "jqc",
    version,
    about = "jq for JSONC — query, view, and edit JSON-with-Comments files without losing your comments."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// jq filter expression (filter mode, used when no subcommand is given)
    filter: Option<String>,

    /// Input file (reads from stdin if omitted)
    #[arg(
        value_name = "FILE",
        requires = "filter",
        conflicts_with = "null_input"
    )]
    file: Option<PathBuf>,

    /// Output strings without quotes (jq -r compatible)
    #[arg(short = 'r', long = "raw-output")]
    raw: bool,

    /// Compact output (no newlines)
    #[arg(short = 'c', long = "compact")]
    compact: bool,

    /// Force color output even when writing to a pipe
    #[arg(short = 'C', long = "color-output")]
    color: bool,

    /// Disable color output
    #[arg(short = 'M', long = "monochrome-output")]
    monochrome: bool,

    /// Use null as the input value instead of reading from stdin or a file
    #[arg(short = 'n', long = "null-input")]
    null_input: bool,

    /// Edit the file in-place (only valid when the filter is an edit
    /// expression such as `.a = 1` or `del(.a)`)
    #[arg(short = 'i', long = "in-place", conflicts_with = "null_input")]
    in_place: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate and output JSONC, preserving comments
    Fmt {
        /// Input file (reads from stdin if omitted)
        file: Option<PathBuf>,
        /// Edit the file in-place
        #[arg(short = 'i', long = "in-place")]
        in_place: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let use_color = resolve_color(cli.color, cli.monochrome);

    match cli.command {
        Some(Command::Fmt { file, in_place }) => {
            if in_place && file.is_none() {
                anyhow::bail!("--in-place requires a file argument");
            }
            let text = read_input(file.as_deref())?;
            jsonc_parser::parse_to_serde_value::<serde_json::Value>(&text, &Default::default())
                .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;
            if in_place {
                write_output(&text, file.as_deref())
            } else if use_color {
                print_colored(&text);
                Ok(())
            } else {
                println!("{text}");
                Ok(())
            }
        }
        None => {
            let filter = cli.filter.unwrap_or_else(|| ".".to_string());
            match edit_detect::detect(&filter)? {
                Some(form) => run_edit_form(form, &filter, cli.file, cli.in_place, use_color),
                None => {
                    if cli.in_place {
                        anyhow::bail!(
                            "--in-place requires an edit expression (e.g. '.a = 1' or 'del(.a)'), not a read-only filter"
                        );
                    }
                    let values = if cli.null_input {
                        jaq::run_null(&filter)?
                    } else {
                        let text = read_input(cli.file.as_deref())?;
                        jaq::run(&filter, &text)?
                    };
                    for val in values {
                        print_value(&format!("{val}"), cli.raw, cli.compact, use_color)?;
                    }
                    Ok(())
                }
            }
        }
    }
}

fn run_edit_form(
    form: edit_detect::EditForm<'_>,
    filter: &str,
    file: Option<PathBuf>,
    in_place: bool,
    use_color: bool,
) -> Result<()> {
    if in_place && file.is_none() {
        anyhow::bail!("--in-place requires a file argument");
    }
    let text = read_input(file.as_deref())?;
    let result = match form {
        edit_detect::EditForm::Assign { lhs } => edit::apply_assign(&text, lhs, filter)?,
        edit_detect::EditForm::Del { path } => edit::del(&text, path)?,
    };
    if in_place {
        write_output(&result, file.as_deref())
    } else if use_color {
        print_colored(&result);
        Ok(())
    } else {
        write_output(&result, None)
    }
}

fn print_colored(text: &str) {
    let palette = color::Palette::from_env();
    println!("{}", color::colorize_jsonc(text, &palette));
}

fn resolve_color(force_color: bool, monochrome: bool) -> bool {
    if monochrome {
        return false;
    }
    if force_color {
        return true;
    }
    // NO_COLOR spec: https://no-color.org/
    if std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty()) {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn print_value(output: &str, raw: bool, compact: bool, use_color: bool) -> Result<()> {
    if raw {
        // Strip surrounding quotes from string values
        if output.starts_with('"') && output.ends_with('"') {
            println!("{}", &output[1..output.len() - 1]);
        } else {
            println!("{output}");
        }
    } else if compact {
        println!("{output}");
    } else {
        let v: serde_json::Value = serde_json::from_str(output)?;
        let pretty = serde_json::to_string_pretty(&v)?;
        if use_color {
            print_colored(&pretty);
        } else {
            println!("{pretty}");
        }
    }
    Ok(())
}

/// Write `content` to `file` in-place (atomic via temp file), or to stdout if `file` is None.
fn write_output(content: &str, file: Option<&std::path::Path>) -> Result<()> {
    match file {
        None => {
            println!("{content}");
            Ok(())
        }
        Some(path) => {
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
    }
}

fn read_input(file: Option<&std::path::Path>) -> Result<String> {
    match file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read '{}': {}", path.display(), e)),
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow!("Failed to read stdin: {}", e))?;
            Ok(buf)
        }
    }
}
