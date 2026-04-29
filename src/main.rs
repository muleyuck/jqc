mod edit;
mod error;
mod parser;
mod query;

use anyhow::{anyhow, Result};
use clap::Parser;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "jqc",
    version,
    about = "A jq-like CLI for JSONC (JSON with Comments)"
)]
struct Cli {
    /// jq filter expression
    filter: String,

    /// Input file (reads from stdin if omitted)
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Output strings without quotes (jq -r compatible)
    #[arg(short = 'r', long = "raw-output")]
    raw: bool,

    /// Compact output (no newlines)
    #[arg(short = 'c', long = "compact")]
    compact: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let text = read_input(cli.file.as_deref())?;
    let results = query::run_filter(&cli.filter, &text)?;

    for val in results {
        let output = format!("{val}");
        if cli.raw {
            // Strip surrounding quotes from string values
            if output.starts_with('"') && output.ends_with('"') {
                println!("{}", &output[1..output.len() - 1]);
            } else {
                println!("{output}");
            }
        } else if cli.compact {
            println!("{output}");
        } else {
            // Pretty-print via serde_json
            let v: serde_json::Value = serde_json::from_str(&output)?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }

    Ok(())
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
