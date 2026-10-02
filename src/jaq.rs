use std::borrow::Cow;

use anyhow::{Result, anyhow, bail};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, compile, data, load, val::unwrap_valr};
use jaq_json::read::parse_single_num;
use jaq_json::write::{Pp, write};
use jaq_json::{Map, Num, Rc, Val};
use jsonc_parser::{JsonValue, ParseOptions};

type CompileErrors<'a> = Vec<(File<&'a str, ()>, Vec<compile::Error<&'a str>>)>;

/// Parse `text` as JSONC and apply `filter_str` as a jq filter.
/// Returns all output values produced by the filter.
pub fn run(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    run_with_input(filter_str, parse(text)?)
}

/// Parse `text` as JSONC into a jaq value. Numbers are read the way jaq
/// reads them, so integers keep every digit and decimals keep their text.
pub fn parse(text: &str) -> Result<Val> {
    let value = jsonc_parser::parse_to_value(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;
    value.map_or(Ok(Val::Null), from_jsonc)
}

fn from_jsonc(v: JsonValue) -> Result<Val> {
    Ok(match v {
        JsonValue::Null => Val::Null,
        JsonValue::Boolean(b) => Val::Bool(b),
        JsonValue::Number(n) => Val::Num(parse_number(n)?),
        JsonValue::String(s) => Val::from(s.into_owned()),
        JsonValue::Array(a) => a.into_iter().map(from_jsonc).collect::<Result<_>>()?,
        JsonValue::Object(o) => Val::Obj(Rc::new(
            o.into_iter()
                .map(|(k, v)| Ok((Val::from(k.into_owned()), from_jsonc(v)?)))
                .collect::<Result<Map>>()?,
        )),
    })
}

/// Read a JSONC number literal. JSONC also allows a leading `+` and
/// hexadecimal integers, which jaq's reader doesn't.
fn parse_number(raw: &str) -> Result<Num> {
    let invalid = || anyhow!("Failed to parse JSONC: invalid number {raw:?}");
    let unsigned = raw.trim_start_matches(['+', '-']);
    let negative = raw.starts_with('-');
    if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        let n = Num::from_str_radix(hex, 16).ok_or_else(invalid)?;
        return Ok(if negative { -n } else { n });
    }
    let literal = raw.trim_start_matches('+');
    match parse_single_num(literal.as_bytes()).ok_or_else(invalid)? {
        Num::Dec(d) => Ok(match canonical_decimal(&d) {
            Cow::Borrowed(_) => Num::Dec(d),
            Cow::Owned(c) => Num::Dec(Rc::new(c)),
        }),
        n => Ok(n),
    }
}

