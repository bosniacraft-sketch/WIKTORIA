//! The AI foreign minister's war logic:
//! the antagonize list and the war target, justifying a casus belli, the goal picker, the
//! declaration with its strength test, calling allies, and the peace offer.
//!
//! Pure functions: the caller (game.rs, the foreign-minister step) gathers the facts. Numbers the
//! design gives are rules file rows `FLD_MIL_WARAI_*`; stand-ins for what the design leaves open are
//! marked STAND-IN and are hypotheses.

use wiktoria_data::engine_rules::EngineRules;

use super::war::{self, CbType, WarRules};

/// Rules file numbers of the war AI.
#[derive(Debug, Clone, PartialEq)]
pub struct WarAiRules {
    /// Countries kept on the antagonize list (ten).
    pub list_size: usize,
    /// Antagonize value: provinces counted at most (6), cap of the base (800).
    pub base_province_cap: f64,
    pub base_cap: f64,
    /// STAND-IN: conquer value of one province, only for a country without a save `conquer_prov` list.
    pub province_value: f64,
    /// Neighbour bonus (25), casus belli bonus (50), civilised against uncivilised (25).
    pub neighbour_bonus: f64,
    pub cb_bonus: f64,
    pub unciv_bonus: f64,
    /// A candidate needs at least this score to be drawn as war target (20).
    pub target_min_score: f64,
    /// Fewer own regiments refuse the target (4).
    pub min_regiments: f64,
    /// Land strength test: own side / target side at least this (1.1); allies next door x 1.1.
    pub strength_ratio: f64,
    pub ally_neighbour_mult: f64,
    /// (min(r / 2, 1) - 1) x this (200).
    pub strength_score_factor: f64,
    /// Score multiplier without a casus belli (0.5), civilised against uncivilised (0.7).
    pub no_cb_mult: f64,
    pub no_cb_unciv_mult: f64,
    /// Goal picker margins below BADBOY_LIMIT: target choice (10), justifying / declaring (2).
    pub target_goal_margin: f64,
    pub justify_goal_margin: f64,
    /// Goal values: target refused at or below (10); justified only above (20).
    pub target_goal_min_value: f64,
    pub justify_min_value: f64,
    /// Held types only above this own infamy: target (5), justify (7), mid-war (10).
    pub held_only_infamy_target: f64,
    pub held_only_infamy_justify: f64,
    pub held_only_infamy_war: f64,
    /// Justification chance per mille: AGGRESSION_BASE (5) + AGGRESSION_UNCIV_BONUS (10).
    pub aggression_base: f64,
    pub aggression_unciv_bonus: f64,
    /// A war is declared only at or below this relation (0).
    pub declare_relation_floor: f64,
    /// STAND-IN: weight of a demand_state option (the helper that values a state was not read).
    pub demand_state_weight: f64,
    /// STAND-IN: an ally answers a call to arms when its relation with the caller is at least this.
    pub call_ally_min_relation: f64,
    /// STAND-IN: relation change of one decrease-relation move.
    pub relation_decrease: f64,
    /// All goals of one side cost at most this (100).
    pub goals_cost_cap: f64,
}

