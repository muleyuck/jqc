//! JSONC to JSON conversion. jq reads the converted JSON, so numbers keep the
//! text they are written with: jq then sees the literals as written.

use std::ops::Range;

use anyhow::{Result, anyhow};
use jsonc_parser::errors::ParseError;
use jsonc_parser::tokens::Token;
use jsonc_parser::{JsonValue, ParseOptions, Scanner, ScannerOptions};

/// Convert `text` to one JSON text per top-level value, in input order.
/// `name` names the input in errors: a file path, or `<stdin>`.
pub fn convert(text: &str, name: &str) -> Result<Vec<String>> {
    let prefix = convert_prefix(text, name);
    match prefix.error {
        None => Ok(prefix.values),
        Some(e) => Err(e),
    }
}

/// The values before the first error in an input. jq outputs the values it
/// read before a parse error, and only then fails.
pub struct Prefix {
    pub values: Vec<String>,
    pub error: Option<anyhow::Error>,
}

pub fn convert_prefix(text: &str, name: &str) -> Prefix {
    let (ranges, scan_error) = split_values(text);
    let mut values = Vec::new();
    for range in ranges {
        match convert_value(text, range, name) {
            Ok(json) => values.push(json),
            Err(e) => {
                return Prefix {
                    values,
                    error: Some(e),
                };
            }
        }
    }
    Prefix {
        values,
        error: scan_error.map(|e| parse_error(name, text, 0, &e)),
    }
}

fn convert_value(text: &str, range: Range<usize>, name: &str) -> Result<String> {
    let value = jsonc_parser::parse_to_value(&text[range.clone()], &ParseOptions::default())
        .map_err(|e| parse_error(name, text, range.start, &e))?
        .ok_or_else(|| anyhow!("Failed to parse JSONC in {name}: empty value"))?;
    let mut json = String::new();
    write_value(&mut json, value).map_err(|e| anyhow!("Failed to parse JSONC in {name}: {e}"))?;
    Ok(json)
}

/// Byte ranges of the top-level values, and the scanner error that stopped
/// the scan, if any. jsonc-parser parses one value per text, while jq reads
/// any number of them.
fn split_values(text: &str) -> (Vec<Range<usize>>, Option<ParseError>) {
    let mut scanner = Scanner::new(text, &ScannerOptions::default());
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut last_word_like = false;
    loop {
        let token = match scanner.scan() {
            Ok(Some(token)) => token,
            Ok(None) => break,
            Err(e) => {
                // A scalar the error cuts off (`1@`) is part of the bad
                // literal, not a complete value.
                if last_word_like
                    && ranges
                        .last()
                        .is_some_and(|r: &Range<usize>| r.end == scanner.token_start())
                {
                    ranges.pop();
                }
                return (ranges, Some(e));
            }
        };
        match token {
            Token::CommentLine(_) | Token::CommentBlock(_) => {}
            Token::OpenBrace | Token::OpenBracket => {
                if depth == 0 {
                    start = scanner.token_start();
                }
                depth += 1;
            }
            Token::CloseBrace | Token::CloseBracket if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    ranges.push(start..scanner.token_end());
                    last_word_like = false;
                }
            }
            // A scalar, or a stray token such as `,` that the parser reports.
            _ if depth == 0 => {
                let word_like = matches!(
                    token,
                    Token::Number(_) | Token::Word(_) | Token::Boolean(_) | Token::Null
                );
                match ranges.last_mut() {
                    // jq reads letters glued to a number or literal (`1foo`)
                    // as one bad literal, so keep them one range.
                    Some(last)
                        if word_like && last_word_like && last.end == scanner.token_start() =>
                    {
                        last.end = scanner.token_end();
                    }
                    _ => ranges.push(scanner.token_start()..scanner.token_end()),
                }
                last_word_like = word_like;
                continue;
            }
            _ => {}
        }
    }
    if depth > 0 {
        // Unclosed: let the parser report it.
        ranges.push(start..text.len());
    }
    (ranges, None)
}

/// `offset` is where the parsed slice starts in `text`, so the position
/// points into the original input.
fn parse_error(name: &str, text: &str, offset: usize, e: &ParseError) -> anyhow::Error {
    let before = &text[..offset + e.range().start];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    anyhow!(
        "Failed to parse JSONC in {name}: {} on line {line} column {column}",
        e.kind()
    )
}

fn write_value(out: &mut String, value: JsonValue) -> Result<()> {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Boolean(b) => out.push_str(if b { "true" } else { "false" }),
        JsonValue::Number(n) => write_number(out, n)?,
        JsonValue::String(s) => write_string(out, &s),
        JsonValue::Array(a) => {
            out.push('[');
            for (i, v) in a.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, v)?;
            }
            out.push(']');
        }
        JsonValue::Object(o) => {
            out.push('{');
            for (i, (k, v)) in o.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, &k);
                out.push(':');
                write_value(out, v)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// JSONC also allows `+1` and hexadecimal integers, which JSON doesn't.
