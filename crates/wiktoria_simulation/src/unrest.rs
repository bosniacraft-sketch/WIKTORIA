//! Rebels and political movements as pure rules.
//!
//! Nothing here touches armies: a rising returns a list of [`SpawnOrder`]s ("raise this many
//! regiments for this faction in this province") for the military code to carry out. Winning
//! returns a [`Demands`] value: the type's own effect block is handed back so the existing
//! script runner executes it (for pan-nationalists that block fires the event that creates the
//! union country, which stays script).
//!
//! Constants: `defines.lua` of the install ([`UnrestRules::new`]), the type files of the install
//! (`common/rebel_types.txt`, [`RebelType`]) and, for literals of the program that no file
//! carries, the rules file rows `FLD_UNR_*` (hypotheses or stand-ins, see their notes).

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::defs::{Cultures, IssueGroup};
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{self, Block, Value};

use crate::num_field;

// ---------------------------------------------------------------- rules

/// Install defines plus the rules file's literals.
#[derive(Debug, Clone, PartialEq)]
pub struct UnrestRules {
    // defines.lua
    pub mil_to_join_rebel: f64,
    pub mil_to_join_rising: f64,
    pub mil_to_autorise: f64,
    pub mil_on_reb_move: f64,
    pub reduction_after_rising: f64,
    /// REDUCTION_AFTER_DEFEAT: the militancy of a pop whose rebels are beaten is divided by this.
    pub reduction_after_defeat: f64,
    pub issue_join_limit: f64,
    pub issue_leave_limit: f64,
    pub lit_factor: f64,
    pub con_factor: f64,
    pub nationalist_mil_cap: f64,
    pub radicalism_base: f64,
    pub passed_reform_effect: f64,
    pub nationalism_factor: f64,
    pub pop_radical_factor: f64,
    pub suppression_hit: f64,
    pub max_suppression: f64,
    pub suppress_bureaucrat_factor: f64,
    pub suppression_gain_base: f64,
    pub population_suppression_factor: f64,
    pub prestige_hit_on_break: f64,
    pub pop_min_size_for_regiment: f64,
    pub pop_size_per_regiment: f64,
    pub support_uh_factor: f64,
    pub occupation_bonus: f64,
    // rules file (FLD_UNR_*)
    pub join_con_max: f64,
    pub join_lit_min: f64,
    pub join_n_scale: f64,
    pub join_n_min: f64,
    pub random_range: u32,
    pub indep_roll: u32,
    pub support_total: f64,
    pub radicalism_turn: f64,
    pub org_money_offset: f64,
    pub org_money_div: f64,
    pub org_mul_autorise: f64,
    pub org_mul_rising: f64,
    pub org_div: f64,
    /// The saved money times this stands in for the money counter of the day.
    pub org_money_fit: f64,
    pub rise_day_mod: u64,
    pub rise_org_scale: f64,
    pub rise_army_scale: f64,
    pub rise_org_base: f64,
    pub rise_offset: f64,
    pub rise_supply_div: f64,
    pub suppress_consciousness: f64,
}

const DEFINE_SECTIONS: [&str; 7] = ["country", "economy", "military", "diplomacy", "pops", "ai", "graphics"];

fn define(d: &Defines, key: &str) -> Result<f64, String> {
    DEFINE_SECTIONS.iter().find_map(|s| d.get(s, key)).ok_or(format!("defines.lua: {key} missing"))
}

