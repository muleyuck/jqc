mod edit;
mod jaq;
mod query;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::io::{self, Read, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "jqc",
    version,
    about = "A jq-like CLI for JSONC (JSON with Comments)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// jq filter expression (filter mode, used when no subcommand is given)
    filter: Option<String>,

    /// Input file (reads from stdin if omitted)
    #[arg(value_name = "FILE", requires = "filter")]
    file: Option<PathBuf>,

    /// Output strings without quotes (jq -r compatible)
    #[arg(short = 'r', long = "raw-output")]
    raw: bool,

    /// Compact output (no newlines)
    #[arg(short = 'c', long = "compact")]
    compact: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Set a value at the given path (comment-preserving)
    Set {
        /// jq-style path (e.g. .server.port)
        path: String,
        /// New value (JSON-encoded, e.g. 8080 or '"hello"')
        value: String,
        /// Input file (reads from stdin if omitted)
        file: Option<PathBuf>,
        /// Edit the file in-place
        #[arg(short = 'i', long = "in-place")]
        in_place: bool,
    },
    /// Delete an object key at the given path (comment-preserving)
    Del {
        /// jq-style path (e.g. .debug)
        path: String,
        /// Input file (reads from stdin if omitted)
        file: Option<PathBuf>,
        /// Edit the file in-place
        #[arg(short = 'i', long = "in-place")]
        in_place: bool,
    },
    /// Append a value to the array at the given path (comment-preserving)
    Push {
        /// jq-style path (e.g. .plugins)
        path: String,
        /// Value to append (JSON-encoded)
        value: String,
        /// Input file (reads from stdin if omitted)
        file: Option<PathBuf>,
        /// Edit the file in-place
        #[arg(short = 'i', long = "in-place")]
        in_place: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(cmd) => run_edit(cmd),
        None => {
            let filter = cli.filter.unwrap_or_else(|| ".".to_string());
            let text = read_input(cli.file.as_deref())?;
            run_filter(&filter, &text, cli.raw, cli.compact)
        }
    }
}

fn run_filter(filter: &str, text: &str, raw: bool, compact: bool) -> Result<()> {
    let results = query::run_filter(filter, text)?;
    for val in results {
        let output = format!("{val}");
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
            // Pretty-print via serde_json
            let v: serde_json::Value = serde_json::from_str(&output)?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

fn run_edit(cmd: Command) -> Result<()> {
    match cmd {
        Command::Set { path, value, file, in_place } =>
            run_edit_op(file, in_place, |text| edit::set(text, &path, &value)),
        Command::Del { path, file, in_place } =>
            run_edit_op(file, in_place, |text| edit::del(text, &path)),
        Command::Push { path, value, file, in_place } =>
            run_edit_op(file, in_place, |text| edit::push(text, &path, &value)),
    }
}

fn run_edit_op(
    file: Option<PathBuf>,
    in_place: bool,
    op: impl FnOnce(&str) -> Result<String>,
) -> Result<()> {
    if in_place && file.is_none() {
        anyhow::bail!("--in-place requires a file argument");
    }
    let text = read_input(file.as_deref())?;
    let result = op(&text)?;
    write_output(&result, if in_place { file.as_deref() } else { None })
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
