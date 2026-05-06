/// ANSI color codes for JSONC token types.
/// Configurable via JQC_COLORS environment variable (colon-separated, 9 fields).
/// Index: [0] null, [1] false, [2] true, [3] numbers, [4] strings, [5] arrays, [6] objects, [7] object-keys, [8] comments
const DEFAULT_COLORS: [&str; 9] = [
    "1;30", // null         — bold dark gray
    "0;39", // false        — default
    "0;39", // true         — default
    "0;39", // numbers      — default
    "0;32", // strings      — green
    "1;39", // arrays       — bold
    "1;39", // objects      — bold
    "34",   // object-keys  — blue
    "0;90", // comments     — dark gray (jqc-specific; jq has no comments)
];

/// JSON/JSONC token kinds used to look up colors from the palette.
pub enum TokenKind {
    Null,
    BoolFalse,
    BoolTrue,
    Number,
    StringValue,
    ObjectKey,
    ArrayBracket,
    ObjectBrace,
    Comment,
}

/// Resolved palette after applying `JQC_COLORS` overrides.
pub struct Palette {
    null: String,
    bool_false: String,
    bool_true: String,
    number: String,
    string: String,
    array: String,
    object: String,
    key: String,
    comment: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            null: DEFAULT_COLORS[0].to_string(),
            bool_false: DEFAULT_COLORS[1].to_string(),
            bool_true: DEFAULT_COLORS[2].to_string(),
            number: DEFAULT_COLORS[3].to_string(),
            string: DEFAULT_COLORS[4].to_string(),
            array: DEFAULT_COLORS[5].to_string(),
            object: DEFAULT_COLORS[6].to_string(),
            key: DEFAULT_COLORS[7].to_string(),
            comment: DEFAULT_COLORS[8].to_string(),
        }
    }
}

impl Palette {
    /// Parse `JQC_COLORS` environment variable and override defaults.
    /// Format: "null:false:true:number:string:array:object:key:comment" (ANSI partial escapes, 9 fields)
    pub fn from_env() -> Self {
        match std::env::var("JQC_COLORS") {
            Ok(val) => Self::from_jqc_colors(&val),
            Err(_) => Self::default(),
        }
    }

    /// Parse a `JQC_COLORS`-formatted string (colon-separated, 9 fields).
    fn from_jqc_colors(s: &str) -> Self {
        let mut p = Palette::default();
        let fields: [&mut String; 9] = [
            &mut p.null,
            &mut p.bool_false,
            &mut p.bool_true,
            &mut p.number,
            &mut p.string,
            &mut p.array,
            &mut p.object,
            &mut p.key,
            &mut p.comment,
        ];
        for (field, part) in fields.into_iter().zip(s.split(':')) {
            if !part.is_empty() {
                *field = part.to_string();
            }
        }
        p
    }

    /// Apply ANSI color for the given token kind. Single source of truth for all colorization.
    pub fn paint_token(&self, kind: TokenKind, text: &str) -> String {
        let code = match kind {
            TokenKind::Null => &self.null,
            TokenKind::BoolFalse => &self.bool_false,
            TokenKind::BoolTrue => &self.bool_true,
            TokenKind::Number => &self.number,
            TokenKind::StringValue => &self.string,
            TokenKind::ObjectKey => &self.key,
            TokenKind::ArrayBracket => &self.array,
            TokenKind::ObjectBrace => &self.object,
            TokenKind::Comment => &self.comment,
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

/// Tracks whether the next string token is an object key or a value.
enum Ctx {
    ObjKey,
    ObjVal,
    Arr,
}

/// Scans a `// …` line comment; advances `i` past the last character before `\n`.
fn scan_line_comment<'a>(bytes: &[u8], text: &'a str, i: &mut usize) -> &'a str {
    let start = *i;
    while *i < bytes.len() && bytes[*i] != b'\n' {
        *i += 1;
    }
    &text[start..*i]
}

/// Scans a `/* … */` block comment; advances `i` past the closing `*/`.
fn scan_block_comment<'a>(bytes: &[u8], text: &'a str, i: &mut usize) -> &'a str {
    let start = *i;
    *i += 2;
    while *i + 1 < bytes.len() && !(bytes[*i] == b'*' && bytes[*i + 1] == b'/') {
        *i += 1;
    }
    if *i + 1 < bytes.len() {
        *i += 2; // consume */
    }
    &text[start..*i]
}

