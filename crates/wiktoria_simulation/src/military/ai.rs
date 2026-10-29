//! The computer's army control as pure decision functions. Inputs are plain data, outputs are orders
//! `(army id, target province)` and mobilise yes/no per country. The day loop applies them.
//!
//! Rules kept:
//! * a stack picks its target among provinces held by the enemy (to besiege) and own provinces held by
//!   the enemy (lifted first, occupied cores raise war exhaustion), plus provinces with enemy armies;
//! * the distance score decides across all targets: an own province held by the enemy and a war-goal province
//!   get their factors, and a province holding an enemy army it expects to beat gets [`AiRules::enemy_army_factor`],
//!   so it ranks with sieges at a similar distance instead of after all of them (MIL_AI_003);
//! * where enemy armies stand or march, it only goes when its side is about 1.1 to 1 stronger
//!   ([`AiRules::attack_limit`]), at lower organisation it needs more;
//! * a siege takes `SIEGE_BRIGADES_MIN` to `SIEGE_BRIGADES_MAX` regiments (defines);
//! * an army keeps its target until it is reached or no longer valid, so it does not shuffle;
//! * stacks are merged up to the supply limit of the province ([`plan_stacks`]), no fixed cap;
//! * mobilise when the side's land strength is under 1.5 times the enemy's ([`mobilise_decisions`]);
//! * forces are compared by the AI's own measure ([`regiment_power`]): a stack at half organisation counts half;
//! * a march whose next step is no longer safe is cancelled ([`safe_step`], [`unsafe_marches`]);
//! * a besieging stack stays; only against rebels the excess over the siege maximum leaves ([`siege_excess`]);
//! * a worn stack rests in a quiet own province ([`plan_recovery`]); stacks standing together merge
//!   ([`plan_merges`]) and a small one waits for those marching to it ([`regroup_wait`]);
//! * the regiments an area needs ([`needed_regiments`]; nothing in the port asks for it yet);
//! * idle fleets with warships blockade enemy ports while [`blockade_allowed`] says so ([`plan_blockades`]).
//!
//! Constants: siege limits from the install's defines, the rest `FLD_MIL_AI_*` in the rules file (hypotheses).

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::defines::Defines;
use wiktoria_data::map::ProvinceId;
use wiktoria_data::engine_rules::EngineRules;

use super::movement::MoveMap;
use super::war::War;
use crate::num_field;

/// Engine constants of the army AI.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRules {
    /// Attack only when R = foe / own (blend of strength and regiment ratios) is at most this (0.9: about 1.1 to 1).
    pub attack_limit: f64,
    /// Below this share of maximum organisation an army attacks nothing.
    pub org_floor: f64,
    /// Regiments per 1000 men of supply limit in the siege test (3: a regiment is 3000 men).
    pub siege_men_per_regiment: f64,
    /// Score factor of an own province held by the enemy, and of a province in a war goal's state (lower = better).
    pub own_lift_factor: f64,
    pub goal_factor: f64,
    /// Score factor of a province whose only reason to go is an enemy army the stack expects to beat (lower = better).
    pub enemy_army_factor: f64,
    /// Mobilise when own land strength is under this times the enemy's.
    pub mobilise_ratio: f64,
    /// Area value: cap on size, capital bonus, bonus per war-goal province.
    pub area_size_cap: f64,
    pub area_capital: f64,
    pub area_goal: f64,
    /// Blockade only with own warships over this times the enemy's, and no enemy fleet above `blockade_max_fleet`.
    pub blockade_ratio: f64,
    pub blockade_max_fleet: usize,
    /// Defines `SIEGE_BRIGADES_MIN` and `_MAX`.
    pub siege_min: f64,
    pub siege_max: f64,
    /// The rebel tag, hostile to everyone.
    pub rebel_tag: String,
    /// Strength measure: a regiment counts 1 + this per experience point; under the strength floor
    /// (thousand men) or the organisation floor it counts times the weak factor.
    pub power_experience: f64,
    pub power_weak_strength: f64,
    pub power_weak_org: f64,
    pub power_weak_factor: f64,
    /// A next step is unsafe at hostile / friendly strength from this ratio on, or with no friends to speak of.
    pub safe_step_ratio: f64,
    pub safe_step_min_friends: f64,
    /// Recover: start under this organisation share, rest up to that one, never with this share of the homeland lost.
    pub recover_below: f64,
    pub recover_until: f64,
    pub recover_homeland_lost: f64,
    /// Recover province score: own capital, the fixed spread (step, number of steps), a neighbour owned / held by a hostile.
    pub recover_capital: f64,
    pub recover_spread: f64,
    pub recover_spread_steps: u64,
    pub recover_hostile_owner: f64,
    pub recover_hostile_holder: f64,
    /// A stack under this share of its nearby regiments waits; not in a province with more neighbours than the maximum.
    pub regroup_share: f64,
    pub regroup_max_neighbours: usize,
    /// Needed regiments of an area: the least for a defended one, for an empty hostile one, the factor on
    /// support next to it, the share of other enemies around it.
    pub needed_min: f64,
    pub needed_empty: f64,
    pub needed_support: f64,
    pub needed_others_share: f64,
    /// Guard posts in peace ([`plan_posts`], our own design): regiments per post, the least number
    /// of land steps between two posts, how often a country looks again (days), and the threat
    /// factor of a neighbour whose relation is under the hostile line.
    pub post_regiments: f64,
    pub post_spacing: usize,
    pub post_days: i64,
    pub post_hostile_relation: f64,
    pub post_hostile_factor: f64,
    /// Gathering at war ([`plan_gather`], our own design): a field army under this share of the
    /// main army's power within this many land steps joins it.
    pub gather_share: f64,
    pub gather_steps: usize,
    /// Rules switched off for a measuring run (the CLI's `--ai-off`); nothing is off in a game.
    pub off: AiOff,
}

/// The army rules a measuring run can switch off one at a time, to see what each does to a war.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AiOff {
    /// Stacks standing together stay apart ([`plan_merges`]).
    pub merge: bool,
    /// Worn stacks do not go to rest ([`plan_recovery`]).
    pub recover: bool,
    /// A march goes on whatever waits at its next step ([`unsafe_marches`]).
    pub safe_step: bool,
    /// A small stack does not wait for those marching to it ([`regroup_wait`]).
    pub regroup: bool,
    /// A besieging stack keeps its excess against rebels too ([`siege_excess`]).
    pub siege_split: bool,
    /// The strength measure counts men only, not organisation or experience ([`regiment_power`]).
    pub power: bool,
    /// Targets are ranked by kind first (own occupied, then sieges, then enemy armies) and by the
    /// distance score only within a kind: the ranking before MIL_AI_003 ([`plan_armies`]).
    pub enemy_rank: bool,
    /// Armies with no target over land stay at home instead of sailing to one ([`plan_landings`]).
    pub landing: bool,
    /// Armies at peace stay where they stand instead of going to guard posts ([`plan_posts`]).
    pub posts: bool,
    /// Field armies at war choose their targets each on its own instead of gathering ([`plan_gather`]).
    pub gather: bool,
}