fn write_number(out: &mut String, raw: &str) -> Result<()> {
    let (sign, unsigned) = match raw.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", raw.trim_start_matches('+')),
    };
    out.push_str(sign);
    match unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        Some(hex) => {
            let n = u64::from_str_radix(hex, 16)
                .map_err(|_| anyhow!("hexadecimal number {raw} is out of range"))?;
            out.push_str(&n.to_string());
        }
        None => out.push_str(unsigned),
    }
    Ok(())
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> String {
        let values = convert(text, "test").unwrap();
        assert_eq!(values.len(), 1, "{values:?}");
        values.into_iter().next().unwrap()
    }

    fn error(text: &str) -> String {
        convert(text, "test").unwrap_err().to_string()
    }

    #[test]
    fn test_convert_drops_comments_and_trailing_commas() {
        let text = "{\n  // line\n  \"a\": 1, /* block */\n  \"b\": [1, 2,],\n}";
        assert_eq!(one(text), r#"{"a":1,"b":[1,2]}"#);
    }

    #[test]
    fn test_convert_accepts_loose_jsonc() {
        assert_eq!(
            one("{a: 'x', b: 0x1F, c: +1, d: -0x10}"),
            r#"{"a":"x","b":31,"c":1,"d":-16}"#
        );
        assert_eq!(one("[1 2]"), "[1,2]");
    }

    #[test]
    fn test_convert_keeps_number_text() {
        assert_eq!(
            one("[1.000, 1e2, 100000000000000000001, -0, 1E+400]"),
            "[1.000,1e2,100000000000000000001,-0,1E+400]"
        );
    }

    #[test]
    fn test_convert_escapes_strings() {
        assert_eq!(one(r#""a\"b\\c\nd\u0001é""#), r#""a\"b\\c\nd\u0001é""#);
        assert_eq!(one(r#"'say "hi"'"#), r#""say \"hi\"""#);
    }

    #[test]
    fn test_convert_merges_duplicate_keys_like_jq() {
        // jq keeps the first position with the last value.
        assert_eq!(one(r#"{"a":1,"b":2,"a":3}"#), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn test_convert_splits_top_level_values() {
        let values = convert("1 {\"a\": 2} // c\n[3]\n\"s\" null", "test").unwrap();
        assert_eq!(values, vec!["1", r#"{"a":2}"#, "[3]", r#""s""#, "null"]);
    }

    #[test]
    fn test_convert_comments_only_is_empty() {
        assert_eq!(
            convert("// nothing\n/* here */", "test").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(convert("", "test").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn test_convert_error_names_input_and_position() {
        let err = error("{\"a\": }");
        assert!(err.starts_with("Failed to parse JSONC in test: "), "{err}");
        assert!(err.contains(" on line 1 column "), "{err}");
    }

    #[test]
    fn test_convert_error_position_is_in_the_original_text() {
        // The same broken value, alone and after another value on line 1.
        let alone = error("{\"b\": }");
        let later = error("1\n{\"b\": }");
        assert_eq!(later, alone.replace("on line 1", "on line 2"));
    }

    #[test]
    fn test_convert_rejects_broken_input() {
        assert!(error("]").contains("on line 1 column 1"));
        assert!(error("{\"a\": 1").starts_with("Failed to parse JSONC in test: "));
        assert!(error("1, 2").starts_with("Failed to parse JSONC in test: "));
    }

    #[test]
    fn test_convert_prefix_keeps_values_before_an_error() {
        let text = "1 2 {\"x\": } 3";
        let p = convert_prefix(text, "test");
        assert_eq!(p.values, vec!["1", "2"]);
        assert!(p.error.unwrap().to_string().contains(" on line 1 column "));
        // A scanner error (unterminated string) after a good value.
        let text = "1\n\"open";
        let p = convert_prefix(text, "test");
        assert_eq!(p.values, vec!["1"]);
        assert!(p.error.unwrap().to_string().contains("on line 2"));
        let p = convert_prefix("1 2", "test");
        assert_eq!(p.values, vec!["1", "2"]);
        assert!(p.error.is_none());
    }

    #[test]
    fn test_convert_prefix_drops_a_scalar_cut_by_a_scanner_error() {
        // `1@` is one bad literal for jq, not `1` followed by an error.
        let p = convert_prefix("2 1@", "test");
        assert_eq!(p.values, vec!["2"]);
        assert!(p.error.is_some());
        assert!(convert_prefix("true@", "test").values.is_empty());
        // The error points inside the failing token (`-`), not at its start.
        assert!(convert_prefix("1-", "test").values.is_empty());
    }

    #[test]
    fn test_convert_rejects_letters_glued_to_a_scalar() {
        // jq reads `1foo` and `truefalse` as one bad literal, but `1"a"` as two values.
        assert!(convert_prefix("1foo", "test").values.is_empty());
        assert!(convert("1foo", "test").is_err());
        assert!(convert("truefalse", "test").is_err());
        assert_eq!(convert("1\"a\"", "test").unwrap(), vec!["1", "\"a\""]);
        assert_eq!(convert("[1]2", "test").unwrap(), vec!["[1]", "2"]);
    }

    #[test]
    fn test_convert_rejects_hex_beyond_u64() {
        let err = error("0x10000000000000000");
        assert!(err.contains("out of range"), "{err}");
    }
}
