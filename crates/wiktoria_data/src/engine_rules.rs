//! Reads `data/engine_rules.json`, the engine constants (one entry per stable `FLD_*` ID).
//! The file is plain JSON and is edited by hand.
//! The engine looks values up by stable field ID instead of hard-coding them.
//! A small built-in JSON reader keeps the crate dependency-free.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

pub fn parse_json(src: &str) -> Result<Json, String> {
    let mut p = P { s: src.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn err<T>(&self, m: &str) -> Result<T, String> {
        Err(format!("{m} at byte {}", self.i))
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("invalid literal")
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = match self.value()? {
                        Json::Str(k) => k,
                        _ => return self.err("object key must be a string"),
                    };
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expected ':'");
                    }
                    self.i += 1;
                    let v = self.value()?;
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' {
                        self.i += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?);
                    match self.s.get(self.i) {
                        Some(b'"') => {
                            self.i += 1;
                            return Ok(Json::Str(out));
                        }
                        Some(b'\\') => {
                            self.i += 1;
                            let c = *self.s.get(self.i).ok_or("unterminated escape")?;
                            self.i += 1;
                            match c {
                                b'"' => out.push('"'),
                                b'\\' => out.push('\\'),
                                b'/' => out.push('/'),
                                b'n' => out.push('\n'),
                                b't' => out.push('\t'),
                                b'r' => out.push('\r'),
                                b'b' => out.push('\u{8}'),
                                b'f' => out.push('\u{c}'),
                                b'u' => {
                                    let hex = |p: &mut Self| -> Result<u32, String> {
                                        let h = p.s.get(p.i..p.i + 4).ok_or("short \\u escape")?;
                                        p.i += 4;
                                        u32::from_str_radix(std::str::from_utf8(h).map_err(|e| e.to_string())?, 16)
                                            .map_err(|e| e.to_string())
                                    };
                                    let mut cp = hex(self)?;
                                    if (0xD800..0xDC00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                        self.i += 2;
                                        let lo = hex(self)?;
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    }
                                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                }
                                _ => return self.err("bad escape"),
                            }
                        }
                        _ => return self.err("unterminated string"),
                    }
                }
            }
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .map(Json::Num)
                    .ok_or_else(|| format!("invalid number at byte {start}"))
            }
            None => self.err("unexpected end of input"),
        }
    }
}

/// The generated model engine_rules: Entities, Fields and Rules keyed by stable ID.
#[derive(Debug)]
pub struct EngineRules {
    pub sha256: String,
    fields: BTreeMap<String, Json>,
}

impl EngineRules {
    pub fn from_json(src: &str) -> Result<Self, String> {
        let root = parse_json(src)?;
        if root.get("schema_version").and_then(Json::as_f64) != Some(1.0) {
            return Err("unsupported engine_rules schema_version (expected 1)".into());
        }
        let sha256 = root.get("source").and_then(|s| s.get("sha256")).and_then(Json::as_str).unwrap_or("").to_string();
        let rows = root
            .get("tables")
            .and_then(|t| t.get("Fields"))
            .and_then(Json::as_array)
            .ok_or("engine_rules has no Fields table")?;
        let mut fields = BTreeMap::new();
        for row in rows {
            let id = row.get("field_id").and_then(Json::as_str).ok_or("field row without field_id")?;
            fields.insert(id.to_string(), row.get("value").cloned().unwrap_or(Json::Null));
        }
        Ok(EngineRules { sha256, fields })
    }

    pub fn field(&self, id: &str) -> Option<&Json> {
        self.fields.get(id)
    }

    pub fn require_str(&self, id: &str) -> Result<&str, String> {
        self.field(id).and_then(Json::as_str).ok_or_else(|| format!("engine_rules field {id} missing or not text"))
    }
}
