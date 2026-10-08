//! Writes an edit back into JSONC: jq computes the edited value, and only
//! what differs from the original value is written into the original text,
//! so comments and formatting elsewhere stay as they are.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use jsonc_parser::cst::{
    CstArray, CstContainerNode, CstInputValue, CstLeafNode, CstNode, CstObject, CstObjectProp,
    CstRootNode,
};
use jsonc_parser::{JsonArray, JsonObject, JsonValue, ParseOptions};

/// Write the difference between `source` (the value of `text`, converted to
/// JSON) and `result` (jq's output for the edit) into `text`.
pub fn write_back(text: &str, source: &str, result: &str) -> Result<String> {
    let source = parse_json(source)?;
    let result = parse_json(result)?;
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;
    let node = root
        .value()
        .ok_or_else(|| anyhow!("JSONC input is empty"))?;
    write_node(&node, &source, &result)?;
    Ok(root.to_string())
}

fn parse_json(json: &str) -> Result<JsonValue<'_>> {
    jsonc_parser::parse_to_value(json, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSON: {e}"))?
        .ok_or_else(|| anyhow!("empty JSON value"))
}

/// `JsonValue` equality compares numbers by their text and strings by their
/// decoded content, which is what "unchanged" means here.
fn write_node(node: &CstNode, old: &JsonValue, new: &JsonValue) -> Result<()> {
    if old == new {
        return Ok(());
    }
    match (old, new) {
        (JsonValue::Object(old), JsonValue::Object(new)) => {
            let obj = node
                .as_object()
                .ok_or_else(|| anyhow!("expected an object in the JSONC text"))?;
            write_object(&obj, old, new)
        }
        (JsonValue::Array(old), JsonValue::Array(new)) => {
            let arr = node
                .as_array()
                .ok_or_else(|| anyhow!("expected an array in the JSONC text"))?;
            write_array(node, &arr, old, new)
        }
        _ => {
            if !set_in_place(node, new) {
                replace(node.clone(), to_cst_input(new))?;
            }
            Ok(())
        }
    }
}

fn write_object(obj: &CstObject, old: &JsonObject, new: &JsonObject) -> Result<()> {
    // In source order per key; JSONC allows duplicate keys.
    let mut props: HashMap<String, Vec<CstObjectProp>> = HashMap::new();
    for prop in obj.properties() {
        if let Some(name) = prop.name().and_then(|name| name.decoded_value().ok()) {
            props.entry(name).or_default().push(prop);
        }
    }
    for (key, old_value) in old.clone().take_inner().iter() {
        match new.get(key) {
            // jq treats duplicates as one key; any left behind would resurface.
            None => {
                for prop in props.remove(key.as_ref()).unwrap_or_default() {
                    prop.remove();
                }
            }
            Some(new_value) if new_value != old_value => {
                // jq reads duplicate keys last-wins.
                let prop = props
                    .get(key.as_ref())
                    .and_then(|named| named.last())
                    .ok_or_else(|| anyhow!("key {key:?} not found in the JSONC text"))?;
                let value = prop
                    .value()
                    .ok_or_else(|| anyhow!("key {key:?} has no value"))?;
                write_node(&value, old_value, new_value)?;
            }
            Some(_) => {}
        }
    }
    for (key, new_value) in new.clone().take_inner().iter() {
        if old.get(key).is_none() {
            obj.append(key, to_cst_input(new_value));
        }
    }
    Ok(())
}

fn write_array(node: &CstNode, arr: &CstArray, old: &JsonArray, new: &JsonArray) -> Result<()> {
    if old.len() == new.len() {
        for ((element, old_value), new_value) in
            arr.elements().iter().zip(old.iter()).zip(new.iter())
        {
            write_node(element, old_value, new_value)?;
        }
        return Ok(());
    }
    if new.len() > old.len() && old.iter().zip(new.iter()).all(|(o, n)| o == n) {
        for new_value in new.iter().skip(old.len()) {
            arr.append(to_cst_input(new_value));
        }
        return Ok(());
    }
    replace(node.clone(), to_cst_input(&JsonValue::Array(new.clone())))
}

/// Numbers keep the text jq printed; strings are written the CST's way.
fn to_cst_input(value: &JsonValue) -> CstInputValue {
    match value {
        JsonValue::Null => CstInputValue::Null,
        JsonValue::Boolean(b) => CstInputValue::Bool(*b),
        JsonValue::Number(n) => CstInputValue::Number(n.to_string()),
        JsonValue::String(s) => CstInputValue::String(s.to_string()),
        JsonValue::Array(a) => CstInputValue::Array(a.iter().map(to_cst_input).collect()),
        JsonValue::Object(o) => CstInputValue::Object(
            o.clone()
                .take_inner()
                .iter()
                .map(|(k, v)| (k.to_string(), to_cst_input(v)))
                .collect(),
        ),
    }
}

/// A scalar that keeps its kind is rewritten in place: replacing the node
/// rescans its siblings for formatting, which makes bulk edits quadratic.
fn set_in_place(node: &CstNode, new: &JsonValue) -> bool {
    match (node, new) {
        (CstNode::Leaf(CstLeafNode::NumberLit(n)), JsonValue::Number(v)) => {
            n.set_raw_value(v.to_string())
        }
        (CstNode::Leaf(CstLeafNode::StringLit(n)), JsonValue::String(s)) => {
            n.set_raw_value(escape_string(s))
        }
        (CstNode::Leaf(CstLeafNode::BooleanLit(n)), JsonValue::Boolean(b)) => n.set_value(*b),
        _ => return false,
    }
    true
}

