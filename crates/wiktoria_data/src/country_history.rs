//! Country history: `history/countries/<TAG> - <Name>.txt`, evaluated at a date.
//! Keys are either fixed fields (capital, government, ...), a reform group from issues.txt
//! (`slavery = no_slavery`) or a technology (`flintlock_rifles = 1`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::date::Date;
use crate::script::{Block, Document, Value};

/// What the loader needs to know to classify keys.
pub struct Classifier {
    /// Issue group name → its options.
    pub issue_groups: HashMap<String, Vec<String>>,
    pub technologies: HashSet<String>,
    pub inventions: HashSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CountryHistory {
    pub capital: Option<u16>,
    pub primary_culture: Option<String>,
    /// Accepted cultures (`culture = x`, repeatable).
    pub cultures: Vec<String>,
    pub religion: Option<String>,
    pub government: Option<String>,
    pub plurality: Option<f64>,
    pub national_value: Option<String>,
    pub literacy: Option<f64>,
    pub non_state_culture_literacy: Option<f64>,
    pub civilized: Option<bool>,
    pub prestige: Option<f64>,
    pub consciousness: Option<f64>,
    pub nonstate_consciousness: Option<f64>,
    pub ruling_party: Option<String>,
    pub last_election: Option<Date>,
    /// Ideology → share of the upper house.
    pub upper_house: BTreeMap<String, f64>,
    /// Issue group → chosen option.
    pub reforms: BTreeMap<String, String>,
    pub technologies: BTreeSet<String>,
    /// Inventions set in history (`silver_standard = yes`).
    pub inventions: BTreeSet<String>,
    pub decisions: Vec<String>,
    /// Order-of-battle file (armies), relative to history/units.
    pub oob: Option<String>,
    pub schools: Option<String>,
    pub is_releasable_vassal: Option<bool>,
    /// Tag → amount (`foreign_investment = { BEL = 10000 }`).
    pub foreign_investment: BTreeMap<String, f64>,
    /// (government, flag) overrides: which flag variant a government type uses.
    pub govt_flags: Vec<(String, String)>,
    pub colonial_points: Option<f64>,
    pub country_flags: BTreeSet<String>,
}

/// `"ENG - United Kingdom.txt"` → `"ENG"`.
pub fn tag_from_filename(name: &str) -> Option<String> {
    let tag: String = name.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    (tag.len() == 3).then_some(tag)
}

impl CountryHistory {
    pub fn apply(&mut self, doc: &Document, date: Date, cls: &Classifier, warnings: &mut Vec<String>, label: &str) {
        let mut dated: Vec<(Date, &Block)> = Vec::new();
        for (key, value, line) in doc.root.entries() {
            match (Date::parse(key), value.as_block()) {
                (Some(d), Some(b)) => dated.push((d, b)),
                (Some(_), None) => warnings.push(format!("{label} line {line}: dated entry {key} is not a block")),
                _ => self.apply_key(key, value, cls, warnings, label, line),
            }
        }
        dated.sort_by_key(|(d, _)| *d);
        for (d, block) in dated {
            if d <= date {
                for (key, value, line) in block.entries() {
                    self.apply_key(key, value, cls, warnings, label, line);
                }
            }
        }
    }

    fn apply_key(&mut self, key: &str, value: &Value, cls: &Classifier, warnings: &mut Vec<String>, label: &str, line: usize) {
        let text = || value.as_str().map(str::to_string);
        let num = || value.as_f64();
        match key {
            "capital" => self.capital = num().map(|v| v as u16),
            "primary_culture" => self.primary_culture = text(),
            "culture" => {
                if let Some(c) = text()
                    && !self.cultures.contains(&c)
                {
                    self.cultures.push(c);
                }
            }
            "religion" => self.religion = text(),
            "government" => self.government = text(),
            "plurality" => self.plurality = num(),
            "nationalvalue" => self.national_value = text(),
            "literacy" => self.literacy = num(),
            "non_state_culture_literacy" => self.non_state_culture_literacy = num(),
            "civilized" => self.civilized = value.as_bool(),
            "prestige" => self.prestige = num(),
            "consciousness" => self.consciousness = num(),
            "nonstate_consciousness" => self.nonstate_consciousness = num(),
            "ruling_party" => self.ruling_party = text(),
            "last_election" => self.last_election = value.as_str().and_then(Date::parse),
            "upper_house" => match value.as_block() {
                Some(b) => {
                    self.upper_house = b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect();
                }
                None => warnings.push(format!("{label} line {line}: upper_house is not a block")),
            },
            "decision" => {
                if let Some(d) = text() {
                    self.decisions.push(d);
                }
            }
            "oob" => self.oob = text(),
            "schools" => self.schools = text(),
            "is_releasable_vassal" => self.is_releasable_vassal = value.as_bool(),
            "colonial_points" => self.colonial_points = num(),
            "set_country_flag" => {
                if let Some(f) = text() {
                    self.country_flags.insert(f);
                }
            }
            "foreign_investment" => match value.as_block() {
                Some(b) => {
                    for (tag, v, _) in b.entries() {
                        if let Some(x) = v.as_f64() {
                            self.foreign_investment.insert(tag.to_string(), x);
                        }
                    }
                }
                None => warnings.push(format!("{label} line {line}: foreign_investment is not a block")),
            },
            "govt_flag" => {
                let b = value.as_block();
                let field = |k: &str| b.and_then(|b| b.get(k)).and_then(Value::as_str).map(str::to_string);
                match (field("government"), field("flag")) {
                    (Some(g), Some(f)) => self.govt_flags.push((g, f)),
                    _ => warnings.push(format!("{label} line {line}: govt_flag needs government and flag")),
                }
            }
            _ => {
                if let Some(options) = cls.issue_groups.get(key) {
                    match value.as_str() {
                        Some(v) if options.iter().any(|o| o == v) => {
                            self.reforms.insert(key.to_string(), v.to_string());
                        }
                        other => warnings.push(format!("{label} line {line}: {key} = {other:?} is not an option of that reform")),
                    }
                } else if cls.technologies.contains(key) {
                    if value.as_f64().unwrap_or(0.0) > 0.0 || value.as_bool() == Some(true) {
                        self.technologies.insert(key.to_string());
                    } else {
                        self.technologies.remove(key);
                    }
                } else if cls.inventions.contains(key) {
                    if value.as_bool() == Some(true) || value.as_f64().unwrap_or(0.0) > 0.0 {
                        self.inventions.insert(key.to_string());
                    } else {
                        self.inventions.remove(key);
                    }
                } else {
                    warnings.push(format!("{label} line {line}: unhandled country history key '{key}'"));
                }
            }
        }
    }
}