impl UnrestRules {
    pub fn new(m: &EngineRules, d: &Defines) -> Result<UnrestRules, String> {
        let f = |id: &str| num_field(m, id);
        Ok(UnrestRules {
            mil_to_join_rebel: define(d, "MIL_TO_JOIN_REBEL")?,
            mil_to_join_rising: define(d, "MIL_TO_JOIN_RISING")?,
            mil_to_autorise: define(d, "MIL_TO_AUTORISE")?,
            mil_on_reb_move: define(d, "MIL_ON_REB_MOVE")?,
            reduction_after_rising: define(d, "REDUCTION_AFTER_RISEING")?,
            reduction_after_defeat: define(d, "REDUCTION_AFTER_DEFEAT")?,
            issue_join_limit: define(d, "ISSUE_MOVEMENT_JOIN_LIMIT")?,
            issue_leave_limit: define(d, "ISSUE_MOVEMENT_LEAVE_LIMIT")?,
            lit_factor: define(d, "MOVEMENT_LIT_FACTOR")?,
            con_factor: define(d, "MOVEMENT_CON_FACTOR")?,
            nationalist_mil_cap: define(d, "NATIONALIST_MOVEMENT_MIL_CAP")?,
            radicalism_base: define(d, "MOVEMENT_RADICALISM_BASE")?,
            passed_reform_effect: define(d, "MOVEMENT_RADICALISM_PASSED_REFORM_EFFECT")?,
            nationalism_factor: define(d, "MOVEMENT_RADICALISM_NATIONALISM_FACTOR")?,
            pop_radical_factor: define(d, "POPULATION_MOVEMENT_RADICAL_FACTOR")?,
            suppression_hit: define(d, "SUPPRESSION_RADICALISATION_HIT")?,
            max_suppression: define(d, "MAX_SUPPRESSION")?,
            suppress_bureaucrat_factor: define(d, "SUPPRESS_BUREAUCRAT_FACTOR")?,
            suppression_gain_base: define(d, "SUPPRESSION_POINTS_GAIN_BASE")?,
            population_suppression_factor: define(d, "POPULATION_SUPPRESSION_FACTOR")?,
            prestige_hit_on_break: define(d, "PRESTIGE_HIT_ON_BREAK_COUNTRY")?,
            pop_min_size_for_regiment: define(d, "POP_MIN_SIZE_FOR_REGIMENT")?,
            pop_size_per_regiment: define(d, "POP_SIZE_PER_REGIMENT")?,
            support_uh_factor: define(d, "MOVEMENT_SUPPORT_UH_FACTOR")?,
            occupation_bonus: define(d, "REBEL_OCCUPATION_STRENGTH_BONUS")?,
            join_con_max: f("FLD_UNR_JOIN_CON_MAX")?,
            join_lit_min: f("FLD_UNR_JOIN_LIT_MIN")?,
            join_n_scale: f("FLD_UNR_JOIN_N_SCALE")?,
            join_n_min: f("FLD_UNR_JOIN_N_MIN")?,
            random_range: f("FLD_UNR_RANDOM_RANGE")? as u32,
            indep_roll: f("FLD_UNR_INDEP_ROLL")? as u32,
            support_total: f("FLD_UNR_SUPPORT_TOTAL")?,
            radicalism_turn: f("FLD_UNR_RADICALISM_TURN")?,
            org_money_offset: f("FLD_UNR_ORG_MONEY_OFFSET")?,
            org_money_div: f("FLD_UNR_ORG_MONEY_DIV")?,
            org_mul_autorise: f("FLD_UNR_ORG_MUL_AUTORISE")?,
            org_mul_rising: f("FLD_UNR_ORG_MUL_RISING")?,
            org_div: f("FLD_UNR_ORG_DIV")?,
            org_money_fit: f("FLD_UNR_ORG_MONEY_FIT")?,
            rise_day_mod: f("FLD_UNR_RISE_DAY_MOD")? as u64,
            rise_org_scale: f("FLD_UNR_RISE_ORG_SCALE")?,
            rise_army_scale: f("FLD_UNR_RISE_ARMY_SCALE")?,
            rise_org_base: f("FLD_UNR_RISE_ORG_BASE")?,
            rise_offset: f("FLD_UNR_RISE_OFFSET")?,
            rise_supply_div: f("FLD_UNR_RISE_SUPPLY_DIV")?,
            suppress_consciousness: f("FLD_UNR_SUPPRESS_CONSCIOUSNESS")?,
        })
    }
}

// ---------------------------------------------------------------- rebel types (the install's file)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Nation,
    Culture,
    CultureGroup,
    NationCulture,
    NationReligion,
    Religion,
    Other,
}

/// `defection` and `independence` share this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    Culture,
    CultureGroup,
    PanNationalist,
    Religion,
    Colonial,
    Ideology,
    None,
    Any,
}

impl Aim {
    fn parse(s: &str) -> Aim {
        match s {
            "culture" => Aim::Culture,
            "culture_group" => Aim::CultureGroup,
            "pan_nationalist" => Aim::PanNationalist,
            "religion" => Aim::Religion,
            "colonial" => Aim::Colonial,
            "ideology" => Aim::Ideology,
            "none" => Aim::None,
            _ => Aim::Any,
        }
    }
}

