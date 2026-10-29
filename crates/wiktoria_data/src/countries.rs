//! `common/countries.txt` (tag → file) and each `common/countries/*.txt` (colour etc.).

use crate::script::{self, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Country {
    pub tag: String,
    /// Path relative to `common/`, as written in countries.txt.
    pub file: String,
    pub color: [u8; 3],
    pub graphical_culture: Option<String>,
    pub dynamic: bool,
    /// Political parties defined in the country file, in file order.
    pub parties: Vec<Party>,
}

/// A `party = { ... }` block of a country file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Party {
    pub name: String,
    pub ideology: Option<String>,
    pub start_date: Option<crate::date::Date>,
    pub end_date: Option<crate::date::Date>,
    /// Party issue group (e.g. `trade_policy`) -> chosen option.
    pub policies: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountryEntry {
    pub tag: String,
    pub file: String,
    /// Listed after `dynamic_tags = yes` (formable/releasable tags).
    pub dynamic: bool,
}

/// Returns entries in file order. A later duplicate tag is reported and ignored (first wins).
pub fn parse_country_list(src: &str) -> Result<(Vec<CountryEntry>, Vec<String>), String> {
    let doc = script::parse(src).map_err(|e| e.to_string())?;
    let mut out: Vec<CountryEntry> = Vec::new();
    let mut warnings = doc.warnings.clone();
    let mut dynamic = false;
    for (tag, value, line) in doc.root.entries() {
        if tag == "dynamic_tags" {
            dynamic = value.as_bool().unwrap_or(false);
            continue;
        }
        let Some(file) = value.as_str() else {
            warnings.push(format!("countries.txt line {line}: '{tag}' is not a file path"));
            continue;
        };
        if out.iter().any(|e| e.tag == tag) {
            warnings.push(format!("countries.txt line {line}: duplicate tag {tag} ignored"));
            continue;
        }
        out.push(CountryEntry { tag: tag.to_string(), file: file.to_string(), dynamic });
    }
    Ok((out, warnings))
}

pub fn parse_country_file(entry: &CountryEntry, src: &str) -> Result<Country, String> {
    let (tag, file) = (&entry.tag, &entry.file);
    let doc = script::parse(src).map_err(|e| e.to_string())?;
    let color = match doc.root.get("color").and_then(Value::as_block) {
        Some(b) => {
            let c: Vec<u8> = b.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect();
            if c.len() != 3 {
                return Err(format!("{file}: color must be three 0-255 integers"));
            }
            [c[0], c[1], c[2]]
        }
        None => return Err(format!("{file}: no color")),
    };
    Ok(Country {
        tag: tag.to_string(),
        file: file.to_string(),
        color,
        graphical_culture: doc.root.get("graphical_culture").and_then(Value::as_str).map(str::to_string),
        dynamic: entry.dynamic,
        parties: doc.root.get_all("party").filter_map(Value::as_block).map(parse_party).collect(),
    })
}

fn parse_party(b: &script::Block) -> Party {
    let text = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_string);
    let date = |k: &str| b.get(k).and_then(Value::as_str).and_then(crate::date::Date::parse);
    Party {
        name: text("name").unwrap_or_default(),
        ideology: text("ideology"),
        start_date: date("start_date"),
        end_date: date("end_date"),
        policies: b
            .entries()
            .filter(|(k, _, _)| !matches!(*k, "name" | "ideology" | "start_date" | "end_date"))
            .filter_map(|(k, v, _)| Some((k.to_string(), v.as_str()?.to_string())))
            .collect(),
    }
}
