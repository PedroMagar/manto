// Minimal zero-dependency JSON parser (shared by menu, session and config).
// `serde` stays commented in Cargo.toml, matching the portability policy of
// ARCHITECTURE.md.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn field<'a>(&'a self, key: &str) -> Option<&'a Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str_value(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

/// Parse a full JSON document, failing on trailing data.
pub fn parse(source: &str) -> Result<Json, String> {
    let mut parser = JsonParser::new(source);
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != source.len() {
        return Err(parser.error("trailing data after JSON value"));
    }
    Ok(value)
}

pub struct JsonParser<'a> {
    pub src: &'a [u8],
    pub pos: usize,
}

impl<'a> JsonParser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { src: source.as_bytes(), pos: 0 }
    }

    pub fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek();
        if byte.is_some() {
            self.pos += 1;
        }
        byte
    }

    pub fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    pub fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.parse_number(),
            _ => Err(self.error("unexpected token")),
        }
    }

    fn parse_literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.bump(); // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(self.error("unterminated array")),
                Some(b']') => {
                    self.bump();
                    return Ok(Json::Arr(items));
                }
                _ => {
                    items.push(self.parse_value()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.bump();
                        }
                        Some(b']') => {}
                        _ => return Err(self.error("expected ',' or ']'")),
                    }
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.bump(); // '{'
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(self.error("unterminated object")),
                Some(b'}') => {
                    self.bump();
                    return Ok(Json::Obj(fields));
                }
                Some(b'"') => {
                    let key = self.parse_string()?;
                    self.skip_ws();
                    if self.bump() != Some(b':') {
                        return Err(self.error("expected ':'"));
                    }
                    let value = self.parse_value()?;
                    fields.push((key, value));
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.bump();
                        }
                        Some(b'}') => {}
                        _ => return Err(self.error("expected ',' or '}'")),
                    }
                }
                _ => return Err(self.error("expected string key")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bump() != Some(b'"') {
            return Err(self.error("expected string"));
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    None => return Err(self.error("unterminated escape")),
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{0008}'),
                    Some(b'f') => out.push('\u{000c}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let cp = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                                return Err(self.error("expected low surrogate"));
                            }
                            let low = self.parse_hex4()?;
                            let cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                            out.push(char::from_u32(cp)
                                .ok_or_else(|| self.error("invalid surrogate pair"))?);
                        } else {
                            out.push(char::from_u32(cp)
                                .ok_or_else(|| self.error("invalid unicode escape"))?);
                        }
                    }
                    _ => return Err(self.error("invalid escape")),
                },
                Some(byte) if byte < 0x20 => return Err(self.error("control char in string")),
                Some(byte) => {
                    let extra: usize = match byte {
                        0x00..=0x7F => 0,
                        0xC2..=0xDF => 1,
                        0xE0..=0xEF => 2,
                        0xF0..=0xF4 => 3,
                        _ => return Err(self.error("invalid UTF-8 in string")),
                    };
                    let mut buf = vec![byte];
                    for _ in 0..extra {
                        buf.push(self.bump().ok_or_else(|| self.error("unterminated string"))?);
                    }
                    out.push_str(std::str::from_utf8(&buf)
                        .map_err(|_| self.error("invalid UTF-8 in string"))?);
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let byte = self.bump().ok_or_else(|| self.error("unterminated unicode escape"))?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(self.error("invalid hex digit")),
            };
            value = value * 16 + digit as u32;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.error("invalid number"))?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.error("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_json_value(source: &str) -> Json {
        let mut parser = JsonParser::new(source);
        let value = parser.parse_value().unwrap();
        parser.skip_ws();
        assert_eq!(parser.pos, source.len(), "trailing data in {source:?}");
        value
    }

    #[test]
    fn json_primitives_parse() {
        let value = parse_json_value(r#"{"a": 1, "b": [true, false, null], "c": "x"}"#);
        let Json::Obj(fields) = value else { panic!("expected object") };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("a".to_string(), Json::Num(1.0)));
        assert_eq!(
            fields[1],
            ("b".to_string(), Json::Arr(vec![Json::Bool(true), Json::Bool(false), Json::Null]))
        );
        assert_eq!(fields[2], ("c".to_string(), Json::Str("x".to_string())));
    }

    #[test]
    fn json_string_handles_escapes_and_unicode() {
        let source = r#""tab\t quote\" slash\\ uni \u00e7\u00e3 emoji \ud83d\ude00""#;
        let Json::Str(text) = parse_json_value(source) else { panic!("expected string") };
        assert_eq!(text, "tab\t quote\" slash\\ uni çã emoji 😀");
    }

    #[test]
    fn json_accepts_trailing_commas() {
        let source = r#"{"a": 1, "b": [1, 2, ], }"#;
        let Json::Obj(fields) = parse_json_value(source) else { panic!("expected object") };
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn json_rejects_garbage() {
        for bad in ["tru", "{", "[1,", "\"ab", "{1: 2}", "\"bad \x01 ctrl\""] {
            let mut parser = JsonParser::new(bad);
            assert!(parser.parse_value().is_err(), "{bad:?} should fail");
        }
        assert!(parse("{} trailing").is_err());
        assert!(parse("[1] 2").is_err());
    }

    #[test]
    fn json_field_accessors() {
        let source = r#"{"theme": 1, "name": "manto", "on": true, "items": [1, 2]}"#;
        let value = parse_json_value(source);
        assert_eq!(value.field("theme").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(value.field("name").and_then(|v| v.str_value()), Some("manto"));
        assert_eq!(value.field("on").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(value.field("items").and_then(|v| v.as_arr()).map(|a| a.len()), Some(2));
        assert_eq!(value.field("missing"), None);
    }
}