impl Area {
    fn parse(s: &str) -> Area {
        match s {
            "nation" => Area::Nation,
            "culture" => Area::Culture,
            "culture_group" => Area::CultureGroup,
            "nation_culture" => Area::NationCulture,
            "nation_religion" => Area::NationReligion,
            "religion" => Area::Religion,
            _ => Area::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RebelType {
    pub name: String,
    pub area: Area,
    pub defection: Aim,
    pub independence: Aim,
    pub defect_delay: u32,
    /// The ideology the type belongs to (`None` for types without one).
    pub ideology: Option<String>,
    pub allow_all_cultures: bool,
    pub allow_all_religions: bool,
    pub allow_all_ideologies: bool,
    pub allow_all_culture_groups: bool,
    pub unit_transfer: bool,
    pub break_alliance_on_win: bool,
    pub occupation_mult: f64,
    /// Government in force -> government the rebels want.
    pub government: BTreeMap<String, String>,
    /// Run on the country when the demands are enforced (script, handed to the effect runner).
    pub demands_enforced_effect: Option<Block>,
    pub demands_enforced_trigger: Option<Block>,
    /// Conditions with factors; the caller evaluates them on a pop with the script engine.
    pub spawn_chance: Option<Block>,
}

fn word(b: &Block, k: &str) -> Option<String> {
    b.get(k).and_then(Value::as_str).map(str::to_string)
}

impl RebelType {
    pub fn parse(name: &str, b: &Block) -> RebelType {
        let yes = |k: &str, d: bool| b.get(k).and_then(Value::as_bool).unwrap_or(d);
        RebelType {
            name: name.to_string(),
            area: Area::parse(&word(b, "area").unwrap_or_default()),
            defection: Aim::parse(&word(b, "defection").unwrap_or_else(|| "none".into())),
            independence: Aim::parse(&word(b, "independence").unwrap_or_else(|| "none".into())),
            defect_delay: b.get("defect_delay").and_then(Value::as_f64).unwrap_or(0.0) as u32,
            ideology: word(b, "ideology"),
            allow_all_cultures: yes("allow_all_cultures", false),
            allow_all_religions: yes("allow_all_religions", false),
            allow_all_ideologies: yes("allow_all_ideologies", false),
            allow_all_culture_groups: yes("allow_all_culture_groups", false),
            unit_transfer: yes("unit_transfer", false),
            break_alliance_on_win: yes("break_alliance_on_win", false),
            occupation_mult: b.get("occupation_mult").and_then(Value::as_f64).unwrap_or(1.0),
            government: b
                .get("government")
                .and_then(Value::as_block)
                .map(|g| g.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default(),
            demands_enforced_effect: b.get("demands_enforced_effect").and_then(Value::as_block).cloned(),
            demands_enforced_trigger: b.get("demands_enforced_trigger").and_then(Value::as_block).cloned(),
            spawn_chance: b.get("spawn_chance").and_then(Value::as_block).cloned(),
        }
    }

    /// Reads every type of `rebel_types.txt` in file order (file order matters: ties go to the first).
    pub fn parse_file(bytes: &[u8]) -> Result<Vec<RebelType>, String> {
        let doc = script::parse_bytes(bytes).map_err(|e| format!("rebel_types.txt: {e}"))?;
        Ok(doc.root.entries().filter_map(|(k, v, _)| Some(RebelType::parse(k, v.as_block()?))).collect())
    }
}

/// The union country of a culture's group (`union = GER` in cultures.txt), if it has one.
pub fn union_tag(cultures: &Cultures, culture: &str) -> Option<String> {
    let c = cultures.cultures.get(cultures.index.get(culture)? as usize)?;
    cultures.groups.get(c.group as usize)?.union.clone()
}

/// The country a faction creates or joins: the pan-nationalist case is the union tag of the
/// culture group (the event that makes it real is script), otherwise what the faction carries.
pub fn faction_country(t: &RebelType, faction_culture: &str, explicit: Option<&str>, cultures: &Cultures) -> Option<String> {
    if t.defection == Aim::PanNationalist || t.independence == Aim::PanNationalist {
        union_tag(cultures, faction_culture)
    } else {
        explicit.filter(|s| *s != "---" && !s.is_empty()).map(str::to_string)
    }
}

// ---------------------------------------------------------------- pops and factions

/// What the rules ask of a pop.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PopFacts {
    pub id: u64,
    pub size: f64,
    pub literacy: f64,
    pub consciousness: f64,
    pub militancy: f64,
    pub culture: String,
    pub culture_group: String,
    pub religion: String,
    /// Name of the ideology with the largest support.
    pub top_ideology: Option<String>,
    pub in_movement: bool,
    pub in_faction: bool,
    /// The pop's province lies in another land area than the owner's main one.
    pub other_land_area: bool,
}

/// A faction as the rules see it (and as a save writes it).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Faction {
    pub id: u64,
    pub kind: String,
    pub name: String,
    pub country: String,
    /// The country it wants to create (`independence = "PHI"`), `None` for `"---"`.
    pub creates: Option<String>,
    pub culture: String,
    pub culture_group: String,
    pub religion: String,
    pub government: String,
    pub province: u32,
    pub leader: Option<u64>,
    pub organization: f64,
    pub pops: Vec<u64>,
}

/// Whole brigades a pop supports: 0 below the minimum size, else size / per + 1.
pub fn brigades(r: &UnrestRules, size: f64) -> u32 {
    if size < r.pop_min_size_for_regiment {
        0
    } else {
        (size / r.pop_size_per_regiment).floor() as u32 + 1
    }
}

/// Join rule: militancy above MIL_TO_JOIN_REBEL, in no faction, in no movement.
pub fn may_join_faction(r: &UnrestRules, p: &PopFacts) -> bool {
    !p.in_faction && !p.in_movement && p.militancy > r.mil_to_join_rebel
}

/// Leave rule: militancy below MIL_TO_JOIN_REBEL.
pub fn leaves_faction(r: &UnrestRules, p: &PopFacts) -> bool {
    p.militancy < r.mil_to_join_rebel
}

/// Which type a pop picks: every type in file order, skipped unless it allows all ideologies or its
/// ideology is the pop's largest; `value` is the type's spawn_chance on this pop (0 = skipped, the
/// highest strictly wins, so it is a ranking, not a chance); a type that wants a country is
/// accepted only when `finds_country` says one exists.
pub fn pick_rebel_type<'a>(
    types: &'a [RebelType],
    pop: &PopFacts,
    value: &dyn Fn(&RebelType) -> f64,
    finds_country: &dyn Fn(&RebelType) -> bool,
) -> Option<&'a RebelType> {
    let mut best: Option<(&RebelType, f64)> = None;
    for t in types {
        if !t.allow_all_ideologies && t.ideology.as_deref() != pop.top_ideology.as_deref() {
            continue;
        }
        let v = value(t);
        if v <= 0.0 || best.is_some_and(|(_, b)| v <= b) {
            continue;
        }
        let needs_country = t.independence != Aim::None || pop.other_land_area;
        if needs_country && !finds_country(t) {
            continue;
        }
        best = Some((t, v));
    }
    best.map(|(t, _)| t)
}