/// Write a decimal literal the way jq prints one it read: in decNumber's
/// scientific notation, which keeps the digits as written (`1.000`) but
/// moves exponents into `E+n` form (`1e2` becomes `1E+2`).
fn canonical_decimal(lit: &str) -> Cow<'_, str> {
    if is_canonical_plain(lit) {
        return Cow::Borrowed(lit);
    }
    let (sign, unsigned) = match lit.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", lit.trim_start_matches('+')),
    };
    let (mantissa, exp) = match unsigned.split_once(['e', 'E']) {
        Some((m, e)) => match e.trim_start_matches('+').parse::<i64>() {
            Ok(e) => (m, e),
            // Leave an exponent beyond i64 as written rather than misread it.
            Err(_) => return Cow::Borrowed(lit),
        },
        None => (unsigned, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{int}{frac}");
    let digits = match digits.trim_start_matches('0') {
        "" => "0",
        d => d,
    };
    let len = digits.len() as i64;
    let Some((exp, adjusted)) = exp
        .checked_sub(frac.len() as i64)
        .and_then(|exp| Some((exp, exp.checked_add(len - 1)?)))
    else {
        return Cow::Borrowed(lit);
    };
    let body = if exp <= 0 && adjusted >= -6 {
        if exp == 0 {
            digits.to_string()
        } else if len > -exp {
            let (i, f) = digits.split_at((len + exp) as usize);
            format!("{i}.{f}")
        } else {
            format!("0.{}{digits}", "0".repeat((-exp - len) as usize))
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if adjusted < 0 { '-' } else { '+' };
        format!("{first}{rest}E{exp_sign}{}", adjusted.unsigned_abs())
    };
    Cow::Owned(format!("{sign}{body}"))
}

/// A plain decimal such as `1.5` is already canonical unless it has more
/// than 6 places after the point (`0.0000001` becomes `1E-7`). Checking
/// this first avoids rewriting every decimal in a large input.
fn is_canonical_plain(lit: &str) -> bool {
    if lit.contains(['e', 'E', '+']) {
        return false;
    }
    let unsigned = lit.trim_start_matches('-');
    match unsigned.split_once('.') {
        Some((int, frac)) => int != "0" || frac.len() <= 6,
        None => true,
    }
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
        .map(|r| {
            let v = unwrap_valr(r).map_err(|e| anyhow!("runtime error: {e}"))?;
            Ok(if needs_json_numbers(&v)? {
                to_json_numbers(v)
            } else {
                v
            })
        })
        .collect()
}

/// Rewrite computed numbers the way jq prints them. jaq keeps them as
/// floats and prints them as `1.0`, `1e20`, `NaN` or `Infinity`; jq prints
/// `1`, `1e+20`, `null` and the largest finite double instead.
fn to_json_numbers(v: Val) -> Val {
    match v {
        Val::Num(Num::Dec(d)) => Val::Num(Num::Dec(Rc::new(canonical_decimal(&d).into_owned()))),
        Val::Num(Num::Float(f)) if f.is_nan() => Val::Null,
        Val::Num(Num::Float(f)) => {
            let f = if f.is_infinite() {
                f64::MAX.copysign(f)
            } else {
                f
            };
            Val::Num(Num::Dec(Rc::new(format_float(f))))
        }
        Val::Arr(a) => a.iter().cloned().map(to_json_numbers).collect(),
        Val::Obj(o) => Val::Obj(Rc::new(
            o.iter()
                .map(|(k, v)| (k.clone(), to_json_numbers(v.clone())))
                .collect(),
        )),
        v => v,
    }
}

/// Whether `v` holds numbers `to_json_numbers` rewrites. Decimals read from
/// the input are already canonical; only those written in the filter (`1e2`)
/// and floats need rewriting. Also rejects what JSON can't represent: jaq
/// builds objects with non-string keys, which jq refuses.
fn needs_json_numbers(v: &Val) -> Result<bool> {
    Ok(match v {
        Val::Num(Num::Float(_)) => true,
        Val::Num(Num::Dec(d)) => matches!(canonical_decimal(d), Cow::Owned(ref c) if c != &**d),
        Val::Arr(a) => a.iter().try_fold(false, |any, v| -> Result<bool> {
            Ok(needs_json_numbers(v)? || any)
        })?,
        Val::Obj(o) => o.iter().try_fold(false, |any, (k, v)| -> Result<bool> {
            if !matches!(k, Val::TStr(_) | Val::BStr(_)) {
                bail!("cannot use {k} as object key: object keys must be strings");
            }
            Ok(needs_json_numbers(v)? || any)
        })?,
        _ => false,
    })
}

/// Format a finite float like jq: the shortest digits that round-trip,
/// written in exponent form when the decimal point would sit more than 3
/// places before the first digit or more than 15 places after the last.
fn format_float(f: f64) -> String {
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let exp: i32 = exp.parse().expect("{:e} exponent is an integer");
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits = mantissa.replace('.', "");
    if digits == "0" {
        return format!("{sign}0");
    }
    let n = digits.len() as i32;
    let point = exp + 1;
    let body = if point <= -4 || point > n + 15 {
        let (first, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{first}{frac}e{exp_sign}{:02}", exp.abs())
    } else if point <= 0 {
        format!("0.{}{digits}", "0".repeat(-point as usize))
    } else if point >= n {
        format!("{digits}{}", "0".repeat((point - n) as usize))
    } else {
        let (int, frac) = digits.split_at(point as usize);
        format!("{int}.{frac}")
    };
    format!("{sign}{body}")
}

/// Pretty-print `v` the way jq does without `-c`.
pub fn to_pretty_json(v: &Val) -> String {
    let pp = Pp {
        indent: Some("  ".to_string()),
        sep_space: true,
        ..Pp::default()
    };
    let mut out = Vec::new();
    write(&mut out, &pp, 0, v).expect("writing to a Vec never fails");
    String::from_utf8_lossy(&out).into_owned()
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
    fn test_run_null_non_finite_numbers_become_json() {
        let result = run_null("[nan, infinite, -infinite, {a: nan}]").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].to_string(),
            r#"[null,1.7976931348623157e+308,-1.7976931348623157e+308,{"a":null}]"#
        );
    }

    #[test]
    fn test_canonical_decimal_matches_jq() {
        // Expected strings are jq 1.8.2's output for the same input literal.
        let cases = [
            ("1.000", "1.000"),
            ("1.10", "1.10"),
            ("0.0", "0.0"),
            ("-0.0", "-0.0"),
            ("100e-2", "1.00"),
            ("0e10", "0E+10"),
            ("-0e5", "-0E+5"),
            ("1e0", "1"),
            ("1E+0", "1"),
            ("1e2", "1E+2"),
            ("10e1", "1.0E+2"),
            ("1.0e2", "1.0E+2"),
            ("1.5E3", "1.5E+3"),
            ("0.001e3", "1"),
            ("0.000001", "0.000001"),
            ("0.0000001", "1E-7"),
            ("1.2e-6", "0.0000012"),
            ("12e-8", "1.2E-7"),
            ("-1.5e-3", "-0.0015"),
            ("1.23e-10", "1.23E-10"),
            ("1e1000", "1E+1000"),
            ("1e-1000", "1E-1000"),
            (
                "123456789012345678901234567890.123456789",
                "123456789012345678901234567890.123456789",
            ),
        ];
        for (lit, want) in cases {
            assert_eq!(canonical_decimal(lit), want, "canonical_decimal({lit:?})");
        }
    }

    #[test]
    fn test_canonical_decimal_keeps_out_of_range_exponents() {
        for lit in ["1e-9999999999999999999", "1.0e-9223372036854775808"] {
            assert_eq!(canonical_decimal(lit), lit);
        }
    }

    #[test]
    fn test_run_null_rejects_non_string_object_keys() {
        let err = run_null("{(1): 2}").unwrap_err().to_string();
        assert!(err.contains("object keys must be strings"), "got: {err}");
    }

    #[test]
    fn test_parse_keeps_number_literals() {
        let v = parse("[100000000000000000001, 1e1000, -0, +1, 0x1F, -0x10]").unwrap();
        assert_eq!(v.to_string(), "[100000000000000000001,1E+1000,0,1,31,-16]");
    }

    #[test]
    fn test_format_float_matches_jq() {
        // Expected strings are jq 1.8.2's output for the same computed value.
        let cases = [
            (1.0, "1"),
            (-0.0, "-0"),
            (0.1, "0.1"),
            (1.0 / 3.0, "0.3333333333333333"),
            (0.0001, "0.0001"),
            (0.00012345, "0.00012345"),
            (0.00001, "1e-05"),
            (0.000012345, "1.2345e-05"),
            (2.5e-8, "2.5e-08"),
            (5e-324, "5e-324"),
            (1e15, "1000000000000000"),
            (1e16, "1e+16"),
            (1.5e16, "15000000000000000"),
            (1.2345678901234568e17, "123456789012345680"),
            (1.2345678901234568e21, "1234567890123456800000"),
            (1e20, "1e+20"),
            (-1e20, "-1e+20"),
            (1.5e21, "1.5e+21"),
            (1e100, "1e+100"),
            (f64::MAX, "1.7976931348623157e+308"),
        ];
        for (f, want) in cases {
            assert_eq!(format_float(f), want, "format_float({f:e})");
        }
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
