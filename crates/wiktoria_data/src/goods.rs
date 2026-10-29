//! Trade goods, loaded from `common/goods.txt` in the user's own Victoria 2 install.
//!
//! Layout: top-level category blocks (e.g. `military_goods = { ... }`), each holding
//! one block per good with `cost`, `color` and optional yes/no flags.
//! Defaults for absent flags are hypotheses noted in the rules file.

use crate::script::{Block, Document};

#[derive(Debug, Clone, PartialEq)]
pub struct Good {
    pub id: String,
    pub category: String,
    /// Base price as written in the file.
    pub cost: f64,
    pub color: [u8; 3],
    pub available_from_start: bool,
    pub tradeable: bool,
    pub money: bool,
    pub overseas_penalty: bool,
}

#[derive(Debug, Default)]
pub struct GoodsLoad {
    pub goods: Vec<Good>,
    pub warnings: Vec<String>,
}

pub fn load_goods(doc: &Document) -> Result<GoodsLoad, String> {
    let mut out = GoodsLoad { warnings: doc.warnings.clone(), ..Default::default() };
    for (category, value, line) in doc.root.entries() {
        let Some(cat) = value.as_block() else {
            out.warnings.push(format!("line {line}: '{category}' is not a category block, skipped"));
            continue;
        };
        for (id, good, gline) in cat.entries() {
            let Some(g) = good.as_block() else {
                out.warnings.push(format!("line {gline}: good '{id}' is not a block, skipped"));
                continue;
            };
            out.goods.push(parse_good(id, category, g, gline, &mut out.warnings)?);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for g in &out.goods {
        if !seen.insert(g.id.as_str()) {
            return Err(format!("duplicate good id '{}'", g.id));
        }
    }
    Ok(out)
}

fn parse_good(id: &str, category: &str, b: &Block, line: usize, warnings: &mut Vec<String>) -> Result<Good, String> {
    let cost = b
        .get("cost")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("line {line}: good '{id}' has no numeric cost"))?;
    let color = match b.get("color").and_then(|v| v.as_block()) {
        Some(c) => {
            let parts: Vec<u8> = c.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect();
            if parts.len() != 3 {
                return Err(format!("line {line}: good '{id}' color must be three 0-255 integers"));
            }
            [parts[0], parts[1], parts[2]]
        }
        None => {
            warnings.push(format!("line {line}: good '{id}' has no color, using black"));
            [0, 0, 0]
        }
    };
    let flag = |key: &str, default: bool| b.get(key).and_then(|v| v.as_bool()).unwrap_or(default);
    for (key, _, l) in b.entries() {
        if !matches!(key, "cost" | "color" | "available_from_start" | "tradeable" | "money" | "overseas_penalty") {
            warnings.push(format!("line {l}: good '{id}' has unhandled key '{key}'"));
        }
    }
    Ok(Good {
        id: id.to_string(),
        category: category.to_string(),
        cost,
        color,
        available_from_start: flag("available_from_start", true),
        tradeable: flag("tradeable", true),
        money: flag("money", false),
        overseas_penalty: flag("overseas_penalty", false),
    })
}