impl AiOff {
    pub const NAMES: [&'static str; 10] = ["merge", "recover", "safe-step", "regroup", "siege-split", "power", "enemy-rank", "landing", "posts", "gather"];

    /// The names of the rules switched off, in the order of [`AiOff::NAMES`].
    pub fn names_off(&self) -> Vec<&'static str> {
        let on = [self.merge, self.recover, self.safe_step, self.regroup, self.siege_split, self.power, self.enemy_rank, self.landing, self.posts, self.gather];
        AiOff::NAMES.into_iter().zip(on).filter(|x| x.1).map(|x| x.0).collect()
    }

    /// From a comma-separated list of [`AiOff::NAMES`]; `all` is every one of them.
    pub fn parse(list: &str) -> Result<AiOff, String> {
        let mut off = AiOff::default();
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match name {
                "merge" => off.merge = true,
                "recover" => off.recover = true,
                "safe-step" => off.safe_step = true,
                "regroup" => off.regroup = true,
                "siege-split" => off.siege_split = true,
                "power" => off.power = true,
                "enemy-rank" => off.enemy_rank = true,
                "landing" => off.landing = true,
                "posts" => off.posts = true,
                "gather" => off.gather = true,
                "all" => off = AiOff { merge: true, recover: true, safe_step: true, regroup: true, siege_split: true, power: true, enemy_rank: true, landing: true, posts: true, gather: true },
                other => return Err(format!("--ai-off: no rule called {other}; there are {} and all", AiOff::NAMES.join(", "))),
            }
        }
        Ok(off)
    }
}

impl AiRules {
    pub fn new(m: &EngineRules, defines: &Defines) -> Result<AiRules, String> {
        let f = |id: &str| num_field(m, id);
        let d = |k: &str| defines.get("military", k).ok_or(format!("defines.military.{k} missing"));
        Ok(AiRules {
            attack_limit: f("FLD_MIL_AI_ATTACK_LIMIT")?,
            org_floor: f("FLD_MIL_AI_ORG_FLOOR")?,
            siege_men_per_regiment: f("FLD_MIL_AI_SIEGE_SUPPLY_PER_REGIMENT")?,
            own_lift_factor: f("FLD_MIL_AI_OWN_LIFT_FACTOR")?,
            goal_factor: f("FLD_MIL_AI_GOAL_FACTOR")?,
            enemy_army_factor: f("FLD_MIL_AI_ENEMY_ARMY_FACTOR")?,
            mobilise_ratio: f("FLD_MIL_AI_MOBILISE_RATIO")?,
            area_size_cap: f("FLD_MIL_AI_AREA_SIZE_CAP")?,
            area_capital: f("FLD_MIL_AI_AREA_CAPITAL")?,
            area_goal: f("FLD_MIL_AI_AREA_GOAL")?,
            blockade_ratio: f("FLD_MIL_AI_BLOCKADE_RATIO")?,
            blockade_max_fleet: f("FLD_MIL_AI_BLOCKADE_MAX_FLEET")? as usize,
            siege_min: d("SIEGE_BRIGADES_MIN")?,
            siege_max: d("SIEGE_BRIGADES_MAX")?,
            rebel_tag: m.require_str("FLD_MIL_REBEL_TAG")?.to_string(),
            power_experience: f("FLD_MIL_AI_POWER_EXPERIENCE")?,
            power_weak_strength: f("FLD_MIL_AI_POWER_WEAK_STRENGTH")?,
            power_weak_org: f("FLD_MIL_AI_POWER_WEAK_ORG")?,
            power_weak_factor: f("FLD_MIL_AI_POWER_WEAK_FACTOR")?,
            safe_step_ratio: f("FLD_MIL_AI_SAFE_STEP_RATIO")?,
            safe_step_min_friends: f("FLD_MIL_AI_SAFE_STEP_MIN_FRIENDS")?,
            recover_below: f("FLD_MIL_AI_RECOVER_BELOW")?,
            recover_until: f("FLD_MIL_AI_RECOVER_UNTIL")?,
            recover_homeland_lost: f("FLD_MIL_AI_RECOVER_HOMELAND_LOST")?,
            recover_capital: f("FLD_MIL_AI_RECOVER_CAPITAL")?,
            recover_spread: f("FLD_MIL_AI_RECOVER_SPREAD")?,
            recover_spread_steps: f("FLD_MIL_AI_RECOVER_SPREAD_STEPS")?.max(1.0) as u64,
            recover_hostile_owner: f("FLD_MIL_AI_RECOVER_HOSTILE_OWNER")?,
            recover_hostile_holder: f("FLD_MIL_AI_RECOVER_HOSTILE_HOLDER")?,
            regroup_share: f("FLD_MIL_AI_REGROUP_SHARE")?,
            regroup_max_neighbours: f("FLD_MIL_AI_REGROUP_MAX_NEIGHBOURS")? as usize,
            needed_min: f("FLD_MIL_AI_NEEDED_MIN")?,
            needed_empty: f("FLD_MIL_AI_NEEDED_EMPTY")?,
            needed_support: f("FLD_MIL_AI_NEEDED_SUPPORT")?,
            needed_others_share: f("FLD_MIL_AI_NEEDED_OTHERS_SHARE")?,
            post_regiments: f("FLD_MIL_AI_POST_REGIMENTS")?,
            post_spacing: f("FLD_MIL_AI_POST_SPACING")?.max(1.0) as usize,
            post_days: f("FLD_MIL_AI_POST_DAYS")?.max(1.0) as i64,
            post_hostile_relation: f("FLD_MIL_AI_POST_HOSTILE_RELATION")?,
            post_hostile_factor: f("FLD_MIL_AI_POST_HOSTILE_FACTOR")?,
            gather_share: f("FLD_MIL_AI_GATHER_SHARE")?,
            gather_steps: f("FLD_MIL_AI_GATHER_STEPS")?.max(1.0) as usize,
            off: AiOff::default(),
        })
    }
}

/// A regiment for the AI's strength measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiRegiment {
    pub strength: f64,
    pub max_strength: f64,
    pub organisation: f64,
    pub max_organisation: f64,
    pub experience: f64,
}

/// What a regiment counts wherever the AI compares forces: (strength share) x (organisation share) x
/// (1 + experience x step), times the weak factor under the strength floor or the organisation floor.
/// NOT WIRED: the factor of the country on the sum (a define plus a technology value, both not read).
pub fn regiment_power(r: &AiRules, g: &AiRegiment) -> f64 {
    let share = |x: f64, max: f64| if max > 0.0 { (x / max).clamp(0.0, 1.0) } else { 0.0 };
    if r.off.power {
        return share(g.strength, g.max_strength);
    }
    let v = share(g.strength, g.max_strength) * share(g.organisation, g.max_organisation) * (1.0 + g.experience * r.power_experience);
    if g.strength < r.power_weak_strength || g.organisation < r.power_weak_org { v * r.power_weak_factor } else { v }
}

/// The strength measure of a stack: the sum over its regiments (about 1 per full, rested regiment).
pub fn stack_power(r: &AiRules, regiments: &[AiRegiment]) -> f64 {
    regiments.iter().map(|g| regiment_power(r, g)).sum()
}