/// The escaping jsonc-parser uses for a new string (`CstStringLit::new_escaped`
/// is private), so an in-place write reads like a replaced one.
fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn replace(node: CstNode, value: CstInputValue) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(text: &str, source: &str, result: &str) -> String {
        write_back(text, source, result).unwrap()
    }

    #[test]
    fn test_unchanged_value_keeps_the_text() {
        let text = "{\n  // comment\n  \"a\": 1.0, /* b */ \"b\": 0x10\n}";
        assert_eq!(
            patch(text, r#"{"a":1.0,"b":16}"#, r#"{"a":1.0,"b":16}"#),
            text
        );
    }

    #[test]
    fn test_nested_change_keeps_comments_and_other_spelling() {
        let text =
            "{\n  // keep\n  \"a\": 1.0, /* b */\n  \"b\": {\"c\": 2, \"d\": \"\\u0041\"}\n}";
        let out = patch(
            text,
            r#"{"a":1.0,"b":{"c":2,"d":"A"}}"#,
            r#"{"a":1.0,"b":{"c":3,"d":"A"}}"#,
        );
        assert!(out.contains("// keep"), "{out}");
        assert!(out.contains("/* b */"), "{out}");
        assert!(out.contains("\"a\": 1.0"), "{out}");
        assert!(out.contains("\"c\": 3"), "{out}");
        assert!(out.contains("\"d\": \"\\u0041\""), "{out}");
    }

    #[test]
    fn test_removed_key_is_deleted_with_its_duplicates() {
        let out = patch(r#"{"a":1,"b":2,"a":3}"#, r#"{"a":3,"b":2}"#, r#"{"b":2}"#);
        assert!(!out.contains("\"a\""), "{out}");
        assert!(out.contains("\"b\":2"), "{out}");
    }

    #[test]
    fn test_new_key_is_appended() {
        let out = patch(r#"{"a":1}"#, r#"{"a":1}"#, r#"{"a":1,"b":{"c":true}}"#);
        let a = out.find("\"a\"").unwrap();
        let b = out.find("\"b\"").unwrap();
        assert!(a < b, "{out}");
        assert!(out.contains("\"c\": true"), "{out}");
    }

    #[test]
    fn test_key_order_change_alone_keeps_the_text() {
        let text = r#"{"a":1,"b":2}"#;
        assert_eq!(patch(text, r#"{"a":1,"b":2}"#, r#"{"b":2,"a":1}"#), text);
    }

    #[test]
    fn test_duplicate_key_writes_the_last_one() {
        assert_eq!(
            patch(r#"{"a":1,"a":2}"#, r#"{"a":2}"#, r#"{"a":5}"#),
            r#"{"a":1,"a":5}"#
        );
    }

    #[test]
    fn test_same_length_array_is_written_per_element() {
        let out = patch("[1, // one\n 2]", "[1,2]", "[1,3]");
        assert!(out.contains("// one"), "{out}");
        assert!(out.contains('3'), "{out}");
    }

    #[test]
    fn test_appended_array_elements_are_added() {
        let out = patch("[\"a\", // x\n \"b\"]", r#"["a","b"]"#, r#"["a","b","c"]"#);
        assert!(out.contains("// x"), "{out}");
        assert!(out.contains("\"c\""), "{out}");
    }

    #[test]
    fn test_other_array_change_replaces_the_array() {
        assert_eq!(patch("[1, 2, 3]", "[1,2,3]", "[3]"), "[3]");
    }

    #[test]
    fn test_type_change_replaces_the_value() {
        let out = patch(r#"{"a": {"b": 1}}"#, r#"{"a":{"b":1}}"#, r#"{"a":1}"#);
        assert_eq!(out, r#"{"a": 1}"#);
    }

    #[test]
    fn test_root_can_be_replaced() {
        assert_eq!(patch(r#"{"a": 1}"#, r#"{"a":1}"#, "1"), "1");
    }

    #[test]
    fn test_numbers_are_written_as_jq_printed_them() {
        let out = patch(r#"{"a": 0x10}"#, r#"{"a":16}"#, r#"{"a":1E+2}"#);
        assert_eq!(out, r#"{"a": 1E+2}"#);
    }

    #[test]
    fn test_same_kind_scalars_are_written_in_place() {
        // In-place writes must look exactly like a replaced node did.
        let text = "{\n  \"n\": 1, // n\n  \"s\": 'x', /* s */\n  \"b\": true\n}";
        let out = patch(
            text,
            r#"{"n":1,"s":"x","b":true}"#,
            r#"{"n":2.50,"s":"say \"hi\"\n\u0001","b":false}"#,
        );
        assert_eq!(
            out,
            "{\n  \"n\": 2.50, // n\n  \"s\": \"say \\\"hi\\\"\\n\\u0001\", /* s */\n  \"b\": false\n}"
        );
    }

    #[test]
    fn test_many_changed_values_are_written() {
        let n = 3000;
        let text = format!(
            "{{\n{}\n}}",
            (0..n)
                .map(|i| format!("  \"k{i}\": {i}, // c"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let source = format!(
            "{{{}}}",
            (0..n)
                .map(|i| format!("\"k{i}\":{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let result = format!(
            "{{{}}}",
            (0..n)
                .map(|i| format!("\"k{i}\":{}", i + 1))
                .collect::<Vec<_>>()
                .join(",")
        );
        let start = std::time::Instant::now();
        let out = patch(&text, &source, &result);
        assert!(
            out.contains("\"k0\": 1, // c") && out.contains(&format!("\"k{}\": {n}, // c", n - 1))
        );
        // Quadratic node replacement took over a second here; in place it is milliseconds.
        assert!(
            start.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            start.elapsed()
        );
    }
}