/// A faction takes a pop when the country it would create matches, and culture group, culture,
/// religion and ideology agree unless the type allows all of them.
pub fn faction_accepts(t: &RebelType, f: &Faction, pop: &PopFacts, pop_creates: Option<&str>) -> bool {
    if f.kind != t.name {
        return false;
    }
    if f.creates.is_some() && pop_creates != f.creates.as_deref() {
        return false;
    }
    (t.allow_all_culture_groups || f.culture_group == pop.culture_group)
        && (t.allow_all_cultures || f.culture == pop.culture)
        && (t.allow_all_religions || f.religion == pop.religion)
        && (t.allow_all_ideologies || t.ideology.as_deref() == pop.top_ideology.as_deref())
}

/// Newcomers dilute organisation: old brigades / (old + new).
pub fn organisation_after_join(org: f64, old_brigades: u32, new_brigades: u32) -> f64 {
    let total = old_brigades + new_brigades;
    if total == 0 {
        org
    } else {
        org * f64::from(old_brigades) / f64::from(total)
    }
}

/// A faction is finished when it has no members, its aim cannot be reached or the country it wants
/// already exists. `country_owns_land`: the country it rises against; `wanted_exists`: the country
/// it wants owns a province.
pub fn faction_finished(t: &RebelType, f: &Faction, country_owns_land: bool, wanted_exists: bool, found_country_to_create: bool, union_is_own_country: bool) -> bool {
    (!country_owns_land && matches!(t.area, Area::Nation | Area::NationCulture))
        || (f.creates.is_some() && wanted_exists)
        || (t.independence != Aim::None && !found_country_to_create && f.creates.is_none())
        || f.pops.is_empty()
        || (t.defection == Aim::PanNationalist && union_is_own_country)
}

// ---------------------------------------------------------------- organisation and rising

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Member {
    /// The pop's money counter of the day (`x`), literacy, militancy and brigades.
    pub money_today: f64,
    pub literacy: f64,
    pub militancy: f64,
    pub brigades: u32,
}

/// Organisation gained today. `admin_spending` is S, `tech_bonus` the technology value for the
/// type. Result is in 0..1 (the caller caps the organisation itself at 1).
pub fn organisation_gain(r: &UnrestRules, members: &[Member], admin_spending: f64, tech_bonus: f64) -> f64 {
    let total: u32 = members.iter().map(|m| m.brigades).sum();
    if total == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    for m in members {
        let x = m.money_today + r.org_money_offset;
        if x < 0.0 {
            continue;
        }
        let mut g = x / r.org_money_div * m.literacy / (1.0 + admin_spending);
        if m.militancy > r.mil_to_autorise {
            g *= r.org_mul_autorise;
        } else if m.militancy > r.mil_to_join_rising {
            g *= r.org_mul_rising;
        }
        sum += g;
    }
    (sum / f64::from(total) * (1.0 + tech_bonus) / r.org_div).clamp(0.0, 1.0)
}

/// The faction's own day of the month for the rising roll: drawn as (random mod 30), redrawn while 0.
pub fn rising_day(r: &UnrestRules, mut random: impl FnMut() -> u64) -> u64 {
    loop {
        let d = random() % r.rise_day_mod;
        if d != 0 {
            return d;
        }
    }
}

/// Chance in 0..1000 of a general rising on the faction's day. `angry_brigades` (R) are the brigades of
/// members with militancy above MIL_TO_JOIN_RISING, `army_regiments` (G) the country's regiments.
/// No roll at all when R = 0.
pub fn rising_chance(r: &UnrestRules, organisation: f64, angry_brigades: u32, army_regiments: u32) -> Option<f64> {
    if angry_brigades == 0 {
        return None;
    }
    let c = organisation * r.rise_org_scale * f64::from(angry_brigades) / (r.rise_army_scale * f64::from(army_regiments)).max(1.0)
        + r.rise_org_base * organisation
        + r.rise_offset;
    Some(c.clamp(0.0, r.rise_org_scale))
}

/// The roll: rising when (random mod 1000) is below the chance.
pub fn rises(r: &UnrestRules, chance: f64, random: u64) -> bool {
    ((random % u64::from(r.random_range)) as f64) < chance
}

