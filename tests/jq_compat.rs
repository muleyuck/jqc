//
// jq compatibility: jqc against jq's recorded output (tests/fixtures/jq-compat)
//

use assert_cmd::Command;
use jsonc_parser::ParseOptions;
use serde_json::Value;
use std::collections::HashSet;
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
        .unwrap_or_else(|| panic!("case {name:?}: \"args\" must be an array of strings"));
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
    if jq.status != 0 {
        return true;
    }
    let jq_value = serde_json::from_str::<Value>(&jq.stdout);
    let jqc_value =
        jsonc_parser::parse_to_serde_value::<Value>(&jqc.stdout, &ParseOptions::default());
    matches!((jq_value, jqc_value), (Ok(a), Ok(b)) if a == b)
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
