use anyhow::{anyhow, bail, Result};
use jaq_json::Val;
use jsonc_parser::cst::{CstContainerNode, CstInputValue, CstLeafNode, CstNode, CstRootNode};
use jsonc_parser::ParseOptions;

use crate::jaq;

/// A resolved segment of a path returned by jq's path() built-in.
#[derive(Debug, PartialEq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// Evaluate `[path(EXPR)]` against `text` using jaq-core and return the single resolved path.
/// Errors if the expression matches more than one path.
pub fn resolve_path(path_expr: &str, text: &str) -> Result<Vec<PathSegment>> {
    let filter_str = format!("[path({path_expr})]");
    let mut results = jaq::run(&filter_str, text)?;

    // [path(expr)] produces exactly one array value
    let paths_val = results
        .pop()
        .ok_or_else(|| anyhow!("path() returned no output"))?;

    let Val::Arr(paths) = paths_val else {
        bail!("expected array from [path()], got: {paths_val}");
    };
    let paths = std::rc::Rc::unwrap_or_clone(paths);

    match paths.len() {
        0 => bail!("path expression {path_expr:?} matched no paths in the input"),
        1 => {}
        n => bail!("path expression {path_expr:?} matched {n} paths; edit requires exactly one"),
    }

    let Val::Arr(segments_val) = paths.into_iter().next().unwrap() else {
        bail!("expected inner array from path()");
    };

    std::rc::Rc::unwrap_or_clone(segments_val)
        .into_iter()
        .map(|seg| match seg {
            Val::BStr(b) | Val::TStr(b) => {
                let s = std::str::from_utf8(&b)
                    .map_err(|e| anyhow!("non-UTF8 path key: {e}"))?;
                Ok(PathSegment::Key(s.to_string()))
            }
            Val::Num(n) => {
                let i = n.as_isize().ok_or_else(|| anyhow!("path index is not an integer: {n}"))?;
                let idx = usize::try_from(i)
                    .map_err(|_| anyhow!("negative array index from path(): {i}"))?;
                Ok(PathSegment::Index(idx))
            }
            other => bail!("unexpected path segment type: {other}"),
        })
        .collect()
}

/// Navigate the CST from the root to the value node indicated by `segments`.
pub fn navigate(root: &CstRootNode, segments: &[PathSegment]) -> Result<CstNode> {
    let mut current: CstNode = root
        .value()
        .ok_or_else(|| anyhow!("JSONC input is empty"))?;

    for (i, seg) in segments.iter().enumerate() {
        match seg {
            PathSegment::Key(key) => {
                let obj = current
                    .as_object()
                    .ok_or_else(|| anyhow!("expected object at segment {i} (key={key:?}), got: {current}"))?;
                current = obj
                    .get(key)
                    .ok_or_else(|| anyhow!("key {key:?} not found"))?
                    .value()
                    .ok_or_else(|| anyhow!("key {key:?} has no value"))?;
            }
            PathSegment::Index(idx) => {
                let arr = current
                    .as_array()
                    .ok_or_else(|| anyhow!("expected array at segment {i} (index={idx}), got: {current}"))?;
                let elements = arr.elements();
                current = elements
                    .get(*idx)
                    .ok_or_else(|| anyhow!("array index {idx} out of bounds (len={})", elements.len()))?
                    .clone();
            }
        }
    }
    Ok(current)
}

/// Parse `text` as JSONC and return the root node for CST-based mutations.
fn parse_cst(text: &str) -> Result<CstRootNode> {
    CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))
}

/// Convert a `serde_json::Value` to `CstInputValue` for CST mutations.
fn to_cst_input(v: serde_json::Value) -> CstInputValue {
    match v {
        serde_json::Value::Null => CstInputValue::Null,
        serde_json::Value::Bool(b) => CstInputValue::Bool(b),
        serde_json::Value::Number(n) => CstInputValue::Number(n.to_string()),
        serde_json::Value::String(s) => CstInputValue::String(s),
        serde_json::Value::Array(arr) => {
            CstInputValue::Array(arr.into_iter().map(to_cst_input).collect())
        }
        serde_json::Value::Object(obj) => {
            CstInputValue::Object(obj.into_iter().map(|(k, v)| (k, to_cst_input(v))).collect())
        }
    }
}

