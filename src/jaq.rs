use anyhow::{Result, anyhow};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, compile, data, load, val::unwrap_valr};
use jaq_json::Val;
use jsonc_parser::ParseOptions;

type CompileErrors<'a> = Vec<(File<&'a str, ()>, Vec<compile::Error<&'a str>>)>;

/// Parse `text` as JSONC and apply `filter_str` as a jq filter.
/// Returns all output values produced by the filter.
pub fn run(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    let input_val = jsonc_parser::parse_to_serde_value::<Val>(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;
    run_with_input(filter_str, input_val)
}

/// Apply `filter_str` as a jq filter against `null` as the input value,
/// without reading or parsing any input text (jq `-n` / `--null-input` behavior).
pub fn run_null(filter_str: &str) -> Result<Vec<Val>> {
    run_with_input(filter_str, Val::Null)
}

/// Compile and run `filter_str` against a pre-built `input_val`.
fn run_with_input(filter_str: &str, input_val: Val) -> Result<Vec<Val>> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs::<data::JustLut<Val>>()
        .chain(jaq_std::funs::<data::JustLut<Val>>())
        .chain(jaq_json::funs::<data::JustLut<Val>>());

    let program = File {
        code: filter_str,
        path: (),
    };
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program)
        .map_err(|errs| anyhow!("{}", format_load_errors(&errs)))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errs| anyhow!("{}", format_compile_errors(&errs)))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input_val))
        .map(|r| unwrap_valr(r).map_err(|e| anyhow!("runtime error: {e}")))
        .collect()
}

/// Format jaq-core load errors (lex / parse / io) into a user-readable string.
fn format_load_errors(errs: &[(File<&str, ()>, load::Error<&str>)]) -> String {
    errs.iter()
        .map(|(file, err)| {
            let filter = file.code;
            match err {
                load::Error::Lex(lex_errs) => {
                    let details: Vec<_> = lex_errs
                        .iter()
                        .map(|(expect, remaining)| {
                            if remaining.is_empty() {
                                format!("expected {}, got end of input", expect.as_str())
                            } else {
                                format!(
                                    "expected {}, got {:?}",
                                    expect.as_str(),
                                    truncate(remaining, 10)
                                )
                            }
                        })
                        .collect();
                    format!(
                        "filter syntax error in {:?}: {}",
                        filter,
                        details.join("; ")
                    )
                }
                load::Error::Parse(parse_errs) => {
                    let details: Vec<_> = parse_errs
                        .iter()
                        .map(|(expect, _)| format!("expected {}", expect.as_str()))
                        .collect();
                    format!(
                        "filter syntax error in {:?}: {}",
                        filter,
                        details.join("; ")
                    )
                }
                load::Error::Io(io_errs) => {
                    let details: Vec<_> = io_errs.iter().map(|(_, msg)| msg.as_str()).collect();
                    format!("filter load error: {}", details.join("; "))
                }
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Format jaq-core compile errors (undefined variables / filters) into a user-readable string.
fn format_compile_errors(errs: &CompileErrors<'_>) -> String {
    errs.iter()
        .flat_map(|(_, compile_errs)| {
            compile_errs
                .iter()
                .map(|(name, undefined)| format!("undefined {} {:?}", undefined.as_str(), name))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_err(filter: &str, input: &str) -> String {
        run(filter, input).unwrap_err().to_string()
    }

    #[test]
    fn test_run_null_identity() {
        let result = run_null(".").unwrap();
        assert_eq!(result, vec![Val::Null]);
    }

    #[test]
    fn test_run_null_constructs_object() {
        let result = run_null("{a: 1, b: 2}").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].to_string(), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn test_run_null_range_produces_array() {
        let result = run_null("[range(3)]").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].to_string(), "[0,1,2]");
    }

    #[test]
    fn test_run_null_syntax_error_still_reported() {
        let err = run_null(".foo[").unwrap_err().to_string();
        assert!(err.contains("filter syntax error"), "got: {err}");
    }

    #[test]
    fn test_invalid_jsonc_input() {
        let err = run_err(".", "{not valid");
        assert!(err.contains("Failed to parse JSONC"), "got: {err}");
    }

    #[test]
    fn test_lex_error_truncates_long_remaining() {
        // 50 invalid chars after "[": truncate(remaining, 10) must cut each "got" value to ≤ 10 chars
        let filter = format!(".foo[{}]", "^".repeat(50));
        let err = run_err(&filter, "{}");
        assert!(err.contains("filter syntax error"), "got: {err}");
        // Each `got "..."` section must contain ≤ 10 chars between the quotes
        let got_sections: Vec<_> = err.split(r#"got ""#).skip(1).collect();
        assert!(
            !got_sections.is_empty(),
            "no 'got \"...' section found in error: {err}"
        );
        for part in got_sections {
            let got_content = part.split('"').next().unwrap_or("");
            assert!(
                got_content.len() <= 10,
                "remaining was not truncated (len={}): {err}",
                got_content.len()
            );
        }
    }

    #[test]
    fn test_lex_error_unclosed_bracket() {
        let err = run_err(".foo[", "{}");
        assert!(err.contains("filter syntax error"), "got: {err}");
        assert!(err.contains("got end of input"), "got: {err}");
        assert!(err.contains(".foo["), "got: {err}");
    }

    #[test]
    fn test_compile_error_undefined_variable() {
        let err = run_err("$x", "{}");
        assert!(err.contains("undefined"), "got: {err}");
        assert!(err.contains("$x"), "got: {err}");
    }

    #[test]
    fn test_runtime_error_null() {
        let err = run_err("null | error", "{}");
        assert!(err.contains("runtime error"), "got: {err}");
        assert!(err.contains("null"), "got: {err}");
    }

    #[test]
    fn test_runtime_error_type_mismatch() {
        let err = run_err(".foo + 1", r#"{"foo": "bar"}"#);
        assert!(err.contains("runtime error"), "got: {err}");
    }
}
