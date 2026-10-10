use jsonc_parser::tokens::Token;
use jsonc_parser::{Scanner, ScannerOptions};

/// jq 1.8.2's default colors, in `JQ_COLORS` order, then jqc's comment color.
/// Index: [0] null, [1] false, [2] true, [3] numbers, [4] strings, [5] arrays, [6] objects, [7] object-keys, [8] comments
const DEFAULT_COLORS: [&str; 9] = [
    "0;90", // null
    "0;39", // false
    "0;39", // true
    "0;39", // numbers
    "0;32", // strings
    "1;39", // arrays
    "1;39", // objects
    "1;34", // object keys
    "3;90", // comments — italic dark gray (jqc's own; jq has no comments)
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

/// An open container while coloring, and for an object whether a key
/// comes next.
struct Frame {
    object: bool,
    key_next: bool,
}

/// After a value, an object expects a key next, with or without a comma
/// (jsonc-parser accepts a missing one).
fn value_done(stack: &mut [Frame]) {
    if let Some(frame) = stack.last_mut() {
        frame.key_next = frame.object;
    }
}

/// Colorize JSONC source text the way `jq -C` colors JSON, keeping
/// comments and whitespace. The tokens come from jsonc-parser's scanner, so
/// everything `fmt` accepts (single quotes, unquoted keys, hex numbers) is
/// colored by what it is; the text between tokens is copied as it is.
pub fn colorize_jsonc(text: &str, palette: &Palette) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut scanner = Scanner::new(text, &ScannerOptions::default());
    let mut stack: Vec<Frame> = Vec::new();
    let mut copied = 0;
    // fmt and --edit check the text first; after a scan error, the rest is
    // copied uncolored.
    while let Ok(Some(token)) = scanner.scan() {
        let start = scanner.token_start();
        let mut end = scanner.token_end();
        out.push_str(&text[copied..start]);
        let key = stack.last().is_some_and(|f| f.object && f.key_next);
        let kind = match token {
            Token::CommentLine(_) | Token::CommentBlock(_) => Some(TokenKind::Comment),
            Token::OpenBrace | Token::OpenBracket => {
                let object = token == Token::OpenBrace;
                let close = if object { b'}' } else { b']' };
                if text.as_bytes().get(end) == Some(&close) {
                    // jq prints an empty container as one token.
                    let _ = scanner.scan();
                    end = scanner.token_end();
                    value_done(&mut stack);
                } else {
                    stack.push(Frame {
                        object,
                        key_next: object,
                    });
                }
                Some(if object {
                    TokenKind::ObjectBrace
                } else {
                    TokenKind::ArrayBracket
                })
            }
            Token::CloseBrace | Token::CloseBracket => {
                stack.pop();
                value_done(&mut stack);
                Some(if token == Token::CloseBrace {
                    TokenKind::ObjectBrace
                } else {
                    TokenKind::ArrayBracket
                })
            }
            // As in jq, `:` takes the object's color and `,` its container's.
            Token::Colon => Some(TokenKind::ObjectBrace),
            Token::Comma => stack.last_mut().map(|frame| {
                frame.key_next = frame.object;
                if frame.object {
                    TokenKind::ObjectBrace
                } else {
                    TokenKind::ArrayBracket
                }
            }),
            _ if key => {
                if let Some(frame) = stack.last_mut() {
                    frame.key_next = false;
                }
                Some(TokenKind::ObjectKey)
            }
            Token::String(_) => {
                value_done(&mut stack);
                Some(TokenKind::StringValue)
            }
            Token::Number(_) => {
                value_done(&mut stack);
                Some(TokenKind::Number)
            }
            Token::Boolean(true) => {
                value_done(&mut stack);
                Some(TokenKind::BoolTrue)
            }
            Token::Boolean(false) => {
                value_done(&mut stack);
                Some(TokenKind::BoolFalse)
            }
            Token::Null => {
                value_done(&mut stack);
                Some(TokenKind::Null)
            }
            Token::Word(_) => {
                value_done(&mut stack);
                None
            }
        };
        let raw = &text[start..end];
        match kind {
            Some(kind) => out.push_str(&palette.paint_token(kind, raw)),
            None => out.push_str(raw),
        }
        copied = end;
    }
    out.push_str(&text[copied..]);
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
        // key uses DEFAULT_COLORS[7], value uses DEFAULT_COLORS[4]
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

    fn paint(code: &str, text: &str) -> String {
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    #[test]
    fn test_colorize_matches_jq_compact_output() {
        // `jq -C -c .` (jq 1.8.2, JQ_COLORS unset) on the same input.
        let input = r#"{"a":[1,null,"s",true,false],"b":{},"c":[],"d":{"e":0}}"#;
        let expected = "\x1b[1;39m{\x1b[0m\x1b[1;34m\"a\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[\x1b[0m\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0m\x1b[0;90mnull\x1b[0m\x1b[1;39m,\x1b[0m\x1b[0;32m\"s\"\x1b[0m\x1b[1;39m,\x1b[0m\x1b[0;39mtrue\x1b[0m\x1b[1;39m,\x1b[0m\x1b[0;39mfalse\x1b[0m\x1b[1;39m]\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"b\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{}\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"c\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m[]\x1b[0m\x1b[1;39m,\x1b[0m\x1b[1;34m\"d\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[1;39m{\x1b[0m\x1b[1;34m\"e\"\x1b[0m\x1b[1;39m:\x1b[0m\x1b[0;39m0\x1b[0m\x1b[1;39m}\x1b[0m\x1b[1;39m}\x1b[0m";
        assert_eq!(colorize_jsonc(input, &Palette::default()), expected);
    }

    #[test]
    fn test_colorize_separators_take_their_container_color() {
        // `JQ_COLORS=':::::4;36:7;37:1;31' jq -C -c .` (jq 1.8.2) on the same input:
        // fields 1-5 are empty, so numbers are `\x1b[m`.
        let p = Palette {
            number: String::new(),
            array: "4;36".into(),
            object: "7;37".into(),
            key: "1;31".into(),
            ..Palette::default()
        };
        let expected = "\x1b[7;37m{\x1b[0m\x1b[1;31m\"a\"\x1b[0m\x1b[7;37m:\x1b[0m\x1b[4;36m[\x1b[0m\x1b[m1\x1b[0m\x1b[4;36m,\x1b[0m\x1b[7;37m{}\x1b[0m\x1b[4;36m]\x1b[0m\x1b[7;37m}\x1b[0m";
        assert_eq!(colorize_jsonc(r#"{"a":[1,{}]}"#, &p), expected);
    }

    #[test]
    fn test_colorize_keeps_whitespace_and_comments() {
        let p = Palette::default();
        let input = "{\n  // c\n  \"a\": [ ], /* b */\n}";
        let expected = format!(
            "{}\n  {}\n  {}{} {} {}{} {}\n{}",
            paint("1;39", "{"),
            paint("3;90", "// c"),
            paint("1;34", "\"a\""),
            paint("1;39", ":"),
            paint("1;39", "["),
            paint("1;39", "]"),
            paint("1;39", ","),
            paint("3;90", "/* b */"),
            paint("1;39", "}"),
        );
        assert_eq!(colorize_jsonc(input, &p), expected);
    }

    #[test]
    fn test_colorize_loose_jsonc() {
        let p = Palette::default();
        // Unquoted, number and literal keys; single quotes; hex.
        let out = colorize_jsonc("{foo: 'bar', 1: true, null: 0x1F}", &p);
        assert!(out.contains(&paint("1;34", "foo")), "{out}");
        assert!(out.contains(&paint("0;32", "'bar'")), "{out}");
        assert!(out.contains(&paint("1;34", "1")), "{out}");
        assert!(out.contains(&paint("0;39", "true")), "{out}");
        assert!(out.contains(&paint("1;34", "null")), "{out}");
        assert!(out.contains(&paint("0;39", "0x1F")), "{out}");
        // `//` inside a single-quoted string is part of the string.
        let out = colorize_jsonc("{'k': 'v // x'}", &p);
        assert!(out.contains(&paint("0;32", "'v // x'")), "{out}");
        // A missing comma still starts the next key.
        let out = colorize_jsonc(r#"{"a":1 "b":"c"}"#, &p);
        assert!(out.contains(&paint("1;34", "\"b\"")), "{out}");
        assert!(out.contains(&paint("0;32", "\"c\"")), "{out}");
        // The key after a nested container, and a key after a comment.
        let out = colorize_jsonc("{\"a\":{\"b\":1},/*c*/\"d\":2}", &p);
        assert!(out.contains(&paint("1;34", "\"d\"")), "{out}");
    }

    #[test]
    fn test_colorize_keeps_non_ascii_text() {
        let out = colorize_jsonc("{日本: \"語\"}", &Palette::default());
        assert!(out.contains(&paint("1;34", "日本")), "{out}");
        assert!(out.contains(&paint("0;32", "\"語\"")), "{out}");
    }
}
