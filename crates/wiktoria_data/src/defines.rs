//! `common/defines.lua`: a Lua table of named constants.
//!
//! The file only uses a flat subset of Lua (nested tables, `key = value,`, `--` comments,
//! single-quoted dates), so it is rewritten into Clausewitz syntax and parsed with
//! [`crate::script`] instead of embedding a Lua interpreter.

use crate::date::Date;
use crate::script::{self, Block, Value};

#[derive(Debug)]
pub struct Defines {
    root: Block,
}

fn lua_to_script(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let mut in_quote = None;
        let mut prev = '\0';
        for c in line.chars() {
            match (in_quote, c) {
                (None, '-') if prev == '-' => {
                    out.pop();
                    break;
                }
                (None, '\'') | (None, '"') => {
                    in_quote = Some(c);
                    out.push('"');
                }
                (Some(q), c) if c == q => {
                    in_quote = None;
                    out.push('"');
                }
                (None, ',') => out.push(' '),
                _ => out.push(c),
            }
            prev = c;
        }
        out.push('\n');
    }
    out
}

impl Defines {
    pub fn parse(src: &str) -> Result<Self, String> {
        let doc = script::parse(&lua_to_script(src)).map_err(|e| e.to_string())?;
        let root = doc
            .root
            .get("defines")
            .and_then(Value::as_block)
            .cloned()
            .ok_or("defines.lua has no top-level `defines` table")?;
        Ok(Defines { root })
    }

    fn date(&self, key: &str) -> Result<Date, String> {
        let raw = self.root.get(key).and_then(Value::as_str).ok_or(format!("defines.{key} missing"))?;
        Date::parse(raw).ok_or(format!("defines.{key} = '{raw}' is not a date"))
    }

    pub fn start_date(&self) -> Result<Date, String> {
        self.date("start_date")
    }

    pub fn end_date(&self) -> Result<Date, String> {
        self.date("end_date")
    }

    /// A numeric constant such as `get("economy", "MAX_FACTORY_LEVEL")`.
    pub fn get(&self, section: &str, key: &str) -> Option<f64> {
        self.root.get(section)?.as_block()?.get(key)?.as_f64()
    }

    /// Number of constants per section, for diagnostics.
    pub fn sections(&self) -> Vec<(&str, usize)> {
        self.root
            .entries()
            .filter_map(|(k, v, _)| v.as_block().map(|b| (k, b.entries().count())))
            .collect()
    }
}
