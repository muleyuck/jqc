use anyhow::{anyhow, Result};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{compile, data, load, val::unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::Val;
use jsonc_parser::ParseOptions;

/// Parse `text` as JSONC and apply `filter_str` as a jq filter.
/// Returns all output values produced by the filter.
pub fn run(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    let input_val = jsonc_parser::parse_to_serde_value::<Val>(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs::<data::JustLut<Val>>()
        .chain(jaq_std::funs::<data::JustLut<Val>>())
        .chain(jaq_json::funs::<data::JustLut<Val>>());

    let program = File { code: filter_str, path: () };
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
fn format_load_errors(errs: &[( File<&str, ()>, load::Error<&str>)]) -> String {
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
                                format!("expected {}, got {:?}", expect.as_str(), truncate(remaining, 10))
                            }
                        })
                        .collect();
                    format!("filter syntax error in {:?}: {}", filter, details.join("; "))
                }
                load::Error::Parse(parse_errs) => {
                    let details: Vec<_> = parse_errs
                        .iter()
                        .map(|(expect, _)| format!("expected {}", expect.as_str()))
                        .collect();
                    format!("filter syntax error in {:?}: {}", filter, details.join("; "))
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
fn format_compile_errors(errs: &[(File<&str, ()>, Vec<compile::Error<&str>>)]) -> String {
    errs.iter()
        .flat_map(|(_, compile_errs)| {
            compile_errs.iter().map(|(name, undefined)| {
                format!("undefined {} {:?}", undefined.as_str(), name)
            })
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
    fn test_invalid_jsonc_input() {
        let err = run_err(".", "{not valid");
        assert!(err.contains("Failed to parse JSONC"), "got: {err}");
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