impl WarAiRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<WarAiRules, String> {
        let f = |k: &str| crate::num_field(m, &format!("FLD_MIL_WARAI_{k}"));
        Ok(WarAiRules {
            list_size: f("LIST_SIZE")?.max(1.0) as usize,
            base_province_cap: f("BASE_PROVINCE_CAP")?,
            base_cap: f("BASE_CAP")?,
            province_value: f("PROVINCE_VALUE")?,
            neighbour_bonus: f("NEIGHBOUR_BONUS")?,
            cb_bonus: f("CB_BONUS")?,
            unciv_bonus: f("UNCIV_BONUS")?,
            target_min_score: f("TARGET_MIN_SCORE")?,
            min_regiments: f("MIN_REGIMENTS")?,
            strength_ratio: f("STRENGTH_RATIO")?,
            ally_neighbour_mult: f("ALLY_NEIGHBOUR_MULT")?,
            strength_score_factor: f("STRENGTH_SCORE_FACTOR")?,
            no_cb_mult: f("NO_CB_MULT")?,
            no_cb_unciv_mult: f("NO_CB_UNCIV_MULT")?,
            target_goal_margin: f("TARGET_GOAL_MARGIN")?,
            justify_goal_margin: f("JUSTIFY_GOAL_MARGIN")?,
            target_goal_min_value: f("TARGET_GOAL_MIN_VALUE")?,
            justify_min_value: f("JUSTIFY_MIN_VALUE")?,
            held_only_infamy_target: f("HELD_ONLY_INFAMY_TARGET")?,
            held_only_infamy_justify: f("HELD_ONLY_INFAMY_JUSTIFY")?,
            held_only_infamy_war: f("HELD_ONLY_INFAMY_WAR")?,
            aggression_base: f("AGGRESSION_BASE")?,
            aggression_unciv_bonus: f("AGGRESSION_UNCIV_BONUS")?,
            declare_relation_floor: f("DECLARE_RELATION_FLOOR")?,
            demand_state_weight: f("DEMAND_STATE_WEIGHT")?,
            call_ally_min_relation: f("CALL_ALLY_MIN_RELATION")?,
            relation_decrease: f("RELATION_DECREASE")?,
            goals_cost_cap: f("GOALS_COST_CAP")?,
        })
    }
}

// ---- A. The antagonize list and the war target ---------------------------------------------

/// Facts about another country X for C's antagonize list (simplified).
#[derive(Debug, Clone, Copy, Default)]
pub struct AntagonizeInput {
    pub x_provinces: f64,
    /// Sum of C's conquer_prov values of the provinces X owns.
    pub x_wanted_value: f64,
    pub x_regiments: f64,
    pub own_provinces: f64,
    pub own_regiments: f64,
    pub neighbour: bool,
    pub x_infamy: f64,
    pub holds_cb: bool,
    pub civ_vs_unciv: bool,
    pub in_own_sphere: bool,
    pub x_is_subject: bool,
}

/// Size ratio R of X to C.
pub fn size_ratio(i: &AntagonizeInput) -> f64 {
    ((i.x_regiments + 1.0) / (i.own_regiments + 1.0) + i.x_provinces / i.own_provinces.max(1.0)) / 2.0
}

/// X's value on C's antagonize list; 0 keeps it off.
pub fn antagonize_value(r: &WarAiRules, i: &AntagonizeInput) -> f64 {
    if i.in_own_sphere || i.x_is_subject || i.x_provinces <= 0.0 {
        return 0.0;
    }
    let n = i.x_provinces;
    let mut v = (i.x_wanted_value * r.base_province_cap / n.min(r.base_province_cap)).min(r.base_cap);
    let ratio = size_ratio(i);
    if ratio > 3.0 {
        v /= 3.0;
    }
    if i.neighbour {
        // NOT WIRED: the per-country table value and the two position tests.
        v += r.neighbour_bonus + 2.0 * i.x_infamy;
    }
    if i.holds_cb {
        v += r.cb_bonus;
    }
    if i.civ_vs_unciv {
        v += r.unciv_bonus;
    }
    if v > 0.0 && i.x_provinces < i.own_provinces {
        v += 100.0 * (1.0 - i.x_provinces / i.own_provinces.max(1.0)) * 0.5;
    }
    v.max(0.0)
}

/// The highest `list_size` values, best first (ties by tag).
pub fn antagonize_list(r: &WarAiRules, mut values: Vec<(String, f64)>) -> Vec<(String, f64)> {
    values.retain(|(_, v)| *v > 0.0);
    values.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    values.truncate(r.list_size);
    values
}

