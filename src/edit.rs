use anyhow::{Result, anyhow, bail};
use jaq_core::ValT;
use jaq_json::Val;
use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstContainerNode, CstInputValue, CstLeafNode, CstNode, CstRootNode};

use crate::jaq;

/// A resolved segment of a path returned by jq's path() built-in.
#[derive(Debug, PartialEq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// Evaluate `[path(EXPR)]` against `text` using jaq-core and return all
/// resolved paths (each as a segment list). Returns an empty `Vec` if the
/// expression matches no paths in the input; this is not an error.
pub fn resolve_path(path_expr: &str, text: &str) -> Result<Vec<Vec<PathSegment>>> {
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

    paths
        .into_iter()
        .map(|path_val| {
            let Val::Arr(segments_val) = path_val else {
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
                        let i = n
                            .as_isize()
                            .ok_or_else(|| anyhow!("path index is not an integer: {n}"))?;
                        let idx = usize::try_from(i)
                            .map_err(|_| anyhow!("negative array index from path(): {i}"))?;
                        Ok(PathSegment::Index(idx))
                    }
                    other => bail!("unexpected path segment type: {other}"),
                })
                .collect()
        })
        .collect()
}

/// Order matched paths so that applying edits sequentially to the same
/// `CstRootNode` never invalidates a later path.
///
/// Object-key segments are always safe to process in any order (lookup is
/// by name, unaffected by sibling mutations). Array-index segments are only
/// unsafe when two matched paths share the same parent array: removing an
/// earlier element shifts the indices of later ones, so within each such
/// group, indices must be processed in descending order. Unrelated paths
/// are left in their original relative order (stable sort).
fn sort_paths_for_application(paths: &mut [Vec<PathSegment>]) {
    paths.sort_by(|a, b| match (a.split_last(), b.split_last()) {
        (Some((PathSegment::Index(ia), pa)), Some((PathSegment::Index(ib), pb))) if pa == pb => {
            ib.cmp(ia)
        }
        _ => std::cmp::Ordering::Equal,
    });
}