/// Replace a CST node in-place with `value`.
fn replace_cst_node(node: CstNode, value: CstInputValue) -> Result<()> {
    match node {
        CstNode::Leaf(leaf) => match leaf {
            CstLeafNode::StringLit(n) => { n.replace_with(value); }
            CstLeafNode::NumberLit(n) => { n.replace_with(value); }
            CstLeafNode::BooleanLit(n) => { n.replace_with(value); }
            CstLeafNode::NullKeyword(n) => { n.replace_with(value); }
            CstLeafNode::WordLit(n) => { n.replace_with(value); }
            other => bail!("cannot replace trivia node: {other}"),
        }
        CstNode::Container(container) => match container {
            CstContainerNode::Object(n) => { n.replace_with(value); }
            CstContainerNode::Array(n) => { n.replace_with(value); }
            other => bail!("cannot replace root or object property node: {other}"),
        }
    }
    Ok(())
}

/// Replace the value at `path_expr` with `new_value_str` (JSON-encoded).
/// Returns the modified JSONC text with all comments preserved.
pub fn set(text: &str, path_expr: &str, new_value_str: &str) -> Result<String> {
    let new_val: serde_json::Value = serde_json::from_str(new_value_str)
        .map_err(|e| anyhow!("new value is not valid JSON: {e}"))?;
    let cst_val = to_cst_input(new_val);

    let segments = resolve_path(path_expr, text)?;
    let root = parse_cst(text)?;
    let target = navigate(&root, &segments)?;
    replace_cst_node(target, cst_val)?;
    Ok(format!("{root}"))
}

