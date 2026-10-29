//! `wiktoria-cli check-colonies <save.v2>`
//!
//! Colonies and protectorates per great power (provinces and states), the colonial points the rules
//! give, and a recount of the same numbers straight from the save's text, so the two can be compared.

use std::collections::{BTreeMap, HashSet};

use wiktoria_simulation::colonies::{ColonyRules, ColonyState, Status};

use crate::{config, root, Args};

pub fn check_colonies(args: &Args) -> Result<(), String> {
    let usage = "usage: check-colonies <save.v2>";
    let path = args.save.as_ref().or(args.positional.first()).ok_or(usage)?;
    let engine_rules = config::load_engine_rules(&root())?;
    let install = config::install_dir(&root(), args.install.clone())?;
    let defines_path = install.join(engine_rules.require_str("FLD_DEFINES_FILE")?);
    let src = std::fs::read(&defines_path).map_err(|e| format!("cannot read {}: {e}", defines_path.display()))?;
    let defines = wiktoria_data::defines::Defines::parse(&wiktoria_data::script::decode_windows1252(&src))?;
    let rules = ColonyRules::from_defines(&defines, &engine_rules)?;
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let save = wiktoria_data::save::parse_save(&bytes, &HashSet::new(), &HashSet::new())?;
    let c = ColonyState::from_save(&save);

    let text = decode(&bytes);
    let raw = recount(&text);

    println!("{path}: {} (colonial rank limit {}, life rating {})", c.date, rules.colonial_rank, rules.life_rating);
    println!();
    println!("great powers: provinces as colony / protectorate, then states as colony / protectorate, then the upkeep in colonial points, races entered; (recount from the text in brackets)");
    println!("{:<5} {:>9} {:>9}   {:>7} {:>7}  {:>7}  {:>6}", "tag", "col prov", "prot prov", "col st", "prot st", "upkeep", "races");
    for (tag, k) in c.countries.iter().filter(|(_, k)| k.great_power) {
        let (cp, pp) = c.counts_by_province(tag);
        let (cs, ps) = c.counts_by_state(tag);
        let upkeep: f64 = c.holdings_of(tag).iter().map(|h| wiktoria_simulation::colonies::upkeep_of(&rules, h)).sum();
        let r = raw.get(tag.as_str()).copied().unwrap_or_default();
        let ok = if (cp, pp, cs, ps) == (r.col_provinces, r.prot_provinces, r.col_states, r.prot_states) { "ok" } else { "DIFFERS" };
        println!("{tag:<5} {cp:>9} {pp:>9}   {cs:>7} {ps:>7}  {upkeep:>7.1}  {:>6}   [{} {} {} {}] {ok}", k.races.len(), r.col_provinces, r.prot_provinces, r.col_states, r.prot_states);
    }
    let all: (usize, usize) = c.countries.keys().map(|t| c.counts_by_province(t)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    println!();
    println!("world: {} colony provinces, {} protectorate provinces owned", all.0, all.1);
    println!("states with colonial status: {} colonies, {} protectorates", c.states.iter().filter(|s| s.status == Status::Colony).count(), c.states.iter().filter(|s| s.status == Status::Protectorate).count());
    Ok(())
}

#[derive(Default, Clone, Copy)]
struct Raw {
    col_provinces: usize,
    prot_provinces: usize,
    col_states: usize,
    prot_states: usize,
}

fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A line scan that shares no code with the parser: province blocks are `<n>=` at column 0, country
/// blocks `TAG=` at column 0, states `\tstate=` inside a country, `is_colonial=` one tab deeper.
fn recount(text: &str) -> BTreeMap<&str, Raw> {
    let mut out: BTreeMap<&str, Raw> = BTreeMap::new();
    let mut owner: Option<&str> = None;
    let mut in_province = false;
    let mut country: Option<&str> = None;
    for line in text.lines() {
        let t = line.trim_end();
        if t == "{" || t == "}" || t.is_empty() {
            // the braces of a top-level block sit at column 0 too
            if t == "}" {
                owner = None;
                in_province = false;
                country = None;
            }
            continue;
        }
        if !t.starts_with('\t') && !t.starts_with(' ') {
            let key = t.strip_suffix('=');
            if key.is_none() {
                // bare numbers of a list, written at column 0
                continue;
            }
            owner = None;
            in_province = false;
            country = None;
            if let Some(k) = key {
                if k.bytes().all(|b| b.is_ascii_digit()) && !k.is_empty() {
                    in_province = true;
                } else if k.len() == 3 && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
                    country = Some(k);
                }
            }
            continue;
        }
        if in_province {
            if let Some(v) = t.strip_prefix("\towner=") {
                owner = Some(v.trim_matches('"'));
            } else if let Some(v) = t.strip_prefix("\tcolonial=") {
                // the owner line comes before colonial in the save
                if let Some(o) = owner {
                    let e = out.entry(o).or_default();
                    match v.trim() {
                        "1" => e.prot_provinces += 1,
                        "2" => e.col_provinces += 1,
                        _ => {}
                    }
                }
            }
        } else if let Some((c, v)) = country.zip(t.strip_prefix("\t\tis_colonial=")) {
            let e = out.entry(c).or_default();
            match v.trim() {
                "1" => e.prot_states += 1,
                "2" => e.col_states += 1,
                _ => {}
            }
        }
    }
    out
}