/// Scans a `"…"` string literal (with escape handling); advances `i` past the closing `"`.
fn scan_string<'a>(bytes: &[u8], text: &'a str, i: &mut usize) -> &'a str {
    let start = *i;
    *i += 1;
    while *i < bytes.len() {
        if bytes[*i] == b'\\' {
            *i += 2; // skip escape sequence
            continue;
        }
        if bytes[*i] == b'"' {
            *i += 1;
            break;
        }
        *i += 1;
    }
    &text[start..*i]
}

/// Scans a number (`-` or digit prefix); advances `i` past the last numeric character.
fn scan_number<'a>(bytes: &[u8], text: &'a str, i: &mut usize) -> &'a str {
    let start = *i;
    *i += 1;
    while *i < bytes.len() && matches!(bytes[*i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
        *i += 1;
    }
    &text[start..*i]
}

/// Scans an alphabetic keyword (`true` / `false` / `null`); advances `i` past the word.
fn scan_keyword<'a>(bytes: &[u8], text: &'a str, i: &mut usize) -> &'a str {
    let start = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_alphabetic() {
        *i += 1;
    }
    &text[start..*i]
}

/// Colorize raw JSONC source text with ANSI codes, preserving comments and whitespace.
///
/// Tokenizes byte-by-byte. Safe for UTF-8 because all JSONC structural bytes are ASCII,
/// and multi-byte UTF-8 sequences never share byte values with ASCII.
pub fn colorize_jsonc(text: &str, palette: &Palette) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len * 2);
    let mut i = 0;
    // Stack tracks nesting context to distinguish object keys from string values.
    let mut stack: Vec<Ctx> = Vec::new();

    while i < len {
        match bytes[i] {
            // Whitespace — pass through unchanged (preserves original indentation)
            b' ' | b'\t' | b'\n' | b'\r' => {
                out.push(bytes[i] as char);
                i += 1;
            }
            // Line comment: // … \n
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                let token = scan_line_comment(bytes, text, &mut i);
                out.push_str(&palette.paint_token(TokenKind::Comment, token));
            }
            // Block comment: /* … */
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                let token = scan_block_comment(bytes, text, &mut i);
                out.push_str(&palette.paint_token(TokenKind::Comment, token));
            }
            // String literal
            b'"' => {
                let token = scan_string(bytes, text, &mut i);
                // Paint as key when the current object context expects a key; otherwise as string.
                match stack.last() {
                    Some(Ctx::ObjKey) => {
                        out.push_str(&palette.paint_token(TokenKind::ObjectKey, token));
                        *stack.last_mut().unwrap() = Ctx::ObjVal;
                    }
                    _ => out.push_str(&palette.paint_token(TokenKind::StringValue, token)),
                }
            }
            // Number: starts with '-' or a digit
            b'-' | b'0'..=b'9' => {
                let token = scan_number(bytes, text, &mut i);
                out.push_str(&palette.paint_token(TokenKind::Number, token));
            }
            // Keywords: true / false / null
            b'a'..=b'z' | b'A'..=b'Z' => {
                let token = scan_keyword(bytes, text, &mut i);
                out.push_str(&match token {
                    "null" => palette.paint_token(TokenKind::Null, token),
                    "false" => palette.paint_token(TokenKind::BoolFalse, token),
                    "true" => palette.paint_token(TokenKind::BoolTrue, token),
                    _ => token.to_string(),
                });
            }
            // Structural characters
            b'{' => {
                out.push_str(&palette.paint_token(TokenKind::ObjectBrace, "{"));
                stack.push(Ctx::ObjKey);
                i += 1;
            }
            b'}' => {
                out.push_str(&palette.paint_token(TokenKind::ObjectBrace, "}"));
                stack.pop();
                i += 1;
            }
            b'[' => {
                out.push_str(&palette.paint_token(TokenKind::ArrayBracket, "["));
                stack.push(Ctx::Arr);
                i += 1;
            }
            b']' => {
                out.push_str(&palette.paint_token(TokenKind::ArrayBracket, "]"));
                stack.pop();
                i += 1;
            }
            b':' => {
                out.push(':');
                i += 1;
            }
            b',' => {
                out.push(',');
                // After ',' inside an object, the next string is a key again.
                if let Some(Ctx::ObjVal) = stack.last() {
                    *stack.last_mut().unwrap() = Ctx::ObjKey;
                }
                i += 1;
            }
            _ => {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colorize_value(v: &serde_json::Value, p: &Palette) -> String {
        let pretty = serde_json::to_string_pretty(v).unwrap();
        colorize_jsonc(&pretty, p)
    }

    #[test]
    fn test_colorize_null() {
        let p = Palette::default();
        let out = colorize_value(&serde_json::Value::Null, &p);
        assert!(out.contains("null"), "got: {out}");
        assert!(out.contains("\x1b["), "no ANSI code: {out}");
    }

    #[test]
    fn test_colorize_string() {
        let p = Palette::default();
        let out = colorize_value(&serde_json::json!("hello"), &p);
        assert!(out.contains("\"hello\""), "got: {out}");
    }

    #[test]
    fn test_colorize_number() {
        let p = Palette::default();
        let out = colorize_value(&serde_json::json!(42), &p);
        assert!(out.contains("42"), "got: {out}");
    }

    #[test]
    fn test_colorize_object_has_key_color() {
        let p = Palette::default();
        let out = colorize_value(&serde_json::json!({"port": 3000}), &p);
        assert!(out.contains("\"port\""), "got: {out}");
        assert!(out.contains("3000"), "got: {out}");
    }

    #[test]
    fn test_colorize_array() {
        let p = Palette::default();
        let out = colorize_value(&serde_json::json!([1, 2]), &p);
        assert!(out.contains("1"), "got: {out}");
        assert!(out.contains("2"), "got: {out}");
    }

    #[test]
    fn test_jqc_colors_override() {
        let p = Palette::from_jqc_colors("0;31::::::::");
        assert_eq!(p.null, "0;31");
        assert_eq!(p.string, DEFAULT_COLORS[4]);
    }

    #[test]
    fn test_jqc_colors_partial() {
        let p = Palette::from_jqc_colors("::::0;33::::");
        assert_eq!(p.string, "0;33");
        assert_eq!(p.null, DEFAULT_COLORS[0]);
    }

    #[test]
    fn test_jqc_colors_comment() {
        // 9th field overrides comment color
        let p = Palette::from_jqc_colors("::::::::0;31");
        assert_eq!(p.comment, "0;31");
        assert_eq!(p.null, DEFAULT_COLORS[0]);
    }

    #[test]
    fn test_colorize_jsonc_line_comment() {
        let p = Palette::default();
        let out = colorize_jsonc("{ // comment\n\"port\": 3000 }", &p);
        assert!(out.contains("// comment"), "got: {out}");
        assert!(out.contains("\x1b["), "no ANSI code: {out}");
    }

    #[test]
    fn test_colorize_jsonc_block_comment() {
        let p = Palette::default();
        let out = colorize_jsonc("{ /* hi */ \"port\": 3000 }", &p);
        assert!(out.contains("/* hi */"), "got: {out}");
    }

    #[test]
    fn test_colorize_jsonc_key_differs_from_string_value() {
        let p = Palette::default();
        let out = colorize_jsonc(r#"{"host": "localhost"}"#, &p);
        // key uses DEFAULT_COLORS[7] (blue=34), value uses DEFAULT_COLORS[4] (green=0;32)
        let key_colored = format!("\x1b[{}m\"host\"\x1b[0m", DEFAULT_COLORS[7]);
        let val_colored = format!("\x1b[{}m\"localhost\"\x1b[0m", DEFAULT_COLORS[4]);
        assert!(out.contains(&key_colored), "key color missing: {out}");
        assert!(
            out.contains(&val_colored),
            "string value color missing: {out}"
        );
    }

    #[test]
    fn test_colorize_jsonc_preserves_whitespace() {
        let p = Palette::default();
        let input = "{\n  \"port\": 3000\n}";
        let out = colorize_jsonc(input, &p);
        assert!(out.contains('\n'), "newlines lost");
        assert!(out.contains("  "), "indentation lost");
    }
}
