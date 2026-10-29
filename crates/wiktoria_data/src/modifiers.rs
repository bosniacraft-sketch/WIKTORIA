//! Modifier definitions from the player's install: named blocks of `key = number` entries that the
//! game adds up into a country's (or province's) modifier totals.
//!
//! Loaded here: static modifiers (common/static_modifiers.txt), national values
//! (common/nationalvalues.txt), the modifiers of every reform / party-issue option
//! (common/issues.txt), technology schools (`schools` in common/technology.txt) and the direct
//! modifier keys of inventions (top-level numbers in an invention block, not its `effect`).
//! Keys are kept lower-case; values exactly as written in the files.

use std::collections::BTreeMap;

use crate::script::{Block, Value};

/// Modifier key -> value, as written in a file.
pub type ModifierValues = BTreeMap<String, f64>;

/// Numeric `key = value` entries of a block (nested blocks and text values are not modifiers).
pub fn modifier_values(b: &Block) -> ModifierValues {
    let mut out = ModifierValues::new();
    for (k, v, _) in b.entries() {
        if let Some(x) = v.as_f64() {
            *out.entry(k.to_ascii_lowercase()).or_default() += x;
        }
    }
    out
}

/// Every named block of a file (`name = { ... }`) as modifier values.
pub fn parse_named_blocks(root: &Block) -> BTreeMap<String, ModifierValues> {
    root.entries().filter_map(|(k, v, _)| Some((k.to_string(), modifier_values(v.as_block()?)))).collect()
}

/// Named blocks of the event modifiers file: like `parse_named_blocks`, without the entries
/// that are not modifiers (`icon` only chooses the picture).
pub fn parse_event_modifiers(root: &Block) -> BTreeMap<String, ModifierValues> {
    let mut out = parse_named_blocks(root);
    for values in out.values_mut() {
        values.remove("icon");
    }
    out
}

/// Modifiers of every option of every issue group in issues.txt, by option name.
pub fn parse_issue_option_modifiers(root: &Block) -> BTreeMap<String, ModifierValues> {
    let mut out = BTreeMap::new();
    for (_, cat, _) in root.entries() {
        let Some(cat) = cat.as_block() else { continue };
        for (_, group, _) in cat.entries() {
            let Some(group) = group.as_block() else { continue };
            for (opt, ob, _) in group.entries() {
                if let Some(ob) = ob.as_block() {
                    out.insert(opt.to_string(), modifier_values(ob));
                }
            }
        }
    }
    out
}

/// `schools = { name = { ... } }` in technology.txt.
pub fn parse_schools(root: &Block) -> BTreeMap<String, ModifierValues> {
    root.get("schools").and_then(Value::as_block).map(parse_named_blocks).unwrap_or_default()
}

/// Direct modifier keys of each invention in one inventions file, in file order.
pub fn parse_invention_modifiers(root: &Block) -> Vec<(String, ModifierValues)> {
    root.entries()
        .filter_map(|(k, v, _)| {
            let b = v.as_block()?;
            let mut values = modifier_values(b);
            if let Some(effect) = b.get("effect").and_then(Value::as_block) {
                for (name, x) in modifier_values(effect) {
                    *values.entry(name).or_insert(0.0) += x;
                }
            }
            Some((k.to_string(), values))
        })
        .collect()
}

/// All modifier sources the port knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModifierLibrary {
    pub static_modifiers: BTreeMap<String, ModifierValues>,
    pub national_values: BTreeMap<String, ModifierValues>,
    pub issue_options: BTreeMap<String, ModifierValues>,
    pub schools: BTreeMap<String, ModifierValues>,
    /// Named modifiers that events and decisions put on a country or province for a time
    /// (FLD_EVENT_MODIFIERS_FILE). The display-only `icon` entry is dropped.
    pub event_modifiers: BTreeMap<String, ModifierValues>,
    /// In the order the game numbers inventions (inventions files sorted by name, then file order).
    pub inventions: Vec<(String, ModifierValues)>,
}
