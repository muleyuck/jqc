use anyhow::{anyhow, bail, Result};
use jaq_json::Val;
use jsonc_parser::cst::{CstNode, CstRootNode};
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
}