/// An army as the AI sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiArmy {
    pub id: u64,
    pub owner: String,
    pub location: ProvinceId,
    /// Current target (the army's `target`), kept while valid.
    pub target: Option<ProvinceId>,
    /// Total strength in thousands of men (`logistics::stack_weight`).
    pub strength: f64,
    pub regiments: usize,
    /// Organisation as a share of the maximum.
    pub org: f64,
    pub sieging: bool,
    /// In a battle, retreating or aboard: no new orders.
    pub busy: bool,
    /// The strength measure of the stack ([`stack_power`]), used in every comparison of forces.
    pub power: f64,
    /// The next province of its march, the days until it gets there, and where the march ends.
    pub next: Option<ProvinceId>,
    pub arrive: f64,
    pub destination: Option<ProvinceId>,
    pub fighting: bool,
    pub retreating: bool,
    /// Recovering: it rests and takes no other order ([`plan_recovery`]).
    pub resting: bool,
}

/// A land province's facts for the AI.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiProvince {
    pub owner: Option<String>,
    pub controller: Option<String>,
    /// The state is demanded by a goal of a running war.
    pub war_goal: bool,
    /// A country's capital lies here.
    pub capital: bool,
}

/// Who is at war with whom and who fights on the same side, from the wars.
#[derive(Debug, Clone, Default)]
pub struct Sides {
    enemies: BTreeSet<(String, String)>,
    allies: BTreeSet<(String, String)>,
    rebel: String,
}

impl Sides {
    pub fn from_wars(wars: &[War], rebel_tag: &str) -> Sides {
        let mut s = Sides { rebel: rebel_tag.to_string(), ..Sides::default() };
        for w in wars.iter().filter(|w| w.active) {
            for a in &w.attackers {
                for d in &w.defenders {
                    s.enemies.insert((a.clone(), d.clone()));
                    s.enemies.insert((d.clone(), a.clone()));
                }
                for b in &w.attackers {
                    s.allies.insert((a.clone(), b.clone()));
                }
            }
            for a in &w.defenders {
                for b in &w.defenders {
                    s.allies.insert((a.clone(), b.clone()));
                }
            }
        }
        s
    }

    /// Different countries at war, or either is the rebel tag.
    pub fn hostile(&self, a: &str, b: &str) -> bool {
        a != b && (self.enemies.contains(&(a.to_string(), b.to_string())) || a == self.rebel || b == self.rebel)
    }

    pub fn friendly(&self, a: &str, b: &str) -> bool {
        a == b || self.allies.contains(&(a.to_string(), b.to_string()))
    }

    pub fn at_war(&self, tag: &str) -> bool {
        self.enemies.iter().any(|(a, _)| a == tag)
    }

    /// The two fight each other in a war (rebels are hostile, but not at war).
    pub fn at_war_with(&self, a: &str, b: &str) -> bool {
        self.enemies.contains(&(a.to_string(), b.to_string()))
    }
}

/// Everything the army decisions look at.
pub struct AiWorld<'a> {
    pub map: &'a MoveMap,
    pub provinces: &'a BTreeMap<ProvinceId, AiProvince>,
    /// All armies of all countries.
    pub armies: &'a [AiArmy],
    pub sides: &'a Sides,
    /// Supply limit in thousands of men for a country in a province (`logistics::supply_limit`).
    pub supply: &'a dyn Fn(&str, ProvinceId) -> i64,
}

/// What a province is to a country at war.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// Own territory held by the enemy: lifted first.
    OwnOccupied,
    /// Enemy territory held by the enemy: besieged.
    EnemyHeld,
    /// Neither, but an enemy army is there.
    EnemyArmy,
}

fn holder_hostile(w: &AiWorld, tag: &str, p: &AiProvince) -> bool {
    p.controller.as_deref().is_some_and(|c| w.sides.hostile(tag, c))
}

/// The province's class for `tag`, if it is a target at all.
pub fn classify(w: &AiWorld, tag: &str, id: ProvinceId) -> Option<Class> {
    let p = w.provinces.get(&id)?;
    if holder_hostile(w, tag, p) {
        return Some(if p.owner.as_deref() == Some(tag) { Class::OwnOccupied } else { Class::EnemyHeld });
    }
    w.armies.iter().any(|a| a.location == id && w.sides.hostile(tag, &a.owner)).then_some(Class::EnemyArmy)
}

fn passable(w: &AiWorld, tag: &str, id: ProvinceId) -> bool {
    if w.map.provinces.get(&id).is_some_and(|m| m.sea) {
        return false;
    }
    let Some(p) = w.provinces.get(&id) else { return false };
    let owner_ok = p.owner.as_deref().is_none_or(|o| w.sides.friendly(tag, o) || w.sides.hostile(tag, o));
    owner_ok || p.controller.as_deref().is_some_and(|c| w.sides.friendly(tag, c))
}

/// Walking cost from `from` to every reachable land province (border distances, deterministic).
pub fn reach(w: &AiWorld, tag: &str, from: ProvinceId) -> BTreeMap<ProvinceId, f64> {
    let mut best: BTreeMap<ProvinceId, u64> = BTreeMap::from([(from, 0)]);
    let mut heap = std::collections::BinaryHeap::from([std::cmp::Reverse((0u64, from))]);
    while let Some(std::cmp::Reverse((cost, p))) = heap.pop() {
        if best.get(&p).is_some_and(|&c| c < cost) {
            continue;
        }
        for n in w.map.neighbours(p) {
            let Some(b) = w.map.border(p, n) else { continue };
            if b.kind == super::movement::BorderKind::Impassable || !passable(w, tag, n) {
                continue;
            }
            let c = cost + (b.distance * 1000.0).round().max(0.0) as u64;
            if best.get(&n).is_none_or(|&o| c < o) {
                best.insert(n, c);
                heap.push(std::cmp::Reverse((c, n)));
            }
        }
    }
    best.into_iter().map(|(k, v)| (k, v as f64 / 1000.0)).collect()
}

/// Strength (the AI's measure) committed to a province: armies of `tag`'s side standing in it or marching to it,
/// except `skip`. Returns (strength, regiments).
fn friends_at(w: &AiWorld, tag: &str, id: ProvinceId, skip: u64, extra: &BTreeMap<u64, ProvinceId>) -> (f64, usize) {
    let mut s = (0.0, 0);
    for a in w.armies.iter().filter(|a| a.id != skip && w.sides.friendly(tag, &a.owner)) {
        let t = extra.get(&a.id).copied().or(a.target);
        if a.location == id || t == Some(id) {
            s.0 += a.power;
            s.1 += a.regiments;
        }
    }
    s
}

fn foes_at(w: &AiWorld, tag: &str, id: ProvinceId) -> (f64, usize) {
    let mut s = (0.0, 0);
    for a in w.armies.iter().filter(|a| w.sides.hostile(tag, &a.owner)) {
        if a.location == id || a.target == Some(id) {
            s.0 += a.power;
            s.1 += a.regiments;
        }
    }
    s
}

/// The strength test: R = ((foe + 1) / (own + friends + 1) + (foe regs + 1) / (own regs + friend regs + 1)) / 2;
/// attack only when R is at most the limit, lowered by (1 - org) below full organisation.
pub fn attack_ok(r: &AiRules, org: f64, own: (f64, usize), friends: (f64, usize), foes: (f64, usize)) -> bool {
    if org < r.org_floor {
        return false;
    }
    let ratio = ((foes.0 + 1.0) / (own.0 + friends.0 + 1.0) + (foes.1 as f64 + 1.0) / ((own.1 + friends.1) as f64 + 1.0)) / 2.0;
    let limit = if org < 1.0 { r.attack_limit - (1.0 - org) } else { r.attack_limit };
    ratio <= limit
}