/// Facts for the score of a candidate target T.
#[derive(Debug, Clone, Copy, Default)]
pub struct TargetInput {
    pub own_war_exhaustion: f64,
    pub truce: bool,
    /// T has an overlord, or is in C's sphere: refused.
    pub t_subject_or_own_sphere: bool,
    pub neighbour: bool,
    pub own_regiments: f64,
    /// Land strength of each side, allies counted (`side_strength`).
    pub own_side: f64,
    pub target_side: f64,
    pub t_over_limit: bool,
    pub antagonize: f64,
    /// T's values on C's rival, threat and befriend lists (0 when absent).
    pub rival: f64,
    pub threat: f64,
    pub befriend: f64,
    /// Places in the overall ranking (1 = first); 0 = unranked (the rank factors are skipped).
    pub own_rank: f64,
    pub t_rank: f64,
    pub relation: f64,
    pub t_infamy: f64,
    pub holds_cb: bool,
    pub civ_vs_unciv: bool,
    /// Value of the best goal the picker finds (`pick_goal`), 0 when none.
    pub goal_value: f64,
    /// That goal is neither held nor an "always" type: its infamy and C's.
    pub unjust_goal_infamy: Option<(f64, f64)>,
}

/// Land strength of a side: its own armies plus allies, x 1.1 for an ally next to the enemy (2.1 A 10).
pub fn side_strength(r: &WarAiRules, own: f64, allies: &[(f64, bool)]) -> f64 {
    own + allies.iter().map(|&(s, next_door)| if next_door { s * r.ally_neighbour_mult } else { s }).sum::<f64>()
}

/// Score of T as war target; 0 = refused.
pub fn target_score(r: &WarAiRules, w: &WarRules, i: &TargetInput) -> f64 {
    if i.own_war_exhaustion >= 1.0 || i.truce || i.t_subject_or_own_sphere {
        return 0.0;
    }
    // NOT WIRED: reach of a non-neighbour (distance tests) and naval strength: only neighbours qualify.
    if !i.neighbour || i.own_regiments < r.min_regiments {
        return 0.0;
    }
    // Step 7: a befriended T that is barely antagonised is left alone.
    if !i.t_over_limit && i.befriend > 0.0 && i.antagonize < 1.0 {
        return 0.0;
    }
    let mut ratio = i.own_side / i.target_side.max(1.0);
    if i.t_over_limit {
        ratio += 0.5;
    }
    if ratio < r.strength_ratio {
        return 0.0;
    }
    // NOT WIRED: 5 x T's militancy average, the union target (x 3).
    let mut s = ((ratio / 2.0).min(1.0) - 1.0) * r.strength_score_factor + (i.antagonize + i.rival + i.threat - i.befriend) / 4.0 + 2.0 * i.t_infamy;
    if i.relation > 0.0 {
        s -= if i.holds_cb { i.relation / 2.0 } else { i.relation };
    }
    if !i.t_over_limit && !i.holds_cb && i.relation > 100.0 {
        return 0.0;
    }
    if !i.holds_cb {
        s *= if i.civ_vs_unciv { r.no_cb_unciv_mult } else { r.no_cb_mult };
    }
    // Step 17: close in rank, and T ranked better.
    if i.own_rank > 0.0 && i.t_rank > 0.0 {
        let d = (i.own_rank - i.t_rank).abs();
        if d < 8.0 {
            s *= 1.1;
            if d < 4.0 {
                s *= 1.2;
            }
        }
        if i.t_rank < i.own_rank {
            s *= 1.3;
        }
    }
    if i.goal_value <= r.target_goal_min_value {
        return 0.0;
    }
    s *= 1.0 + i.goal_value.min(100.0) / 100.0;
    if let Some((goal, own)) = i.unjust_goal_infamy {
        s *= (1.0 - (goal + own) / (w.badboy_limit - 5.0)).max(0.0);
    }
    s.max(0.0)
}

