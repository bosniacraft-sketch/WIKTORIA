//! Wars as an original save holds them: participants, war goals, the history of who joined and
//! left, and the battle results that feed the war score.
//!
//! Rules (declaration, war score, peace, exhaustion) belong to task E of the plan.

use std::collections::BTreeMap;

use wiktoria_data::date::Date;
use wiktoria_data::defines::Defines;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::{Block, Value};

use super::{date, num, text};

/// One war goal (`war_goal` of an active war, or the war's `original_wargoal`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WarGoal {
    pub casus_belli: String,
    pub actor: String,
    pub receiver: String,
    /// A province of the wanted state (`state_province_id`).
    pub state_province: Option<u16>,
    /// The ticking war score of the goal and its last daily change.
    pub score: f64,
    pub change: f64,
    /// `date` of the last change; the save writes `-1.1.1` for "never", read as `None`.
    pub date: Option<Date>,
    pub fulfilled: bool,
}

/// One battle result kept in the war's history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleRecord {
    pub date: Option<Date>,
    pub name: String,
    pub location: u16,
    /// `result = yes`: the attacker won.
    pub attacker_won: bool,
    pub attacker: BattleSide,
    pub defender: BattleSide,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleSide {
    pub country: String,
    pub leader: String,
    /// Men (land) or ships (sea) it brought, by unit type name.
    pub forces: BTreeMap<String, f64>,
    pub losses: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarEventKind {
    AddAttacker(String),
    AddDefender(String),
    RemAttacker(String),
    RemDefender(String),
    Battle(BattleRecord),
    WarGoal(WarGoal),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WarEvent {
    pub date: Date,
    pub kind: WarEventKind,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct War {
    pub name: String,
    /// `active_war` in the save; `previous_war` is a finished one.
    pub active: bool,
    /// Present participants, the first of each side is its war leader. Empty for a finished war.
    pub attackers: Vec<String>,
    pub defenders: Vec<String>,
    pub original_attacker: Option<String>,
    pub original_defender: Option<String>,
    pub original_goal: Option<WarGoal>,
    /// The goals now in force.
    pub goals: Vec<WarGoal>,
    /// The save's `action` date: the last thing that happened in the war.
    pub action: Option<Date>,
    pub history: Vec<WarEvent>,
}

impl War {
    pub fn takes_part(&self, tag: &str) -> bool {
        self.attackers.iter().chain(&self.defenders).any(|t| t == tag)
    }

    pub fn attacker_leader(&self) -> Option<&str> {
        self.attackers.first().map(String::as_str)
    }

    pub fn defender_leader(&self) -> Option<&str> {
        self.defenders.first().map(String::as_str)
    }

    /// The day the first country joined.
    pub fn started(&self) -> Option<Date> {
        self.history.iter().filter(|e| matches!(e.kind, WarEventKind::AddAttacker(_) | WarEventKind::AddDefender(_))).map(|e| e.date).min()
    }

    /// The day the last country left, for a finished war.
    pub fn ended(&self) -> Option<Date> {
        if self.active { None } else { self.history.iter().filter(|e| matches!(e.kind, WarEventKind::RemAttacker(_) | WarEventKind::RemDefender(_))).map(|e| e.date).max() }
    }

    pub fn battles(&self) -> impl Iterator<Item = &BattleRecord> {
        self.history.iter().filter_map(|e| if let WarEventKind::Battle(b) = &e.kind { Some(b) } else { None })
    }
}

pub fn read_goal(b: &Block) -> Option<WarGoal> {
    Some(WarGoal {
        casus_belli: text(b, "casus_belli")?,
        actor: text(b, "actor").unwrap_or_default(),
        receiver: text(b, "receiver").unwrap_or_default(),
        state_province: b.get("state_province_id").and_then(Value::as_f64).map(|x| x as u16),
        score: num(b, "score"),
        change: num(b, "change"),
        date: date(b, "date").filter(|d| d.year > 0),
        fulfilled: b.get("is_fulfilled").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn read_side(b: Option<&Block>) -> BattleSide {
    let Some(b) = b else { return BattleSide::default() };
    let mut side = BattleSide { country: text(b, "country").unwrap_or_default(), leader: text(b, "leader").unwrap_or_default(), losses: num(b, "losses"), forces: BTreeMap::new() };
    // Every other number is a unit type with the strength it brought.
    for (k, v, _) in b.entries() {
        if let (false, Some(n)) = (matches!(k, "country" | "leader" | "losses"), v.as_f64()) {
            *side.forces.entry(k.to_string()).or_default() += n;
        }
    }
    side
}

pub fn read_battle(d: Option<Date>, b: &Block) -> BattleRecord {
    BattleRecord {
        date: d,
        name: text(b, "name").unwrap_or_default(),
        location: num(b, "location") as u16,
        attacker_won: b.get("result").and_then(Value::as_bool).unwrap_or(false),
        attacker: read_side(b.get("attacker").and_then(Value::as_block)),
        defender: read_side(b.get("defender").and_then(Value::as_block)),
    }
}

/// One `active_war` or `previous_war` block.
pub fn read_war(b: &Block, active: bool) -> War {
    let tags = |k: &str| b.get_all(k).filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
    let original = |k: &str| text(b, k).filter(|t| t != "---");
    let mut history = Vec::new();
    if let Some(h) = b.get("history").and_then(Value::as_block) {
        for (k, v, _) in h.entries() {
            let (Some(d), Some(inner)) = (Date::parse(k), v.as_block()) else { continue };
            for (kind, v, _) in inner.entries() {
                let who = || v.as_str().unwrap_or_default().to_string();
                let kind = match kind {
                    "add_attacker" => WarEventKind::AddAttacker(who()),
                    "add_defender" => WarEventKind::AddDefender(who()),
                    "rem_attacker" => WarEventKind::RemAttacker(who()),
                    "rem_defender" => WarEventKind::RemDefender(who()),
                    "battle" => match v.as_block() {
                        Some(x) => WarEventKind::Battle(read_battle(Some(d), x)),
                        None => continue,
                    },
                    "war_goal" => match v.as_block().and_then(read_goal) {
                        Some(g) => WarEventKind::WarGoal(g),
                        None => continue,
                    },
                    _ => continue,
                };
                history.push(WarEvent { date: d, kind });
            }
        }
    }
    War {
        name: text(b, "name").unwrap_or_default().trim().to_string(),
        active,
        attackers: tags("attacker"),
        defenders: tags("defender"),
        original_attacker: original("original_attacker"),
        original_defender: original("original_defender"),
        original_goal: b.get("original_wargoal").and_then(Value::as_block).and_then(read_goal),
        goals: b.get_all("war_goal").filter_map(Value::as_block).filter_map(read_goal).collect(),
        action: date(b, "action"),
        history,
    }
}

/// Running wars first, then finished ones, each in the save's order.
pub fn read_wars(root: &Block) -> Vec<War> {
    let mut wars: Vec<War> = root.get_all("active_war").filter_map(Value::as_block).map(|b| read_war(b, true)).collect();
    wars.extend(root.get_all("previous_war").filter_map(Value::as_block).map(|b| read_war(b, false)));
    wars
}

// ---------------------------------------------------------------------------------------------
// Rules. Pure functions: the caller supplies the facts.
// ---------------------------------------------------------------------------------------------

/// Peace option names as the casus belli lists them (`po_annex` is `"annex"`), the suffix of the
/// per-option tables of the defines (`INFAMY_ANNEX`, `PEACE_COST_ANNEX`...).
pub const PEACE_OPTIONS: [&str; 18] = [
    "add_to_sphere",
    "release_puppet",
    "make_puppet",
    "disarmament",
    "destroy_forts",
    "destroy_naval_bases",
    "reparations",
    "transfer_provinces",
    "remove_cores",
    "prestige",
    "status_quo",
    "annex",
    "demand_state",
    "install_communist_gov_type",
    "uninstall_communist_gov_type",
    "colony",
    "clear_union_sphere",
    "gunboat",
];

/// What one peace option costs and pays, all from the install's defines.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OptionValues {
    pub infamy: f64,
    pub prestige_base: f64,
    pub prestige_share: f64,
    pub break_truce_infamy: f64,
    pub break_truce_prestige: f64,
    pub break_truce_militancy: f64,
    pub peace_cost: f64,
}

/// A casus belli type, as far as the war rules need it (from `common/cb_types.txt` at runtime).
#[derive(Debug, Clone, PartialEq)]
pub struct CbType {
    pub name: String,
    /// The `po_*` flags without the prefix (see [`PEACE_OPTIONS`]).
    pub options: Vec<String>,
    pub badboy_factor: f64,
    pub prestige_factor: f64,
    pub peace_cost_factor: f64,
    pub penalty_factor: f64,
    pub break_truce_infamy_factor: f64,
    pub break_truce_prestige_factor: f64,
    pub break_truce_militancy_factor: f64,
    pub truce_months: f64,
    pub construction_speed: f64,
    pub tws_battle_factor: f64,
    /// The type needs no justification (`always`): a war with it is just without a held one.
    pub needs_no_justification: bool,
    /// `great_war_obligatory`: the goal costs nothing.
    pub zero_cost: bool,
    /// `constructing_cb`: the type may be justified (default yes).
    pub constructing: bool,
}

impl CbType {
    pub fn new(name: &str, options: &[&str]) -> CbType {
        CbType {
            name: name.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            badboy_factor: 1.0,
            prestige_factor: 1.0,
            peace_cost_factor: 1.0,
            penalty_factor: 1.0,
            break_truce_infamy_factor: 1.0,
            break_truce_prestige_factor: 1.0,
            break_truce_militancy_factor: 1.0,
            truce_months: 0.0,
            construction_speed: 1.0,
            tws_battle_factor: 0.0,
            needs_no_justification: false,
            zero_cost: false,
            constructing: true,
        }
    }

    pub fn has(&self, option: &str) -> bool {
        self.options.iter().any(|o| o == option)
    }

    /// Goals that tick up the war score (4.4): territory, annex, or a battle factor.
    pub fn ticks(&self) -> bool {
        self.has("demand_state") || self.has("transfer_provinces") || self.has("annex") || self.tws_battle_factor > 0.0
    }
}

/// Engine constants of the war rules: install defines read at runtime, rules file literals.
#[derive(Debug, Clone, PartialEq)]
pub struct WarRules {
    // defines.lua
    pub cb_base_speed: f64,
    pub cb_detection_chance: f64,
    pub cb_detected_relation: f64,
    pub gw_cb_speed: f64,
    pub cb_colony_bonus: f64,
    pub cb_troops_bonus: f64,
    pub cb_valid_months: f64,
    pub gw_badboy_impact: f64,
    pub max_score_from_battles: f64,
    pub tws_limit: f64,
    pub tws_idle_space: f64,
    pub tws_fulfilled_speed: f64,
    pub tws_not_fulfilled_speed: f64,
    pub tws_grace_days: f64,
    pub tws_battle_min_count: f64,
    pub tws_battle_max_aspect: f64,
    pub failed_goal_militancy: f64,
    pub failed_goal_prestige_base: f64,
    pub failed_goal_prestige: f64,
    pub jingoism_requirement: f64,
    pub gw_jingoism_mod: f64,
    pub base_truce_months: f64,
    pub gw_cost_mod: f64,
    pub gw_cost_mod_2: f64,
    pub good_peace_militancy: f64,
    pub good_peace_exhaustion: f64,
    pub badboy_limit: f64,
    pub combat_loss_exhaustion: f64,
    pub peace_base_reluctance: f64,
    pub peace_time_months: f64,
    pub peace_time_factor: f64,
    pub peace_time_factor_no_goals: f64,
    pub peace_exhaustion_factor: f64,
    pub peace_direction_factor: f64,
    pub peace_direction_winning_mult: f64,
    pub peace_force_factor: f64,
    pub ally_base_mult: f64,
    pub ally_time_mult: f64,
    pub ally_exhaustion_mult: f64,
    pub ally_direction_mult: f64,
    pub ally_force_mult: f64,
    pub options: BTreeMap<String, OptionValues>,
    // rules file (FLD_MIL_WAR_*)
    pub battle_point: f64,
    pub battle_loss_divisor_land: f64,
    pub battle_loss_divisor_sea: f64,
    pub state_cost_factor: f64,
    pub state_cost_scale: f64,
    pub peace_cost_cap: f64,
    pub blockade_share: f64,
    pub core_mult: f64,
    pub capital_state_mult: f64,
    pub exh_per_percent: f64,
    pub exh_blockade: f64,
    pub exh_occupation_max: f64,
    pub exh_peace_monthly: f64,
    pub exh_peace_max: f64,
    pub exh_war_max: f64,
    pub cb_target_progress: f64,
    pub add_goal_min_score: f64,
    pub good_peace_score: f64,
    pub white_peace_infamy: f64,
    pub white_peace_score: f64,
    pub r1_min: f64,
    pub r1_max: f64,
    pub r2_max: f64,
    pub r1_weight: f64,
    pub r2_weight: f64,
    pub obligatory_bonus: f64,
    pub annex_penalty: f64,
    pub cost_score_cap: f64,
    /// Months of one window of the war's direction (3).
    pub direction_months: f64,
    /// Weight of the enemies in other wars in the force term (0.5).
    pub force_third_weight: f64,
    /// An unselected own goal past this much cost is not counted as wanted (100).
    pub own_goals_cost_cutoff: f64,
    /// At this score against an enemy the AI demands, below it concedes (1).
    pub ai_peace_winning_score: f64,
    pub ai_goal_per_goal: f64,
    pub ai_goal_per_goal_gw: f64,
    pub ai_goal_first_high: f64,
    pub ai_goal_first_low: f64,
    pub ai_goal_exhaustion_ratio: f64,
    pub ai_goal_land_ratio: f64,
    pub ai_goal_min_value: f64,
    pub ai_goal_badboy_margin: f64,
}

/// The sections of defines.lua that hold war constants.
const SECTIONS: [&str; 5] = ["diplomacy", "country", "military", "pops", "ai"];

impl WarRules {
    pub fn from_engine_rules(defines: &Defines, m: &EngineRules) -> Result<WarRules, String> {
        let d = |k: &str| SECTIONS.iter().find_map(|s| defines.get(s, k)).ok_or(format!("defines.{k} missing"));
        let w = |k: &str| crate::num_field(m, &format!("FLD_MIL_WAR_{k}"));
        let mut options = BTreeMap::new();
        for o in PEACE_OPTIONS {
            let u = o.to_uppercase();
            let g = |p: &str, s: &str| SECTIONS.iter().find_map(|sec| defines.get(sec, &format!("{p}{u}{s}"))).unwrap_or(0.0);
            options.insert(
                o.to_string(),
                OptionValues {
                    infamy: g("INFAMY_", ""),
                    prestige_base: g("PRESTIGE_", "_BASE"),
                    prestige_share: g("PRESTIGE_", ""),
                    break_truce_infamy: g("BREAKTRUCE_INFAMY_", ""),
                    break_truce_prestige: g("BREAKTRUCE_PRESTIGE_", ""),
                    break_truce_militancy: g("BREAKTRUCE_MILITANCY_", ""),
                    peace_cost: g("PEACE_COST_", ""),
                },
            );
        }
        Ok(WarRules {
            cb_base_speed: d("CB_GENERATION_BASE_SPEED")?,
            cb_detection_chance: d("CB_DETECTION_CHANCE_BASE")?,
            cb_detected_relation: d("ON_CB_DETECTED_RELATION_CHANGE")?,
            gw_cb_speed: d("GW_CB_CONSTRUCTION_SPEED")?,
            cb_colony_bonus: d("CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION")?,
            cb_troops_bonus: d("CB_GENERATION_SPEED_BONUS_ON_COLONY_COMPETITION_TROOPS_PRESENCE")?,
            cb_valid_months: d("CREATED_CB_VALID_TIME")?,
            gw_badboy_impact: d("GW_JUSTIFY_CB_BADBOY_IMPACT")?,
            max_score_from_battles: d("MAX_WARSCORE_FROM_BATTLES")?,
            tws_limit: d("TWS_CB_LIMIT_DEFAULT")?,
            tws_idle_space: d("TWS_FULFILLED_IDLE_SPACE")?,
            tws_fulfilled_speed: d("TWS_FULFILLED_SPEED")?,
            tws_not_fulfilled_speed: d("TWS_NOT_FULFILLED_SPEED")?,
            tws_grace_days: d("TWS_GRACE_PERIOD_DAYS")?,
            tws_battle_min_count: d("TWS_BATTLE_MIN_COUNT")?,
            tws_battle_max_aspect: d("TWS_BATTLE_MAX_ASPECT")?,
            failed_goal_militancy: d("WAR_FAILED_GOAL_MILITANCY")?,
            failed_goal_prestige_base: d("WAR_FAILED_GOAL_PRESTIGE_BASE")?,
            failed_goal_prestige: d("WAR_FAILED_GOAL_PRESTIGE")?,
            jingoism_requirement: d("WARGOAL_JINGOISM_REQUIREMENT")?,
            gw_jingoism_mod: d("GW_WARGOAL_JINGOISM_REQUIREMENT_MOD")?,
            base_truce_months: d("BASE_TRUCE_MONTHS")?,
            gw_cost_mod: d("GW_WARSCORE_COST_MOD")?,
            gw_cost_mod_2: d("GW_WARSCORE_COST_MOD_2")?,
            good_peace_militancy: d("GOOD_PEACE_REFUSAL_MILITANCY")?,
            good_peace_exhaustion: d("GOOD_PEACE_REFUSAL_WAREXH")?,
            badboy_limit: d("BADBOY_LIMIT")?,
            combat_loss_exhaustion: d("COMBATLOSS_WAR_EXHAUSTION")?,
            peace_base_reluctance: d("PEACE_BASE_RELUCTANCE")?,
            peace_time_months: d("PEACE_TIME_MONTHS")?,
            peace_time_factor: d("PEACE_TIME_FACTOR")?,
            peace_time_factor_no_goals: d("PEACE_TIME_FACTOR_NO_GOALS")?,
            peace_exhaustion_factor: d("PEACE_WAR_EXHAUSTION_FACTOR")?,
            peace_direction_factor: d("PEACE_WAR_DIRECTION_FACTOR")?,
            peace_direction_winning_mult: d("PEACE_WAR_DIRECTION_WINNING_MULT")?,
            peace_force_factor: d("PEACE_FORCE_BALANCE_FACTOR")?,
            ally_base_mult: d("PEACE_ALLY_BASE_RELUCTANCE_MULT")?,
            ally_time_mult: d("PEACE_ALLY_TIME_MULT")?,
            ally_exhaustion_mult: d("PEACE_ALLY_WAR_EXHAUSTION_MULT")?,
            ally_direction_mult: d("PEACE_ALLY_WAR_DIRECTION_MULT")?,
            ally_force_mult: d("PEACE_ALLY_FORCE_BALANCE_MULT")?,
            options,
            battle_point: w("BATTLE_POINT")?,
            battle_loss_divisor_land: w("BATTLE_LOSS_DIVISOR_LAND")?,
            battle_loss_divisor_sea: w("BATTLE_LOSS_DIVISOR_SEA")?,
            state_cost_factor: w("STATE_COST_FACTOR")?,
            state_cost_scale: w("STATE_COST_SCALE")?,
            peace_cost_cap: w("PEACE_COST_CAP")?,
            blockade_share: w("BLOCKADE_PROVINCE_SHARE")?,
            core_mult: w("PROVINCE_CORE_MULT")?,
            capital_state_mult: w("PROVINCE_CAPITAL_STATE_MULT")?,
            exh_per_percent: w("EXH_PER_OCCUPIED_PERCENT")?,
            exh_blockade: w("EXH_BLOCKADE")?,
            exh_occupation_max: w("EXH_OCCUPATION_MAX")?,
            exh_peace_monthly: w("EXH_PEACE_MONTHLY")?,
            exh_peace_max: w("EXH_PEACE_MAX")?,
            exh_war_max: w("EXH_WAR_MAX")?,
            cb_target_progress: w("CB_TARGET_PROGRESS")?,
            add_goal_min_score: w("ADD_GOAL_MIN_SCORE")?,
            good_peace_score: w("GOOD_PEACE_SCORE")?,
            white_peace_infamy: w("AI_INFAMY_LIMIT_WHITE_PEACE")?,
            white_peace_score: w("AI_WHITE_PEACE_SCORE")?,
            r1_min: w("PEACE_FORCE_R1_MIN")?,
            r1_max: w("PEACE_FORCE_R1_MAX")?,
            r2_max: w("PEACE_FORCE_R2_MAX")?,
            r1_weight: w("PEACE_FORCE_R1_WEIGHT")?,
            r2_weight: w("PEACE_FORCE_R2_WEIGHT")?,
            obligatory_bonus: w("PEACE_OBLIGATORY_BONUS")?,
            annex_penalty: w("PEACE_ANNEX_PENALTY")?,
            cost_score_cap: w("PEACE_COST_SCORE_CAP")?,
            direction_months: w("DIRECTION_MONTHS")?,
            force_third_weight: w("PEACE_FORCE_THIRD_WEIGHT")?,
            own_goals_cost_cutoff: w("PEACE_OWN_GOALS_COST_CUTOFF")?,
            ai_peace_winning_score: w("AI_PEACE_WINNING_SCORE")?,
            ai_goal_per_goal: w("AI_GOAL_PER_OWN_GOAL")?,
            ai_goal_per_goal_gw: w("AI_GOAL_PER_OWN_GOAL_GREAT_WAR")?,
            ai_goal_first_high: w("AI_GOAL_FIRST_HIGH")?,
            ai_goal_first_low: w("AI_GOAL_FIRST_LOW")?,
            ai_goal_exhaustion_ratio: w("AI_GOAL_EXHAUSTION_RATIO")?,
            ai_goal_land_ratio: w("AI_GOAL_LAND_RATIO")?,
            ai_goal_min_value: w("AI_GOAL_MIN_VALUE")?,
            ai_goal_badboy_margin: w("AI_GOAL_BADBOY_MARGIN")?,
        })
    }

    fn opt(&self, o: &str) -> OptionValues {
        self.options.get(o).copied().unwrap_or_default()
    }

    fn sum(&self, cb: &CbType, f: impl Fn(&OptionValues) -> f64) -> f64 {
        cb.options.iter().map(|o| f(&self.opt(o))).sum()
    }
}

/// The great-war situation of a goal's two parties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GreatWarStage {
    #[default]
    None,
    GreatWar,
    WorldWar,
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// The same day `months` later (the day is cut to the length of the target month).
pub fn add_months(d: Date, months: u32) -> Date {
    let total = d.year * 12 + (d.month as i32 - 1) + months as i32;
    let (year, month) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u8);
    Date::new(year, month, d.day.min(days_in_month(year, month)))
}

// ---- 2. Casus belli ----------------------------------------------------------------------

/// Circumstances that speed up a casus belli (2.1).
#[derive(Debug, Clone, Copy, Default)]
pub struct CbSpeedInput {
    /// Sum of `cb_generation_speed_modifier` of both modifier blocks (jingoism, pacifism, ...).
    pub speed_modifier: f64,
    pub colony_competition: bool,
    pub troops_present: bool,
    pub great_war_with_target: bool,
}

/// Points a casus belli gains today.
pub fn cb_daily_progress(r: &WarRules, cb: &CbType, i: &CbSpeedInput) -> f64 {
    let mut p = r.cb_base_speed * cb.construction_speed * (1.0 + i.speed_modifier);
    if i.colony_competition {
        p *= 1.0 + r.cb_colony_bonus;
    }
    if i.troops_present {
        p *= 1.0 + r.cb_troops_bonus;
    }
    if i.great_war_with_target {
        p *= 1.0 + r.gw_cb_speed;
    }
    p.clamp(0.0, 100.0)
}

/// Progress after one day, and whether the casus belli is finished.
pub fn cb_build_day(r: &WarRules, progress: f64, gain: f64) -> (f64, bool) {
    let p = (progress + gain).min(r.cb_target_progress);
    (p, p >= r.cb_target_progress)
}

/// `roll` is a uniform number in 0..1000. Detection only while not yet discovered.
pub fn cb_detected(r: &WarRules, roll: u32) -> bool {
    (roll as f64) < r.cb_detection_chance
}

/// Infamy paid on detection: the goal's infamy for the part not yet built (2.1).
pub fn cb_detection_infamy(r: &WarRules, cb: &CbType, progress: f64, great_war: bool) -> f64 {
    goal_infamy(r, cb, great_war) * (1.0 - progress / r.cb_target_progress)
}

/// A casus belli held against a target.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldCb {
    pub cb_type: String,
    pub end: Date,
}

/// A finished casus belli is valid `CREATED_CB_VALID_TIME` months.
pub fn cb_gain(r: &WarRules, cb_type: &str, today: Date) -> HeldCb {
    HeldCb { cb_type: cb_type.to_string(), end: add_months(today, r.cb_valid_months as u32) }
}

/// The port expires a held one on its end date (the daily removal was not observed).
pub fn cb_expired(cb: &HeldCb, today: Date) -> bool {
    today >= cb.end
}

// ---- 3. Declaring war and adding goals ---------------------------------------------------

/// Infamy of a goal: sum of INFAMY_<option> x badboy_factor, cut in a great war (2.3).
pub fn goal_infamy(r: &WarRules, cb: &CbType, great_war: bool) -> f64 {
    let s = r.sum(cb, |o| o.infamy) * cb.badboy_factor;
    if great_war { s * r.gw_badboy_impact } else { s }
}

/// Prestige a goal pays the winner (and takes from the loser): sum of
/// max(base, share x winner prestige) x prestige_factor.
pub fn goal_prestige(r: &WarRules, cb: &CbType, winner_prestige: f64) -> f64 {
    cb.options.iter().map(|o| r.opt(o)).map(|v| v.prestige_base.max(v.prestige_share * winner_prestige)).sum::<f64>() * cb.prestige_factor
}

/// Penalties for breaking a truce: (infamy, prestige change, militancy).
pub fn break_truce_penalty(r: &WarRules, cb: &CbType) -> (f64, f64, f64) {
    (
        r.sum(cb, |o| o.break_truce_infamy) * cb.break_truce_infamy_factor,
        r.sum(cb, |o| o.break_truce_prestige) * cb.break_truce_prestige_factor,
        r.sum(cb, |o| o.break_truce_militancy) * cb.break_truce_militancy_factor,
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeclareInput {
    pub holds_cb: bool,
    /// The action carries the "just" flag (scripted / crisis).
    pub just_flag: bool,
    pub truce_active: bool,
    pub great_war_with_target: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Declaration {
    pub just: bool,
    /// Added to the actor's infamy (never below 0 in total).
    pub infamy: f64,
    pub prestige: f64,
    pub militancy: f64,
    /// The held casus belli of that type is used up.
    pub consumes_cb: bool,
}

/// Declaring a war with a goal of type `cb` (3. 1 to 4).
pub fn declare_war(r: &WarRules, cb: &CbType, i: &DeclareInput) -> Declaration {
    let just = i.holds_cb || cb.needs_no_justification || i.just_flag;
    let mut d = Declaration { just, consumes_cb: i.holds_cb, ..Declaration::default() };
    if !just {
        d.infamy += goal_infamy(r, cb, i.great_war_with_target);
    }
    if i.truce_active {
        let (inf, pres, mil) = break_truce_penalty(r, cb);
        d.infamy += inf.max(0.0);
        d.prestige = pres;
        d.militancy = mil;
    }
    d
}

/// Adding a goal mid-war: the same just/unjust test, no truce.
pub fn add_goal_cost(r: &WarRules, cb: &CbType, holds_cb: bool, just_flag: bool, great_war: bool) -> Declaration {
    declare_war(r, cb, &DeclareInput { holds_cb, just_flag, truce_active: false, great_war_with_target: great_war })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddGoalRefusal {
    StatusQuoGoal,
    NoWarScore,
    NotEnoughJingoism,
}

#[derive(Debug, Clone, Copy)]
pub struct AddGoalInput {
    /// The own side holds a status-quo goal.
    pub status_quo_goal: bool,
    /// The country's war score against the enemy leader.
    pub score_vs_leader: f64,
    /// Share (0..1) of the population backing jingoism.
    pub jingoism_share: f64,
    pub great_war: bool,
}

/// Jingoism share needed to add a goal. The great-war cut is read as required x (1 - mod).
pub fn jingoism_needed(r: &WarRules, great_war: bool) -> f64 {
    if great_war { r.jingoism_requirement * (1.0 - r.gw_jingoism_mod) } else { r.jingoism_requirement }
}

/// Adding a war goal later (3).
pub fn can_add_goal(r: &WarRules, i: &AddGoalInput) -> Result<(), AddGoalRefusal> {
    if i.status_quo_goal {
        return Err(AddGoalRefusal::StatusQuoGoal);
    }
    if i.score_vs_leader <= r.add_goal_min_score {
        return Err(AddGoalRefusal::NoWarScore);
    }
    if i.jingoism_share < jingoism_needed(r, i.great_war) {
        return Err(AddGoalRefusal::NotEnoughJingoism);
    }
    Ok(())
}

// ---- 4. War score ------------------------------------------------------------------------

/// War score points of one battle for the winner's side (4.2). `naval` selects the divisor.
pub fn battle_points(r: &WarRules, winner_losses: f64, loser_losses: f64, naval: bool) -> f64 {
    let div = if naval { r.battle_loss_divisor_sea } else { r.battle_loss_divisor_land };
    r.battle_point + (loser_losses - winner_losses / 2.0).max(0.0) / div
}

/// Battle part of the leaders' score from the attacker's side: attacker points minus defender
/// points, cut to whole points, clamped to +-MAX_WARSCORE_FROM_BATTLES. `naval` tells a battle's kind.
pub fn battles_part(r: &WarRules, war: &War, naval: &dyn Fn(&BattleRecord) -> bool) -> f64 {
    let mut net = 0.0;
    for b in war.battles() {
        let (w, l) = if b.attacker_won { (&b.attacker, &b.defender) } else { (&b.defender, &b.attacker) };
        let p = battle_points(r, w.losses, l.losses, naval(b));
        net += if b.attacker_won { p } else { -p };
    }
    net.trunc().clamp(-r.max_score_from_battles, r.max_score_from_battles)
}

/// Facts of one province for its value (4.3).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProvinceValueInput {
    pub naval_base_level: f64,
    /// The state's factory count, only for the state's first province.
    pub state_factories: f64,
    pub fort_level: f64,
    pub owner_has_core: bool,
    pub in_owner_capital_state: bool,
    /// Other continent and not connected by land: the multipliers are skipped.
    pub overseas: bool,
    /// Colonial state or unowned land: always 1.
    pub colonial_or_unowned: bool,
}

pub fn province_value(r: &WarRules, i: &ProvinceValueInput) -> f64 {
    if i.colonial_or_unowned {
        return 1.0;
    }
    let mut v = 1.0 + i.naval_base_level + i.state_factories + i.fort_level;
    if !i.overseas {
        if i.owner_has_core {
            v *= r.core_mult;
        }
        if i.in_owner_capital_state {
            v *= r.capital_state_mult;
        }
    }
    v
}

/// Score of a province: its share of the owner's total value, in points.
pub fn province_score(value: f64, owner_total: f64) -> f64 {
    if owner_total <= 0.0 { 0.0 } else { 100.0 * value / owner_total }
}

/// Occupation part: scores of provinces held by the enemy, plus a quarter of the score of
/// blockaded ports. The caller takes the scores from the siege step (task D).
pub fn occupation_part(r: &WarRules, occupied_scores: &[f64], blockaded_scores: &[f64]) -> f64 {
    occupied_scores.iter().sum::<f64>() + blockaded_scores.iter().sum::<f64>() * r.blockade_share
}

/// Fulfilment of a battle goal (0..1): needs enough battles, then the won/lost ratio against
/// the type's factor.
pub fn battle_fulfilment(r: &WarRules, battle_count: u32, points_won: f64, points_lost: f64, tws_battle_factor: f64) -> f64 {
    if (battle_count as f64) < r.tws_battle_min_count {
        return 0.0;
    }
    let ratio = if points_lost <= 0.0 { r.tws_battle_max_aspect + tws_battle_factor } else { points_won / points_lost };
    if ratio < tws_battle_factor {
        return 0.0;
    }
    (ratio - tws_battle_factor).min(r.tws_battle_max_aspect) / r.tws_battle_max_aspect
}

/// One day of a goal's ticking score (4.4). `fulfilment` is 0..1, `days_since_goal` counts from
/// the goal's date. Returns the new score.
pub fn tick_goal(r: &WarRules, score: f64, fulfilment: f64, days_since_goal: f64) -> f64 {
    let (limit, speed) = (r.tws_limit, r.tws_fulfilled_speed);
    if fulfilment > 0.0 {
        if fulfilment >= r.tws_idle_space {
            (score + speed * fulfilment).min(limit)
        } else if score < 0.0 {
            (score + speed * fulfilment).min(0.0)
        } else {
            score
        }
    } else if score > 0.0 {
        (score - r.tws_not_fulfilled_speed).max(0.0)
    } else if days_since_goal >= r.tws_grace_days {
        (score - r.tws_not_fulfilled_speed).max(-limit)
    } else {
        score
    }
}

/// What a goal adds to its actor's war score: whole part of the score x min(cost, limit) / limit.
pub fn goal_score_contribution(r: &WarRules, score: f64, cost: f64) -> f64 {
    score.trunc() * cost.min(r.tws_limit) / r.tws_limit
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScoreInput {
    /// A controls every province B owns (A's side when A leads).
    pub a_holds_all_of_b: bool,
    pub b_holds_all_of_a: bool,
    /// B controls nothing and A holds B's capital or more than half of B's provinces.
    pub b_has_nothing_and_a_holds_capital_or_half: bool,
    pub a_has_nothing_and_b_holds_capital_or_half: bool,
    pub battles: f64,
    pub occupation: f64,
    pub ticking: f64,
}

/// Score of A against B, -100..100 (4.1). The battle part belongs in the input only when both are leaders.
pub fn war_score(i: &ScoreInput) -> f64 {
    if i.a_holds_all_of_b || i.b_has_nothing_and_a_holds_capital_or_half {
        return 100.0;
    }
    if i.b_holds_all_of_a || i.a_has_nothing_and_b_holds_capital_or_half {
        return -100.0;
    }
    (i.battles + i.occupation + i.ticking).clamp(-100.0, 100.0)
}

// ---- 4.5 Failed goals --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FailedGoalPenalty {
    pub militancy: f64,
    /// Negative: prestige lost.
    pub prestige: f64,
}

pub fn failed_goal_penalty(r: &WarRules, cb: &CbType, actor_prestige: f64) -> FailedGoalPenalty {
    let fall = (-r.failed_goal_prestige_base).max(-r.failed_goal_prestige * actor_prestige);
    FailedGoalPenalty { militancy: r.failed_goal_militancy * cb.penalty_factor, prestige: -fall * cb.penalty_factor }
}

// ---- 5. Peace ----------------------------------------------------------------------------

/// Facts for the cost of one goal (5.1).
#[derive(Debug, Clone, Default)]
pub struct GoalCostInput {
    /// For a state / transfer goal: (province value, owner's total value) of each province.
    pub provinces: Vec<(f64, f64)>,
    /// Countries of the actor's culture group in the receiver's sphere or its vassals.
    pub union_sphere_count: u32,
    pub stage: GreatWarStage,
}

/// Cost of a demand in war score points.
pub fn goal_cost(r: &WarRules, cb: &CbType, i: &GoalCostInput) -> f64 {
    if cb.zero_cost {
        return 0.0;
    }
    let mut c = 0.0;
    for o in &cb.options {
        match o.as_str() {
            "demand_state" | "transfer_provinces" => {
                c += i
                    .provinces
                    .iter()
                    .map(|&(v, total)| if total > 0.0 { (r.state_cost_scale * v / total).clamp(1.0_f64.min(v), v.max(1.0)) } else { 1.0 } * r.state_cost_factor)
                    .sum::<f64>();
            }
            "clear_union_sphere" => c += r.opt(o).peace_cost * (1.0 + i.union_sphere_count as f64),
            _ => c += r.opt(o).peace_cost,
        }
    }
    c *= match i.stage {
        GreatWarStage::None => 1.0,
        GreatWarStage::GreatWar => r.gw_cost_mod,
        GreatWarStage::WorldWar => r.gw_cost_mod_2,
    };
    (c * cb.peace_cost_factor).min(r.peace_cost_cap)
}

/// Truce length: BASE_TRUCE_MONTHS plus the largest truce_months of the goals on the table.
/// A type with truce_months 0 still gives the plain base.
pub fn truce_months(r: &WarRules, table: &[&CbType]) -> f64 {
    r.base_truce_months + table.iter().map(|c| c.truce_months).fold(0.0, f64::max)
}

/// Why a peace may not be made (5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeaceRefusal {
    NotAtWar,
    NeitherIsLeader,
    /// Crisis war: both must be war leaders.
    NoSeparatePeaceCrisis,
    /// Would leave some but not all of the member's own goals in the war.
    LeftoverGoals,
    /// A status-quo goal is held and not every goal between the two is an annexation.
    StatusQuoLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeaceScope {
    /// Both are war leaders: the war ends for everyone.
    WholeWar,
    /// Only this non-leader leaves the war.
    Separate(String),
}

/// Which side a country is on: `Some(true)` attackers.
fn side_of(war: &War, tag: &str) -> Option<bool> {
    if war.attackers.iter().any(|t| t == tag) {
        Some(true)
    } else if war.defenders.iter().any(|t| t == tag) {
        Some(false)
    } else {
        None
    }
}

pub fn is_leader(war: &War, tag: &str) -> bool {
    war.attacker_leader() == Some(tag) || war.defender_leader() == Some(tag)
}

#[derive(Debug, Clone, Copy)]
pub struct PeaceRequest<'a> {
    pub war: &'a War,
    pub crisis: bool,
    pub sender: &'a str,
    pub target: &'a str,
    /// Indices into `war.goals` of the goals the treaty enforces (both sides' demands).
    pub selected: &'a [usize],
}

/// Who may negotiate (5.2). Peace between the two war leaders ends the whole war; a separate
/// peace needs a leader on one side.
pub fn peace_allowed(req: &PeaceRequest, cbs: &BTreeMap<String, CbType>) -> Result<PeaceScope, PeaceRefusal> {
    let war = req.war;
    let (Some(a), Some(b)) = (side_of(war, req.sender), side_of(war, req.target)) else { return Err(PeaceRefusal::NotAtWar) };
    if a == b {
        return Err(PeaceRefusal::NotAtWar);
    }
    let (sl, tl) = (is_leader(war, req.sender), is_leader(war, req.target));
    if !sl && !tl {
        return Err(PeaceRefusal::NeitherIsLeader);
    }
    if sl && tl {
        return Ok(PeaceScope::WholeWar);
    }
    if req.crisis {
        return Err(PeaceRefusal::NoSeparatePeaceCrisis);
    }
    let member = if sl { req.target } else { req.sender };
    let other = if sl { req.sender } else { req.target };
    // Some but not all of the member's own goals in the treaty.
    let own: Vec<usize> = (0..war.goals.len()).filter(|&i| war.goals[i].actor == member).collect();
    let included = own.iter().filter(|i| req.selected.contains(i)).count();
    if included > 0 && included < own.len() {
        return Err(PeaceRefusal::LeftoverGoals);
    }
    // A status-quo goal in the war: only annexations may be between the two.
    let kind = |g: &WarGoal| cbs.get(&g.casus_belli);
    if war.goals.iter().any(|g| kind(g).is_some_and(|c| c.has("status_quo"))) {
        let between = war.goals.iter().enumerate().filter(|(i, g)| req.selected.contains(i) && [&g.actor, &g.receiver].iter().all(|t| *t == member || *t == other));
        if between.into_iter().any(|(_, g)| !kind(g).is_some_and(|c| c.has("annex"))) {
            return Err(PeaceRefusal::StatusQuoLimit);
        }
    }
    Ok(PeaceScope::Separate(member.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeaceOutcome {
    pub scope: PeaceScope,
    /// Indices into `war.goals` that are enforced.
    pub enforced: Vec<usize>,
    /// Goals on the table that are not enforced: (index, actor, penalty).
    pub failed: Vec<(usize, String, FailedGoalPenalty)>,
    /// Countries that leave the war.
    pub leaving: Vec<String>,
    pub truce_months: f64,
}

/// Executing an accepted peace (5.3): every goal on the table that is selected and still
/// possible is enforced, any other costs its actor the failed-goal penalty. The truce counts
/// all goals on the table.
pub fn plan_peace(
    r: &WarRules,
    req: &PeaceRequest,
    cbs: &BTreeMap<String, CbType>,
    possible: &dyn Fn(&WarGoal) -> bool,
    prestige_of: &dyn Fn(&str) -> f64,
) -> Result<PeaceOutcome, PeaceRefusal> {
    let scope = peace_allowed(req, cbs)?;
    let war = req.war;
    let leaving: Vec<String> = match &scope {
        PeaceScope::WholeWar => war.attackers.iter().chain(&war.defenders).cloned().collect(),
        PeaceScope::Separate(m) => vec![m.clone()],
    };
    let on_table: Vec<usize> = (0..war.goals.len())
        .filter(|&i| match &scope {
            PeaceScope::WholeWar => true,
            PeaceScope::Separate(m) => war.goals[i].actor == *m || war.goals[i].receiver == *m,
        })
        .collect();
    let (mut enforced, mut failed) = (Vec::new(), Vec::new());
    let default = CbType::new("", &[]);
    for &i in &on_table {
        let g = &war.goals[i];
        if req.selected.contains(&i) && possible(g) {
            enforced.push(i);
        } else {
            let cb = cbs.get(&g.casus_belli).unwrap_or(&default);
            failed.push((i, g.actor.clone(), failed_goal_penalty(r, cb, prestige_of(&g.actor))));
        }
    }
    let table: Vec<&CbType> = on_table.iter().filter_map(|&i| cbs.get(&war.goals[i].casus_belli)).collect();
    Ok(PeaceOutcome { scope, enforced, failed, leaving, truce_months: truce_months(r, &table) })
}

// ---- 5.5 The AI's answer -----------------------------------------------------------------

/// The war's direction (4.1): the change of the leaders' score over the last window, in whole
/// points as the attackers see it. It stays 0 during the first window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Direction {
    pub value: f64,
    /// The attackers' score at the last update.
    pub last_score: f64,
    /// The update runs on the first day after this one.
    pub next: Option<Date>,
}

/// One day of a war's direction: past the end of the window it becomes the score's change and a
/// new window of `direction_months` starts. A war seen for the first time starts its first window.
pub fn update_direction(r: &WarRules, d: &mut Direction, attackers_score: f64, today: Date) {
    let score = attackers_score.trunc();
    let window = r.direction_months.max(1.0) as u32;
    match d.next {
        None => *d = Direction { value: 0.0, last_score: score, next: Some(add_months(today, window)) },
        Some(next) if next < today => *d = Direction { value: score - d.last_score, last_score: score, next: Some(add_months(today, window)) },
        Some(_) => {}
    }
}

/// Strength ratio of M's side to O's side for the force term: each sum starts at 1, and the
/// enemies in the two countries' other wars count at `force_third_weight`. `own_enemies` is every
/// country M is at war with (O's side among them), `foe_other_enemies` every country O is at war
/// with outside M's side.
pub fn force_ratio(r: &WarRules, own_side: f64, foe_side: f64, own_enemies: f64, foe_other_enemies: f64) -> f64 {
    (1.0 + own_side + r.force_third_weight * (1.0 + foe_other_enemies)) / (1.0 + foe_side + r.force_third_weight * (1.0 + own_enemies))
}

/// M's own goals in an offer: `goals` holds (cost, selected) of the goals of M's side that M may
/// count (its own, or its side's when it leads), in the war's order. An unselected goal past the
/// first `own_goals_cost_cutoff` points of cost is not counted. Returns (own goals, granted ones).
pub fn count_own_goals(r: &WarRules, goals: &[(f64, bool)]) -> (u32, u32) {
    let (mut running, mut own, mut granted) = (0.0, 0, 0);
    for &(cost, selected) in goals {
        running += cost.trunc();
        if running < r.own_goals_cost_cutoff || selected {
            own += 1;
        }
        if selected {
            granted += 1;
        }
    }
    (own, granted)
}

#[derive(Debug, Clone, Copy)]
pub struct PeaceAnswerInput {
    pub m_is_leader: bool,
    pub o_is_leader: bool,
    pub m_score: f64,
    /// O's score against M.
    pub o_score: f64,
    /// The war's direction seen from M (negated for a defender).
    pub direction: f64,
    /// Calendar months since the war began.
    pub months_of_war: f64,
    pub m_exhaustion: f64,
    /// Strength ratios of M's side to O's side ([`force_ratio`], raw, clamped here).
    pub land: f64,
    pub naval: f64,
    /// Cost of every selected goal of the offer.
    pub offer_cost: f64,
    /// See [`count_own_goals`].
    pub own_goals: u32,
    pub own_granted: u32,
    /// A selected goal annexes M.
    pub annexes_m: bool,
    pub has_enemy_obligatory: bool,
    pub has_own_obligatory: bool,
    pub m_infamy: f64,
    /// M may not add goals to the war.
    pub m_cannot_add_goals: bool,
    /// M holds a usable casus belli on a country of O's side.
    pub m_holds_cb: bool,
}

/// Willingness to make peace W (5.5), every term in whole points. The strength weights keep their
/// rules file names: r1 is the naval ratio, r2 the land ratio.
pub fn ai_willingness(r: &WarRules, i: &PeaceAnswerInput) -> f64 {
    let mut base = r.peace_base_reluctance;
    let mut time = r.peace_time_months - i.months_of_war;
    if time < 0.0 {
        time *= r.peace_time_factor;
        if i.own_goals == 0 {
            time *= r.peace_time_factor_no_goals;
        }
    }
    let mut exh = r.peace_exhaustion_factor * i.m_exhaustion;
    let mut dir = r.peace_direction_factor * i.direction;
    if dir > 0.0 {
        dir *= r.peace_direction_winning_mult;
    }
    let (naval, land) = (i.naval.clamp(r.r1_min, r.r1_max), i.land.min(r.r2_max));
    let mut force = r.peace_force_factor * 100.0 * (r.r1_weight * naval + r.r2_weight * land - 1.0);
    if !i.m_is_leader {
        base *= r.ally_base_mult;
        if time < 0.0 {
            time *= r.ally_time_mult;
        }
        exh *= r.ally_exhaustion_mult;
        dir *= r.ally_direction_mult;
        force *= r.ally_force_mult;
    }
    let mut w = exh.trunc() - force.trunc() - dir.trunc() - time.trunc() - base.trunc();
    if i.annexes_m {
        w -= r.annex_penalty;
    }
    if i.has_enemy_obligatory {
        w -= r.annex_penalty;
    }
    if i.has_own_obligatory {
        w += r.obligatory_bonus;
    }
    w
}

/// Would M accept the offer (5.5)?
pub fn ai_accepts_peace(r: &WarRules, i: &PeaceAnswerInput) -> bool {
    let w = ai_willingness(r, i);
    let cost = (i.offer_cost + 0.5).floor();
    if i.own_granted == 0 {
        // M gets nothing.
        if i.has_enemy_obligatory && i.m_is_leader && !i.o_is_leader {
            return false;
        }
        if cost >= 1.0 && i.o_score < 0.0 {
            return false;
        }
        if cost < 1.0 && i.own_goals == 0 {
            // A white peace for a country that wants nothing.
            if w >= 1.0 {
                return true;
            }
            if i.m_is_leader && !i.has_enemy_obligatory {
                let long_war = i.months_of_war > r.peace_time_months;
                if i.m_score > r.white_peace_score || (long_war && (i.m_infamy > r.white_peace_infamy || i.m_cannot_add_goals) && !i.m_holds_cb) {
                    return true;
                }
            }
        }
        let limit = (w / 2.0).trunc() + i.o_score;
        let limit = if limit > r.cost_score_cap || i.o_score >= 100.0 { r.cost_score_cap } else { limit };
        return cost <= limit;
    }
    if i.own_granted == i.own_goals && i.m_is_leader {
        return true;
    }
    // Only a part of what M wants: a willing M signs when the part is worth its score.
    w > 0.0 && (i.m_score - (w / 2.0).trunc()).min(r.cost_score_cap) <= cost
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclinePenalty {
    pub exhaustion: f64,
    pub militancy: f64,
}

/// Declining a peace offer (5.6): the penalty applies only to a "good" offer.
pub fn decline_penalty(r: &WarRules, sender_score_vs_decliner: f64, ai_reversed_accepts: bool, better_flag: bool) -> Option<DeclinePenalty> {
    if (sender_score_vs_decliner > r.good_peace_score && ai_reversed_accepts) || better_flag {
        Some(DeclinePenalty { exhaustion: r.good_peace_exhaustion, militancy: r.good_peace_militancy })
    } else {
        None
    }
}

// ---- 6. War exhaustion -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct ExhaustionInput {
    pub current: f64,
    pub at_war: bool,
    /// Share (0..1) of the home-continent provinces held by an enemy; rebel-held and overseas ones not counted.
    pub occupied_share: f64,
    /// Blockade fraction 0..1.
    pub blockade_share: f64,
    /// The `war_exhaustion` of the country's modifiers (event effects and the like).
    pub modifier_change: f64,
    /// Lower cap from modifiers, if any (`max_war_exhaustion`).
    pub modifier_max: Option<f64>,
}

/// Monthly change from occupation and blockade (6), only while at war.
pub fn exhaustion_change(r: &WarRules, i: &ExhaustionInput) -> f64 {
    if !i.at_war {
        return r.exh_peace_monthly;
    }
    (r.exh_per_percent * 100.0 * i.occupied_share.clamp(0.0, 1.0)).min(r.exh_occupation_max) + r.exh_blockade * i.blockade_share.clamp(0.0, 1.0) + i.modifier_change
}

/// New war exhaustion after one month.
pub fn exhaustion_monthly(r: &WarRules, i: &ExhaustionInput) -> f64 {
    let cap = if i.at_war { r.exh_war_max } else { r.exh_peace_max };
    let cap = i.modifier_max.map_or(cap, |m| m.min(cap));
    (i.current + exhaustion_change(r, i)).clamp(0.0, cap)
}

/// Exhaustion from the losses of a battle.
pub fn battle_exhaustion(r: &WarRules, losses: f64) -> f64 {
    r.combat_loss_exhaustion * losses
}

// ---- When the AI adds a goal --------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct AiGoalInput {
    pub exhaustion: f64,
    pub max_exhaustion: f64,
    /// Own side to enemy side on land.
    pub land_ratio: f64,
    /// The country's war score against this enemy.
    pub score: f64,
    pub own_goals: u32,
    pub great_war: bool,
    /// With no goal yet: the leaders' score exceeds the cost already demanded (picks +5 over +2).
    pub score_above_demanded: bool,
}

/// Score the AI needs before it adds a goal: 15 per own goal (8 in a great war); with none, 5 or 2.
pub fn ai_goal_threshold(r: &WarRules, own_goals: u32, great_war: bool, score_above_demanded: bool) -> f64 {
    if own_goals == 0 {
        return if score_above_demanded { r.ai_goal_first_high } else { r.ai_goal_first_low };
    }
    own_goals as f64 * if great_war { r.ai_goal_per_goal_gw } else { r.ai_goal_per_goal }
}

/// The AI looks for a new goal as the war drags on while it wins and is not worn out.
pub fn ai_wants_goal(r: &WarRules, i: &AiGoalInput) -> bool {
    let ratio = if i.max_exhaustion > 0.0 { i.exhaustion / i.max_exhaustion } else { 0.0 };
    ratio < r.ai_goal_exhaustion_ratio && i.land_ratio > r.ai_goal_land_ratio && i.score > ai_goal_threshold(r, i.own_goals, i.great_war, i.score_above_demanded)
}

/// Infamy ceiling of the AI's goal picker.
pub fn ai_goal_infamy_ceiling(r: &WarRules) -> f64 {
    r.badboy_limit - r.ai_goal_badboy_margin
}

/// The picked goal is added only when its value is above the minimum.
pub fn ai_goal_value_ok(r: &WarRules, value: f64) -> bool {
    value > r.ai_goal_min_value
}