/// A member in the rising, with what the military code knows about its province.
#[derive(Debug, Clone, PartialEq)]
pub struct RisingPop {
    pub pop_id: u64,
    pub size: f64,
    pub militancy: f64,
    /// Brigades this pop already has in service.
    pub brigades_in_service: u32,
    /// Province where the regiments appear: a faction army's province, else the pop's own.
    pub province: u32,
    /// Supply limit of that province for REB (the cut is a third of it, rounded).
    pub rebel_supply_limit: f64,
}

/// "Spawn these regiments here for this faction."
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnOrder {
    pub faction: u64,
    pub pop_id: u64,
    pub province: u32,
    pub regiments: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RisingResult {
    pub orders: Vec<SpawnOrder>,
    /// New militancy per pop id (reduced by REDUCTION_AFTER_RISEING, 0 in the install, no clamp).
    pub militancy: BTreeMap<u64, f64>,
    /// Organisation after the rising.
    pub organisation: f64,
}

/// A general rising: every member at or above MIL_TO_JOIN_RISING raises
/// floor((its brigades - those in service) x organisation) regiments, cut to a third of the
/// province's supply limit.
pub fn rise(r: &UnrestRules, f: &Faction, members: &[RisingPop]) -> RisingResult {
    let mut orders = Vec::new();
    let mut militancy = BTreeMap::new();
    for m in members.iter().filter(|m| m.militancy >= r.mil_to_join_rising) {
        let may = brigades(r, m.size);
        if may == 0 {
            continue;
        }
        let n = may.saturating_sub(m.brigades_in_service);
        let cap = (m.rebel_supply_limit / r.rise_supply_div).round().max(0.0) as u32;
        let count = ((f64::from(n) * f.organization).floor() as u32).min(cap);
        if count > 0 {
            orders.push(SpawnOrder { faction: f.id, pop_id: m.pop_id, province: m.province, regiments: count });
        }
        militancy.insert(m.pop_id, m.militancy - r.reduction_after_rising);
    }
    RisingResult { orders, militancy, organisation: 0.0 }
}

/// Organisation change when a province becomes REB-held (the type's multiplier does not apply to
/// defection = culture) and the same amount taken off again when it is lost.
pub fn occupation_bonus(r: &UnrestRules, t: &RebelType) -> f64 {
    if t.defection == Aim::Culture {
        r.occupation_bonus
    } else {
        r.occupation_bonus * t.occupation_mult
    }
}

// ---------------------------------------------------------------- winning and defection

/// What a won faction asks the rest of the program to do.
#[derive(Debug, Clone, PartialEq)]
pub struct Demands<'a> {
    /// The type's effect block: hand to the script runner on the country. For pan-nationalists this
    /// is where the event that creates the union country fires (script, not Rust).
    pub effect: Option<&'a Block>,
    /// Government after the break: the type's entry for the present government, if different.
    pub government: Option<String>,
    /// Prestige factor: prestige changes by (prestige + tech part) x this (PRESTIGE_HIT_ON_BREAK_COUNTRY).
    pub prestige_hit: f64,
    /// Country created or joined (the union tag for pan-nationalists).
    pub creates: Option<String>,
    pub break_alliances: bool,
    /// Faction armies go to the created country (true) or are deleted (false).
    pub unit_transfer: bool,
}

pub fn demands_enforced<'a>(r: &UnrestRules, t: &'a RebelType, f: &Faction, current_government: &str, cultures: &Cultures) -> Demands<'a> {
    Demands {
        effect: t.demands_enforced_effect.as_ref(),
        government: t.government.get(current_government).filter(|g| g.as_str() != current_government).cloned(),
        prestige_hit: r.prestige_hit_on_break,
        creates: faction_country(t, &f.culture, f.creates.as_deref(), cultures),
        break_alliances: t.break_alliance_on_win,
        unit_transfer: t.unit_transfer,
    }
}

/// Prestige after the break: (prestige + technology prestige) x PRESTIGE_HIT_ON_BREAK_COUNTRY is added.
pub fn prestige_after_break(r: &UnrestRules, prestige: f64, tech_prestige: f64) -> f64 {
    prestige + (prestige + tech_prestige) * r.prestige_hit_on_break
}

/// Whole months from `from` to `to`.
pub fn months_between(from: Date, to: Date) -> i32 {
    let m = (to.year - from.year) * 12 + i32::from(to.month) - i32::from(from.month);
    if to.day < from.day {
        m - 1
    } else {
        m
    }
}

/// A province held by the faction defects once held for `defect_delay` months.
pub fn defection_due(t: &RebelType, controller_since: Date, today: Date) -> bool {
    t.defection != Aim::None && months_between(controller_since, today) >= t.defect_delay as i32
}

/// A country that could receive a defecting province.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Candidate {
    pub tag: String,
    pub primary_culture: String,
    pub culture_group: String,
    pub religion: String,
    pub government: String,
    pub union_of_group: bool,
    pub neighbour: bool,
    pub distance: f64,
}

