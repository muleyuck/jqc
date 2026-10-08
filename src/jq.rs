//! Runs the installed jq.

use std::io::{self, Write};
use std::process::{Child, Command, Stdio};

use anyhow::{Result, anyhow};

/// Start jq with `args`. jq writes to jqc's own stdout and stderr, so it
/// decides on colors and streams its results itself.
pub fn spawn(args: &[String], stdin: Stdio) -> Result<Child> {
    spawn_with(args, stdin, Stdio::inherit())
}

fn spawn_with(args: &[String], stdin: Stdio, stdout: Stdio) -> Result<Child> {
    Command::new("jq")
        .args(args)
        .stdin(stdin)
        .stdout(stdout)
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => anyhow!(
                "jq not found: jqc runs filters with jq. Install jq (https://jqlang.org/download/) and make sure it is on PATH"
            ),
            _ => anyhow!("Failed to run jq: {e}"),
        })
}

/// Run jq with `args` on `input` and capture its stdout. jq's stderr is
/// jqc's own, so jq reports its own errors.
pub fn output(args: &[String], input: &str) -> Result<(std::process::ExitStatus, String)> {
    let mut child = spawn_with(args, Stdio::piped(), Stdio::piped())?;
    let mut stdin = child.stdin.take().expect("jq's stdin is piped");
    let input = input.to_string();
    // Write from a thread so a large input can't deadlock against stdout.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let out = child
        .wait_with_output()
        .map_err(|e| anyhow!("Failed to run jq: {e}"))?;
    let _ = writer.join();
    Ok((
        out.status,
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

/// What `jq --version` prints, or `None` when jq can't run.
pub fn version() -> Option<String> {
    let out = Command::new("jq").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
