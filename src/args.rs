//! Splits jqc's command line into jqc's own options, the input files, and
//! the arguments passed to jq unchanged.

use anyhow::{Result, bail};

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Version,
    Fmt {
        file: Option<String>,
        in_place: bool,
        color: Option<bool>,
    },
    Run(Run),
}

#[derive(Debug, Default, PartialEq)]
pub struct Run {
    /// Arguments for jq, in their original order. Includes the filter (or
    /// the filter file with `-f`) but not the input files, which jqc reads.
    pub jq_args: Vec<String>,
    /// The filter text, unless jq reads it from a file (`-f`).
    pub filter: Option<String>,
    pub files: Vec<String>,
    /// Positions in `jq_args` of `--slurpfile` paths, which jqc converts.
    pub slurpfiles: Vec<usize>,
    pub null_input: bool,
    pub raw_input: bool,
    /// `-C` is `Some(true)` and `-M` `Some(false)`; the last one wins.
    pub color: Option<bool>,
    pub in_place: bool,
}

const COLOR_OPTIONS: [&str; 4] = ["-C", "-M", "--color-output", "--monochrome-output"];

/// `args` excludes the program name.
pub fn parse(args: Vec<String>) -> Result<Command> {
    // `jqc -C fmt` worked before, so color options may come first.
    let fmt_at = args
        .iter()
        .position(|a| !COLOR_OPTIONS.contains(&a.as_str()));
    if let Some(i) = fmt_at
        && args[i] == "fmt"
    {
        let mut rest = args;
        rest.remove(i);
        return parse_fmt(rest);
    }

    let mut run = Run::default();
    let mut from_file = false;
    let mut filter_seen = false;
    let mut positional_values = false;
    let mut options_ended = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        // jq treats `-` followed by `-` or an ASCII letter as an option.
        let optionish = arg.starts_with('-')
            && arg
                .as_bytes()
                .get(1)
                .is_some_and(|b| *b == b'-' || b.is_ascii_alphabetic());
        if options_ended || !optionish {
            if !filter_seen {
                filter_seen = true;
                run.filter = Some(arg.clone());
                run.jq_args.push(arg);
            } else if positional_values {
                run.jq_args.push(arg);
            } else {
                run.files.push(arg);
            }
            continue;
        }
        match arg.as_str() {
            "--help" => return Ok(Command::Help),
            "--version" => return Ok(Command::Version),
            "--in-place" => run.in_place = true,
            "--" => {
                options_ended = true;
                run.jq_args.push(arg);
            }
            "--arg" | "--argjson" | "--rawfile" => take(&mut args, &mut run.jq_args, arg, 2),
            "--slurpfile" => {
                let before = run.jq_args.len();
                take(&mut args, &mut run.jq_args, arg, 2);
                if run.jq_args.len() == before + 3 {
                    run.slurpfiles.push(before + 2);
                }
            }
            "--indent" | "--library-path" => take(&mut args, &mut run.jq_args, arg, 1),
            "--args" | "--jsonargs" => {
                positional_values = true;
                run.jq_args.push(arg);
            }
            "--from-file" => {
                from_file = true;
                run.jq_args.push(arg);
            }
            "--null-input" => {
                run.null_input = true;
                run.jq_args.push(arg);
            }
            "--raw-input" => {
                run.raw_input = true;
                run.jq_args.push(arg);
            }
            "--color-output" => {
                run.color = Some(true);
                run.jq_args.push(arg);
            }
            "--monochrome-output" => {
                run.color = Some(false);
                run.jq_args.push(arg);
            }
            long if long.starts_with("--") => run.jq_args.push(arg),
            _ => {
                // A cluster of short options, such as `-nr` or `-nL dir`.
                let cluster = arg.clone();
                run.jq_args.push(arg);
                for (i, c) in cluster.char_indices().skip(1) {
                    match c {
                        'n' => run.null_input = true,
                        'R' => run.raw_input = true,
                        'C' => run.color = Some(true),
                        'M' => run.color = Some(false),
                        'f' => from_file = true,
                        'L' => {
                            // `-L dir`, or `-Ldir` with the directory attached.
                            if i + 1 == cluster.len() {
                                run.jq_args.extend(args.next());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if from_file {
        run.filter = None;
    }
    Ok(Command::Run(run))
}

/// Moves `option` and up to `count` values into `jq_args`. When values are
/// missing, jq reports it.
fn take(
    args: &mut impl Iterator<Item = String>,
    jq_args: &mut Vec<String>,
    option: String,
    count: usize,
) {
    jq_args.push(option);
    jq_args.extend(args.take(count));
}

fn parse_fmt(args: Vec<String>) -> Result<Command> {
    let mut file = None;
    let mut in_place = false;
    let mut color = None;
    for arg in args {
        match arg.as_str() {
            "--in-place" => in_place = true,
            "-C" | "--color-output" => color = Some(true),
            "-M" | "--monochrome-output" => color = Some(false),
            option if option.starts_with('-') && option != "-" => {
                bail!("fmt doesn't take {option}")
            }
            _ if file.is_none() => file = Some(arg),
            _ => bail!("fmt takes one file, got another: {arg}"),
        }
    }
    Ok(Command::Fmt {
        file,
        in_place,
        color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Run {
        match parse(args.iter().map(|a| a.to_string()).collect()).unwrap() {
            Command::Run(run) => run,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn test_filter_and_files() {
        let r = run(&["-c", ".a", "x.jsonc", "y.jsonc"]);
        assert_eq!(r.jq_args, strings(&["-c", ".a"]));
        assert_eq!(r.filter.as_deref(), Some(".a"));
        assert_eq!(r.files, strings(&["x.jsonc", "y.jsonc"]));
    }

    #[test]
    fn test_options_after_files_still_go_to_jq() {
        let r = run(&[".a", "x.jsonc", "-c"]);
        assert_eq!(r.jq_args, strings(&[".a", "-c"]));
        assert_eq!(r.files, strings(&["x.jsonc"]));
    }

    #[test]
    fn test_options_with_values_are_skipped() {
        let r = run(&[
            "--arg",
            "a",
            "1",
            "--argjson",
            "b",
            "2",
            "--rawfile",
            "c",
            "c.txt",
            "--indent",
            "3",
            "--library-path",
            "lib",
            "-L",
            "lib2",
            ".",
            "in.jsonc",
        ]);
        assert_eq!(r.files, strings(&["in.jsonc"]));
        assert_eq!(r.filter.as_deref(), Some("."));
        assert_eq!(r.jq_args.len(), 16);
    }

    #[test]
    fn test_slurpfile_paths_are_recorded() {
        let r = run(&["--slurpfile", "v", "v.jsonc", "$v", "in.jsonc"]);
        assert_eq!(r.slurpfiles, vec![2]);
        assert_eq!(r.jq_args[2], "v.jsonc");
        assert_eq!(r.files, strings(&["in.jsonc"]));
    }

    #[test]
    fn test_from_file_makes_the_first_positional_the_filter_file() {
        let r = run(&["prog.jq", "-f", "in.jsonc"]);
        assert_eq!(r.filter, None);
        assert_eq!(r.jq_args, strings(&["prog.jq", "-f"]));
        assert_eq!(r.files, strings(&["in.jsonc"]));
        assert_eq!(run(&["--from-file", "prog.jq"]).filter, None);
    }

    #[test]
    fn test_short_option_clusters() {
        let r = run(&["-nrC", "1"]);
        assert!(r.null_input);
        assert_eq!(r.color, Some(true));
        assert_eq!(r.jq_args, strings(&["-nrC", "1"]));
        let r = run(&["-nL", "lib", "1"]);
        assert_eq!(r.jq_args, strings(&["-nL", "lib", "1"]));
        assert!(r.files.is_empty());
        let r = run(&["-L/lib", "1", "in.jsonc"]);
        assert_eq!(r.files, strings(&["in.jsonc"]));
        assert!(run(&["-R", "."]).raw_input);
        assert!(run(&["--raw-input", "."]).raw_input);
        assert!(run(&["--null-input", "1"]).null_input);
    }

    #[test]
    fn test_last_color_option_wins() {
        assert_eq!(run(&["-C", "-M", "."]).color, Some(false));
        assert_eq!(
            run(&["--monochrome-output", "--color-output", "."]).color,
            Some(true)
        );
        assert_eq!(run(&["."]).color, None);
    }

    #[test]
    fn test_positional_values_after_args() {
        let r = run(&["-n", "--args", "$ARGS", "a", "b"]);
        assert_eq!(r.jq_args, strings(&["-n", "--args", "$ARGS", "a", "b"]));
        assert!(r.files.is_empty());
        assert_eq!(r.filter.as_deref(), Some("$ARGS"));
    }

    #[test]
    fn test_double_dash_ends_options() {
        let r = run(&["--", "-1", "in.jsonc"]);
        assert_eq!(r.jq_args, strings(&["--", "-1"]));
        assert_eq!(r.filter.as_deref(), Some("-1"));
        assert_eq!(r.files, strings(&["in.jsonc"]));
    }

    #[test]
    fn test_non_option_dash_arguments_are_positionals() {
        let r = run(&["-.a", "x.jsonc"]);
        assert_eq!(r.filter.as_deref(), Some("-.a"));
        assert_eq!(r.files, strings(&["x.jsonc"]));
        assert_eq!(r.jq_args, strings(&["-.a"]));
        let r = run(&["-c", "-1", "x.jsonc"]);
        assert_eq!(r.filter.as_deref(), Some("-1"));
        assert_eq!(r.files, strings(&["x.jsonc"]));
    }

    #[test]
    fn test_slurpfile_without_values_records_nothing() {
        let r = run(&["--slurpfile", "v"]);
        assert!(r.slurpfiles.is_empty());
        assert_eq!(r.jq_args, strings(&["--slurpfile", "v"]));
    }

    #[test]
    fn test_dash_is_a_positional() {
        assert_eq!(run(&[".", "-"]).files, strings(&["-"]));
    }

    #[test]
    fn test_jqc_options() {
        let r = run(&[".a = 1", "--in-place", "x.jsonc"]);
        assert!(r.in_place);
        assert_eq!(r.jq_args, strings(&[".a = 1"]));
        let parse_one = |a: &str| parse(vec![a.to_string()]).unwrap();
        assert_eq!(parse_one("--help"), Command::Help);
        assert_eq!(parse_one("--version"), Command::Version);
        // jq's own short forms stay jq's.
        assert_eq!(run(&["-h"]).jq_args, strings(&["-h"]));
        assert_eq!(run(&["-V"]).jq_args, strings(&["-V"]));
    }

    #[test]
    fn test_unknown_options_go_to_jq() {
        let r = run(&["--some-future-option", "-x", "."]);
        assert_eq!(r.jq_args, strings(&["--some-future-option", "-x", "."]));
    }

    #[test]
    fn test_fmt() {
        let fmt = |args: &[&str]| parse(strings(args)).unwrap();
        assert_eq!(
            fmt(&["fmt", "x.jsonc"]),
            Command::Fmt {
                file: Some("x.jsonc".into()),
                in_place: false,
                color: None
            }
        );
        assert_eq!(
            fmt(&["fmt", "--in-place", "x.jsonc"]),
            Command::Fmt {
                file: Some("x.jsonc".into()),
                in_place: true,
                color: None
            }
        );
        assert_eq!(
            fmt(&["-C", "fmt"]),
            Command::Fmt {
                file: None,
                in_place: false,
                color: Some(true)
            }
        );
        assert_eq!(
            fmt(&["fmt", "-M", "x.jsonc"]),
            Command::Fmt {
                file: Some("x.jsonc".into()),
                in_place: false,
                color: Some(false)
            }
        );
        assert!(parse(strings(&["fmt", "-c"])).is_err());
        assert!(parse(strings(&["fmt", "a", "b"])).is_err());
    }
}