/// Delete the object property at `path_expr`.
/// The last segment must be a key (array element deletion is not supported).
/// Returns the modified JSONC text with all comments preserved.
pub fn del(text: &str, path_expr: &str) -> Result<String> {
    let segments = resolve_path(path_expr, text)?;

    let last = segments.last().ok_or_else(|| anyhow!("path is empty"))?;
    let PathSegment::Key(key) = last else {
        bail!("del only supports object key paths; last segment must be a key, not an index");
    };

    let root = parse_cst(text)?;

    let parent_node = if segments.len() == 1 {
        root.value().ok_or_else(|| anyhow!("JSONC input is empty"))?
    } else {
        navigate(&root, &segments[..segments.len() - 1])?
    };

    let obj = parent_node
        .as_object()
        .ok_or_else(|| anyhow!("parent of key {key:?} is not an object"))?;

    let prop = obj
        .get(key)
        .ok_or_else(|| anyhow!("key {key:?} not found"))?;

    prop.remove();
    Ok(format!("{root}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"server": {"host": "localhost", "port": 3000}, "tags": ["a", "b"]}"#;

    #[test]
    fn test_resolve_nested_key() {
        let segs = resolve_path(".server.port", SAMPLE).unwrap();
        assert_eq!(segs, vec![PathSegment::Key("server".into()), PathSegment::Key("port".into())]);
    }

    #[test]
    fn test_resolve_array_index() {
        let segs = resolve_path(".tags[0]", SAMPLE).unwrap();
        assert_eq!(segs, vec![PathSegment::Key("tags".into()), PathSegment::Index(0)]);
    }

    #[test]
    fn test_resolve_missing_key_returns_path() {
        // path() returns the structural path even if the key is absent
        let segs = resolve_path(".server.missing", SAMPLE).unwrap();
        assert_eq!(
            segs,
            vec![PathSegment::Key("server".into()), PathSegment::Key("missing".into())]
        );
    }

    #[test]
    fn test_resolve_multi_match_error() {
        assert!(resolve_path(".tags[]", SAMPLE).is_err());
    }

    #[test]
    fn test_navigate_array_index() {
        let root = CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![PathSegment::Key("tags".into()), PathSegment::Index(1)];
        let node = navigate(&root, &segs).unwrap();
        assert_eq!(node.to_string(), "\"b\"");
    }

    #[test]
    fn test_navigate_same_key_different_levels() {
        // Ensures navigation is step-by-step and does not pick up the top-level "port"
        let input = r#"{"port": 80, "server": {"port": 3000}}"#;
        let root = CstRootNode::parse(input, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![PathSegment::Key("server".into()), PathSegment::Key("port".into())];
        let node = navigate(&root, &segs).unwrap();
        assert_eq!(node.to_string(), "3000");
    }

    #[test]
    fn test_navigate_key_not_found_error() {
        let root = CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![PathSegment::Key("server".into()), PathSegment::Key("nonexistent".into())];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_type_mismatch_error() {
        // "tags" is an array; navigating into it with a Key segment must fail
        let root = CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![PathSegment::Key("tags".into()), PathSegment::Key("foo".into())];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_index_out_of_bounds_error() {
        let root = CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![PathSegment::Key("tags".into()), PathSegment::Index(99)];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_empty_segments() {
        // Empty path returns the root value itself
        let root = CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap();
        let node = navigate(&root, &[]).unwrap();
        assert!(node.as_object().is_some());
    }

    #[test]
    fn test_set_number() {
        let input = r#"{"port": 3000}"#;
        let result = set(input, ".port", "8080").unwrap();
        assert_eq!(result, r#"{"port": 8080}"#);
    }

    #[test]
    fn test_set_string() {
        let input = r#"{"host": "localhost"}"#;
        let result = set(input, ".host", "\"production.example.com\"").unwrap();
        assert_eq!(result, r#"{"host": "production.example.com"}"#);
    }

    #[test]
    fn test_set_bool() {
        let input = r#"{"debug": true}"#;
        let result = set(input, ".debug", "false").unwrap();
        assert_eq!(result, r#"{"debug": false}"#);
    }

    #[test]
    fn test_set_nested_preserves_comments() {
        let input = "{\n  // server config\n  \"server\": {\"port\": 3000}\n}";
        let result = set(input, ".server.port", "8080").unwrap();
        assert!(result.contains("// server config"), "comment must be preserved");
        assert!(result.contains("8080"));
        assert!(!result.contains("3000"));
    }

    #[test]
    fn test_set_array_element() {
        let input = r#"{"tags": ["a", "b", "c"]}"#;
        let result = set(input, ".tags[1]", "\"x\"").unwrap();
        assert_eq!(result, r#"{"tags": ["a", "x", "c"]}"#);
    }

    #[test]
    fn test_set_invalid_json_error() {
        assert!(set(r#"{"port": 3000}"#, ".port", "not-json").is_err());
    }

    #[test]
    fn test_del_key() {
        let input = r#"{"host": "localhost", "port": 3000}"#;
        let result = del(input, ".port").unwrap();
        assert!(!result.contains("port"));
        assert!(result.contains("localhost"));
    }

    #[test]
    fn test_del_nested_key() {
        let input = r#"{"server": {"host": "localhost", "port": 3000}}"#;
        let result = del(input, ".server.port").unwrap();
        assert!(!result.contains("3000"));
        assert!(result.contains("localhost"));
    }

    #[test]
    fn test_del_preserves_comments() {
        let input = "{\n  // keep this\n  \"host\": \"localhost\",\n  \"port\": 3000\n}";
        let result = del(input, ".port").unwrap();
        assert!(result.contains("// keep this"), "comment must be preserved");
        assert!(!result.contains("3000"));
    }

    #[test]
    fn test_del_only_key_remaining() {
        // Deleting the only property should leave an empty object
        let input = r#"{"port": 3000}"#;
        let result = del(input, ".port").unwrap();
        assert!(!result.contains("port"));
        assert!(!result.contains("3000"));
    }

    #[test]
    fn test_del_first_property() {
        let input = r#"{"host": "localhost", "port": 3000}"#;
        let result = del(input, ".host").unwrap();
        assert!(!result.contains("host"));
        assert!(!result.contains("localhost"));
        assert!(result.contains("3000"));
        // No stray leading comma
        assert!(!result.contains(", \"port\""), "no leading comma before remaining key");
    }

    #[test]
    fn test_del_middle_property() {
        let input = r#"{"a": 1, "b": 2, "c": 3}"#;
        let result = del(input, ".b").unwrap();
        assert!(!result.contains("\"b\""));
        assert!(result.contains("\"a\""));
        assert!(result.contains("\"c\""));
        // No double comma
        assert!(!result.contains(",,"));
    }

    #[test]
    fn test_del_index_path_error() {
        assert!(del(r#"{"tags": ["a", "b"]}"#, ".tags[0]").is_err());
    }

    #[test]
    fn test_del_missing_key_error() {
        assert!(del(r#"{"port": 3000}"#, ".nonexistent").is_err());
    }

    #[test]
    fn test_del_nested_missing_key_error() {
        assert!(del(r#"{"server": {"port": 3000}}"#, ".server.nonexistent").is_err());
    }
}
