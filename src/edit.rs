use anyhow::{anyhow, bail, Result};
use jaq_json::Val;

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
}
