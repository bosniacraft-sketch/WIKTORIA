//! Production definitions from the player's install: resource-gathering production types
//! (common/production_types.txt) and the production effects of technologies and inventions.

use std::collections::{BTreeMap, HashMap};

use crate::script::{self, Block, Value};

/// What a worker or owner pop type improves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JobEffect {
    Input,
    Output,
    Throughput,
}

impl JobEffect {
    pub const ALL: [JobEffect; 3] = [JobEffect::Input, JobEffect::Output, JobEffect::Throughput];

    fn parse(s: &str) -> Option<JobEffect> {
        match s {
            "input" => Some(JobEffect::Input),
            "output" => Some(JobEffect::Output),
            "throughput" => Some(JobEffect::Throughput),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// One `employees` entry or the `owner` of a production type.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub pop_type: String,
    pub effect: JobEffect,
    /// `amount` (employee share cap), 1 when absent.
    pub amount: f64,
    /// `effect_multiplier`, 1 when absent.
    pub multiplier: f64,
}

/// A resource-gathering (`type = rgo`) production type.
#[derive(Debug, Clone, PartialEq)]
pub struct RgoType {
    pub name: String,
    pub good: String,
    pub value: f64,
    pub workforce: f64,
    pub farm: bool,
    pub mine: bool,
    pub owner: Option<Job>,
    pub employees: Vec<Job>,
}

fn job(b: &Block) -> Option<Job> {
    Some(Job {
        pop_type: b.get("poptype")?.as_str()?.to_string(),
        effect: JobEffect::parse(b.get("effect")?.as_str()?)?,
        amount: b.get("amount").and_then(Value::as_f64).unwrap_or(1.0),
        multiplier: b.get("effect_multiplier").and_then(Value::as_f64).unwrap_or(1.0),
    })
}

/// Every `type = rgo` production type, a field missing from the type itself taken from its
/// `template` (one level, as in the files).
pub fn parse_rgo_types(src: &str) -> Result<Vec<RgoType>, String> {
    let d = script::parse(src).map_err(|e| format!("production_types.txt: {e}"))?;
    let blocks: HashMap<&str, &Block> = d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))).collect();
    let field = |b: &Block, k: &str| -> Option<Value> {
        b.get(k).cloned().or_else(|| {
            let t = b.get("template")?.as_str()?;
            blocks.get(t)?.get(k).cloned()
        })
    };
    let mut out = Vec::new();
    for (name, b) in d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))) {
        let Some(good) = b.get("output_goods").and_then(Value::as_str) else { continue };
        if field(b, "type").as_ref().and_then(Value::as_str) != Some("rgo") {
            continue;
        }
        let flag = |k: &str| field(b, k).as_ref().and_then(Value::as_bool).unwrap_or(false);
        let employees = field(b, "employees")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| e.items.iter().filter_map(|i| job(i.value.as_block()?)).collect())
            .unwrap_or_default();
        out.push(RgoType {
            name: name.to_string(),
            good: good.to_string(),
            value: field(b, "value").as_ref().and_then(Value::as_f64).unwrap_or(0.0),
            workforce: field(b, "workforce").as_ref().and_then(Value::as_f64).unwrap_or(0.0),
            farm: flag("farm"),
            mine: flag("mine"),
            owner: field(b, "owner").as_ref().and_then(Value::as_block).and_then(job),
            employees,
        });
    }
    Ok(out)
}

/// An artisan production type.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtisanType {
    pub name: String,
    pub output_good: String,
    pub input_goods: BTreeMap<String, f64>,
    pub value: f64,
    pub workforce: f64,
}

/// Parses the artisan production types from production_types.txt.
pub fn parse_artisan_types(src: &str) -> Result<Vec<ArtisanType>, String> {
    let d = script::parse(src).map_err(|e| format!("production_types.txt: {e}"))?;
    let blocks: HashMap<&str, &Block> = d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))).collect();
    let field = |b: &Block, k: &str| -> Option<Value> {
        b.get(k).cloned().or_else(|| {
            let t = b.get("template")?.as_str()?;
            blocks.get(t)?.get(k).cloned()
        })
    };
    let mut out = Vec::new();
    for (name, b) in d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))) {
        let Some(good) = b.get("output_goods").and_then(Value::as_str) else { continue };
        if field(b, "type").as_ref().and_then(Value::as_str) != Some("artisan") {
            continue;
        }
        let input_goods = field(b, "input_goods")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| e.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect())
            .unwrap_or_default();
        out.push(ArtisanType {
            name: name.to_string(),
            output_good: good.to_string(),
            input_goods,
            value: field(b, "value").as_ref().and_then(Value::as_f64).unwrap_or(1.0),
            workforce: field(b, "workforce").as_ref().and_then(Value::as_f64).unwrap_or(10000.0),
        });
    }
    Ok(out)
}

/// A factory production type.
#[derive(Debug, Clone, PartialEq)]
pub struct FactoryType {
    pub name: String,
    pub output_good: String,
    pub input_goods: BTreeMap<String, f64>,
    pub value: f64,
    pub workforce: f64,
    pub maintenance: BTreeMap<String, f64>,
    pub owner: Option<Job>,
    pub employees: Vec<Job>,
}