/// A siege has room: fewer than the maximum regiments there or coming, and from the minimum on the
/// supply limit must carry the stack.
pub fn siege_room(r: &AiRules, supply_limit: i64, friends_regiments: usize, own_regiments: usize) -> bool {
    let f = friends_regiments as f64;
    if f >= r.siege_max {
        return false;
    }
    !(f >= r.siege_min && (supply_limit as f64) < r.siege_men_per_regiment * (f + own_regiments as f64))
}

/// Is `target` still a good target for this army? (Used to keep a target until it is reached or invalid.)
fn evaluate(r: &AiRules, w: &AiWorld, a: &AiArmy, target: ProvinceId, committed: &BTreeMap<u64, ProvinceId>) -> Option<Class> {
    let class = classify(w, &a.owner, target)?;
    let foes = foes_at(w, &a.owner, target);
    let friends = friends_at(w, &a.owner, target, a.id, committed);
    if foes.1 > 0 {
        return attack_ok(r, a.org, (a.power, a.regiments), friends, foes).then_some(class);
    }
    if class == Class::EnemyArmy {
        return None;
    }
    // Besieging: room for this army (own territory lifted needs no limit: the garrison is not fortified by us).
    siege_room(r, (w.supply)(&a.owner, target), friends.1, a.regiments).then_some(class)
}

/// Orders for the armies of `tag`: one `(army id, target)` per army that needs a new target. Armies
/// that keep a valid target, are besieging, are busy or have nothing to do get no order.
/// The lowest score wins across all kinds of target (MIL_AI_003); ties go to the lower province id.
/// With [`AiOff::enemy_rank`] the kind decides first and the score only within a kind.
pub fn plan_armies(r: &AiRules, w: &AiWorld, tag: &str) -> Vec<(u64, ProvinceId)> {
    let mut orders: Vec<(u64, ProvinceId)> = Vec::new();
    let mut committed: BTreeMap<u64, ProvinceId> = BTreeMap::new();
    let mut mine: Vec<&AiArmy> = w.armies.iter().filter(|a| a.owner == tag && !a.busy).collect();
    mine.sort_by_key(|a| a.id);
    for a in mine {
        // A besieging stack does nothing else until the siege ends; nor does one that rests or waits.
        if a.sieging || a.resting || (!r.off.regroup && regroup_wait(r, w, a)) {
            continue;
        }
        if let Some(t) = a.target {
            // Keep it until reached or invalid. A reached target is kept too while it still is one.
            if evaluate(r, w, a, t, &committed).is_some() {
                continue;
            }
        }
        let costs = reach(w, tag, a.location);
        let mut best: Option<((u8, u64), ProvinceId)> = None;
        for (&id, &cost) in &costs {
            let Some(class) = evaluate(r, w, a, id, &committed) else { continue };
            let p = &w.provinces[&id];
            let held_neighbours = w.map.neighbours(id).filter(|n| w.provinces.get(n).is_some_and(|q| q.controller.as_deref() == Some(tag))).count();
            let mut score = cost.max(0.001) / (held_neighbours as f64 + 1.0);
            if class == Class::OwnOccupied {
                score *= r.own_lift_factor;
            }
            if p.war_goal {
                score *= r.goal_factor;
            }
            // The old ranking puts the kind first; the new one lets the score decide across kinds.
            let rank = if r.off.enemy_rank { class as u8 } else { 0 };
            if class == Class::EnemyArmy && !r.off.enemy_rank {
                score *= r.enemy_army_factor;
            }
            let key = (rank, (score * 1000.0) as u64);
            if best.is_none_or(|(k, bid)| (key, id) < (k, bid)) {
                best = Some((key, id));
            }
        }
        if let Some((_, id)) = best {
            if id != a.location {
                orders.push((a.id, id));
            }
            committed.insert(a.id, id);
        }
    }
    orders
}

/// Walking and sailing cost from `from`: as [`reach`], and sea provinces may be crossed (each
/// sea leg is sailed by a fleet of the country, `military::day`). Every reached land province
/// with its cost and the province it is reached from.
pub fn reach_by_sea(w: &AiWorld, tag: &str, from: ProvinceId) -> BTreeMap<ProvinceId, (f64, ProvinceId)> {
    let mut best: BTreeMap<ProvinceId, (u64, ProvinceId)> = BTreeMap::from([(from, (0, from))]);
    let mut heap = std::collections::BinaryHeap::from([std::cmp::Reverse((0u64, from))]);
    while let Some(std::cmp::Reverse((cost, p))) = heap.pop() {
        if best.get(&p).is_some_and(|&(c, _)| c < cost) {
            continue;
        }
        for n in w.map.neighbours(p) {
            let Some(b) = w.map.border(p, n) else { continue };
            let sea = w.map.provinces.get(&n).is_some_and(|m| m.sea);
            if !super::movement::army_passable(w.map, true, p, n) || (!sea && !passable(w, tag, n)) {
                continue;
            }
            let c = cost + (b.distance * 1000.0).round().max(0.0) as u64;
            if best.get(&n).is_none_or(|&(o, _)| c < o) {
                best.insert(n, (c, p));
                heap.push(std::cmp::Reverse((c, n)));
            }
        }
    }
    best.into_iter().map(|(k, (v, f))| (k, (v as f64 / 1000.0, f))).collect()
}

/// Landings. THE PORT'S STAND-IN for the original's overseas planning, which is not measured: an
/// idle army of at least `SIEGE_BRIGADES_MIN` regiments that has no target over land takes the
/// best one over the sea (scored as in [`plan_armies`]) when one of its country's free fleets
/// (`rooms`: the regiments each can carry) carries it whole; each fleet takes one army, the
/// armies in id order. Returns the path:
/// over land it only crosses provinces the army may enter. The march itself (wait on the coast,
/// board, sail, land) is the day step's.
pub fn plan_landings(r: &AiRules, w: &AiWorld, tag: &str, mut rooms: Vec<usize>, skip: &BTreeSet<u64>) -> Vec<(u64, Vec<ProvinceId>)> {
    let mut orders = Vec::new();
    if r.off.landing {
        return orders;
    }
    let committed: BTreeMap<u64, ProvinceId> = BTreeMap::new();
    let mut mine: Vec<&AiArmy> = w.armies.iter().filter(|a| a.owner == tag && !a.busy && !a.sieging && !a.resting && a.target.is_none() && a.destination.is_none() && !skip.contains(&a.id)).collect();
    mine.sort_by_key(|a| a.id);
    for a in mine {
        // The smallest free fleet that carries the whole army.
        let Some(fleet) = rooms.iter().enumerate().filter(|(_, x)| **x >= a.regiments).min_by_key(|(i, x)| (**x, *i)).map(|(i, _)| i) else { continue };
        if (a.regiments as f64) < r.siege_min {
            continue;
        }
        let land = reach(w, tag, a.location);
        if land.keys().any(|id| evaluate(r, w, a, *id, &committed).is_some()) {
            continue;
        }
        let by_sea = reach_by_sea(w, tag, a.location);
        let mut best: Option<(u64, ProvinceId)> = None;
        for (&id, &(cost, _)) in by_sea.iter().filter(|(id, _)| !land.contains_key(id) && !w.map.provinces.get(id).is_some_and(|m| m.sea)) {
            let Some(class) = evaluate(r, w, a, id, &committed) else { continue };
            let mut score = cost.max(0.001);
            if class == Class::OwnOccupied {
                score *= r.own_lift_factor;
            }
            if w.provinces[&id].war_goal {
                score *= r.goal_factor;
            }
            if class == Class::EnemyArmy {
                score *= r.enemy_army_factor;
            }
            let key = (score * 1000.0) as u64;
            if best.is_none_or(|(k, bid)| (key, id) < (k, bid)) {
                best = Some((key, id));
            }
        }
        if let Some((_, id)) = best {
            let mut path = vec![id];
            while let Some(&(_, prev)) = by_sea.get(path.last().expect("not empty")) {
                if prev == a.location {
                    break;
                }
                path.push(prev);
            }
            path.reverse();
            orders.push((a.id, path));
            rooms.remove(fleet);
        }
    }
    orders
}