/// Best receiving country by the type's defection kind (stand-in ranking: match count first, then
/// neighbour, then nearness; the original's weights were not taken down). Never the owner.
pub fn defection_target<'a>(t: &RebelType, f: &Faction, owner: &str, candidates: &'a [Candidate]) -> Option<&'a Candidate> {
    let wanted = t.government.values().next().cloned();
    let score = |c: &Candidate| -> u32 {
        let same_culture = c.primary_culture == f.culture;
        let same_group = c.culture_group == f.culture_group;
        let same_religion = c.religion == f.religion;
        let ideology = wanted.as_deref() == Some(c.government.as_str());
        let by = |k: Aim| match k {
            Aim::Culture => 2 * u32::from(same_culture) + u32::from(same_group),
            Aim::CultureGroup => u32::from(same_group),
            Aim::PanNationalist => 2 * u32::from(c.union_of_group && same_group),
            Aim::Religion => u32::from(same_religion),
            Aim::Ideology => u32::from(ideology),
            Aim::Any => u32::from(same_culture) + u32::from(same_group) + u32::from(same_religion) + u32::from(ideology),
            Aim::Colonial | Aim::None => 0,
        };
        by(t.defection)
    };
    if t.defection == Aim::None {
        return None;
    }
    candidates
        .iter()
        .filter(|c| c.tag != owner && score(c) > 0)
        .max_by(|a, b| {
            (score(a), a.neighbour).cmp(&(score(b), b.neighbour)).then(b.distance.partial_cmp(&a.distance).unwrap_or(std::cmp::Ordering::Equal))
        })
}

/// When a faction ends, every member and every pop that would pick the same type has militancy 0.
pub const FINISHED_MILITANCY: f64 = 0.0;

// ---------------------------------------------------------------- movements

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MovementKind {
    /// For a political or social reform option.
    Issue,
    /// For the independence of a country.
    Independence,
}

/// A movement in a country (issue name or the wanted tag, size in people, radicalism).
#[derive(Debug, Clone, PartialEq)]
pub struct Movement {
    pub country: String,
    pub kind: MovementKind,
    /// Reform option name, or the tag wanted.
    pub target: String,
    pub support: f64,
    /// Radicalism as the save caches it.
    pub radicalism: f64,
}

/// A reform option a pop could ask for.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueOption {
    pub name: String,
    /// Support in points of 100.
    pub support: f64,
    /// Political or social reform whose own condition (if any) holds for the country.
    pub available: bool,
}

/// No movement for a pop with consciousness below the limit and literacy above the minimum
/// (the rule description reads this test as inverted; kept as written).
pub fn blocked_from_movements(r: &UnrestRules, p: &PopFacts) -> bool {
    p.consciousness < r.join_con_max && p.literacy > r.join_lit_min
}

/// The option the pop leans to (highest support strictly above ISSUE_MOVEMENT_JOIN_LIMIT) and its
/// joining number N = floor((literacy x LIT + consciousness x CON) x support x 9).
pub fn issue_choice<'a>(r: &UnrestRules, p: &PopFacts, options: &'a [IssueOption]) -> Option<(&'a IssueOption, f64)> {
    let best = options
        .iter()
        .filter(|o| o.available && o.support > r.issue_join_limit)
        .fold(None::<&IssueOption>, |b, o| if b.is_none_or(|b| o.support > b.support) { Some(o) } else { b })?;
    let n = ((p.literacy * r.lit_factor + p.consciousness * r.con_factor) * best.support * r.join_n_scale).floor();
    Some((best, n))
}

/// Joins when N > 10 and N > the pop's random number (0..999); the chance is about N / 1000.
pub fn joins_issue(r: &UnrestRules, n: f64, random: u32) -> bool {
    n > r.join_n_min && n > f64::from(random % r.random_range)
}

/// Independence movement: random number below 20 (2 % a month), militancy above the cap, a culture
/// other than the owner's. The first suitable core is chosen by the caller.
pub fn joins_independence(r: &UnrestRules, p: &PopFacts, owner_primary_culture: &str, random: u32) -> bool {
    random % r.random_range < r.indep_roll && p.militancy > r.nationalist_mil_cap && p.culture != owner_primary_culture
}

/// A member stays in an issue movement while its support exceeds ISSUE_MOVEMENT_LEAVE_LIMIT.
pub fn stays_in_issue_movement(r: &UnrestRules, support_for_option: f64) -> bool {
    support_for_option > r.issue_leave_limit
}

/// A member stays in an independence movement while militancy >= NATIONALIST_MOVEMENT_MIL_CAP.
pub fn stays_in_independence_movement(r: &UnrestRules, militancy: f64) -> bool {
    militancy >= r.nationalist_mil_cap
}

/// Inputs of the radicalism formula.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RadicalismInput {
    pub suppression_part: f64,
    pub reform_steps_passed: f64,
    pub size: f64,
    pub country_population: f64,
    /// Independence movements only: culture value of the wanted country's primary culture
    /// (`radicalism` in cultures.txt) and the highest nationalism among provinces with its core.
    pub independence: Option<(f64, f64)>,
}

pub fn radicalism(r: &UnrestRules, i: &RadicalismInput) -> f64 {
    let mut v = r.radicalism_base + i.suppression_part + i.reform_steps_passed * r.passed_reform_effect;
    if let Some((culture_value, nationalism)) = i.independence {
        v += culture_value + nationalism * r.nationalism_factor;
    }
    if i.country_population > 0.0 {
        v += i.size / i.country_population * r.pop_radical_factor;
    }
    v.max(0.0)
}