/// Parses the factory production types from production_types.txt.
pub fn parse_factory_types(src: &str) -> Result<Vec<FactoryType>, String> {
    let d = script::parse(src).map_err(|e| format!("production_types.txt: {e}"))?;
    let blocks: HashMap<&str, &Block> = d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))).collect();
    let field = |b: &Block, k: &str| -> Option<Value> {
        b.get(k).cloned().or_else(|| {
            let t = b.get("template")?.as_str()?;
            blocks.get(t)?.get(k).cloned()
        })
    };
    let mut out = Vec::new();
    for (name, b) in d.root.entries().filter_map(|(k, v, _)| Some((k, v.as_block()?))) {
        let Some(good) = b.get("output_goods").and_then(Value::as_str) else { continue };
        if field(b, "type").as_ref().and_then(Value::as_str) != Some("factory") {
            continue;
        }
        let input_goods = field(b, "input_goods")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| e.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect())
            .unwrap_or_default();
        let maintenance = field(b, "efficiency")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| e.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect())
            .unwrap_or_default();
        let employees = field(b, "employees")
            .as_ref()
            .and_then(Value::as_block)
            .map(|e| e.items.iter().filter_map(|i| job(i.value.as_block()?)).collect())
            .unwrap_or_default();
        out.push(FactoryType {
            name: name.to_string(),
            output_good: good.to_string(),
            input_goods,
            value: field(b, "value").as_ref().and_then(Value::as_f64).unwrap_or(1.0),
            workforce: field(b, "workforce").as_ref().and_then(Value::as_f64).unwrap_or(10000.0),
            maintenance,
            owner: field(b, "owner").as_ref().and_then(Value::as_block).and_then(job),
            employees,
        });
    }
    Ok(out)
}


/// Production effects of one technology (its top-level entries) or one invention (its `effect`
/// block). Values as written; within one source a repeated key or good replaces the earlier one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductionEffects {
    /// `farm_rgo_size`, `mine_rgo_size`, `farm_rgo_eff`, `mine_rgo_eff`, `rgo_input`,
    /// `rgo_output`, `rgo_throughput`.
    pub values: BTreeMap<String, f64>,
    /// `rgo_size`, `rgo_goods_input`, `rgo_goods_output`, `rgo_goods_throughput`: good -> value.
    pub per_good: BTreeMap<String, BTreeMap<String, f64>>,
}

impl ProductionEffects {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.per_good.is_empty()
    }
}

/// Reads the production effects of a block. `scalar_keys` and `good_keys` name the entries to
/// keep (the rules file lists them).
pub fn production_effects(b: &Block, scalar_keys: &[String], good_keys: &[String]) -> ProductionEffects {
    let mut e = ProductionEffects::default();
    for (k, v, _) in b.entries() {
        if scalar_keys.iter().any(|s| s == k) {
            if let Some(x) = v.as_f64() {
                e.values.insert(k.to_string(), x);
            }
        } else if let (true, Some(goods)) = (good_keys.iter().any(|s| s == k), v.as_block()) {
            let slot = e.per_good.entry(k.to_string()).or_default();
            for (g, x, _) in goods.entries() {
                if let Some(x) = x.as_f64() {
                    slot.insert(g.to_string(), x);
                }
            }
        }
    }
    e
}

/// The `effect` block of every invention in one inventions file, in file order.
pub fn invention_effects(root: &Block, scalar_keys: &[String], good_keys: &[String]) -> Vec<(String, ProductionEffects)> {
    root.entries()
        .filter_map(|(k, v, _)| {
            let b = v.as_block()?;
            let e = b.get("effect").and_then(Value::as_block).map(|e| production_effects(e, scalar_keys, good_keys)).unwrap_or_default();
            Some((k.to_string(), e))
        })
        .collect()
}

/// Top-level production effects of every technology in one technology file.
pub fn technology_effects(root: &Block, scalar_keys: &[String], good_keys: &[String]) -> Vec<(String, ProductionEffects)> {
    root.entries().filter_map(|(k, v, _)| Some((k.to_string(), production_effects(v.as_block()?, scalar_keys, good_keys)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgo_types_use_templates() {
        let src = "T = { owner = { poptype = lords effect = output } employees = { { poptype = diggers effect = throughput amount = 1 } { poptype = serfs effect = output amount = 0.5 effect_multiplier = 2 } } type = rgo workforce = 100 }\n\
                   rock_pit = { template = T output_goods = rock value = 1.5 mine = yes }\n\
                   mill = { output_goods = flour type = factory }";
        let t = parse_rgo_types(src).unwrap();
        assert_eq!(t.len(), 1);
        let r = &t[0];
        assert_eq!((r.good.as_str(), r.value, r.workforce, r.farm, r.mine), ("rock", 1.5, 100.0, false, true));
        assert_eq!(r.owner.as_ref().map(|o| (o.pop_type.as_str(), o.effect)), Some(("lords", JobEffect::Output)));
        assert_eq!(r.employees[1], Job { pop_type: "serfs".into(), effect: JobEffect::Output, amount: 0.5, multiplier: 2.0 });
    }

    #[test]
    fn effects_keep_listed_keys_last_value_wins() {
        let d = script::parse("t = { year = 1 farm_rgo_eff = 0.1 rgo_size = { rock = 0.2 } rgo_size = { rock = 0.3 sand = 0.1 } other = 5 }").unwrap();
        let b = d.root.get("t").and_then(Value::as_block).unwrap();
        let e = production_effects(b, &["farm_rgo_eff".into()], &["rgo_size".into()]);
        assert_eq!(e.values.get("farm_rgo_eff"), Some(&0.1));
        assert_eq!(e.per_good["rgo_size"].get("rock"), Some(&0.3));
        assert_eq!(e.per_good["rgo_size"].get("sand"), Some(&0.1));
        assert_eq!(e.values.len(), 1);
    }
}