/// Gathering at war. OUR OWN DESIGN: a country's field armies do not attack piecemeal. The
/// strongest idle field army is the main army; every other idle field army under `gather_share`
/// of its power that can walk to it within `gather_steps` land steps goes to it (while the main
/// army stands, besieges or fights; not while it marches), as far as the supply limit of the main
/// army's province feeds them all. Returns (army, province) orders; the
/// caller keeps them out of [`plan_armies`] until they arrive and merge.
pub fn plan_gather(r: &AiRules, w: &AiWorld, tag: &str) -> Vec<(u64, ProvinceId)> {
    let field: Vec<&AiArmy> = w.armies.iter().filter(|a| a.owner == tag && !a.busy && !a.resting && !a.retreating).collect();
    let Some(main) = field.iter().copied().filter(|a| a.next.is_none() || a.sieging).max_by(|a, b| a.power.total_cmp(&b.power).then(b.id.cmp(&a.id))) else { return Vec::new() };
    if main.next.is_some() && !main.sieging {
        return Vec::new();
    }
    // Land steps from the main army over passable land.
    let mut steps = BTreeMap::from([(main.location, 0usize)]);
    let mut queue = std::collections::VecDeque::from([main.location]);
    while let Some(p) = queue.pop_front() {
        let d = steps[&p];
        if d >= r.gather_steps {
            continue;
        }
        for n in w.map.neighbours(p) {
            if land_border(w, p, n) && passable(w, tag, n) && !steps.contains_key(&n) {
                steps.insert(n, d + 1);
                queue.push_back(n);
            }
        }
    }
    // Only as many as the province feeds: the main army, those there and those already coming.
    let mut room = (w.supply)(tag, main.location) as f64 - w.armies.iter().filter(|u| u.owner == tag && (u.location == main.location || u.destination == Some(main.location))).map(|u| u.strength).sum::<f64>();
    let mut out = Vec::new();
    for a in field {
        if a.id == main.id || a.sieging || a.location == main.location || a.power >= r.gather_share * main.power {
            continue;
        }
        if a.destination == Some(main.location) || !steps.contains_key(&a.location) || a.strength > room {
            continue;
        }
        room -= a.strength;
        out.push((a.id, main.location));
    }
    out
}

/// Merge orders so stacks stand near the supply limit of the province: an idle army with room to
/// spare walks to a neighbouring province where another idle army of the same owner stands and the
/// two together stay within the limit. The smaller moves (ties: the higher id), so it never goes both ways.
pub fn plan_stacks(w: &AiWorld, tag: &str) -> Vec<(u64, ProvinceId)> {
    let idle: Vec<&AiArmy> = w.armies.iter().filter(|a| a.owner == tag && !a.busy && !a.sieging && !a.resting && a.target.is_none()).collect();
    let mut orders = Vec::new();
    let mut moved: BTreeSet<u64> = BTreeSet::new();
    for a in &idle {
        if moved.contains(&a.id) {
            continue;
        }
        let mut best: Option<(&AiArmy, f64)> = None;
        for b in &idle {
            if b.id == a.id || moved.contains(&b.id) || b.location == a.location || w.map.border(a.location, b.location).is_none() {
                continue;
            }
            let smaller = a.strength < b.strength || (a.strength == b.strength && a.id > b.id);
            let limit = (w.supply)(tag, b.location) as f64;
            if smaller && a.strength + b.strength <= limit && best.is_none_or(|(o, _)| b.strength > o.strength) {
                best = Some((b, limit));
            }
        }
        if let Some((b, _)) = best {
            orders.push((a.id, b.location));
            moved.insert(a.id);
            moved.insert(b.id);
        }
    }
    orders
}

