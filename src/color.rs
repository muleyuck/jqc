/// ANSI color codes for JSON token types, matching jq's default palette.
/// Order: null, false, true, numbers, strings, arrays, objects, object-keys
const DEFAULT_COLORS: [&str; 8] = [
    "1;30", // null       — bold dark gray
    "0;39", // false      — default
    "0;39", // true       — default
    "0;39", // numbers    — default
    "0;32", // strings    — green
    "1;39", // arrays     — bold
    "1;39", // objects    — bold
    "34",   // object-keys — blue
];

/// Resolved palette after applying `JQ_COLORS` overrides.
pub struct Palette {
    null: String,
    bool_false: String,
    bool_true: String,
    number: String,
    string: String,
    array: String,
    object: String,
    key: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            null:       DEFAULT_COLORS[0].to_string(),
            bool_false: DEFAULT_COLORS[1].to_string(),
            bool_true:  DEFAULT_COLORS[2].to_string(),
            number:     DEFAULT_COLORS[3].to_string(),
            string:     DEFAULT_COLORS[4].to_string(),
            array:      DEFAULT_COLORS[5].to_string(),
            object:     DEFAULT_COLORS[6].to_string(),
            key:        DEFAULT_COLORS[7].to_string(),
        }
    }
}

impl Palette {
    /// Parse `JQ_COLORS` environment variable and override defaults.
    /// Format: "null:false:true:number:string:array:object:key" (ANSI partial escapes)
    pub fn from_env() -> Self {
        match std::env::var("JQ_COLORS") {
            Ok(val) => Self::from_jq_colors(&val),
            Err(_) => Self::default(),
        }
    }

    /// Parse a `JQ_COLORS`-formatted string (colon-separated, 8 fields).
    fn from_jq_colors(s: &str) -> Self {
        let mut p = Palette::default();
        let fields: [&mut String; 8] = [
            &mut p.null, &mut p.bool_false, &mut p.bool_true,
            &mut p.number, &mut p.string, &mut p.array,
            &mut p.object, &mut p.key,
        ];
        for (field, part) in fields.into_iter().zip(s.split(':')) {
            if !part.is_empty() {
                *field = part.to_string();
            }
        }
        p
    }

    fn paint(&self, code: &str, text: &str) -> String {
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

/// Colorize a `serde_json::Value` into a pretty-printed string with ANSI codes.
pub fn colorize(value: &serde_json::Value, indent: usize, palette: &Palette) -> String {
    match value {
        serde_json::Value::Null => palette.paint(&palette.null, "null"),
        serde_json::Value::Bool(false) => palette.paint(&palette.bool_false, "false"),
        serde_json::Value::Bool(true)  => palette.paint(&palette.bool_true, "true"),
        serde_json::Value::Number(n)   => palette.paint(&palette.number, &n.to_string()),
        serde_json::Value::String(s)   => {
            palette.paint(&palette.string, &format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return palette.paint(&palette.array, "[]");
            }
            let inner_indent = " ".repeat((indent + 1) * 2);
            let close_indent = " ".repeat(indent * 2);
            let items: Vec<String> = arr
                .iter()
                .map(|v| format!("{}{}", inner_indent, colorize(v, indent + 1, palette)))
                .collect();
            format!(
                "{}\n{}\n{}{}",
                palette.paint(&palette.array, "["),
                items.join(",\n"),
                close_indent,
                palette.paint(&palette.array, "]"),
            )
        }
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return palette.paint(&palette.object, "{}");
            }
            let inner_indent = " ".repeat((indent + 1) * 2);
            let close_indent = " ".repeat(indent * 2);
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = palette.paint(&palette.key, &format!("\"{}\"", k));
                    let val = colorize(v, indent + 1, palette);
                    format!("{}{}: {}", inner_indent, key, val)
                })
                .collect();
            format!(
                "{}\n{}\n{}{}",
                palette.paint(&palette.object, "{"),
                items.join(",\n"),
                close_indent,
                palette.paint(&palette.object, "}"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_colorize_null() {
        let p = Palette::default();
        let out = colorize(&json!(null), 0, &p);
        assert!(out.contains("null"), "got: {out}");
        assert!(out.contains("\x1b["), "no ANSI code: {out}");
    }

    #[test]
    fn test_colorize_string() {
        let p = Palette::default();
        let out = colorize(&json!("hello"), 0, &p);
        assert!(out.contains("\"hello\""), "got: {out}");
    }

    #[test]
    fn test_colorize_number() {
        let p = Palette::default();
        let out = colorize(&json!(42), 0, &p);
        assert!(out.contains("42"), "got: {out}");
    }

    #[test]
    fn test_colorize_object_has_key_color() {
        let p = Palette::default();
        let out = colorize(&json!({"port": 3000}), 0, &p);
        assert!(out.contains("\"port\""), "got: {out}");
        assert!(out.contains("3000"), "got: {out}");
    }

    #[test]
    fn test_colorize_array() {
        let p = Palette::default();
        let out = colorize(&json!([1, 2]), 0, &p);
        assert!(out.contains("1"), "got: {out}");
        assert!(out.contains("2"), "got: {out}");
    }

    #[test]
    fn test_jq_colors_override() {
        let p = Palette::from_jq_colors("0;31:::::::");
        assert_eq!(p.null, "0;31");
        assert_eq!(p.string, DEFAULT_COLORS[4]);
    }

    #[test]
    fn test_jq_colors_partial() {
        let p = Palette::from_jq_colors("::::0;33:::");
        assert_eq!(p.string, "0;33");
        assert_eq!(p.null, DEFAULT_COLORS[0]);
    }
}