/// Navigate the CST from the root to the value node indicated by `segments`.
pub fn navigate(root: &CstRootNode, segments: &[PathSegment]) -> Result<CstNode> {
    let mut current: CstNode = root
        .value()
        .ok_or_else(|| anyhow!("JSONC input is empty"))?;

    for (i, seg) in segments.iter().enumerate() {
        match seg {
            PathSegment::Key(key) => {
                let obj = current.as_object().ok_or_else(|| {
                    anyhow!("expected object at segment {i} (key={key:?}), got: {current}")
                })?;
                current = obj
                    .get(key)
                    .ok_or_else(|| anyhow!("key {key:?} not found at path segment [{i}]"))?
                    .value()
                    .ok_or_else(|| anyhow!("key {key:?} has no value"))?;
            }
            PathSegment::Index(idx) => {
                let arr = current.as_array().ok_or_else(|| {
                    anyhow!("expected array at segment {i} (index={idx}), got: {current}")
                })?;
                let elements = arr.elements();
                current = elements
                    .get(*idx)
                    .ok_or_else(|| {
                        anyhow!("array index {idx} out of bounds (len={})", elements.len())
                    })?
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
            CstLeafNode::StringLit(n) => {
                n.replace_with(value);
            }
            CstLeafNode::NumberLit(n) => {
                n.replace_with(value);
            }
            CstLeafNode::BooleanLit(n) => {
                n.replace_with(value);
            }
            CstLeafNode::NullKeyword(n) => {
                n.replace_with(value);
            }
            CstLeafNode::WordLit(n) => {
                n.replace_with(value);
            }
            other => bail!("cannot replace trivia node: {other}"),
        },
        CstNode::Container(container) => match container {
            CstContainerNode::Object(n) => {
                n.replace_with(value);
            }
            CstContainerNode::Array(n) => {
                n.replace_with(value);
            }
            other => bail!("cannot replace root or object property node: {other}"),
        },
    }
    Ok(())
}

/// Write `new_val` into the CST node `existing`, preserving comments.
///
/// If `existing` and `new_val` are both arrays and `new_val`'s elements
/// begin with exactly `existing`'s current elements (a pure append, e.g.
/// from `.tags += [...]`), only the appended tail elements are added via
/// `CstArray::append`, leaving the array's existing structure — and any
/// comments within it — untouched. Otherwise `existing` is replaced
/// wholesale with `new_val`.
fn write_cst_value(existing: &CstNode, new_val: serde_json::Value) -> Result<()> {
    if let (Some(arr), serde_json::Value::Array(new_elems)) = (existing.as_array(), &new_val)
        && let Some(serde_json::Value::Array(existing_elems)) = arr.to_serde_value()
        && new_elems.len() > existing_elems.len()
        && new_elems[..existing_elems.len()] == existing_elems[..]
    {
        for elem in &new_elems[existing_elems.len()..] {
            arr.append(to_cst_input(elem.clone()));
        }
        return Ok(());
    }
    replace_cst_node(existing.clone(), to_cst_input(new_val))
}

/// Remove a CST node (used for array elements; object properties are
/// removed via `CstObjectProp::remove()` directly).
fn remove_cst_node(node: CstNode) -> Result<()> {
    match node {
        CstNode::Leaf(leaf) => match leaf {
            CstLeafNode::StringLit(n) => n.remove(),
            CstLeafNode::NumberLit(n) => n.remove(),
            CstLeafNode::BooleanLit(n) => n.remove(),
            CstLeafNode::NullKeyword(n) => n.remove(),
            CstLeafNode::WordLit(n) => n.remove(),
            other => bail!("cannot remove trivia node: {other}"),
        },
        CstNode::Container(container) => match container {
            CstContainerNode::Object(n) => n.remove(),
            CstContainerNode::Array(n) => n.remove(),
            other => bail!("cannot remove root or object property node: {other}"),
        },
    }
    Ok(())
}

/// Extract the value at `segments` from an already-evaluated `Val` document.
fn get_at_path(val: &Val, segments: &[PathSegment]) -> Result<Val> {
    let mut current = val.clone();
    for seg in segments {
        let key: Val = match seg {
            PathSegment::Key(k) => Val::from(k.clone()),
            PathSegment::Index(i) => Val::from(*i as isize),
        };
        current = current
            .index(&key)
            .map_err(|e| anyhow!("failed to read evaluated result at path: {e}"))?;
    }
    Ok(current)
}

/// Apply a jq assignment-family expression (`=`, `|=`, `+=`, `-=`, `*=`,
/// `/=`, `%=`, `//=`) to `text`.
///
/// `path_expr` is the LHS (target path expression) of the assignment.
/// `filter_str` is the *entire* original edit expression (e.g. `.a += 1`),
/// evaluated once via the normal jq pipeline to let jaq/jaq-std's own
/// implementation of each operator's semantics run unmodified. The
/// resulting whole-document value is then used purely as a lookup table:
/// for every path that `path_expr` matches, the value at that path in the
/// evaluated result is copied into the corresponding CST node, preserving
/// comments everywhere else.
///
/// If `path_expr` matches no paths, `text` is returned unchanged (no-op).
/// If `filter_str` does not evaluate to exactly one result, this errors.
pub fn apply_assign(text: &str, path_expr: &str, filter_str: &str) -> Result<String> {
    let mut matches = resolve_path(path_expr, text)?;
    if matches.is_empty() {
        return Ok(text.to_string());
    }
    sort_paths_for_application(&mut matches);

    let mut results = jaq::run(filter_str, text)?;
    if results.len() != 1 {
        bail!(
            "edit expression {filter_str:?} produced {} result(s); edit requires exactly one",
            results.len()
        );
    }
    let whole_result = results.pop().unwrap();

    let root = parse_cst(text)?;
    for segments in matches {
        let value_at_path = get_at_path(&whole_result, &segments)?;
        let json_str = value_at_path.to_string();
        let new_val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| anyhow!("failed to convert evaluated value to JSON: {e}"))?;

        let Some((last, parent_segments)) = segments.split_last() else {
            let target = navigate(&root, &segments)?;
            write_cst_value(&target, new_val)?;
            continue;
        };
        let PathSegment::Key(key) = last else {
            let target = navigate(&root, &segments)?;
            write_cst_value(&target, new_val)?;
            continue;
        };
        let parent = navigate(&root, parent_segments)?;
        let obj = parent.as_object().ok_or_else(|| {
            anyhow!(
                "expected object at segment {} (key={key:?}), got: {parent}",
                parent_segments.len()
            )
        })?;
        match obj.get(key) {
            Some(prop) => {
                let existing = prop
                    .value()
                    .ok_or_else(|| anyhow!("key {key:?} has no value"))?;
                write_cst_value(&existing, new_val)?;
            }
            None => {
                obj.append(key, to_cst_input(new_val));
            }
        }
    }
    Ok(format!("{root}"))
}