/// Guard posts in peace. OUR OWN DESIGN (the owner, 2026-10-29: our own logic where the game
/// need not be a replica). A country keeps its capital as a reserve and posts on the borders that
/// face the most regiments: a border province's threat is the largest `threat(neighbour)` over
/// the foreign owners next to it (0 for friends; the caller weighs regiments by relation). It
/// keeps one post per `post_regiments` of its regiments, the capital first, then the most
/// threatened border provinces at least `post_spacing` land steps from every post taken. Each
/// post's share of the regiments follows its threat (the capital counts half the largest, or 1
/// with no threat at all). Idle armies, largest first, walk to the nearest post with room; one
/// already at a post with room stays. Returns (army, province) orders.
pub fn plan_posts(r: &AiRules, w: &AiWorld, tag: &str, capital: Option<ProvinceId>, threat: &dyn Fn(&str) -> f64) -> Vec<(u64, ProvinceId)> {
    let own = |id: ProvinceId| w.provinces.get(&id).is_some_and(|p| p.owner.as_deref() == Some(tag) && p.controller.as_deref() == Some(tag)) && !w.map.provinces.get(&id).is_some_and(|m| m.sea);
    let mut scored: Vec<(f64, ProvinceId)> = Vec::new();
    for &id in w.provinces.keys() {
        if !own(id) {
            continue;
        }
        let t = w
            .map
            .neighbours(id)
            .filter(|n| land_border(w, id, *n))
            .filter_map(|n| w.provinces.get(&n)?.owner.as_deref())
            .filter(|o| *o != tag)
            .map(threat)
            .fold(0.0, f64::max);
        if t > 0.0 {
            scored.push((t, id));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mine: Vec<&AiArmy> = w.armies.iter().filter(|a| a.owner == tag).collect();
    let total: usize = mine.iter().map(|a| a.regiments).sum();
    if total == 0 {
        return Vec::new();
    }
    let wanted = ((total as f64 / r.post_regiments.max(1.0)).ceil() as usize).max(1);
    // Land steps over own provinces from every post taken so far.
    let steps_from = |from: ProvinceId| -> BTreeMap<ProvinceId, usize> {
        let mut out = BTreeMap::from([(from, 0usize)]);
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(p) = queue.pop_front() {
            let d = out[&p];
            if d + 1 >= r.post_spacing {
                continue;
            }
            for n in w.map.neighbours(p) {
                if land_border(w, p, n) && !out.contains_key(&n) {
                    out.insert(n, d + 1);
                    queue.push_back(n);
                }
            }
        }
        out
    };
    let top = scored.first().map_or(0.0, |x| x.0);
    let mut posts: Vec<(ProvinceId, f64)> = Vec::new();
    let mut near: BTreeSet<ProvinceId> = BTreeSet::new();
    if let Some(c) = capital.filter(|c| own(*c)) {
        posts.push((c, if top > 0.0 { top / 2.0 } else { 1.0 }));
        near.extend(steps_from(c).into_keys());
    }
    for (t, id) in &scored {
        if posts.len() >= wanted {
            break;
        }
        if near.contains(id) {
            continue;
        }
        posts.push((*id, *t));
        near.extend(steps_from(*id).into_keys());
    }
    if posts.is_empty() {
        return Vec::new();
    }
    let weights: f64 = posts.iter().map(|p| p.1).sum();
    let mut room: Vec<f64> = posts.iter().map(|p| total as f64 * p.1 / weights).collect();
    let reach_of: Vec<BTreeMap<ProvinceId, f64>> = posts.iter().map(|p| reach(w, tag, p.0)).collect();
    let mut idle: Vec<&&AiArmy> = mine.iter().filter(|a| !a.busy && !a.sieging && !a.resting && a.target.is_none() && a.next.is_none()).collect();
    idle.sort_by_key(|a| (std::cmp::Reverse(a.regiments), a.id));
    let mut out = Vec::new();
    for a in idle {
        let here = posts.iter().position(|p| p.0 == a.location);
        let choice = here.filter(|&i| room[i] > 0.0).or_else(|| (0..posts.len()).filter(|&i| room[i] > 0.0).filter_map(|i| Some((reach_of[i].get(&a.location).copied()?, i))).min_by(|x, y| x.0.total_cmp(&y.0)).map(|x| x.1));
        let Some(i) = choice else { continue };
        room[i] -= a.regiments as f64;
        if posts[i].0 != a.location {
            out.push((a.id, posts[i].0));
        }
    }
    out
}

/// Connected blocks of land provinces under one controller.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub controller: Option<String>,
    pub provinces: Vec<ProvinceId>,
}

pub fn areas(map: &MoveMap, provinces: &BTreeMap<ProvinceId, AiProvince>) -> Vec<Area> {
    let mut seen: BTreeSet<ProvinceId> = BTreeSet::new();
    let mut out = Vec::new();
    for (&start, p) in provinces {
        if seen.contains(&start) || map.provinces.get(&start).is_some_and(|m| m.sea) {
            continue;
        }
        let mut list = vec![start];
        seen.insert(start);
        let mut i = 0;
        while i < list.len() {
            let at = list[i];
            i += 1;
            for n in map.neighbours(at) {
                if seen.contains(&n) || map.provinces.get(&n).is_some_and(|m| m.sea) {
                    continue;
                }
                if provinces.get(&n).is_some_and(|q| q.controller == p.controller) {
                    seen.insert(n);
                    list.push(n);
                }
            }
        }
        list.sort_unstable();
        out.push(Area { controller: p.controller.clone(), provinces: list });
    }
    out
}

// ---------------------------------------------------------------- safe next step

fn land_border(w: &AiWorld, from: ProvinceId, to: ProvinceId) -> bool {
    w.map.border(from, to).is_some_and(|b| b.kind != super::movement::BorderKind::Impassable) && !w.map.provinces.get(&to).is_some_and(|m| m.sea)
}

/// Is the step of `a` into `n`, reached in `days`, safe? Hostile strength that is still in `n` then,
/// plus hostiles marching into it from next door, against the friends there and coming plus the army
/// itself. No terrain, no regiment count: because strength carries the organisation share, a stack that
/// just fought fails this against a rested enemy of its size.
pub fn safe_step(r: &AiRules, w: &AiWorld, a: &AiArmy, n: ProvinceId, days: f64) -> bool {
    let (mut foes, mut friends) = (0.0, 0.0);
    for u in w.armies.iter().filter(|u| u.id != a.id && !u.retreating) {
        // A unit marching out of `n` counts only if it is still there when `a` comes.
        let there = u.location == n && (u.next.is_none() || u.fighting || u.arrive >= days);
        let coming = u.next == Some(n) && !u.fighting && land_border(w, u.location, n);
        if !(there || coming) {
            continue;
        }
        if w.sides.hostile(&a.owner, &u.owner) {
            foes += u.power;
        } else if w.sides.friendly(&a.owner, &u.owner) {
            friends += u.power;
        }
    }
    if foes <= 0.0 {
        return true;
    }
    friends += a.power;
    friends > r.safe_step_min_friends && foes / friends < r.safe_step_ratio
}

/// The armies of `tag` whose march is cancelled today: the next step is no longer safe. They stop
/// and choose again as idle stacks, a day later.
pub fn unsafe_marches(r: &AiRules, w: &AiWorld, tag: &str) -> Vec<u64> {
    w.armies.iter().filter(|a| a.owner == tag && !a.busy).filter(|a| a.next.is_some_and(|n| !safe_step(r, w, a, n, a.arrive))).map(|a| a.id).collect()
}

// ---------------------------------------------------------------- siege

/// Regiments a besieging stack splits off: the excess over the siege maximum. OUR OWN DESIGN in
/// every war (the original splits only against rebels): a big stack leaves a siege detachment and
/// marches on, rather than sitting whole in one siege.
pub fn siege_excess(r: &AiRules, _controller: Option<&str>, regiments: usize) -> usize {
    regiments.saturating_sub(r.siege_max.max(0.0) as usize)
}

// ---------------------------------------------------------------- areas, recover, regroup, needed

/// The areas of the map with the area of every province and the areas next to each.
#[derive(Debug, Clone, Default)]
pub struct AreaIndex {
    pub list: Vec<Area>,
    of: BTreeMap<ProvinceId, usize>,
    next: Vec<BTreeSet<usize>>,
}

impl AreaIndex {
    pub fn new(map: &MoveMap, provinces: &BTreeMap<ProvinceId, AiProvince>) -> AreaIndex {
        let list = areas(map, provinces);
        let of: BTreeMap<ProvinceId, usize> = list.iter().enumerate().flat_map(|(i, a)| a.provinces.iter().map(move |p| (*p, i))).collect();
        let mut next = vec![BTreeSet::new(); list.len()];
        for (&p, &i) in &of {
            for j in map.neighbours(p).filter_map(|n| of.get(&n).copied()).filter(|j| *j != i) {
                next[i].insert(j);
            }
        }
        AreaIndex { list, of, next }
    }

    pub fn of(&self, p: ProvinceId) -> Option<usize> {
        self.of.get(&p).copied()
    }

    pub fn neighbours(&self, area: usize) -> impl Iterator<Item = usize> + '_ {
        self.next.get(area).into_iter().flatten().copied()
    }

    fn held_by(&self, area: usize, tag: &str) -> bool {
        self.list[area].controller.as_deref() == Some(tag)
    }
}

