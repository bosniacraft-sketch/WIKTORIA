//! `wiktoria-cli why --save <file.v2> <pop id>...`
//!
//! Prints the port's reasoning for a pop, in the layout of the original's
//! `debug promotiondesc <pop id>` text so the two can be read side by side: every modifier of
//! the promotion, demotion, migration and emigration chances and of every target type's weight,
//! marked `(*)` when its conditions hold, `(X)` when they do not and `(?)` when the port does not
//! know one of them, with the sums the port works with.

use wiktoria_data::script::{Block, Value};
use wiktoria_simulation::pops::{additive_weight, admin_efficiency, MobilityDefs, MobilityRules};
use wiktoria_simulation::script::{conditions_hold, Scope, State};

/// A block's conditions as one line of script, without its `factor`.
fn text(block: &Block) -> String {
    let mut parts = Vec::new();
    for item in &block.items {
        match (item.key.as_deref(), &item.value) {
            (Some("factor"), _) => {}
            (Some(k), Value::Scalar(v)) => parts.push(format!("{k} = {v}")),
            (Some(k), Value::Block(b)) => parts.push(format!("{k} = {{ {} }}", text(b))),
            (None, Value::Scalar(v)) => parts.push(v.clone()),
            (None, Value::Block(b)) => parts.push(format!("{{ {} }}", text(b))),
        }
    }
    parts.join(" ")
}

fn lines(state: &State, block: &Block, scope: &Scope, out: &mut String) {
    for item in &block.items {
        match (item.key.as_deref(), item.value.as_block()) {
            (Some("modifier"), Some(m)) => {
                let mark = match conditions_hold(state, m, scope) {
                    Ok(true) => "(*)",
                    Ok(false) => "(X)",
                    Err(_) => "(?)",
                };
                out.push_str(&format!("  {mark} {:+} : {}\n", m.get("factor").and_then(Value::as_f64).unwrap_or(0.0), text(m)));
            }
            (Some("group"), Some(g)) => lines(state, g, scope, out),
            _ => {}
        }
    }
}

fn section(state: &State, title: &str, block: &Block, scope: &Scope, out: &mut String) -> f64 {
    let base = block.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
    let sum = additive_weight(state, block, scope) - base;
    out.push_str(&format!("{title}: modifiers {sum:+.3} (the block's own factor is {base})\n"));
    lines(state, block, scope, out);
    sum
}

