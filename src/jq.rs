//! Runs the installed jq.

use std::io;
use std::process::{Child, Command, Stdio};

use anyhow::{Result, anyhow};

/// Start jq with `args`. jq writes to jqc's own stdout and stderr, so it
/// decides on colors and streams its results itself.
pub fn spawn(args: &[String], stdin: Stdio) -> Result<Child> {
    Command::new("jq")
        .args(args)
        .stdin(stdin)
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => anyhow!(
                "jq not found: jqc runs filters with jq. Install jq (https://jqlang.org/download/) and make sure it is on PATH"
            ),
            _ => anyhow!("Failed to run jq: {e}"),
        })
}

/// What `jq --version` prints, or `None` when jq can't run.
pub fn version() -> Option<String> {
    let out = Command::new("jq").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