/// Occupied share of the homeland: 0 at peace, else the share of `tag`'s own provinces it does not
/// hold, over the area of its capital and the areas next to it.
/// NOT WIRED: the weight of a province (not read, every province counts 1) and the extra for a lost capital.
pub fn homeland_lost(w: &AiWorld, areas: &AreaIndex, tag: &str) -> f64 {
    if !w.sides.at_war(tag) {
        return 0.0;
    }
    let Some(home) = w.provinces.iter().find(|(_, p)| p.capital && p.owner.as_deref() == Some(tag)).and_then(|(id, _)| areas.of(*id)) else { return 0.0 };
    let (mut own, mut held) = (0.0, 0.0);
    for p in std::iter::once(home).chain(areas.neighbours(home)).flat_map(|i| &areas.list[i].provinces).filter_map(|p| w.provinces.get(p)) {
        if p.owner.as_deref() == Some(tag) {
            own += 1.0;
            if p.controller.as_deref() == Some(tag) {
                held += 1.0;
            }
        }
    }
    if own > 0.0 { 1.0 - held / own } else { 0.0 }
}

/// Where a worn stack goes to rest, if the rule applies to where it stands: on owned land, in an area
/// that is hostile-held or next to a hostile one. Pool: the own provinces without a battle of its area
/// (if `tag` holds it) or of the neighbouring areas `tag` holds. Score, lowest wins: the walk, less for
/// the own capital, a fixed spread by army and province, more per neighbour a hostile owns, holds or besieges.
pub fn recover_province(r: &AiRules, w: &AiWorld, areas: &AreaIndex, a: &AiArmy) -> Option<ProvinceId> {
    let tag = a.owner.as_str();
    w.provinces.get(&a.location)?.owner.as_ref()?;
    let area = areas.of(a.location)?;
    let hostile_area = |i: usize| areas.list[i].controller.as_deref().is_some_and(|c| w.sides.hostile(tag, c));
    if !hostile_area(area) && !areas.neighbours(area).any(hostile_area) {
        return None;
    }
    let pool: Vec<usize> = if areas.held_by(area, tag) { vec![area] } else { areas.neighbours(area).filter(|i| areas.held_by(*i, tag)).collect() };
    let costs = reach(w, tag, a.location);
    let hostile = |t: &Option<String>| t.as_deref().is_some_and(|c| w.sides.hostile(tag, c));
    let mut best: Option<(f64, ProvinceId)> = None;
    for &p in pool.iter().flat_map(|i| &areas.list[*i].provinces) {
        let (Some(q), Some(&walk)) = (w.provinces.get(&p), costs.get(&p)) else { continue };
        if q.owner.as_deref() != Some(tag) || w.armies.iter().any(|u| u.fighting && u.location == p) {
            continue;
        }
        let mut score = walk + r.recover_spread * ((a.id + p as u64) % r.recover_spread_steps) as f64;
        if q.capital && p != a.location {
            score -= r.recover_capital;
        }
        for n in w.map.neighbours(p).filter(|n| land_border(w, p, *n)) {
            let Some(x) = w.provinces.get(&n) else { continue };
            if hostile(&x.owner) {
                score += r.recover_hostile_owner;
            }
            if hostile(&x.controller) || w.armies.iter().any(|u| u.sieging && u.location == n && w.sides.hostile(tag, &u.owner)) {
                score += r.recover_hostile_holder;
            }
        }
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, p));
        }
    }
    best.map(|(_, p)| p)
}

/// What the recover rule decides for the armies of a country today.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recovery {
    /// Armies that start to recover, with the province they rest in.
    pub go: Vec<(u64, ProvinceId)>,
    /// Resting armies that are done: rested, or too much of the homeland is occupied.
    pub done: Vec<u64>,
}

/// Recover: a stack under `recover_below` organisation goes to rest ([`recover_province`]) and does
/// nothing else until it is back at `recover_until`; nobody rests while `recover_homeland_lost` of the
/// homeland is occupied. Besieging stacks stay in their siege.
pub fn plan_recovery(r: &AiRules, w: &AiWorld, areas: &AreaIndex, tag: &str) -> Recovery {
    let mut out = Recovery::default();
    let overrun = homeland_lost(w, areas, tag) >= r.recover_homeland_lost;
    for a in w.armies.iter().filter(|a| a.owner == tag) {
        if a.resting {
            if a.org >= r.recover_until || overrun {
                out.done.push(a.id);
            }
        } else if !a.busy && !a.sieging && !overrun && a.org < r.recover_below && let Some(p) = recover_province(r, w, areas, a) {
            out.go.push((a.id, p));
        }
    }
    out
}

/// Regroup: a standing stack that is under `regroup_share` of the regiments gathering in its province
/// (itself and its country's armies marching there) waits for them.
/// NOT WIRED: the port has no plans with a leader and followers, so "nearby" is not the plan's armies
/// in the area, and the step of a leader towards its followers' paths is left out.
pub fn regroup_wait(r: &AiRules, w: &AiWorld, a: &AiArmy) -> bool {
    if a.next.is_some() || w.map.neighbours(a.location).count() > r.regroup_max_neighbours {
        return false;
    }
    let coming: usize = w.armies.iter().filter(|u| u.id != a.id && u.owner == a.owner && !u.retreating && u.next.is_some() && u.destination == Some(a.location)).map(|u| u.regiments).sum();
    coming > 0 && (a.regiments as f64) / ((a.regiments + coming).max(1) as f64) < r.regroup_share
}

/// Merges of the armies of `tag`, `(keeper, merged)`: armies standing in one province merge on arrival.
/// Not while a battle runs there, not on rebel-held land, and (the port's stand-in for the room of a
/// front) only while the supply limit of the province carries them together. The keeper is a besieging
/// army if there is one, else the largest (ties: the lowest id).
pub fn plan_merges(r: &AiRules, w: &AiWorld, tag: &str) -> Vec<(u64, u64)> {
    let mut at: BTreeMap<ProvinceId, Vec<&AiArmy>> = BTreeMap::new();
    for a in w.armies.iter().filter(|a| a.owner == tag && !a.busy && a.next.is_none()) {
        at.entry(a.location).or_default().push(a);
    }
    let mut out = Vec::new();
    for (p, mut list) in at {
        let rebel_held = w.provinces.get(&p).and_then(|q| q.controller.as_deref()) == Some(r.rebel_tag.as_str());
        if list.len() < 2 || rebel_held || w.armies.iter().any(|u| u.fighting && u.location == p) {
            continue;
        }
        list.sort_by_key(|a| (!a.sieging, std::cmp::Reverse(a.regiments), a.id));
        let limit = (w.supply)(tag, p) as f64;
        let mut weight = list[0].strength;
        for b in &list[1..] {
            if weight + b.strength <= limit {
                weight += b.strength;
                out.push((list[0].id, b.id));
            }
        }
    }
    out
}