/// One target drawn in proportion to the scores at or above the minimum. `roll` is uniform in 0..1.
pub fn draw_target(r: &WarAiRules, scored: &[(String, f64)], roll: f64) -> Option<String> {
    let ok: Vec<&(String, f64)> = scored.iter().filter(|(_, s)| *s >= r.target_min_score).collect();
    let total: f64 = ok.iter().map(|(_, s)| s).sum();
    if total <= 0.0 {
        return None;
    }
    let mut x = roll.clamp(0.0, 1.0) * total;
    for (t, s) in &ok {
        if x < *s {
            return Some(t.clone());
        }
        x -= s;
    }
    ok.last().map(|(t, _)| t.clone())
}

// ---- The goal picker -------------------------------------------

/// Facts about the pair for the goal picker.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoalContext {
    pub own_infamy: f64,
    pub ceiling: f64,
    /// Only held or "always" types may be used.
    pub held_only: bool,
    pub at_war: bool,
    pub t_states: f64,
    pub own_states: f64,
    pub t_military: f64,
    pub own_military: f64,
    pub t_industry: f64,
    pub own_industry: f64,
    pub t_prestige: f64,
    pub own_prestige: f64,
    pub t_over_limit: bool,
    /// C and T alike (great power or not, civilised or not).
    pub alike: bool,
    pub t_civilised: bool,
    pub great_war: bool,
    /// Cost of the goals already on the own side (at war), and the war's one-side maximum.
    pub cost_so_far: f64,
}

/// One casus belli type offered to the picker.
#[derive(Debug, Clone, Copy)]
pub struct GoalOption<'a> {
    pub cb: &'a CbType,
    pub held: bool,
    /// Cost of this goal in war score points (at war only).
    pub cost: f64,
    /// Already a goal of the war.
    pub present: bool,
}

/// Weight of one type against T.
pub fn type_weight(r: &WarAiRules, cb: &CbType, c: &GoalContext) -> f64 {
    let mut v = 1.0;
    let cut_down = c.t_over_limit || c.alike;
    let peace_cut = if !c.at_war && !c.t_over_limit { 0.1 } else { 1.0 };
    for o in &cb.options {
        match o.as_str() {
            "annex" | "make_puppet" if c.t_states < c.own_states => {
                // STAND-IN: the value of a state was not read; one demand_state weight per state.
                let s = c.t_states * r.demand_state_weight;
                v += if o == "make_puppet" { s * 0.5 } else { s };
            }
            "clear_union_sphere" => v += 1000.0,
            "install_communist_gov_type" | "uninstall_communist_gov_type" => v += 100.0,
            "disarmament" if cut_down => {
                let x = if c.own_military <= 0.0 { 50.0 } else { (50.0 * c.t_military / c.own_military).min(100.0) };
                v += if x < 25.0 { 0.0 } else { x * peace_cut };
            }
            "reparations" if cut_down => v += (50.0 * c.t_industry / c.own_industry.max(1e-9)).min(100.0) * peace_cut,
            "prestige" if cut_down => {
                let x = (100.0 * c.t_prestige / c.own_prestige.max(1e-9)).min(200.0);
                v += if x < 50.0 { 0.0 } else { x * peace_cut };
            }
            // STAND-IN: the helper that values a state was not read.
            "demand_state" => v += r.demand_state_weight,
            // NOT WIRED: gunboat, add_to_sphere, transfer_provinces, release_puppet (third country helpers).
            _ => {}
        }
    }
    v
}

/// The best goal: (index into `options`, value). `spread` gives one roll in 0..1 per option for the
/// +-50 % random spread, or `None`.
pub fn pick_goal(r: &WarAiRules, w: &WarRules, options: &[GoalOption], c: &GoalContext, spread: Option<&[f64]>) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (k, o) in options.iter().enumerate() {
        let just = o.held || o.cb.needs_no_justification;
        if c.held_only && !just {
            continue;
        }
        if c.at_war && (o.present || c.cost_so_far + o.cost > r.goals_cost_cap) {
            continue;
        }
        let mut v = type_weight(r, o.cb, c);
        if !just {
            let sum = c.own_infamy + war::goal_infamy(w, o.cb, c.great_war);
            if sum >= c.ceiling {
                continue;
            }
            v *= (1.0 - sum / c.ceiling) * if c.at_war { 1.0 } else { 0.5 } * if c.t_civilised { 2.0 } else { 1.0 };
        }
        if let Some(s) = spread.and_then(|s| s.get(k)) {
            v *= 0.5 + s.clamp(0.0, 1.0);
        }
        if best.is_none_or(|(_, b)| v > b) {
            best = Some((k, v));
        }
    }
    best
}