/// Delete every value matched by `path_expr` (object keys or array
/// elements). Returns the modified JSONC text with all comments preserved.
///
/// If a matched path (or any of its ancestors) does not exist, that match
/// is silently skipped (no-op), matching jq's own `del()` semantics. If
/// `path_expr` matches nothing at all, `text` is returned unchanged.
pub fn del(text: &str, path_expr: &str) -> Result<String> {
    let mut matches = resolve_path(path_expr, text)?;
    if matches.is_empty() {
        return Ok(text.to_string());
    }
    sort_paths_for_application(&mut matches);

    let root = parse_cst(text)?;
    for segments in matches {
        let Some((last, parent_segments)) = segments.split_last() else {
            bail!("path is empty");
        };
        let Ok(parent_node) = navigate(&root, parent_segments) else {
            continue; // an ancestor is missing: no-op for this match
        };
        match last {
            PathSegment::Key(key) => {
                let Some(obj) = parent_node.as_object() else {
                    continue;
                };
                let Some(prop) = obj.get(key) else {
                    continue; // no-op: key not found
                };
                prop.remove();
            }
            PathSegment::Index(idx) => {
                let Some(arr) = parent_node.as_array() else {
                    continue;
                };
                let elements = arr.elements();
                let Some(element) = elements.get(*idx) else {
                    continue; // no-op: index out of bounds
                };
                remove_cst_node(element.clone())?;
            }
        }
    }
    Ok(format!("{root}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"server": {"host": "localhost", "port": 3000}, "tags": ["a", "b"]}"#;

    fn parse_sample() -> CstRootNode {
        CstRootNode::parse(SAMPLE, &jsonc_parser::ParseOptions::default()).unwrap()
    }

    #[test]
    fn test_resolve_nested_key() {
        let matches = resolve_path(".server.port", SAMPLE).unwrap();
        assert_eq!(
            matches,
            vec![vec![
                PathSegment::Key("server".into()),
                PathSegment::Key("port".into())
            ]]
        );
    }

    #[test]
    fn test_resolve_array_index() {
        let matches = resolve_path(".tags[0]", SAMPLE).unwrap();
        assert_eq!(
            matches,
            vec![vec![PathSegment::Key("tags".into()), PathSegment::Index(0)]]
        );
    }

    #[test]
    fn test_resolve_missing_key_returns_path() {
        // path() returns the structural path even if the key is absent
        let matches = resolve_path(".server.missing", SAMPLE).unwrap();
        assert_eq!(
            matches,
            vec![vec![
                PathSegment::Key("server".into()),
                PathSegment::Key("missing".into())
            ]]
        );
    }

    #[test]
    fn test_resolve_multi_match_returns_all_paths() {
        let matches = resolve_path(".tags[]", SAMPLE).unwrap();
        assert_eq!(
            matches,
            vec![
                vec![PathSegment::Key("tags".into()), PathSegment::Index(0)],
                vec![PathSegment::Key("tags".into()), PathSegment::Index(1)],
            ]
        );
    }

    #[test]
    fn test_resolve_no_match_returns_empty() {
        let matches = resolve_path(".tags[] | select(. == \"nope\")", SAMPLE).unwrap();
        assert_eq!(matches, Vec::<Vec<PathSegment>>::new());
    }

    #[test]
    fn test_sort_paths_descending_within_same_array() {
        let mut matches = vec![
            vec![PathSegment::Key("tags".into()), PathSegment::Index(0)],
            vec![PathSegment::Key("tags".into()), PathSegment::Index(2)],
            vec![PathSegment::Key("tags".into()), PathSegment::Index(1)],
        ];
        sort_paths_for_application(&mut matches);
        assert_eq!(
            matches,
            vec![
                vec![PathSegment::Key("tags".into()), PathSegment::Index(2)],
                vec![PathSegment::Key("tags".into()), PathSegment::Index(1)],
                vec![PathSegment::Key("tags".into()), PathSegment::Index(0)],
            ]
        );
    }

    #[test]
    fn test_sort_paths_keeps_unrelated_paths_stable() {
        let mut matches = vec![
            vec![PathSegment::Key("a".into())],
            vec![PathSegment::Key("b".into())],
        ];
        sort_paths_for_application(&mut matches);
        assert_eq!(
            matches,
            vec![
                vec![PathSegment::Key("a".into())],
                vec![PathSegment::Key("b".into())],
            ]
        );
    }

    #[test]
    fn test_navigate_array_index() {
        let root = parse_sample();
        let segs = vec![PathSegment::Key("tags".into()), PathSegment::Index(1)];
        let node = navigate(&root, &segs).unwrap();
        assert_eq!(node.to_string(), "\"b\"");
    }

    #[test]
    fn test_navigate_same_key_different_levels() {
        // Ensures navigation is step-by-step and does not pick up the top-level "port"
        let input = r#"{"port": 80, "server": {"port": 3000}}"#;
        let root = CstRootNode::parse(input, &jsonc_parser::ParseOptions::default()).unwrap();
        let segs = vec![
            PathSegment::Key("server".into()),
            PathSegment::Key("port".into()),
        ];
        let node = navigate(&root, &segs).unwrap();
        assert_eq!(node.to_string(), "3000");
    }

    #[test]
    fn test_navigate_key_not_found_error() {
        let root = parse_sample();
        let segs = vec![
            PathSegment::Key("server".into()),
            PathSegment::Key("nonexistent".into()),
        ];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_type_mismatch_error() {
        // "tags" is an array; navigating into it with a Key segment must fail
        let root = parse_sample();
        let segs = vec![
            PathSegment::Key("tags".into()),
            PathSegment::Key("foo".into()),
        ];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_index_out_of_bounds_error() {
        let root = parse_sample();
        let segs = vec![PathSegment::Key("tags".into()), PathSegment::Index(99)];
        assert!(navigate(&root, &segs).is_err());
    }

    #[test]
    fn test_navigate_empty_segments() {
        // Empty path returns the root value itself
        let root = parse_sample();
        let node = navigate(&root, &[]).unwrap();
        assert!(node.as_object().is_some());
    }

    #[test]
    fn test_apply_assign_number() {
        let input = r#"{"port": 3000}"#;
        let result = apply_assign(input, ".port", ".port = 8080").unwrap();
        assert_eq!(result, r#"{"port": 8080}"#);
    }

    #[test]
    fn test_apply_assign_bool() {
        let input = r#"{"debug": true}"#;
        let result = apply_assign(input, ".debug", ".debug = false").unwrap();
        assert_eq!(result, r#"{"debug": false}"#);
    }

    #[test]
    fn test_apply_assign_string() {
        let input = r#"{"host": "localhost"}"#;
        let result = apply_assign(input, ".host", ".host = \"production.example.com\"").unwrap();
        assert_eq!(result, r#"{"host": "production.example.com"}"#);
    }

    #[test]
    fn test_apply_assign_update_operator() {
        // |= : the RHS filter runs against the current value at the path
        let input = r#"{"count": 3}"#;
        let result = apply_assign(input, ".count", ".count |= . + 1").unwrap();
        assert_eq!(result, r#"{"count": 4}"#);
    }

    #[test]
    fn test_apply_assign_update_math_operator() {
        // += : RHS is evaluated once against the original input, then added
        let input = r#"{"count": 3}"#;
        let result = apply_assign(input, ".count", ".count += 1").unwrap();
        assert_eq!(result, r#"{"count": 4}"#);
    }

    #[test]
    fn test_apply_assign_update_alt_operator() {
        // //= : replace only if the current value is falsy
        let input = r#"{"a": null, "b": 1}"#;
        let result = apply_assign(input, ".a", ".a //= 99").unwrap();
        assert_eq!(result, r#"{"a": 99, "b": 1}"#);
        let result = apply_assign(input, ".b", ".b //= 99").unwrap();
        assert_eq!(result, r#"{"a": null, "b": 1}"#);
    }

    #[test]
    fn test_apply_assign_nested_preserves_comments() {
        let input = "{\n  // server config\n  \"server\": {\"port\": 3000}\n}";
        let result = apply_assign(input, ".server.port", ".server.port = 8080").unwrap();
        assert!(
            result.contains("// server config"),
            "comment must be preserved"
        );
        assert!(result.contains("8080"));
        assert!(!result.contains("3000"));
    }

    #[test]
    fn test_apply_assign_array_element() {
        let input = r#"{"tags": ["a", "b", "c"]}"#;
        let result = apply_assign(input, ".tags[1]", ".tags[1] = \"x\"").unwrap();
        assert_eq!(result, r#"{"tags": ["a", "x", "c"]}"#);
    }

    #[test]
    fn test_apply_assign_creates_missing_top_level_key() {
        let input = r#"{"port": 3000}"#;
        let result = apply_assign(input, ".newKey", ".newKey = \"value\"").unwrap();
        assert!(result.contains("\"newKey\""));
        assert!(result.contains("\"value\""));
        assert!(result.contains("\"port\""));
    }

    #[test]
    fn test_apply_assign_missing_intermediate_object_errors() {
        assert!(
            apply_assign(
                r#"{"port": 3000}"#,
                ".server.timeout",
                ".server.timeout = 30"
            )
            .is_err()
        );
    }

    #[test]
    fn test_apply_assign_multi_path_bulk_update() {
        // .items[] += 1 : every matched path gets the same operator applied
        let input = r#"{"items": [1, 2, 3]}"#;
        let result = apply_assign(input, ".items[]", ".items[] += 1").unwrap();
        assert_eq!(result, r#"{"items": [2, 3, 4]}"#);
    }

    #[test]
    fn test_apply_assign_no_match_is_noop() {
        let input = r#"{"items": []}"#;
        let result = apply_assign(input, ".items[]", ".items[] += 1").unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_apply_assign_multiple_results_errors() {
        let input = r#"{"a": 1}"#;
        let err = apply_assign(input, ".a", ".a = (1,2)")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("2"),
            "error should mention the result count: {err}"
        );
    }

    #[test]
    fn test_apply_assign_invalid_value_expr_errors() {
        assert!(apply_assign(r#"{"port": 3000}"#, ".port", ".port = (error)").is_err());
    }

    #[test]
    fn test_apply_assign_push_replacement_via_plus_equals() {
        // += replaces the old dedicated `push` command for appending to arrays
        let input = r#"{"tags": ["a", "b"]}"#;
        let result = apply_assign(input, ".tags", ".tags += [\"c\"]").unwrap();
        let pos_b = result.find("\"b\"").unwrap();
        let pos_c = result.find("\"c\"").unwrap();
        assert!(
            pos_c > pos_b,
            "new element must be appended after existing elements"
        );
    }

    #[test]
    fn test_apply_assign_plus_equals_preserves_comment_between_array_elements() {
        let input = "{\n  \"tags\": [\n    \"a\", /* between */ \"b\"\n  ]\n}";
        let result = apply_assign(input, ".tags", ".tags += [\"c\"]").unwrap();
        assert!(result.contains("/* between */"), "comment lost: {result}");
        assert!(
            result.contains("\"c\""),
            "new element not appended: {result}"
        );
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
        assert!(
            !result.contains(", \"port\""),
            "no leading comma before remaining key"
        );
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
    fn test_del_array_index_no_longer_errors() {
        // Previously array-index deletion was unsupported; it is now supported.
        assert!(del(r#"{"tags": ["a", "b"]}"#, ".tags[0]").is_ok());
    }

    #[test]
    fn test_del_array_element() {
        let input = r#"{"tags": ["a", "b", "c"]}"#;
        let result = del(input, ".tags[1]").unwrap();
        assert_eq!(result, r#"{"tags": ["a", "c"]}"#);
    }

    #[test]
    fn test_del_array_element_preserves_comments() {
        let input = "{\n  // list\n  \"tags\": [\"a\", \"b\"]\n}";
        let result = del(input, ".tags[0]").unwrap();
        assert!(result.contains("// list"), "comment must be preserved");
        assert!(!result.contains("\"a\""));
        assert!(result.contains("\"b\""));
    }

    #[test]
    fn test_del_multi_match_bulk_delete() {
        let input = r#"{"tags": ["a", "b", "c"]}"#;
        let result = del(input, ".tags[]").unwrap();
        assert_eq!(result, r#"{"tags": []}"#);
    }

    #[test]
    fn test_del_missing_key_is_noop() {
        let input = r#"{"port": 3000}"#;
        let result = del(input, ".nonexistent").unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_del_nested_missing_key_is_noop() {
        let input = r#"{"server": {"port": 3000}}"#;
        let result = del(input, ".server.nonexistent").unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_del_out_of_bounds_index_is_noop() {
        let input = r#"{"tags": ["a", "b"]}"#;
        let result = del(input, ".tags[5]").unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_del_no_match_is_noop() {
        let input = r#"{"tags": []}"#;
        let result = del(input, ".tags[]").unwrap();
        assert_eq!(result, input);
    }
}