pub fn why(state: &State, defs: &MobilityDefs, rules: &MobilityRules, ids: &[u64]) -> String {
    let mut out = String::new();
    let efficiency = admin_efficiency(state, rules);
    for (province, p) in &state.provinces {
        for (i, q) in p.pops.iter().enumerate().filter(|(_, q)| ids.contains(&q.id)) {
            let scope = Scope::Pop(*province, i);
            let owner = p.owner.clone().unwrap_or_default();
            let admin = efficiency.get(&owner).copied().unwrap_or(0.0);
            out.push_str(&format!(
                "==== pop {} : {} {} {} {}, province {province} of {owner}\n     needs life {:.2} everyday {:.2} luxury {:.2}; literacy {:.1}%; militancy {:.2}; consciousness {:.2}; money {:.1}; administrative efficiency of {owner} {admin:.2}\n",
                q.id, q.size, q.culture, q.religion, q.pop_type, q.life_needs, q.everyday_needs, q.luxury_needs, q.literacy * 100.0, q.militancy, q.consciousness, q.money
            ));
            let up = section(state, "PROMOTION CHANCE", &defs.promotion_chance, &scope, &mut out);
            let base = defs.promotion_chance.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
            out.push_str(&format!("  -> people promoted this month: {:.0}\n", (q.size * rules.scale * up.max(0.0) * base * admin * rules.promotion_tune).trunc()));
            let down = section(state, "DEMOTION CHANCE", &defs.demotion_chance, &scope, &mut out);
            out.push_str(&format!("  -> people demoted this month: {:.0}\n", (q.size * rules.scale * down.max(0.0) * rules.demotion_tune).trunc()));
            let mine = state.strata.get(&q.pop_type);
            for (target, block) in defs.promote_to.get(&q.pop_type).map(Vec::as_slice).unwrap_or(&[]) {
                let rank = |s: Option<&String>| match s.map(String::as_str) { Some("rich") => 2, Some("middle") => 1, _ => 0 };
                let kind = if rank(state.strata.get(target)) > rank(mine) { "promotion" } else { "demotion" };
                out.push_str(&format!("TARGET {target} ({kind}): weight {:+.2}\n", additive_weight(state, block, &scope)));
                lines(state, block, &scope, &mut out);
            }
            for (title, lists, shares, names) in [("IDEOLOGY", &defs.ideologies, &q.ideology, state.ideologies.clone()), ("ISSUE", &defs.issues, &q.issues, state.issues.iter().map(|i| i.0.clone()).collect::<Vec<_>>())] {
                let list = lists.get(&q.pop_type).map(Vec::as_slice).unwrap_or(&[]);
                let weights: Vec<(f64, f64)> = list.iter().map(|(_, b)| (wiktoria_simulation::pops::product_weight(state, b, &scope), additive_weight(state, b, &scope))).collect();
                let (product, added): (f64, f64) = (weights.iter().map(|w| w.0.max(0.0)).sum(), weights.iter().map(|w| w.1.max(0.0)).sum());
                for ((name, _), (p, a)) in list.iter().zip(&weights) {
                    let now = names.iter().position(|n| n == name).and_then(|i| shares.get(&(i as u32 + 1))).copied().unwrap_or(0.0);
                    out.push_str(&format!("{title} {name}: now {now:.2}%; weight as product {p:.3} ({:.2}% of all), as sum {a:.3} ({:.2}% of all)
", 100.0 * p.max(0.0) / product.max(1e-9), 100.0 * a.max(0.0) / added.max(1e-9)));
                    if title == "IDEOLOGY" && std::env::var("GAMEMIX_WHY_LINES").is_ok() {
                        if let Some(l) = list.iter().find(|l| l.0 == *name) {
                            lines(state, &l.1, &scope, &mut out);
                        }
                    }
                }
            }
            let within = section(state, "MIGRATION CHANCE (within the country)", &defs.migration_chance, &scope, &mut out) + defs.migration_chance.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
            out.push_str(&format!("  -> people moving this month: {:.0}\n", (q.size * rules.immigration_scale * (within * rules.migration_tune).max(0.0)).trunc()));
            let abroad = section(state, "EMIGRATION CHANCE", &defs.emigration_chance, &scope, &mut out) + defs.emigration_chance.get("factor").and_then(Value::as_f64).unwrap_or(0.0);
            out.push_str(&format!("  -> people emigrating this month: {:.0}\n\n", (q.size * rules.immigration_scale * (abroad * rules.emigration_tune).max(0.0)).trunc()));
        }
    }
    if out.is_empty() {
        out.push_str("no pop with these ids in the save\n");
    }
    out
}

/// The port's reasoning on reforms for a country: the upper house, its support for political and
/// social reform, what the people ask for, and what each reform group could move to.
pub fn why_country(state: &State, rules: &wiktoria_simulation::reforms::ReformRules, politics: &wiktoria_simulation::politics::PoliticsRules, tag: &str) -> String {
    let Some(c) = state.countries.get(tag) else { return format!("no country {tag}
") };
    let mut out = format!("==== {tag}: upper house {}; last reform {:?}
", c.upper_house.iter().map(|(k, v)| format!("{k} {:.0}%", v * 100.0)).collect::<Vec<_>>().join(" "), c.last_reform);
    out.push_str(&format!("the port would fill the upper house: {}; would elect party {:?}, ruling party now {:?}
", wiktoria_simulation::politics::upper_house(state, politics, tag).map(|h| h.iter().filter(|(_, v)| **v >= 0.005).map(|(k, v)| format!("{k} {:.0}%", v * 100.0)).collect::<Vec<_>>().join(" ")).unwrap_or_default(), wiktoria_simulation::politics::election_winner(state, politics, tag), c.ruling_party));
    for political in [true, false] {
        out.push_str(&format!("support for {} reform: {:.3} (needed above {}); would pass: {:?}
", if political { "political" } else { "social" }, wiktoria_simulation::reforms::house_support(state, rules, tag, political), rules.support_needed, wiktoria_simulation::reforms::pick_reform(state, rules, tag, political)));
    }
    let asked = wiktoria_simulation::reforms::wanted(state, tag);
    let scope = Scope::Country(tag.to_string());
    for g in &rules.groups {
        let now = c.reforms.get(&g.name).cloned().unwrap_or_default();
        let options: Vec<String> = g.options.iter().map(|(n, allow)| {
            let allowed = match allow.as_ref().map(|a| conditions_hold(state, a, &scope)) { None | Some(Ok(true)) => "", Some(Ok(false)) => " (not allowed)", Some(Err(_)) => " (allow: unknown condition)" };
            format!("{n} {:.1}%{allowed}", asked.get(n).copied().unwrap_or(0.0) * 100.0)
        }).collect();
        out.push_str(&format!("  {} [{}] now {now}: {}
", g.name, if g.political { "political" } else { "social" }, options.join(", ")));
    }
    out
}
