//
// E2E Testing
//

use assert_cmd::Command;
use predicates::str::contains;
use std::fs;

fn jqc() -> Command {
    Command::cargo_bin("jqc").unwrap()
}

fn fixture(name: &str) -> String {
    format!("tests/fixtures/{name}")
}

// ---------------------------------------------------------------------------
// Filter mode
// ---------------------------------------------------------------------------

#[test]
fn filter_number() {
    jqc()
        .args([".port", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("3000\n");
}

#[test]
fn filter_string() {
    jqc()
        .args([".host", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("\"localhost\"\n");
}

#[test]
fn filter_bool() {
    jqc()
        .args([".debug", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("false\n");
}

#[test]
fn filter_array() {
    jqc()
        .args([".plugins", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("[\n  \"core\",\n  \"auth\"\n]\n");
}

#[test]
fn filter_identity_pretty_prints() {
    let out = jqc()
        .args([".", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    // comments are stripped in filter output; verify JSON fields survive
    assert!(stdout.contains("\"host\""), "host field missing: {stdout}");
    assert!(stdout.contains("\"port\""), "port field missing: {stdout}");
    assert!(
        stdout.contains("\"plugins\""),
        "plugins field missing: {stdout}"
    );
}

#[test]
fn filter_stdin_plain_json() {
    jqc()
        .arg(".port")
        .write_stdin(r#"{"port": 8080}"#)
        .assert()
        .success()
        .stdout("8080\n");
}

#[test]
fn filter_stdin_jsonc_with_comments() {
    jqc()
        .arg(".port")
        .write_stdin("{ /* config */ \"port\": 8080 }")
        .assert()
        .success()
        .stdout("8080\n");
}

#[test]
fn filter_raw_output_string() {
    jqc()
        .args(["-r", ".host", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("localhost\n");
}

#[test]
fn filter_raw_output_number() {
    // -r on a non-string value outputs the value as-is (no quotes to strip)
    jqc()
        .args(["-r", ".port", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("3000\n");
}

#[test]
fn filter_compact_output() {
    jqc()
        .args(["-c", ".plugins", &fixture("config.jsonc")])
        .assert()
        .success()
        .stdout("[\"core\",\"auth\"]\n");
}

#[test]
fn filter_invalid_syntax_error() {
    jqc()
        .args([".foo[", &fixture("config.jsonc")])
        .assert()
        .failure()
        .stderr(contains("filter syntax error"));
}

// ---------------------------------------------------------------------------
// set
// ---------------------------------------------------------------------------

#[test]
fn set_number_preserves_comments() {
    let out = jqc()
        .args(["set", ".port", "8080", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(stdout.contains("8080"), "value not updated: {stdout}");
    assert!(
        stdout.contains("// default port"),
        "inline comment lost: {stdout}"
    );
    assert!(
        stdout.contains("// Server settings"),
        "line comment lost: {stdout}"
    );
}

#[test]
fn set_string_value() {
    let out = jqc()
        .args(["set", ".host", "\"production\"", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        stdout.contains("\"production\""),
        "value not updated: {stdout}"
    );
}

#[test]
fn set_nested_path() {
    jqc()
        .args(["set", ".server.port", "9090"])
        .write_stdin(r#"{"server": {"port": 3000}}"#)
        .assert()
        .success()
        .stdout(contains("9090"));
}

#[test]
fn set_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.jsonc");
    fs::copy(fixture("config.jsonc"), &path).unwrap();

    jqc()
        .args(["set", ".port", "9090", "-i", path.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("9090"),
        "value not updated in-place: {content}"
    );
    assert!(
        content.contains("// default port"),
        "inline comment lost: {content}"
    );
}

#[test]
fn set_in_place_requires_file() {
    jqc()
        .args(["set", ".port", "9090", "-i"])
        .write_stdin(r#"{"port": 3000}"#)
        .assert()
        .failure()
        .stderr(contains("--in-place requires a file"));
}

#[test]
fn set_invalid_json_value_errors() {
    jqc()
        .args(["set", ".port", "not-json", &fixture("config.jsonc")])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// del
// ---------------------------------------------------------------------------

#[test]
fn del_removes_key_and_preserves_other_comments() {
    let out = jqc()
        .args(["del", ".debug", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(!stdout.contains("\"debug\""), "key not deleted: {stdout}");
    assert!(
        stdout.contains("// Server settings"),
        "unrelated comment lost: {stdout}"
    );
    assert!(
        stdout.contains("// default port"),
        "unrelated inline comment lost: {stdout}"
    );
}

#[test]
fn del_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.jsonc");
    fs::copy(fixture("config.jsonc"), &path).unwrap();

    jqc()
        .args(["del", ".debug", "-i", path.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("\"debug\""),
        "key not deleted in-place: {content}"
    );
    assert!(
        content.contains("// Server settings"),
        "comment lost: {content}"
    );
}

// ---------------------------------------------------------------------------
// push
// ---------------------------------------------------------------------------

#[test]
fn push_appends_to_array_and_preserves_comments() {
    let out = jqc()
        .args(["push", ".plugins", "\"logging\"", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        stdout.contains("\"logging\""),
        "value not appended: {stdout}"
    );
    assert!(stdout.contains("\"core\""), "existing value lost: {stdout}");
    assert!(
        stdout.contains("// Server settings"),
        "comment lost: {stdout}"
    );
}

#[test]
fn push_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.jsonc");
    fs::copy(fixture("config.jsonc"), &path).unwrap();

    jqc()
        .args([
            "push",
            ".plugins",
            "\"logging\"",
            "-i",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("\"logging\""),
        "value not appended in-place: {content}"
    );
}

#[test]
fn push_to_non_array_errors() {
    jqc()
        .args(["push", ".port", "1", &fixture("config.jsonc")])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// Array filter
// ---------------------------------------------------------------------------

#[test]
fn filter_array_index() {
    jqc()
        .arg(".plugins[0]")
        .write_stdin(r#"{"plugins": ["core", "auth"]}"#)
        .assert()
        .success()
        .stdout("\"core\"\n");
}

#[test]
fn filter_array_expansion_multiple_outputs() {
    jqc()
        .arg(".plugins[]")
        .write_stdin(r#"{"plugins": ["core", "auth"]}"#)
        .assert()
        .success()
        .stdout("\"core\"\n\"auth\"\n");
}

// ---------------------------------------------------------------------------
// Real-world fixture: VS Code settings.jsonc
// ---------------------------------------------------------------------------

#[test]
fn vscode_filter_tab_size() {
    // Keys like "editor.tabSize" contain dots; jq quoted-key syntax is required
    jqc()
        .args([r#"."editor.tabSize""#, &fixture("vscode-settings.jsonc")])
        .assert()
        .success()
        .stdout("2\n");
}

#[test]
fn vscode_set_tab_size_preserves_comments() {
    let out = jqc()
        .args([
            "set",
            r#"."editor.tabSize""#,
            "4",
            &fixture("vscode-settings.jsonc"),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        stdout.contains("\"editor.tabSize\": 4"),
        "value not updated: {stdout}"
    );
    assert!(stdout.contains("// Editor"), "comment lost: {stdout}");
    assert!(
        stdout.contains("/* Formatter */"),
        "block comment lost: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Real-world fixture: tsconfig.jsonc
// ---------------------------------------------------------------------------

#[test]
fn tsconfig_filter_nested_target() {
    jqc()
        .args([".compilerOptions.target", &fixture("tsconfig.jsonc")])
        .assert()
        .success()
        .stdout("\"ES2022\"\n");
}

#[test]
fn tsconfig_set_strict_preserves_inline_comment() {
    let out = jqc()
        .args([
            "set",
            ".compilerOptions.strict",
            "false",
            &fixture("tsconfig.jsonc"),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(stdout.contains("false"), "value not updated: {stdout}");
    assert!(
        stdout.contains("// enable all strict checks"),
        "inline comment lost: {stdout}"
    );
    assert!(
        stdout.contains("/* Paths */"),
        "block comment lost: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Real-world fixture: deno.jsonc
// ---------------------------------------------------------------------------

#[test]
fn deno_filter_version() {
    jqc()
        .args([".version", &fixture("deno.jsonc")])
        .assert()
        .success()
        .stdout("\"0.1.0\"\n");
}

#[test]
fn deno_push_lint_tag_preserves_comments() {
    let out = jqc()
        .args([
            "push",
            ".lint.rules.tags",
            "\"strict\"",
            &fixture("deno.jsonc"),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        stdout.contains("\"strict\""),
        "value not appended: {stdout}"
    );
    assert!(
        stdout.contains("\"recommended\""),
        "existing value lost: {stdout}"
    );
    assert!(stdout.contains("// Task runner"), "comment lost: {stdout}");
    assert!(
        stdout.contains("/* Import map */"),
        "block comment lost: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn set_nonexistent_key_errors() {
    jqc()
        .args(["set", ".missing", "42", &fixture("config.jsonc")])
        .assert()
        .failure()
        .stderr(contains("not found"));
}

#[test]
fn del_nonexistent_key_errors() {
    jqc()
        .args(["del", ".missing", &fixture("config.jsonc")])
        .assert()
        .failure()
        .stderr(contains("not found"));
}

#[test]
fn del_preserves_adjacent_block_comment() {
    // /* Feature flags */ sits above "debug"; deleting "debug" must keep the block comment
    let out = jqc()
        .args(["del", ".debug", &fixture("config.jsonc")])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        stdout.contains("/* Feature flags */"),
        "block comment lost: {stdout}"
    );
}

#[test]
fn push_in_place_requires_file() {
    jqc()
        .args(["push", ".plugins", "\"logging\"", "-i"])
        .write_stdin(r#"{"plugins": []}"#)
        .assert()
        .failure()
        .stderr(contains("--in-place requires a file"));
}

#[test]
fn error_nonexistent_file() {
    jqc()
        .args([".", "no_such_file.jsonc"])
        .assert()
        .failure()
        .stderr(contains("Failed to read"));
}

#[test]
fn error_no_filter_no_subcommand() {
    jqc()
        .assert()
        .failure()
        .stderr(contains("filter expression required"));
}
