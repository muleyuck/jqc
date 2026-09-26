//
// jq compatibility: jqc against jq's recorded output (tests/fixtures/jq-compat)
//

use assert_cmd::Command;
use jsonc_parser::tokens::Token;
use jsonc_parser::{JsonValue, ParseOptions, Scanner, ScannerOptions, parse_to_value};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

const REGENERATE_HINT: &str =
    "run scripts/jq-compat.sh (needs jq), or take expected.json from the jq-compat CI artifact";

struct Case {
    name: String,
    args: Vec<String>,
    stdin: String,
    files: Vec<(String, String)>,
    compare_value: bool,
    known_difference: Option<u64>,
}

struct Outcome {
    stdout: String,
    status: i32,
}

fn read_fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/jq-compat")
        .join(name);
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

fn parse_case(v: &Value) -> Case {
    let name = v["name"]
        .as_str()
        .expect("cases.json: every case needs a string \"name\"")
        .to_string();
    let args = v["args"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .map(|x| x.as_str().map(String::from))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| panic!("case {name:?}: \"args\" must be a non-empty array of strings"));
    let stdin = match &v["stdin"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => panic!("case {name:?}: \"stdin\" must be a string"),
    };
    let files = match &v["files"] {
        Value::Null => Vec::new(),
        Value::Object(map) => map
            .iter()
            .map(|(file, content)| {
                let content = content
                    .as_str()
                    .unwrap_or_else(|| panic!("case {name:?}: file {file:?} must be a string"));
                (file.clone(), content.to_string())
            })
            .collect(),
        _ => panic!("case {name:?}: \"files\" must be an object"),
    };
    let compare_value = match &v["compare"] {
        Value::Null => false,
        Value::String(s) if s == "text" => false,
        Value::String(s) if s == "value" => true,
        _ => panic!("case {name:?}: \"compare\" must be \"text\" or \"value\""),
    };
    let known_difference = match &v["known_difference"] {
        Value::Null => None,
        n => Some(n.as_u64().unwrap_or_else(|| {
            panic!("case {name:?}: \"known_difference\" must be an issue number")
        })),
    };
    Case {
        name,
        args,
        stdin,
        files,
        compare_value,
        known_difference,
    }
}

fn recorded_outcome(name: &str, entry: &Value) -> Outcome {
    let stdout = entry["stdout"]
        .as_str()
        .unwrap_or_else(|| panic!("expected.json: {name:?} needs a string \"stdout\""))
        .to_string();
    let status = entry["status"]
        .as_i64()
        .and_then(|s| i32::try_from(s).ok())
        .unwrap_or_else(|| panic!("expected.json: {name:?} needs an integer \"status\""));
    Outcome { stdout, status }
}

fn run_jqc(case: &Case) -> Outcome {
    let dir = tempfile::tempdir().expect("cannot create a temporary directory");
    for (file, content) in &case.files {
        fs::write(dir.path().join(file), content).expect("cannot write a case file");
    }
    let output = Command::cargo_bin("jqc")
        .unwrap()
        .args(&case.args)
        .current_dir(dir.path())
        .write_stdin(case.stdin.as_str())
        .output()
        .expect("cannot run jqc");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

fn same_result(case: &Case, jq: &Outcome, jqc: &Outcome) -> bool {
    if jq.status != jqc.status {
        return false;
    }
    if !case.compare_value {
        return jq.stdout == jqc.stdout;
    }
    matches!((json_values(&jq.stdout), json_values(&jqc.stdout)), (Some(a), Some(b)) if a == b)
}

// jq reads only strict JSON: no comments, trailing commas or other JSONC extensions.
const STRICT_JSON: ParseOptions = ParseOptions {
    allow_comments: false,
    allow_loose_object_property_names: false,
    allow_trailing_commas: false,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
};
const STRICT_SCANNER: ScannerOptions = ScannerOptions {
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
};

#[derive(PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number {
        negative: bool,
        digits: String,
        exponent: i128,
    },
    String(String),
    Array(Vec<Json>),
    Object(HashMap<String, Json>),
}

