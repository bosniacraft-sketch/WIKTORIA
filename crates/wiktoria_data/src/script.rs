//! Parser for Victoria 2's Clausewitz script format (`key = value`, `key = { ... }`).
//!
//! Vic2 text files are Windows-1252 encoded, use `#` line comments, and allow
//! bare values inside blocks (e.g. `color = { 208 202 127 }`). Some shipped and
//! modded files contain a stray closing brace at top level; that is tolerated and
//! reported as a warning rather than treated as fatal.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Scalar(String),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `None` for bare entries such as the numbers in `{ 1 2 3 }`.
    pub key: Option<String>,
    pub value: Value,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub struct Document {
    pub root: Block,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Windows-1252 → UTF-8. Bytes 0x80..=0x9F map to the CP1252 extras; the rest are Latin-1.
pub fn decode_windows1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Open,
    Close,
    Eq,
}

fn tokenize(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    let mut line = 1;
    while let Some(&c) = chars.peek() {
        match c {
            '\n' => {
                line += 1;
                chars.next();
            }
            c if c.is_whitespace() || c == '\u{feff}' => {
                chars.next();
            }
            '#' => {
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '{' => {
                chars.next();
                out.push((Tok::Open, line));
            }
            '}' => {
                chars.next();
                out.push((Tok::Close, line));
            }
            '=' => {
                chars.next();
                out.push((Tok::Eq, line));
            }
            '"' => {
                let start = line;
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\n') => {
                            line += 1;
                            s.push('\n');
                        }
                        Some(c) => s.push(c),
                        None => {
                            return Err(ParseError { line: start, message: "unterminated quoted string".into() });
                        }
                    }
                }
                out.push((Tok::Quoted(s), start));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '{' | '}' | '=' | '#' | '"') {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push((Tok::Word(s), line));
            }
        }
    }
    Ok(out)
}

pub fn parse(src: &str) -> Result<Document, ParseError> {
    let toks = tokenize(src)?;
    let mut pos = 0;
    let mut warnings = Vec::new();
    let mut root = Block::default();
    loop {
        parse_items(&toks, &mut pos, &mut root, true)?;
        if pos < toks.len() {
            // Only a stray `}` at top level can stop parse_items early.
            warnings.push(format!("line {}: stray closing brace ignored", toks[pos].1));
            pos += 1;
        } else {
            break;
        }
    }
    Ok(Document { root, warnings })
}

pub fn parse_bytes(bytes: &[u8]) -> Result<Document, ParseError> {
    parse(&decode_windows1252(bytes))
}

fn parse_items(toks: &[(Tok, usize)], pos: &mut usize, block: &mut Block, top: bool) -> Result<(), ParseError> {
    while *pos < toks.len() {
        let (tok, line) = &toks[*pos];
        let line = *line;
        match tok {
            Tok::Close => {
                if top {
                    return Ok(());
                }
                *pos += 1;
                return Ok(());
            }
            Tok::Eq => return Err(ParseError { line, message: "'=' without a key".into() }),
            Tok::Open => {
                *pos += 1;
                let inner = parse_block_body(toks, pos, line)?;
                block.items.push(Item { key: None, value: Value::Block(inner), line });
            }
            Tok::Word(w) | Tok::Quoted(w) => {
                let w = w.clone();
                *pos += 1;
                if matches!(toks.get(*pos), Some((Tok::Eq, _))) {
                    *pos += 1;
                    let value = match toks.get(*pos) {
                        Some((Tok::Open, l)) => {
                            let l = *l;
                            *pos += 1;
                            Value::Block(parse_block_body(toks, pos, l)?)
                        }
                        Some((Tok::Word(v), _)) | Some((Tok::Quoted(v), _)) => {
                            let v = v.clone();
                            *pos += 1;
                            Value::Scalar(v)
                        }
                        _ => return Err(ParseError { line, message: format!("missing value after '{w} ='") }),
                    };
                    block.items.push(Item { key: Some(w), value, line });
                } else {
                    block.items.push(Item { key: None, value: Value::Scalar(w), line });
                }
            }
        }
    }
    if top {
        Ok(())
    } else {
        Err(ParseError { line: toks.last().map(|t| t.1).unwrap_or(1), message: "unclosed '{'".into() })
    }
}

fn parse_block_body(toks: &[(Tok, usize)], pos: &mut usize, open_line: usize) -> Result<Block, ParseError> {
    let mut b = Block::default();
    parse_items(toks, pos, &mut b, false).map_err(|e| {
        if e.message == "unclosed '{'" {
            ParseError { line: open_line, message: "unclosed '{' opened here".into() }
        } else {
            e
        }
    })?;
    Ok(b)
}

impl Block {
    /// Keyed entries in file order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Value, usize)> {
        self.items.iter().filter_map(|i| i.key.as_deref().map(|k| (k, &i.value, i.line)))
    }

    /// Last value for `key` (Clausewitz convention: later assignments win).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().rev().find(|i| i.key.as_deref() == Some(key)).map(|i| &i.value)
    }

    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.items.iter().filter(move |i| i.key.as_deref() == Some(key)).map(|i| &i.value)
    }

    /// Bare scalar values, e.g. the three numbers in `color = { 1 2 3 }`.
    pub fn bare_scalars(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter(|i| i.key.is_none())
            .filter_map(|i| i.value.as_str())
            .collect()
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Scalar(s) => Some(s),
            Value::Block(_) => None,
        }
    }

    pub fn as_block(&self) -> Option<&Block> {
        match self {
            Value::Block(b) => Some(b),
            Value::Scalar(_) => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_str()?.parse().ok()
    }

    /// Clausewitz booleans are `yes` / `no`.
    pub fn as_bool(&self) -> Option<bool> {
        match self.as_str()? {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        }
    }
}
