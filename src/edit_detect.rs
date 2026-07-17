use anyhow::{Result, anyhow, bail};
use jaq_core::load;
use jaq_core::load::lex::{Lexer, Tok};
use jaq_core::load::parse::{BinaryOp, Term};

const ASSIGN_OPS: [&str; 8] = ["=", "|=", "+=", "-=", "*=", "/=", "%=", "//="];

/// A filter expression recognized as a comment-preserving edit operation.
#[derive(Debug)]
pub enum EditForm<'s> {
    /// `=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, or `//=`.
    /// The application logic is identical regardless of which operator was used
    /// (see `edit::apply_assign`), so only the LHS (target path expression) is kept.
    Assign { lhs: &'s str },
    /// `del(PATH)` with exactly one argument.
    Del { path: &'s str },
}

/// Determine whether `filter_str` is a top-level jq edit expression.
///
/// Returns `Ok(None)` if `filter_str` fails to parse or does not match a
/// recognized edit form (the caller should fall back to normal read-only
/// filter evaluation, which will surface any syntax error itself).
///
/// Returns `Err` if `filter_str` is unambiguously an edit call that this
/// tool does not support (e.g. `del(.a, .b)`), so it is never silently
/// treated as a read-only filter.
pub fn detect(filter_str: &str) -> Result<Option<EditForm<'_>>> {
    let Some(term) = load::parse::<Term<&str>, _>(filter_str, |p| p.term()) else {
        return Ok(None);
    };

    match &term {
        Term::BinOp(_, op, _) if is_assign_op(op) => {
            let lhs = split_at_top_level_op(filter_str).ok_or_else(|| {
                anyhow!("internal error: expected an assignment operator in {filter_str:?}")
            })?;
            Ok(Some(EditForm::Assign { lhs }))
        }
        Term::Call(name, args) if *name == "del" => {
            // jq separates *call arguments* with `;` (e.g. `limit(2; .[])`), so
            // `del(.a; .b)` parses as two args (`args.len() == 2`). `del(.a, .b)`
            // instead parses as a *single* argument that is itself a top-level
            // `,` (Comma) expression, since `,` is allowed inside a single
            // argument term. Both forms mean "multiple paths" and are rejected:
            // the resolved-path multi-match machinery (Task 2/4) already covers
            // the practical need via wildcard path expressions like `.items[]`.
            let is_multi_path =
                args.len() != 1 || matches!(&args[0], Term::BinOp(_, BinaryOp::Comma, _));
            if is_multi_path {
                bail!(
                    "del() with multiple path arguments is not supported; use a single path expression, e.g. del(.a)"
                );
            }
            let path = extract_del_arg(filter_str).ok_or_else(|| {
                anyhow!("internal error: expected a del(...) call in {filter_str:?}")
            })?;
            Ok(Some(EditForm::Del { path }))
        }
        _ => Ok(None),
    }
}

fn is_assign_op<S>(op: &BinaryOp<S>) -> bool {
    matches!(
        op,
        BinaryOp::Assign | BinaryOp::Update | BinaryOp::UpdateMath(_) | BinaryOp::UpdateAlt
    )
}

/// Find the first top-level assignment-family operator token in `filter_str`
/// and return everything before it (trimmed) as the LHS text.
///
/// This scans the *lexer's* flat token stream rather than the parsed `Term`
/// tree: punctuation-only nodes like `Term::Id` (`.`) carry no source token,
/// and field-access keys store `c[1..]` (the token with its leading `.`
/// stripped), so reconstructing a span from `Term` leaves alone loses the
/// leading `.`/`[`. jaq's operator precedence table guarantees that all 8
/// assignment-family operators share the same precedence and are
/// right-associative, so the first top-level occurrence is always the split
/// point that produced the `Term::BinOp` root `detect` already matched.
fn split_at_top_level_op(filter_str: &str) -> Option<&str> {
    let tokens = Lexer::new(filter_str).lex().ok()?;
    let op_start = tokens.iter().find_map(|t| {
        let Tok::Sym = t.1 else { return None };
        ASSIGN_OPS.contains(&t.0).then_some(t.0)
    })?;
    let start = load::span(filter_str, op_start).start;
    Some(filter_str[..start].trim())
}

/// Find a `del` word token immediately followed by a `(...)` block token,
/// and return the block's contents with the surrounding parens stripped.
fn extract_del_arg(filter_str: &str) -> Option<&str> {
    let tokens = Lexer::new(filter_str).lex().ok()?;
    for i in 0..tokens.len().saturating_sub(1) {
        let is_del_word = matches!(tokens[i].1, Tok::Word) && tokens[i].0 == "del";
        if !is_del_word {
            continue;
        }
        if let Tok::Block(_) = tokens[i + 1].1 {
            let full = tokens[i + 1].0;
            if full.starts_with('(') {
                return Some(full[1..full.len() - 1].trim());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_assign() {
        let form = detect(".a = 1").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: ".a" }));
    }

    #[test]
    fn test_detect_update() {
        let form = detect(".a |= . + 1").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: ".a" }));
    }

    #[test]
    fn test_detect_update_math() {
        let form = detect(".count += 1").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: ".count" }));
    }

    #[test]
    fn test_detect_update_alt() {
        let form = detect(".a //= 1").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: ".a" }));
    }

    #[test]
    fn test_detect_nested_lhs() {
        let form = detect(".server.port = 8080").unwrap().unwrap();
        assert!(matches!(
            form,
            EditForm::Assign {
                lhs: ".server.port"
            }
        ));
    }

    #[test]
    fn test_detect_bracket_lhs() {
        let form = detect(".items[0] = 1").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: ".items[0]" }));
    }

    #[test]
    fn test_detect_whole_document_lhs() {
        // `.` alone as LHS has no leaf token in the parsed AST; the
        // token-stream-based span extraction must still handle it.
        let form = detect(". = {}").unwrap().unwrap();
        assert!(matches!(form, EditForm::Assign { lhs: "." }));
    }

    #[test]
    fn test_detect_del() {
        let form = detect("del(.a)").unwrap().unwrap();
        assert!(matches!(form, EditForm::Del { path: ".a" }));
    }

    #[test]
    fn test_detect_del_nested() {
        let form = detect("del(.server.port)").unwrap().unwrap();
        assert!(matches!(
            form,
            EditForm::Del {
                path: ".server.port"
            }
        ));
    }

    #[test]
    fn test_detect_del_multiple_args_errors() {
        let err = detect("del(.a, .b)").unwrap_err().to_string();
        assert!(err.contains("del"), "error should mention del: {err}");
    }

    #[test]
    fn test_detect_fallback_plain_filter() {
        assert!(detect(".a").unwrap().is_none());
        assert!(detect(".a | .b").unwrap().is_none());
        assert!(detect(".a == 1").unwrap().is_none());
        assert!(detect("map(.+1)").unwrap().is_none());
    }

    #[test]
    fn test_detect_fallback_syntax_error() {
        // Malformed filters fall through to the normal filter pipeline,
        // which reports the syntax error itself.
        assert!(detect(".a[").unwrap().is_none());
    }

    #[test]
    fn test_detect_fallback_chained_assign() {
        // Piped compound edits are out of scope (single edit per invocation).
        assert!(detect(".a = 1 | .b = 2").unwrap().is_none());
    }
}