// Splits stdout into top-level values with jsonc-parser's scanner, which keeps numbers as
// text, so values beyond the f64 range or precision survive. A duplicate key keeps its last
// value, which is how jq reads the duplicates jqc deliberately leaves in place.
fn json_values(stdout: &str) -> Option<Vec<Json>> {
    let mut scanner = Scanner::new(stdout, &STRICT_SCANNER);
    let mut values = Vec::new();
    let (mut start, mut depth) = (0, 0usize);
    while let Some(token) = scanner.scan().ok()? {
        // Without whitespace, `01` or `1true` would split into two valid values.
        if depth == 0 && !values.is_empty() && start == scanner.token_start() {
            return None;
        }
        match token {
            Token::OpenBrace | Token::OpenBracket => depth += 1,
            Token::CloseBrace | Token::CloseBracket => depth = depth.checked_sub(1)?,
            _ => {}
        }
        if depth == 0 {
            let end = scanner.token_end();
            let value = parse_to_value(&stdout[start..end], &STRICT_JSON).ok()??;
            values.push(to_json(value)?);
            start = end;
        }
    }
    (depth == 0).then_some(values)
}

fn to_json(value: JsonValue<'_>) -> Option<Json> {
    Some(match value {
        JsonValue::Null => Json::Null,
        JsonValue::Boolean(b) => Json::Bool(b),
        JsonValue::Number(n) => decimal(n)?,
        JsonValue::String(s) => Json::String(s.into_owned()),
        JsonValue::Array(a) => Json::Array(a.into_iter().map(to_json).collect::<Option<_>>()?),
        JsonValue::Object(o) => Json::Object(
            o.into_iter()
                .map(|(k, v)| Some((k.into_owned(), to_json(v)?)))
                .collect::<Option<_>>()?,
        ),
    })
}

// The exact decimal value, which is what jq's `==` compares: 1e2 == 100 and 1.50 == 1.5,
// with no rounding to f64.
fn decimal(literal: &str) -> Option<Json> {
    let (negative, unsigned) = match literal.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, literal),
    };
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(i) => (&unsigned[..i], &unsigned[i + 1..]),
        None => (unsigned, "0"),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let all = format!("{int}{frac}");
    let significant = all.trim_end_matches('0');
    let digits = significant.trim_start_matches('0');
    // Zero is zero whatever its exponent, so it must not depend on the exponent fitting.
    if digits.is_empty() {
        return Some(Json::Number {
            negative: false,
            digits: String::new(),
            exponent: 0,
        });
    }
    let exponent = exponent
        .parse::<i128>()
        .ok()?
        .checked_sub(frac.len() as i128)?
        .checked_add((all.len() - significant.len()) as i128)?;
    Some(Json::Number {
        negative,
        digits: digits.to_string(),
        exponent,
    })
}

fn describe_difference(case: &Case, jq: &Outcome, jqc: &Outcome) -> String {
    format!(
        "{:?} differs from jq\n  args: {:?}\n  jq:   status {}, stdout {:?}\n  jqc:  status {}, stdout {:?}",
        case.name, case.args, jq.status, jq.stdout, jqc.status, jqc.stdout
    )
}