// ---- B. Working on the target: justify, worsen relations, declare ---------------------------

/// Chance per mille that C starts justifying a casus belli against T this move.
pub fn justify_chance(r: &WarAiRules, w: &WarRules, own_infamy: f64, civ_vs_unciv: bool) -> f64 {
    let a = r.aggression_base + if civ_vs_unciv { r.aggression_unciv_bonus } else { 0.0 };
    (a * (1.0 - own_infamy / w.badboy_limit)).max(0.0)
}

/// What C does about its war target in one move.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetMove {
    /// Start justifying a casus belli of this type.
    Justify(String),
    /// Worsen relations first.
    DecreaseRelation,
    /// Declare war with a goal of this type.
    Declare(String),
    Nothing,
}

/// Facts of one move against the target.
#[derive(Debug, Clone, Default)]
pub struct MoveInput {
    pub relation: f64,
    pub truce: bool,
    /// A usable casus belli is held (or an "always" type exists).
    pub has_cb: bool,
    pub justifying: bool,
    /// The justification roll in 0..1000.
    pub roll: f64,
    pub justify_chance: f64,
    /// The picker's choice for justifying (type, value) and for declaring (held or always types).
    pub justify_goal: Option<(String, f64)>,
    pub declare_goal: Option<String>,
}

/// One move of 2.1 B.
pub fn target_move(r: &WarAiRules, i: &MoveInput) -> TargetMove {
    if !i.has_cb
        && !i.justifying
        && i.roll < i.justify_chance
        && let Some((cb, v)) = &i.justify_goal
        && *v > r.justify_min_value
    {
        return TargetMove::Justify(cb.clone());
    }
    if i.relation > r.declare_relation_floor {
        return TargetMove::DecreaseRelation;
    }
    if i.has_cb
        && !i.truce
        && let Some(cb) = &i.declare_goal
    {
        return TargetMove::Declare(cb.clone());
    }
    TargetMove::Nothing
}

/// STAND-IN (the rule description does not read the AI's answer to a call to arms): an ally joins
/// when it is not at war with the caller's side already, is not allied to the enemy, and its
/// relation with the caller is at least `call_ally_min_relation`.
pub fn ally_joins(r: &WarAiRules, relation_with_caller: f64, allied_to_enemy: bool, already_in_war: bool) -> bool {
    !allied_to_enemy && !already_in_war && relation_with_caller >= r.call_ally_min_relation
}

// ---- 2.4 Peace offer ----------------------------------------------------------------------

/// The offer the AI sends (2.4 step 3). `table` holds the goals of the demanding side, all
/// selected at first. From the last to the first a goal is unselected when `signs` (the party that
/// has to swallow the offer: the enemy of a winning sender, the sender itself when it concedes)
/// refuses the offer as it stands, unless the goal is obligatory. The offer is sent when `both`
/// parties accept what is left; an empty list is a white peace.
pub fn peace_offer(table: &[usize], obligatory: &dyn Fn(usize) -> bool, signs: &dyn Fn(&[usize]) -> bool, both: &dyn Fn(&[usize]) -> bool) -> Option<Vec<usize>> {
    let mut kept: Vec<usize> = table.to_vec();
    for &g in table.iter().rev() {
        if !signs(&kept) && !obligatory(g) {
            kept.retain(|k| *k != g);
        }
    }
    both(&kept).then_some(kept)
}