/// Reform steps passed: for every group of the country's political reforms, the index of the option
/// in force (the rule description says "except the first group"; against the 1850 save the group that file order
/// puts first, slavery, counts too:). `current` is the option name per group.
pub fn reform_steps_passed(groups: &[IssueGroup], current: &dyn Fn(&str) -> Option<String>) -> f64 {
    groups
        .iter()
        .filter(|g| g.category == "political_reforms")
        .filter_map(|g| g.options.iter().position(|o| Some(o) == current(&g.name).as_ref()))
        .sum::<usize>() as f64
}

/// Political or social strength of a country's movements, x1000 in the original; here a plain
/// number: members / population x MOVEMENT_SUPPORT_UH_FACTOR.
pub fn movement_strength(r: &UnrestRules, member_sizes: f64, population: f64) -> f64 {
    if population <= 0.0 {
        0.0
    } else {
        member_sizes / population * r.support_uh_factor
    }
}

/// Suppression points gained a month: bureaucrat share x factor x base x (1 + technology + modifier),
/// the country holds at most MAX_SUPPRESSION.
pub fn suppression_gain(r: &UnrestRules, bureaucrats: f64, population: f64, tech: f64, modifier: f64) -> f64 {
    if population <= 0.0 {
        return 0.0;
    }
    bureaucrats / population * r.suppress_bureaucrat_factor * r.suppression_gain_base * (1.0 + tech + modifier)
}

pub fn add_suppression(r: &UnrestRules, points: f64, gain: f64) -> f64 {
    (points + gain).clamp(0.0, r.max_suppression)
}

