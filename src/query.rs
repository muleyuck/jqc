use anyhow::Result;
use jaq_json::Val;

use crate::jaq;

/// Parse JSONC text and apply a jq filter, returning a list of output values.
pub fn run_filter(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    jaq::run(filter_str, text)
}