/// Regiments `tag` needs for the area of `entry`: the hostile regiments in it, or those of its war
/// enemies in and around it less twice what already stands next to it against the holder; 1 or 2
/// count as `needed_min`, an empty area of a hostile owner as `needed_empty`.
/// NOT WIRED: the port has no king that hands areas to fronts; nothing asks for this yet.
pub fn needed_regiments(r: &AiRules, w: &AiWorld, areas: &AreaIndex, tag: &str, entry: ProvinceId) -> usize {
    let Some(area) = areas.of(entry) else { return 0 };
    let inside: BTreeSet<ProvinceId> = areas.list[area].provinces.iter().copied().collect();
    let holder = w.provinces.get(&entry).and_then(|p| p.controller.clone());
    let mut at: BTreeMap<ProvinceId, Vec<&AiArmy>> = BTreeMap::new();
    for u in w.armies {
        at.entry(u.location).or_default().push(u);
    }
    let units = |p: &ProvinceId| at.get(p).into_iter().flatten();
    let (mut host, mut war, mut support) = (0.0, 0.0, 0.0);
    for u in inside.iter().flat_map(units).filter(|u| w.sides.hostile(tag, &u.owner)) {
        host += u.regiments as f64;
        if w.sides.at_war_with(tag, &u.owner) {
            war += u.regiments as f64;
        }
    }
    for n in areas.neighbours(area) {
        let Some(first) = areas.list[n].provinces.first() else { continue };
        if areas.list[n].controller.is_none() || !passable(w, tag, *first) {
            continue;
        }
        for q in &areas.list[n].provinces {
            let mut f = 0.0;
            for u in units(q) {
                if u.owner == tag || holder.as_deref().is_some_and(|h| w.sides.hostile(h, &u.owner)) {
                    f += u.regiments as f64;
                } else if w.sides.at_war_with(tag, &u.owner) {
                    war += u.regiments as f64 * r.needed_others_share;
                }
            }
            if f > 0.0 && w.map.neighbours(*q).any(|x| inside.contains(&x)) {
                support += r.needed_support * f;
            }
        }
    }
    let needed = host.max(war - support).round().max(0.0);
    let owner_hostile = w.provinces.get(&entry).and_then(|p| p.owner.as_deref()).is_some_and(|o| w.sides.hostile(tag, o));
    if needed >= 1.0 {
        needed.max(r.needed_min) as usize
    } else if owner_hostile {
        r.needed_empty as usize
    } else {
        0
    }
}

/// An area is a front when a neighbouring area is held by a country hostile to its holder.
pub fn is_front(w: &AiWorld, all: &[Area], area: &Area) -> bool {
    let Some(holder) = area.controller.as_deref() else { return false };
    let set: BTreeSet<ProvinceId> = area.provinces.iter().copied().collect();
    area.provinces.iter().any(|&p| {
        w.map.neighbours(p).any(|n| {
            !set.contains(&n) && all.iter().any(|o| o.provinces.contains(&n) && o.controller.as_deref().is_some_and(|c| w.sides.hostile(holder, c)))
        })
    })
}

/// Value of an enemy area as a target: size (capped) + capital + war-goal provinces.
pub fn area_value(r: &AiRules, w: &AiWorld, area: &Area) -> f64 {
    let mut v = (area.provinces.len() as f64).min(r.area_size_cap);
    for id in &area.provinces {
        if let Some(p) = w.provinces.get(id) {
            if p.capital {
                v += r.area_capital;
            }
            if p.war_goal {
                v += r.area_goal;
            }
        }
    }
    v
}

/// Mobilise decisions per country: at war, mobilise when its side's land strength is under
/// `mobilise_ratio` times the enemy side's (own side = itself and allies in its wars); at peace, demobilise.
pub fn mobilise_decisions(r: &AiRules, sides: &Sides, land_strength: &BTreeMap<String, f64>) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for tag in land_strength.keys() {
        if !sides.at_war(tag) {
            out.push((tag.clone(), false));
            continue;
        }
        let sum = |pred: &dyn Fn(&str) -> bool| land_strength.iter().filter(|(t, _)| pred(t)).map(|(_, s)| *s).sum::<f64>();
        let own = sum(&|t| sides.friendly(tag, t));
        let enemy = sum(&|t| sides.hostile(tag, t));
        out.push((tag.clone(), own < r.mobilise_ratio * enemy));
    }
    out
}

/// Admiral basics: blockades only with a 3 to 1 superiority in warship count, and none against a fleet
/// of more than the maximum. Existing blockades continue while the enemy has under half of our count.
pub fn blockade_allowed(r: &AiRules, own_warships: usize, enemy_warships: usize, biggest_enemy_fleet: usize, existing: bool) -> bool {
    let (own, enemy) = (own_warships as f64, enemy_warships as f64);
    if own / r.blockade_ratio > enemy && biggest_enemy_fleet <= r.blockade_max_fleet {
        return true;
    }
    existing && enemy < own / 2.0
}

/// A fleet as the navy AI sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiFleet {
    pub id: u64,
    pub owner: String,
    pub location: ProvinceId,
    /// Ships that are not transports.
    pub warships: usize,
    /// Free for an order: standing, not fighting or retreating, carrying no army, not called to carry one.
    pub idle: bool,
}

/// A port (a land province on the coast): the sea province off it and who controls it.
#[derive(Debug, Clone, PartialEq)]
pub struct AiPort {
    pub province: ProvinceId,
    pub sea: ProvinceId,
    pub controller: String,
}

/// Navy AI, kept simple: each country at war counts its warships against those of all the countries
/// it fights; while [`blockade_allowed`] lets it start blockades, each of its idle fleets with warships
/// sails to the nearest sea province off a port held by an enemy that no fleet of its side holds yet
/// (fleets by id, ties to the lower province). A fleet already off an enemy port stays there.
/// NOT WIRED: calling a blockade off when `blockade_allowed` no longer holds, escorts, naval battles sought.
/// `reach` gives the sailing distance from a sea or port province to every province a fleet can reach.
pub fn plan_blockades(r: &AiRules, sides: &Sides, tags: &[String], fleets: &[AiFleet], ports: &[AiPort], reach: &dyn Fn(ProvinceId) -> BTreeMap<ProvinceId, f64>) -> Vec<(u64, ProvinceId)> {
    let mut out = Vec::new();
    for tag in tags {
        let enemy_seas: BTreeSet<ProvinceId> = ports.iter().filter(|p| sides.at_war_with(tag, &p.controller)).map(|p| p.sea).collect();
        if enemy_seas.is_empty() {
            continue;
        }
        let own: usize = fleets.iter().filter(|f| f.owner == *tag).map(|f| f.warships).sum();
        let enemy: Vec<&AiFleet> = fleets.iter().filter(|f| sides.at_war_with(tag, &f.owner)).collect();
        let enemy_total: usize = enemy.iter().map(|f| f.warships).sum();
        let biggest = enemy.iter().map(|f| f.warships).max().unwrap_or(0);
        if !blockade_allowed(r, own, enemy_total, biggest, false) {
            continue;
        }
        let mut held: BTreeSet<ProvinceId> = fleets.iter().filter(|f| sides.friendly(tag, &f.owner) && f.warships > 0 && enemy_seas.contains(&f.location)).map(|f| f.location).collect();
        held.extend(out.iter().map(|(_, p): &(u64, ProvinceId)| *p));
        let mut mine: Vec<&AiFleet> = fleets.iter().filter(|f| f.owner == *tag && f.idle && f.warships > 0 && !enemy_seas.contains(&f.location)).collect();
        mine.sort_by_key(|f| f.id);
        for f in mine {
            if enemy_seas.iter().all(|s| held.contains(s)) {
                break;
            }
            let d = reach(f.location);
            let best = enemy_seas.iter().filter(|s| !held.contains(s)).filter_map(|s| Some((*d.get(s)?, *s))).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, sea)) = best {
                held.insert(sea);
                out.push((f.id, sea));
            }
        }
    }
    out
}