#[test]
fn cases_match_jq() {
    let cases: Vec<Case> = read_fixture("cases.json")["cases"]
        .as_array()
        .expect("cases.json: \"cases\" must be an array")
        .iter()
        .map(parse_case)
        .collect();
    let expected_file = read_fixture("expected.json");
    let expected = expected_file["cases"]
        .as_object()
        .expect("expected.json: \"cases\" must be an object");

    let mut failures = Vec::new();
    let mut names = HashSet::new();
    for case in &cases {
        if !names.insert(case.name.as_str()) {
            failures.push(format!("duplicate case name {:?} in cases.json", case.name));
        }
    }
    for name in expected.keys() {
        if !names.contains(name.as_str()) {
            failures.push(format!(
                "expected.json still has removed case {name:?}; {REGENERATE_HINT}"
            ));
        }
    }

    for case in &cases {
        let Some(entry) = expected.get(&case.name) else {
            failures.push(format!(
                "{:?} is missing from expected.json; {REGENERATE_HINT}",
                case.name
            ));
            continue;
        };
        let jq = recorded_outcome(&case.name, entry);
        let jqc = run_jqc(case);
        match (case.known_difference, same_result(case, &jq, &jqc)) {
            (None, false) => failures.push(describe_difference(case, &jq, &jqc)),
            (Some(issue), true) => failures.push(format!(
                "{:?} now matches jq, so #{issue} looks fixed: remove its known_difference",
                case.name
            )),
            _ => {}
        }
    }

    assert!(
        failures.is_empty(),
        "{} jq compatibility failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

fn value_case() -> Case {
    Case {
        name: "value".into(),
        args: vec![".a = null".into()],
        stdin: String::new(),
        files: Vec::new(),
        compare_value: true,
        known_difference: None,
    }
}

fn success(stdout: &str) -> Outcome {
    Outcome {
        stdout: stdout.into(),
        status: 0,
    }
}

#[test]
#[should_panic(expected = "\"args\" must be a non-empty array of strings")]
fn case_with_empty_args_is_rejected() {
    parse_case(&serde_json::json!({ "name": "no-args", "args": [] }));
}

#[test]
fn value_comparison_checks_output_when_both_fail() {
    let jq = Outcome {
        stdout: "{\"a\":2}\n".into(),
        status: 2,
    };
    let jqc = Outcome {
        stdout: String::new(),
        status: 2,
    };
    assert!(!same_result(&value_case(), &jq, &jqc));
}

#[test]
fn value_comparison_accepts_multiple_values() {
    assert!(same_result(
        &value_case(),
        &success("{\"a\":1}\n{\"a\":2}\n"),
        &success("{\"a\": 1}\n{\"a\": 2}\n")
    ));
}

#[test]
fn value_comparison_keeps_integer_precision() {
    assert!(!same_result(
        &value_case(),
        &success("{\"a\":100000000000000000001}\n"),
        &success("{\"a\":1e+20}\n")
    ));
}

#[test]
fn value_comparison_ignores_number_spelling() {
    assert!(same_result(
        &value_case(),
        &success("{\"a\":9,\"b\":1E+20,\"c\":[100,1.50,0.001,0]}\n"),
        &success("{\"a\": 9, \"b\": 1e20, \"c\": [1e2, 1.5, 1e-3, -0]}\n")
    ));
}

#[test]
fn value_comparison_reads_numbers_beyond_f64_range() {
    assert!(same_result(
        &value_case(),
        &success("{\"a\":1E+1000}\n"),
        &success("{\"a\": 1e1000}\n")
    ));
}

#[test]
fn value_comparison_rejects_output_jq_cannot_read() {
    let jq = success("{\"a\":1,\"c\":true}\n");
    for jqc in [
        "{\"a\":1 \"c\":true}\n",
        "{a:1,\"c\":true}\n",
        "{'a':1,\"c\":true}\n",
        "{\"a\":1,\"c\":true,}\n",
        "// comment\n{\"a\":1,\"c\":true}\n",
        "{\"a\":0x1,\"c\":true}\n",
        "{\"a\":+1,\"c\":true}\n",
    ] {
        assert!(!same_result(&value_case(), &jq, &success(jqc)), "{jqc:?}");
    }
}

#[test]
fn value_comparison_rejects_values_without_separator() {
    assert!(!same_result(
        &value_case(),
        &success("0\n1\n"),
        &success("01\n")
    ));
    assert!(!same_result(
        &value_case(),
        &success("1\ntrue\n"),
        &success("1true\n")
    ));
}

#[test]
fn value_comparison_handles_extreme_exponents() {
    let extreme = success("1.0e-9223372036854775808\n");
    assert!(same_result(&value_case(), &extreme, &extreme));
    assert!(same_result(
        &value_case(),
        &success("0E+999999999\n"),
        &success("0e9223372036854775808\n")
    ));
}

#[test]
fn value_comparison_uses_last_duplicate_key() {
    assert!(same_result(
        &value_case(),
        &success("{\"a\":9}\n"),
        &success("{\"a\": 1, \"a\": 9}\n")
    ));
}

#[test]
fn value_comparison_rejects_missing_jqc_output() {
    assert!(!same_result(
        &value_case(),
        &success("null\n"),
        &success("")
    ));
}

#[test]
fn value_comparison_accepts_no_output_from_both() {
    assert!(same_result(&value_case(), &success(""), &success("")));
}