/// Cost of suppressing: radicalism + 1, or when POPULATION_SUPPRESSION_FACTOR is not 0 the larger of
/// that and size / factor x radicalism.
pub fn suppression_cost(r: &UnrestRules, radicalism: f64, size: f64) -> f64 {
    let base = radicalism + 1.0;
    if r.population_suppression_factor != 0.0 {
        base.max(size / r.population_suppression_factor * radicalism)
    } else {
        base
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suppressed {
    pub points: f64,
    /// Added to the movement's suppression part of radicalism.
    pub radicalism_added: f64,
    /// Every member leaves, with consciousness set to this value.
    pub member_consciousness: f64,
}

/// The suppress command; `None` when the country lacks the points.
pub fn suppress(r: &UnrestRules, points: f64, radicalism: f64, size: f64) -> Option<Suppressed> {
    let cost = suppression_cost(r, radicalism, size);
    (points >= cost).then(|| Suppressed {
        points: (points - cost).clamp(0.0, r.max_suppression),
        radicalism_added: r.suppression_hit,
        member_consciousness: r.suppress_consciousness,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementDay {
    Keep,
    /// Obsolete and below the turning point: members leave, movement deleted.
    Dissolve,
    /// Radicalism at the turning point: every member goes through [`turn_rebel_militancy`] and a
    /// faction, then the movement is deleted.
    TurnRebel,
}

pub fn movement_day(r: &UnrestRules, radicalism: f64, obsolete: bool) -> MovementDay {
    if radicalism >= r.radicalism_turn {
        MovementDay::TurnRebel
    } else if obsolete {
        MovementDay::Dissolve
    } else {
        MovementDay::Keep
    }
}

/// Militancy of a member whose movement turns rebel: at least MIL_ON_REB_MOVE.
pub fn turn_rebel_militancy(r: &UnrestRules, militancy: f64) -> f64 {
    militancy.max(r.mil_on_reb_move)
}

// ---------------------------------------------------------------- state from a save

/// Factions and movements of an original save.
#[derive(Debug, Clone, Default)]
pub struct UnrestState {
    pub date: Option<Date>,
    pub factions: Vec<Faction>,
    pub movements: Vec<Movement>,
    /// Pop id -> movement it is in: (kind, target).
    pub pop_movement: BTreeMap<u64, (MovementKind, String)>,
    /// Pop id -> faction id.
    pub pop_faction: BTreeMap<u64, u64>,
    /// Pop id -> (owner country, size) for recounts.
    pub pop_home: BTreeMap<u64, (String, f64)>,
    /// Country -> suppression points, population, political reform steps passed.
    pub suppression: BTreeMap<String, f64>,
    pub population: BTreeMap<String, f64>,
    pub reform_steps: BTreeMap<String, f64>,
    pub primary_culture: BTreeMap<String, String>,
    /// Province id -> (owner, nationalism, cores).
    pub provinces: BTreeMap<u32, (String, f64, Vec<String>)>,
    pub warnings: Vec<String>,
}

fn id_in(b: &Block, k: &str) -> Option<u64> {
    b.get(k)?.as_block()?.get("id")?.as_f64().map(|x| x as u64)
}

impl UnrestState {
    pub fn from_save(save: &SaveGame, cultures: &Cultures, issues: &[IssueGroup]) -> UnrestState {
        let mut s = UnrestState { date: Some(save.date), ..Default::default() };
        let culture_group = |c: &str| -> String {
            cultures.index.get(c).and_then(|i| cultures.cultures.get(i as usize)).and_then(|c| cultures.groups.get(c.group as usize)).map(|g| g.name.clone()).unwrap_or_default()
        };
        for (key, value, _) in save.doc.root.entries() {
            let Some(b) = value.as_block() else { continue };
            if key == "rebel_faction" {
                let Some(id) = id_in(b, "id") else {
                    s.warnings.push("rebel_faction without id".into());
                    continue;
                };
                let culture = word(b, "culture").unwrap_or_default();
                s.factions.push(Faction {
                    id,
                    kind: word(b, "type").unwrap_or_default(),
                    name: word(b, "name").unwrap_or_default(),
                    country: word(b, "country").unwrap_or_default(),
                    creates: word(b, "independence").filter(|t| t != "---"),
                    culture_group: culture_group(&culture),
                    culture,
                    religion: word(b, "religion").unwrap_or_default(),
                    government: word(b, "government").unwrap_or_default(),
                    province: b.get("province").and_then(Value::as_f64).unwrap_or(0.0) as u32,
                    leader: id_in(b, "leader"),
                    organization: b.get("organization").and_then(Value::as_f64).unwrap_or(0.0),
                    pops: b.get_all("pop").filter_map(|v| v.as_block()?.get("id")?.as_f64()).map(|x| x as u64).collect(),
                });
            }
        }
        for tag in &save.countries {
            let Some(b) = save.doc.root.get(tag).and_then(Value::as_block) else { continue };
            for m in b.get_all("movement").filter_map(Value::as_block) {
                let (kind, target) = match (word(m, "issue"), word(m, "tag")) {
                    (Some(i), _) => (MovementKind::Issue, i),
                    (None, Some(t)) => (MovementKind::Independence, t),
                    _ => {
                        s.warnings.push(format!("{tag}: movement with neither issue nor tag"));
                        continue;
                    }
                };
                s.movements.push(Movement {
                    country: tag.clone(),
                    kind,
                    target,
                    support: m.get("support").and_then(Value::as_f64).unwrap_or(0.0),
                    radicalism: m.get("cache").and_then(Value::as_f64).unwrap_or(0.0),
                });
            }
            s.suppression.insert(tag.clone(), b.get("suppression").and_then(Value::as_f64).unwrap_or(0.0));
            let steps = reform_steps_passed(issues, &|g| word(b, g));
            s.reform_steps.insert(tag.clone(), steps);
            if let Some(c) = word(b, "primary_culture") {
                s.primary_culture.insert(tag.clone(), c);
            }
        }
        for (id, p) in &save.provinces {
            let Some(owner) = &p.owner else { continue };
            *s.population.entry(owner.clone()).or_default() += p.population();
            let Some(pb) = save.doc.root.get(&id.to_string()).and_then(Value::as_block) else { continue };
            s.provinces.insert(u32::from(*id), (owner.clone(), pb.get("nationalism").and_then(Value::as_f64).unwrap_or(0.0), p.cores.clone()));
            for (_, v, _) in pb.entries() {
                let Some(pop) = v.as_block() else { continue };
                let (Some(pid), Some(size)) = (pop.get("id").and_then(Value::as_f64), pop.get("size").and_then(Value::as_f64)) else { continue };
                let pid = pid as u64;
                s.pop_home.insert(pid, (owner.clone(), size));
                if let Some(i) = word(pop, "movement_issue") {
                    s.pop_movement.insert(pid, (MovementKind::Issue, i));
                }
                if let Some(t) = word(pop, "movement_tag") {
                    s.pop_movement.insert(pid, (MovementKind::Independence, t));
                }
                if let Some(fid) = id_in(pop, "faction") {
                    s.pop_faction.insert(pid, fid);
                }
            }
        }
        s
    }

    /// Size of a movement recounted from its member pops.
    pub fn recount_support(&self, country: &str, kind: MovementKind, target: &str) -> f64 {
        self.pop_movement
            .iter()
            .filter(|(_, (k, t))| *k == kind && t == target)
            .filter_map(|(pid, _)| self.pop_home.get(pid))
            .filter(|(owner, _)| owner == country)
            .map(|(_, size)| size)
            .sum()
    }

    /// Highest nationalism among the country's provinces that carry a core of `wanted`.
    pub fn nationalism_for(&self, country: &str, wanted: &str) -> f64 {
        self.provinces.values().filter(|(o, _, cores)| o == country && cores.iter().any(|c| c == wanted)).map(|(_, n, _)| *n).fold(0.0, f64::max)
    }

    pub fn factions_of<'a>(&'a self, country: &'a str) -> impl Iterator<Item = &'a Faction> + 'a {
        self.factions.iter().filter(move |f| f.country == country)
    }

    pub fn movements_of<'a>(&'a self, country: &'a str) -> impl Iterator<Item = &'a Movement> + 'a {
        self.movements.iter().filter(move |m| m.country == country)
    }
}
