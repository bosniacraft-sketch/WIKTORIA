//! The Military window's answer (apps/web/API.md: Military): the seat's leaders, armies, navies,
//! the units being built, the unit types it could build, its mobilisation and a block of its
//! military numbers. Read-only: the only military order the engine takes from a player is the
//! army move (`POST /api/army/move`).
//!
//! The answer is gathered from the game into plain rows (`View`) and then written as JSON
//! (`view_json`), so the writing can be tested on hand-made rows.

use wiktoria_data::date::Date;
use wiktoria_simulation::game::Game;
use wiktoria_simulation::military::battle::CombatKind;
use wiktoria_simulation::military::logistics;
use wiktoria_simulation::military::units::LeaderKind;

use super::{country_name, json_int, json_num, json_opt, json_opt_str, json_str, province_name, Shared};

/// The country's military numbers. `None`: the engine does not have the number.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Stats {
    pub war_exhaustion: Option<f64>,
    pub supply_consumption: Option<f64>,
    pub org_regain: Option<f64>,
    pub start_experience: Option<f64>,
    pub recruit_time: Option<f64>,
    pub combat_width: Option<f64>,
    pub dig_in_cap: Option<f64>,
    pub tactics: Option<f64>,
    pub leadership: Option<f64>,
    pub regiments: usize,
    pub ships: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Mobilisation {
    pub mobilised: bool,
    /// 0..1 while the regiments are being raised.
    pub progress: Option<f64>,
    pub size: Option<f64>,
    pub impact: Option<f64>,
    /// Regiments it would raise today (only while not mobilised).
    pub regiments: Option<u32>,
    /// Share of production lost while mobilised.
    pub production_loss: Option<f64>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct LeaderRow {
    pub id: u64,
    pub name: String,
    pub admiral: bool,
    /// (key, display name); empty key: none.
    pub personality: (String, String),
    pub background: (String, String),
    /// The two traits summed: (our own statistic name, value), only those not zero.
    pub traits: Vec<(&'static str, f64)>,
    pub prestige: f64,
    /// ("army" or "navy", unit id, unit name).
    pub assigned: Option<(&'static str, u64, String)>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct ArmyRow {
    pub id: u64,
    pub name: String,
    pub province: (u16, String),
    pub men: f64,
    pub regiments: usize,
    /// Shares 0..1 of what the regiments can hold.
    pub strength: Option<f64>,
    pub organisation: Option<f64>,
    /// moving, battle, siege, retreating, embarked, dug_in, standing
    pub doing: &'static str,
    pub to: Option<(u16, String)>,
    pub leader: Option<String>,
    pub dig_in: f64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct NavyRow {
    pub id: u64,
    pub name: String,
    pub province: (u16, String),
    pub ships: usize,
    pub strength: Option<f64>,
    pub organisation: Option<f64>,
    /// moving, battle, at_sea, in_port
    pub doing: &'static str,
    pub to: Option<(u16, String)>,
    pub leader: Option<String>,
    /// Armies on board.
    pub carrying: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct UnitRow {
    pub key: String,
    pub name: String,
    pub land: bool,
    pub kind: String,
    /// The unit file's own availability (technologies that unlock types are not followed yet).
    pub active: bool,
    pub days: f64,
    /// The goods at today's world price; `None` when a good has no price.
    pub cost: Option<f64>,
    pub goods: Vec<(String, f64)>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct BuildRow {
    pub name: String,
    pub kind: String,
    pub kind_name: String,
    pub land: bool,
    pub province: (u16, String),
    pub progress: Option<f64>,
    pub ends: Option<Date>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct View {
    pub tag: String,
    pub name: String,
    pub played: bool,
    pub stats: Stats,
    pub mobilisation: Mobilisation,
    pub leaders: Vec<LeaderRow>,
    pub armies: Vec<ArmyRow>,
    pub navies: Vec<NavyRow>,
    pub units: Vec<UnitRow>,
    pub building: Vec<BuildRow>,
}

/// Day number of a date on the game's calendar of 365 days (enough to show progress).
fn day_no(d: Date) -> i64 {
    const START: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    d.year as i64 * 365 + START[(d.month.clamp(1, 12) - 1) as usize] + d.day as i64
}

fn share(have: f64, of: f64) -> Option<f64> {
    (of > 0.0 && have.is_finite()).then(|| (have / of).clamp(0.0, 1.0))
}

/// Everything the window shows for `tag`, from the running game.
pub(super) fn gather(sh: &Shared, g: &Game, tag: &str) -> Option<View> {
    let (s, mil, day) = (&g.state, &g.core.mil, &g.core.day);
    let c = s.countries.get(tag)?;
    let lib = &g.world.defs.modifiers;
    let mc = mil.countries.get(tag);
    let modifier = |key: &str| wiktoria_simulation::pops::country_modifier(c, lib, key);
    let at_war = mil.active_wars_of(tag).next().is_some();
    let rr = &g.econ.rank_rules;
    let block = lib.static_modifiers.get(if at_war { &rr.war_block } else { &rr.peace_block }).and_then(|v| v.get("supply_consumption")).copied().unwrap_or(0.0);
    let stats = Stats {
        war_exhaustion: mc.map(|m| m.war_exhaustion),
        supply_consumption: Some(1.0 + modifier("supply_consumption") + block),
        // The day's regain is lifted by the technologies' morale (logistics::regain_organisation).
        org_regain: Some(day.country_value(s, tag, "morale")),
        start_experience: None,
        recruit_time: None,
        combat_width: Some((day.battle.base_width + day.country_value(s, tag, "combat_width")).max(2.0)),
        dig_in_cap: Some(day.country_value(s, tag, "dig_in_cap")),
        tactics: Some(day.battle.base_tactics + day.country_value(s, tag, "military_tactics")),
        leadership: Some(mc.map_or(c.leadership, |m| m.leadership)),
        regiments: mil.regiment_count(tag),
        ships: mil.ship_count(tag),
    };
    let mobilised = mc.is_some_and(|m| m.mobilized) || c.mobilized;
    let mods = g.core.mobilisation_mods.get(tag).copied();
    let mobilisation = Mobilisation {
        mobilised,
        progress: g.core.mobilising.get(tag).copied(),
        size: mods.map(|m| m.0),
        impact: mods.map(|m| m.1),
        regiments: if mobilised { None } else { g.core.mobilisable_regiments(s, tag) },
        production_loss: mods.map(|(size, impact)| 1.0 - logistics::mobilisation_production_factor(true, impact, size)),
    };
    let place = |id: u16| (id, province_name(g, id));
    let leader_name = |id: Option<u64>| id.and_then(|id| mil.leaders.iter().find(|l| l.id == id)).map(|l| l.name.clone());
    let in_combat = |kind: CombatKind, id: u64| mil.combats.iter().any(|x| x.kind == kind && (x.attacker.units.contains(&id) || x.defender.units.contains(&id)));
    let men = day.men_per_strength;
    let armies: Vec<ArmyRow> = mil
        .armies_of(tag)
        .map(|a| {
            let max_strength: f64 = a.regiments.iter().map(|r| day.units.get(&r.kind).map_or(3.0, |u| if u.max_strength > 0.0 { u.max_strength } else { 3.0 })).sum();
            let max_org: f64 = a.regiments.iter().map(|r| day.max_organisation(mil, s, a, &r.kind)).sum();
            let doing = if a.carried_by.is_some() {
                "embarked"
            } else if in_combat(CombatKind::Land, a.id) {
                "battle"
            } else if in_combat(CombatKind::Siege, a.id) {
                "siege"
            } else if day.is_retreating(a.id) {
                "retreating"
            } else if !a.path.is_empty() {
                "moving"
            } else if a.dig_in > 0.0 {
                "dug_in"
            } else {
                "standing"
            };
            ArmyRow {
                id: a.id,
                name: a.name.clone(),
                province: place(a.location),
                men: a.men(men),
                regiments: a.regiments.len(),
                strength: share(a.regiments.iter().map(|r| r.strength).sum(), max_strength),
                organisation: share(a.regiments.iter().map(|r| r.organisation).sum(), max_org),
                doing,
                to: a.path.last().map(|p| place(*p)),
                leader: leader_name(a.leader),
                dig_in: a.dig_in,
            }
        })
        .collect();
    let navies: Vec<NavyRow> = mil
        .navies_of(tag)
        .map(|n| {
            let def = |k: &str| day.units.get(k);
            let max_strength: f64 = n.ships.iter().map(|x| def(&x.kind).map_or(0.0, |u| u.max_strength)).sum();
            let max_org: f64 = n.ships.iter().map(|x| def(&x.kind).map_or(0.0, |u| u.organisation)).sum();
            let sea = day.map.provinces.get(&n.location).is_some_and(|m| m.sea);
            let doing = if in_combat(CombatKind::Naval, n.id) {
                "battle"
            } else if !n.path.is_empty() {
                "moving"
            } else if sea {
                "at_sea"
            } else {
                "in_port"
            };
            NavyRow {
                id: n.id,
                name: n.name.clone(),
                province: place(n.location),
                ships: n.ships.len(),
                strength: share(n.ships.iter().map(|x| x.strength).sum(), max_strength),
                organisation: share(n.ships.iter().map(|x| x.organisation).sum(), max_org),
                doing,
                to: n.path.last().map(|p| place(*p)),
                leader: leader_name(n.leader),
                carrying: n.carried.len(),
            }
        })
        .collect();
    let traits = &day.content.traits;
    let named = |k: &str| (k.to_string(), if k.is_empty() { String::new() } else { sh.name(k) });
    let leaders: Vec<LeaderRow> = mil
        .leaders_of(tag)
        .map(|l| {
            let t = [l.personality.as_str(), l.background.as_str()].iter().filter_map(|k| traits.get(*k)).fold(Default::default(), |a: wiktoria_simulation::military::units::TraitDef, t| a.plus(*t));
            let list = [("attack", t.attack), ("defence", t.defence), ("morale", t.morale), ("organisation", t.organisation), ("reconnaissance", t.reconnaissance), ("speed", t.speed), ("attrition", t.attrition), ("experience", t.experience), ("reliability", t.reliability)];
            let assigned = if l.kind == LeaderKind::Land {
                mil.armies_of(tag).find(|a| a.leader == Some(l.id)).map(|a| ("army", a.id, a.name.clone()))
            } else {
                mil.navies_of(tag).find(|n| n.leader == Some(l.id)).map(|n| ("navy", n.id, n.name.clone()))
            };
            LeaderRow { id: l.id, name: l.name.clone(), admiral: l.kind == LeaderKind::Sea, personality: named(&l.personality), background: named(&l.background), traits: list.into_iter().filter(|(_, v)| *v != 0.0).collect(), prestige: l.prestige, assigned }
        })
        .collect();
    let mut units: Vec<UnitRow> = day
        .units
        .values()
        .map(|u| {
            let goods: Vec<(String, f64)> = u.build_cost.iter().map(|(k, v)| (k.clone(), *v)).collect();
            let cost = goods.iter().map(|(k, v)| s.prices.get(k).map(|p| p * v)).sum::<Option<f64>>();
            UnitRow { key: u.name.clone(), name: sh.name(&u.name), land: u.land, kind: u.kind.clone(), active: u.active, days: u.build_time, cost, goods }
        })
        .collect();
    units.sort_by(|a, b| b.land.cmp(&a.land).then(b.active.cmp(&a.active)).then(a.name.cmp(&b.name)));
    let today = day_no(s.date);
    let building: Vec<BuildRow> = mil
        .constructions
        .iter()
        .filter(|x| x.owner == tag)
        .map(|x| {
            let progress = match (x.start, x.end) {
                (Some(a), Some(b)) if day_no(b) > day_no(a) => Some(((today - day_no(a)) as f64 / (day_no(b) - day_no(a)) as f64).clamp(0.0, 1.0)),
                _ => None,
            };
            BuildRow { name: x.name.clone(), kind: x.kind.clone(), kind_name: sh.name(&x.kind), land: day.units.get(&x.kind).is_none_or(|u| u.land), province: place(x.location), progress, ends: x.end }
        })
        .collect();
    Some(View { tag: tag.to_string(), name: country_name(g, tag), played: c.human, stats, mobilisation, leaders, armies, navies, units, building })
}

fn place_json(p: &(u16, String)) -> String {
    format!("{{\"id\":{},\"name\":{}}}", p.0, json_str(&p.1))
}

fn opt_place(p: &Option<(u16, String)>) -> String {
    p.as_ref().map_or("null".into(), place_json)
}

/// `GET /api/military` as JSON.
pub(super) fn view_json(v: &View) -> String {
    let st = &v.stats;
    let stats = format!(
        "{{\"war_exhaustion\":{},\"supply_consumption\":{},\"org_regain\":{},\"start_experience\":{},\"recruit_time\":{},\"combat_width\":{},\"dig_in_cap\":{},\"tactics\":{},\"leadership\":{},\"regiments\":{},\"ships\":{}}}",
        json_opt(st.war_exhaustion),
        json_opt(st.supply_consumption),
        json_opt(st.org_regain),
        json_opt(st.start_experience),
        json_opt(st.recruit_time),
        json_opt(st.combat_width),
        json_opt(st.dig_in_cap),
        json_opt(st.tactics),
        json_opt(st.leadership),
        st.regiments,
        st.ships
    );
    let m = &v.mobilisation;
    let mobilisation = format!(
        "{{\"mobilised\":{},\"progress\":{},\"size\":{},\"impact\":{},\"regiments\":{},\"production_loss\":{}}}",
        m.mobilised,
        json_opt(m.progress),
        json_opt(m.size),
        json_opt(m.impact),
        m.regiments.map_or("null".into(), |n| n.to_string()),
        json_opt(m.production_loss)
    );
    let key_name = |(k, n): &(String, String)| if k.is_empty() { "null".into() } else { format!("{{\"key\":{},\"name\":{}}}", json_str(k), json_str(n)) };
    let leaders: Vec<String> = v
        .leaders
        .iter()
        .map(|l| {
            let traits: Vec<String> = l.traits.iter().map(|(k, x)| format!("[{},{}]", json_str(k), json_num(*x))).collect();
            let assigned = l.assigned.as_ref().map_or("null".into(), |(k, id, n)| format!("{{\"kind\":{},\"id\":{id},\"name\":{}}}", json_str(k), json_str(n)));
            format!(
                "{{\"id\":{},\"name\":{},\"kind\":{},\"personality\":{},\"background\":{},\"traits\":[{}],\"prestige\":{},\"assigned\":{assigned}}}",
                l.id,
                json_str(&l.name),
                json_str(if l.admiral { "admiral" } else { "general" }),
                key_name(&l.personality),
                key_name(&l.background),
                traits.join(","),
                json_num(l.prestige)
            )
        })
        .collect();
    let armies: Vec<String> = v
        .armies
        .iter()
        .map(|a| {
            format!(
                "{{\"id\":{},\"name\":{},\"province\":{},\"men\":{},\"regiments\":{},\"strength\":{},\"organisation\":{},\"doing\":{},\"to\":{},\"leader\":{},\"dig_in\":{}}}",
                a.id,
                json_str(&a.name),
                place_json(&a.province),
                json_int(a.men),
                a.regiments,
                json_opt(a.strength),
                json_opt(a.organisation),
                json_str(a.doing),
                opt_place(&a.to),
                json_opt_str(a.leader.as_deref()),
                json_num(a.dig_in)
            )
        })
        .collect();
    let navies: Vec<String> = v
        .navies
        .iter()
        .map(|n| {
            format!(
                "{{\"id\":{},\"name\":{},\"province\":{},\"ships\":{},\"strength\":{},\"organisation\":{},\"doing\":{},\"to\":{},\"leader\":{},\"carrying\":{}}}",
                n.id,
                json_str(&n.name),
                place_json(&n.province),
                n.ships,
                json_opt(n.strength),
                json_opt(n.organisation),
                json_str(n.doing),
                opt_place(&n.to),
                json_opt_str(n.leader.as_deref()),
                n.carrying
            )
        })
        .collect();
    let units: Vec<String> = v
        .units
        .iter()
        .map(|u| {
            let goods: Vec<String> = u.goods.iter().map(|(k, x)| format!("[{},{}]", json_str(k), json_num(*x))).collect();
            format!(
                "{{\"key\":{},\"name\":{},\"land\":{},\"kind\":{},\"active\":{},\"days\":{},\"cost\":{},\"goods\":[{}]}}",
                json_str(&u.key),
                json_str(&u.name),
                u.land,
                json_str(&u.kind),
                u.active,
                json_num(u.days),
                json_opt(u.cost),
                goods.join(",")
            )
        })
        .collect();
    let building: Vec<String> = v
        .building
        .iter()
        .map(|b| {
            format!(
                "{{\"name\":{},\"kind\":{},\"kind_name\":{},\"land\":{},\"province\":{},\"progress\":{},\"ends\":{}}}",
                json_str(&b.name),
                json_str(&b.kind),
                json_str(&b.kind_name),
                b.land,
                place_json(&b.province),
                json_opt(b.progress),
                json_opt_str(b.ends.map(|d| d.to_string()).as_deref())
            )
        })
        .collect();
    // Which of the window's orders the engine takes from a player (only while the seat is played).
    let orders = format!("{{\"move_army\":{},\"build\":false,\"mobilise\":false,\"leaders\":false}}", v.played);
    format!(
        "{{\"tag\":{},\"name\":{},\"played\":{},\"stats\":{stats},\"mobilisation\":{mobilisation},\"leaders\":[{}],\"armies\":[{}],\"navies\":[{}],\"units\":[{}],\"building\":[{}],\"orders\":{orders}}}",
        json_str(&v.tag),
        json_str(&v.name),
        v.played,
        leaders.join(","),
        armies.join(","),
        navies.join(","),
        units.join(","),
        building.join(",")
    )
}

/// `GET /api/military` for the seat; `null` for a country the game does not know.
pub(super) fn military_json(sh: &Shared, g: &Game, tag: &str) -> String {
    gather(sh, g, tag).map_or("null".into(), |v| view_json(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_numbers_are_null_and_rows_are_written() {
        let v = View {
            tag: "BBB".into(),
            name: "Country B".into(),
            played: true,
            stats: Stats { war_exhaustion: Some(2.5), combat_width: Some(30.0), regiments: 3, ships: 1, ..Stats::default() },
            mobilisation: Mobilisation { regiments: Some(12), ..Mobilisation::default() },
            leaders: vec![LeaderRow { id: 7, name: "General One".into(), personality: ("bold".into(), "Bold".into()), traits: vec![("attack", 1.0)], assigned: Some(("army", 3, "First \"Army\"".into())), ..LeaderRow::default() }],
            armies: vec![ArmyRow { id: 3, name: "First Army".into(), province: (12, "Province One".into()), men: 2999.6, regiments: 1, strength: Some(0.5), organisation: None, doing: "moving", to: Some((13, "Province Two".into())), ..ArmyRow::default() }],
            navies: vec![NavyRow { id: 4, name: "First Fleet".into(), province: (900, "Sea One".into()), ships: 2, doing: "at_sea", ..NavyRow::default() }],
            units: vec![UnitRow { key: "foot".into(), name: "Foot".into(), land: true, active: true, days: 120.0, cost: None, goods: vec![("small_arms".into(), 10.0)], ..UnitRow::default() }],
            building: vec![BuildRow { name: "New One".into(), kind: "foot".into(), kind_name: "Foot".into(), land: true, province: (12, "Province One".into()), progress: Some(0.25), ends: Some(Date::new(1836, 5, 1)) }],
        };
        let j = view_json(&v);
        for want in [
            "\"war_exhaustion\":2.5",
            "\"supply_consumption\":null",
            "\"start_experience\":null",
            "\"recruit_time\":null",
            "\"combat_width\":30.0",
            "\"regiments\":12,\"production_loss\":null",
            "\"personality\":{\"key\":\"bold\",\"name\":\"Bold\"},\"background\":null",
            "\"traits\":[[\"attack\",1.0]]",
            "\"assigned\":{\"kind\":\"army\",\"id\":3,\"name\":\"First \\\"Army\\\"\"}",
            "\"men\":3000",
            "\"organisation\":null,\"doing\":\"moving\",\"to\":{\"id\":13,\"name\":\"Province Two\"}",
            "\"doing\":\"at_sea\",\"to\":null,\"leader\":null,\"carrying\":0",
            "\"cost\":null,\"goods\":[[\"small_arms\",10.0]]",
            "\"progress\":0.25,\"ends\":\"1836.5.1\"",
            "\"orders\":{\"move_army\":true,\"build\":false,\"mobilise\":false,\"leaders\":false}",
        ] {
            assert!(j.contains(want), "{want} not in {j}");
        }
        assert_eq!(j.matches('{').count(), j.matches('}').count());
        assert_eq!(j.matches('[').count(), j.matches(']').count());
    }

    #[test]
    fn day_numbers_count_forward() {
        assert_eq!(day_no(Date::new(1836, 1, 2)) - day_no(Date::new(1836, 1, 1)), 1);
        assert_eq!(day_no(Date::new(1837, 1, 1)) - day_no(Date::new(1836, 1, 1)), 365);
        assert_eq!(share(1.5, 3.0), Some(0.5));
        assert_eq!(share(1.0, 0.0), None);
    }
}
