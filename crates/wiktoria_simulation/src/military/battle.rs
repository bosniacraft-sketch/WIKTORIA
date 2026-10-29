//! Battles in progress: the `combat` block of a save and the rules of land and naval
//! battle.
//!
//! The rules are pure functions over the loaded `Combat` plus the fighters (regiments with the unit
//! statistics they need). Randomness is our own seeded `BattleRng` handed in by the caller (D9).
//! Casualties come back as a list for the caller to apply to the soldier pops. Constants are in
//! `BattleRules` (rules file `FLD_MIL_BAT_*`, hypotheses, digits marked "assumed" are placeholders).

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::map::ProvinceId;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::script::{Block, Value};

use super::movement::{border_closed, may_enter_land, may_take_order, BorderContext, MoveMap, MovementRules, Relations};
use super::siege::ProvinceMilitary;
use super::units::{Regiment, UnitDef};
use super::{id_of, num};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatKind {
    Land,
    Naval,
    Siege,
}

/// One side of a battle or siege.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CombatSide {
    /// The side's die, 0..9 (re-rolled every 5 days).
    pub dice: f64,
    /// Ids of the armies or fleets taking part (`unit`).
    pub units: Vec<u64>,
    pub losses: f64,
    /// Losses by unit class, in the order of the original's table of 24 entries.
    pub accumulated_losses: Vec<f64>,
    /// Men or ships it brought, by unit type name (battles only).
    pub forces: BTreeMap<String, f64>,
    /// Front and back line: slot number -> regiment id.
    pub front: BTreeMap<u32, u64>,
    pub back: BTreeMap<u32, u64>,
    /// The army that is retreating from this battle, if any.
    pub retreat: Option<u64>,
    /// Regiments waiting to deploy (`reserves`, in the save's order; regiments of the side's armies
    /// the save names nowhere are added when a save is taken up).
    pub reserve: Vec<u64>,
    /// Regiments that broke after the morale lock (counted in the morale mean, never fight again).
    pub broken: Vec<u64>,
    /// Set when the side was destroyed.
    pub wiped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combat {
    pub kind: CombatKind,
    pub location: u16,
    /// Day counter of the current dice (-1 before the first day).
    pub day: f64,
    /// Total days the combat has lasted.
    pub duration: f64,
    /// Sieges only: a running total the save keeps.
    pub total: f64,
    pub attacker: CombatSide,
    pub defender: CombatSide,
}

impl Combat {
    /// A new battle on the day the attacker arrives (counter -1: nothing rolled yet).
    pub fn new(kind: CombatKind, location: u16, attacker_armies: Vec<u64>, defender_armies: Vec<u64>) -> Combat {
        Combat {
            kind,
            location,
            day: -1.0,
            duration: 0.0,
            total: 0.0,
            attacker: CombatSide { units: attacker_armies, ..CombatSide::default() },
            defender: CombatSide { units: defender_armies, ..CombatSide::default() },
        }
    }

    pub fn side(&self, s: BattleSide) -> &CombatSide {
        match s {
            BattleSide::Attacker => &self.attacker,
            BattleSide::Defender => &self.defender,
        }
    }

    pub fn side_mut(&mut self, s: BattleSide) -> &mut CombatSide {
        match s {
            BattleSide::Attacker => &mut self.attacker,
            BattleSide::Defender => &mut self.defender,
        }
    }
}

fn slots(b: &Block, k: &str) -> BTreeMap<u32, u64> {
    b.get(k).and_then(Value::as_block).map(|s| s.entries().filter_map(|(n, v, _)| Some((n.parse().ok()?, id_of_value(v)?))).collect()).unwrap_or_default()
}

fn id_of_value(v: &Value) -> Option<u64> {
    v.as_block()?.get("id")?.as_f64().map(|x| x as u64)
}

fn read_side(b: Option<&Block>) -> CombatSide {
    let Some(b) = b else { return CombatSide::default() };
    let mut side = CombatSide {
        dice: num(b, "dice"),
        units: b.get_all("unit").filter_map(|v| v.as_block()?.get("id")?.as_f64().map(|x| x as u64)).collect(),
        losses: num(b, "losses"),
        accumulated_losses: b.get("accumulated_losses").and_then(Value::as_block).map(|l| l.bare_scalars().iter().filter_map(|s| s.parse().ok()).collect()).unwrap_or_default(),
        forces: BTreeMap::new(),
        front: slots(b, "front"),
        back: slots(b, "back"),
        retreat: id_of(b, "retreat"),
        reserve: b.get_all("reserves").filter_map(id_of_value).collect(),
        ..CombatSide::default()
    };
    for (k, v, _) in b.entries() {
        if let (false, Some(n)) = (matches!(k, "dice" | "losses"), v.as_f64()) {
            *side.forces.entry(k.to_string()).or_default() += n;
        }
    }
    side
}

/// Every `land_combat`, `naval_combat` and `siege_combat` inside the save's `combat` block.
pub fn read_combats(root: &Block) -> Vec<Combat> {
    let mut out = Vec::new();
    for c in root.get_all("combat").filter_map(Value::as_block) {
        for (k, v, _) in c.entries() {
            let kind = match k {
                "land_combat" => CombatKind::Land,
                "naval_combat" => CombatKind::Naval,
                "siege_combat" => CombatKind::Siege,
                _ => continue,
            };
            let Some(b) = v.as_block() else { continue };
            out.push(Combat {
                kind,
                location: num(b, "location") as u16,
                day: num(b, "day"),
                duration: num(b, "duration"),
                total: num(b, "total"),
                attacker: read_side(b.get("attacker").and_then(Value::as_block)),
                defender: read_side(b.get("defender").and_then(Value::as_block)),
            });
        }
    }
    out
}

// ================================================================== rules

/// Seeded generator of the battle rules (splitmix64), passed in by the caller (D9).
#[derive(Debug, Clone)]
pub struct BattleRng(pub u64);

impl BattleRng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 0.0 up to, not including, 1.0.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A whole number 0..n.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BattleSide {
    Attacker,
    Defender,
}

impl BattleSide {
    pub fn other(self) -> BattleSide {
        match self {
            BattleSide::Attacker => BattleSide::Defender,
            BattleSide::Defender => BattleSide::Attacker,
        }
    }
}

/// Engine constants of battle, from the rules file (`FLD_MIL_BAT_*`, hypotheses).
#[derive(Debug, Clone, PartialEq)]
pub struct BattleRules {
    pub dice_period_days: f64,
    pub dice_sides: u64,
    /// No morale end and no order out of a battle while total days are at most this.
    pub lock_days: f64,
    pub width_start: f64,
    pub base_width: f64,
    pub fit_strength: f64,
    pub fit_organisation: f64,
    pub damage_scale: f64,
    pub stat_step: f64,
    pub fort_step: f64,
    pub experience_step: f64,
    pub org_damage_mult: f64,
    pub recon_unit_ratio: f64,
    pub engineer_unit_ratio: f64,
    pub base_tactics: f64,
    pub pop_size_per_regiment: f64,
    pub soldier_to_pop_damage: f64,
    pub men_per_strength: f64,
    pub destroy_ratio: f64,
    pub overrun_first_day_ratio: f64,
    pub overrun_ratio: f64,
    pub score_men_divisor: f64,
    pub score_min: f64,
    pub score_divisor: f64,
    pub score_cap: f64,
    pub combatloss_war_exhaustion: f64,
    pub war_exhaustion_max: f64,
    pub prestige_land_gain: f64,
    pub prestige_naval_gain: f64,
    pub leader_prestige_share: f64,
    pub table: Vec<f64>,
    pub naval: NavalRules,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalRules {
    pub score_factor: f64,
    pub sink_value_big: f64,
    pub sink_value_light: f64,
    pub sink_value_transport: f64,
    pub seeking_chance: f64,
    pub seeking_chance_min: f64,
    pub stacking_target_select: f64,
    pub stacking_target_change: f64,
    pub change_target_chance: f64,
    pub shift_back: f64,
    pub shift_back_days: f64,
    pub max_targets: usize,
    pub damaged_target: f64,
    pub self_defence: f64,
    pub speed_to_distance: f64,
    pub retreat_speed_mod: f64,
    pub retreat_chance: f64,
    pub retreat_level: f64,
    pub retreat_min_distance: f64,
    pub damage_org_mult: f64,
    pub damage_str_mult: f64,
    pub damage_no_org: f64,
}

impl BattleRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<BattleRules, String> {
        let n = |id: &str| crate::num_field(m, id);
        let mut table = Vec::new();
        for i in 0..18 {
            table.push(n(&format!("FLD_MIL_BAT_TABLE_{i:02}"))?);
        }
        Ok(BattleRules {
            dice_period_days: n("FLD_MIL_BAT_DICE_PERIOD_DAYS")?,
            dice_sides: n("FLD_MIL_BAT_DICE_SIDES")? as u64,
            lock_days: n("FLD_MIL_BAT_LOCK_DAYS")?,
            width_start: n("FLD_MIL_BAT_WIDTH_START")?,
            base_width: n("FLD_MIL_BAT_BASE_WIDTH")?,
            fit_strength: n("FLD_MIL_BAT_FIT_STRENGTH")?,
            fit_organisation: n("FLD_MIL_BAT_FIT_ORGANISATION")?,
            damage_scale: n("FLD_MIL_BAT_DAMAGE_SCALE")?,
            stat_step: n("FLD_MIL_BAT_STAT_STEP")?,
            fort_step: n("FLD_MIL_BAT_FORT_STEP")?,
            experience_step: n("FLD_MIL_BAT_EXPERIENCE_STEP")?,
            org_damage_mult: n("FLD_MIL_BAT_ORG_DAMAGE_MULT")?,
            recon_unit_ratio: n("FLD_MIL_BAT_RECON_UNIT_RATIO")?,
            engineer_unit_ratio: n("FLD_MIL_BAT_ENGINEER_UNIT_RATIO")?,
            base_tactics: n("FLD_MIL_BAT_BASE_TACTICS")?,
            pop_size_per_regiment: n("FLD_MIL_BAT_POP_SIZE_PER_REGIMENT")?,
            soldier_to_pop_damage: n("FLD_MIL_BAT_SOLDIER_TO_POP_DAMAGE")?,
            men_per_strength: n("FLD_MIL_MEN_PER_STRENGTH")?,
            destroy_ratio: n("FLD_MIL_BAT_DESTROY_RATIO")?,
            overrun_first_day_ratio: n("FLD_MIL_BAT_OVERRUN_FIRST_DAY_RATIO")?,
            overrun_ratio: n("FLD_MIL_BAT_OVERRUN_RATIO")?,
            score_men_divisor: n("FLD_MIL_BAT_SCORE_MEN_DIVISOR")?,
            score_min: n("FLD_MIL_BAT_SCORE_MIN")?,
            score_divisor: n("FLD_MIL_BAT_SCORE_DIVISOR")?,
            score_cap: n("FLD_MIL_BAT_SCORE_CAP")?,
            combatloss_war_exhaustion: n("FLD_MIL_BAT_COMBATLOSS_WAR_EXHAUSTION")?,
            war_exhaustion_max: n("FLD_WAR_EXHAUSTION_MAX")?,
            prestige_land_gain: n("FLD_MIL_BAT_PRESTIGE_LAND_GAIN")?,
            prestige_naval_gain: n("FLD_MIL_BAT_PRESTIGE_NAVAL_GAIN")?,
            leader_prestige_share: n("FLD_MIL_BAT_LEADER_PRESTIGE_SHARE")?,
            table,
            naval: NavalRules {
                score_factor: n("FLD_MIL_BAT_NAVAL_SCORE_FACTOR")?,
                sink_value_big: n("FLD_MIL_BAT_NAVAL_SINK_VALUE_BIG")?,
                sink_value_light: n("FLD_MIL_BAT_NAVAL_SINK_VALUE_LIGHT")?,
                sink_value_transport: n("FLD_MIL_BAT_NAVAL_SINK_VALUE_TRANSPORT")?,
                seeking_chance: n("FLD_MIL_BAT_NAVAL_SEEKING_CHANCE")?,
                seeking_chance_min: n("FLD_MIL_BAT_NAVAL_SEEKING_CHANCE_MIN")?,
                stacking_target_select: n("FLD_MIL_BAT_NAVAL_STACKING_TARGET_SELECT")?,
                stacking_target_change: n("FLD_MIL_BAT_NAVAL_STACKING_TARGET_CHANGE")?,
                change_target_chance: n("FLD_MIL_BAT_NAVAL_CHANGE_TARGET_CHANCE")?,
                shift_back: n("FLD_MIL_BAT_NAVAL_SHIFT_BACK")?,
                shift_back_days: n("FLD_MIL_BAT_NAVAL_SHIFT_BACK_DAYS")?,
                max_targets: n("FLD_MIL_BAT_NAVAL_MAX_TARGETS")? as usize,
                damaged_target: n("FLD_MIL_BAT_NAVAL_DAMAGED_TARGET")?,
                self_defence: n("FLD_MIL_BAT_NAVAL_SELF_DEFENCE")?,
                speed_to_distance: n("FLD_MIL_BAT_NAVAL_SPEED_TO_DISTANCE")?,
                retreat_speed_mod: n("FLD_MIL_BAT_NAVAL_RETREAT_SPEED_MOD")?,
                retreat_chance: n("FLD_MIL_BAT_NAVAL_RETREAT_CHANCE")?,
                retreat_level: n("FLD_MIL_BAT_NAVAL_RETREAT_LEVEL")?,
                retreat_min_distance: n("FLD_MIL_BAT_NAVAL_RETREAT_MIN_DISTANCE")?,
                damage_org_mult: n("FLD_MIL_BAT_NAVAL_DAMAGE_ORG_MULT")?,
                damage_str_mult: n("FLD_MIL_BAT_NAVAL_DAMAGE_STR_MULT")?,
                damage_no_org: n("FLD_MIL_BAT_NAVAL_DAMAGE_NO_ORG")?,
            },
        })
    }

    /// Damage factor for die + modifiers (the table index is that + 2, clamped).
    pub fn table_factor(&self, die_plus_mods: f64) -> f64 {
        let i = (die_plus_mods.floor() as i64 + 2).clamp(0, self.table.len() as i64 - 1);
        self.table[i as usize]
    }
}

// ---------------------------------------------------------------- lines

/// Front and second line have 31 slots, numbered 1 to 31, the centre is 16.
pub const CENTRE: u32 = 16;
pub const LINE_SLOTS: usize = 31;

/// Slots from the centre outwards: 16, 15, 17, 14, 18 ...
pub fn slot_order() -> Vec<u32> {
    let mut v = vec![CENTRE];
    for d in 1..=15 {
        v.push(CENTRE - d);
        v.push(CENTRE + d);
    }
    v
}

fn inside(width: usize) -> Vec<u32> {
    slot_order().into_iter().take(width.min(LINE_SLOTS)).collect()
}

/// A regiment in a battle, with the unit statistics the rules read (from the install's unit type).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fighter {
    pub id: u64,
    /// The army it belongs to.
    pub army: u64,
    pub country: String,
    pub attack: f64,
    pub defence: f64,
    pub discipline: f64,
    pub support: f64,
    pub maneuver: f64,
    pub recon: f64,
    pub siege: f64,
    pub max_strength: f64,
    pub strength: f64,
    pub organisation: f64,
    pub experience: f64,
    pub pop: Option<u64>,
    /// The army leader's reconnaissance trait.
    pub leader_recon: f64,
    /// The army's dig-in level.
    pub dig_in: f64,
    pub pending_strength: f64,
    pub pending_organisation: f64,
}

impl Fighter {
    pub fn from_regiment(r: &Regiment, def: &UnitDef, army: u64, country: &str, dig_in: f64, leader_recon: f64) -> Fighter {
        Fighter {
            id: r.id,
            army,
            country: country.to_string(),
            attack: def.attack,
            defence: def.defence,
            discipline: def.discipline,
            support: def.support,
            maneuver: def.maneuver,
            recon: def.reconnaissance,
            siege: def.siege,
            max_strength: def.max_strength,
            strength: r.strength,
            organisation: r.organisation,
            experience: r.experience,
            pop: r.pop,
            leader_recon,
            dig_in,
            ..Fighter::default()
        }
    }

    fn fit(&self, r: &BattleRules) -> bool {
        self.strength >= r.fit_strength && self.organisation >= r.fit_organisation
    }
}

/// What a country on a side adds (modifiers of the summed country modifier block).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CountryInfo {
    pub tag: String,
    pub width_modifier: f64,
    /// Added to BASE_MILITARY_TACTICS (target's country) when dividing strength damage.
    pub tactics_modifier: f64,
    /// Divides pop casualties (1 + this).
    pub casualty_modifier: f64,
    pub exhaustion_modifier: f64,
    pub prestige_modifier: f64,
    /// The country may use gas, and is protected against it (invention effects).
    pub gas_attack: bool,
    pub gas_defence: bool,
    /// N of the exhaustion formula (minimum 1).
    pub exhaustion_count: f64,
    /// The country figure the score divides by (plus 1); not understood in the source, 0 is neutral.
    pub score_scale: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SideInfo {
    pub countries: Vec<CountryInfo>,
    pub leader: Option<u64>,
    /// Leader traits: attack (attacker) and defence (defender), whole part is used.
    pub leader_attack: f64,
    pub leader_defence: f64,
    /// The fort of the province reduces this side's damage (controller is not its country and they
    /// are at war, or rebels are involved).
    pub fort_applies: bool,
    /// The side is rebels (no prestige, no exhaustion against them).
    pub rebel: bool,
}

/// How an attacking army came in (3.1 "crossing").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Plain,
    River,
    Strait,
    /// No previous province and not rebels: landed from ships.
    Landing,
}

/// -2 for a strait or landing anywhere, else -1 for a river, else 0.
pub fn crossing_penalty(entries: &[Entry]) -> i32 {
    if entries.iter().any(|e| matches!(e, Entry::Strait | Entry::Landing)) {
        -2
    } else if entries.contains(&Entry::River) {
        -1
    } else {
        0
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LandContext {
    /// Terrain defence value of the province (penalty to the attacker).
    pub terrain_defence: f64,
    /// Terrain width value: width is multiplied by 1 + this.
    pub terrain_width: f64,
    /// Fort level of the province.
    pub fort: f64,
    /// 0, -1 or -2, from `crossing_penalty`.
    pub crossing: i32,
    /// Added to the dice of a side with gas attack against a side without gas defence
    /// (GAS_ATTACK_MODIFIER of the install's defines).
    pub gas_attack_modifier: f64,
    pub attacker: SideInfo,
    pub defender: SideInfo,
}

impl LandContext {
    fn info(&self, s: BattleSide) -> &SideInfo {
        match s {
            BattleSide::Attacker => &self.attacker,
            BattleSide::Defender => &self.defender,
        }
    }
}

/// Battle width (3.2): smallest country width, times terrain, at least 2.
pub fn width(r: &BattleRules, ctx: &LandContext) -> usize {
    let mut w = r.width_start;
    for c in ctx.attacker.countries.iter().chain(&ctx.defender.countries) {
        w = w.min((r.base_width + c.width_modifier).max(2.0));
    }
    ((w * (1.0 + ctx.terrain_width)).floor().max(2.0)) as usize
}

fn ids(s: &CombatSide) -> impl Iterator<Item = u64> + '_ {
    s.front.values().chain(s.back.values()).chain(&s.reserve).chain(&s.broken).copied()
}

fn side_strength(s: &CombatSide, f: &BTreeMap<u64, Fighter>) -> f64 {
    ids(s).filter_map(|i| f.get(&i)).map(|x| x.strength).sum()
}

/// Dice modifiers of one side, everything except the die (3.1).
pub fn side_modifier(r: &BattleRules, c: &Combat, fighters: &BTreeMap<u64, Fighter>, ctx: &LandContext, who: BattleSide) -> f64 {
    let info = ctx.info(who);
    // Gas: the first country of each side decides.
    let has = |s: BattleSide, f: fn(&CountryInfo) -> bool| ctx.info(s).countries.first().is_some_and(f);
    let gas = if has(who, |c| c.gas_attack) && !has(who.other(), |c| c.gas_defence) { ctx.gas_attack_modifier } else { 0.0 };
    match who {
        BattleSide::Defender => info.leader_defence.trunc() + gas,
        BattleSide::Attacker => {
            let mut m = info.leader_attack.trunc() + gas - ctx.terrain_defence.trunc() + f64::from(ctx.crossing);
            // Dig-in: lowest among the defenders, divided by the best reconnaissance.
            let lowest = ids(&c.defender).filter_map(|i| fighters.get(&i)).map(|x| x.dig_in).fold(f64::INFINITY, f64::min);
            if lowest.is_finite() {
                let atk: Vec<&Fighter> = ids(&c.attacker).filter_map(|i| fighters.get(&i)).collect();
                let total: f64 = atk.iter().map(|x| x.strength).sum();
                let mut recon = atk.iter().map(|x| x.recon * (1.0 + x.leader_recon)).fold(0.0, f64::max);
                let share = if total > 0.0 { atk.iter().filter(|x| x.recon > 0.0).map(|x| x.strength).sum::<f64>() / total } else { 0.0 };
                if share < r.recon_unit_ratio {
                    recon *= share / r.recon_unit_ratio;
                }
                let d = lowest / (1.0 + recon).floor();
                if d >= 1.0 {
                    m -= d.floor();
                }
            }
            m
        }
    }
}

// ---------------------------------------------------------------- deployment

fn pick_reserve(side: &mut CombatSide, f: &BTreeMap<u64, Fighter>, r: &BattleRules, only_non_support: bool) -> Option<u64> {
    let at = side.reserve.iter().position(|id| f.get(id).is_some_and(|x| x.fit(r) && (!only_non_support || x.support < 1.0)))?;
    Some(side.reserve.remove(at))
}

/// Reserve to second line (3.2), three passes.
pub fn deploy(r: &BattleRules, side: &mut CombatSide, enemy_front: &BTreeMap<u32, u64>, w: usize, f: &BTreeMap<u64, Fighter>) {
    let order = inside(w);
    for &p in &order {
        if !side.front.contains_key(&p) && !side.back.contains_key(&p) && enemy_front.contains_key(&p) && let Some(id) = pick_reserve(side, f, r, true) {
            side.back.insert(p, id);
        }
    }
    for &p in &order {
        if !side.front.contains_key(&p) && !side.back.contains_key(&p) && let Some(id) = pick_reserve(side, f, r, true) {
            side.back.insert(p, id);
        }
    }
    for p in slot_order().into_iter().take(30) {
        if !side.back.contains_key(&p) && let Some(id) = pick_reserve(side, f, r, false) {
            side.back.insert(p, id);
        }
    }
}

/// Second line to front (3.2): into the empty slot ahead, else a non-support regiment slides to a
/// free neighbouring front slot.
fn advance(side: &mut CombatSide, w: usize, f: &BTreeMap<u64, Fighter>) {
    let order = inside(w);
    let allowed: BTreeSet<u32> = order.iter().copied().collect();
    for &p in &slot_order() {
        let Some(&id) = side.back.get(&p) else { continue };
        if !side.front.contains_key(&p) {
            side.back.remove(&p);
            side.front.insert(p, id);
        } else if f.get(&id).is_some_and(|x| x.support < 1.0) {
            for n in [p.wrapping_sub(1), p + 1] {
                if allowed.contains(&n) && !side.front.contains_key(&n) {
                    side.back.remove(&p);
                    side.front.insert(n, id);
                    break;
                }
            }
        }
    }
}

fn close_gaps(side: &mut CombatSide) {
    for d in 1..=15u32 {
        for (from, to) in [(CENTRE - d, CENTRE - d + 1), (CENTRE + d, CENTRE + d - 1)] {
            if !side.front.contains_key(&to) && let Some(id) = side.front.remove(&from) {
                side.front.insert(to, id);
            }
        }
    }
}

/// Target of a regiment at `p` with reach `m` (3.2): front slot, front within reach, second line
/// slot, second line within reach. No random choice.
fn find_target(r: &BattleRules, enemy: &CombatSide, f: &BTreeMap<u64, Fighter>, p: u32, m: u32) -> Option<u64> {
    let fit = |id: u64| f.get(&id).is_some_and(|x| x.fit(r));
    let range = |line: &BTreeMap<u32, u64>| {
        let lo = p.saturating_sub(m);
        line.range(lo..=p + m).map(|(_, &id)| id).find(|&id| fit(id))
    };
    if let Some(&id) = enemy.front.get(&p) && fit(id) {
        return Some(id);
    }
    if let Some(id) = range(&enemy.front) {
        return Some(id);
    }
    if let Some(&id) = enemy.back.get(&p) && fit(id) {
        return Some(id);
    }
    range(&enemy.back)
}

/// Daily housekeeping before a side strikes: unfit regiments leave the front, advance,
/// deploy again, close gaps.
fn tidy(r: &BattleRules, c: &mut Combat, who: BattleSide, f: &BTreeMap<u64, Fighter>, w: usize) {
    let enemy = c.side(who.other()).clone();
    let locked = c.duration > r.lock_days;
    let side = c.side_mut(who);
    let mut sent = false;
    for p in slot_order() {
        let Some(&id) = side.front.get(&p) else { continue };
        let Some(x) = f.get(&id) else { continue };
        if !x.fit(r) {
            if locked {
                side.front.remove(&p);
                side.broken.push(id);
            } else if !sent {
                side.front.remove(&p);
                side.reserve.push(id);
                sent = true;
            }
        } else if find_target(r, &enemy, f, p, x.maneuver.floor().max(0.0) as u32).is_none() {
            side.front.remove(&p);
            side.reserve.push(id);
        }
    }
    advance(side, w, f);
    deploy(r, side, &enemy.front, w, f);
    close_gaps(side);
}

fn refill(r: &BattleRules, c: &mut Combat, who: BattleSide, f: &BTreeMap<u64, Fighter>, w: usize) {
    let enemy_front = c.side(who.other()).front.clone();
    let side = c.side_mut(who);
    advance(side, w, f);
    deploy(r, side, &enemy_front, w, f);
    advance(side, w, f);
    close_gaps(side);
}

// ---------------------------------------------------------------- damage

/// Siege value of an army (highest siege of its regiments), scaled down when engineers are a small
/// share of its strength (3.3 step 4).
fn army_siege(r: &BattleRules, fighters: &BTreeMap<u64, Fighter>, army: u64) -> f64 {
    let regs: Vec<&Fighter> = fighters.values().filter(|x| x.army == army).collect();
    let total: f64 = regs.iter().map(|x| x.strength).sum();
    let mut v = regs.iter().map(|x| x.siege).fold(0.0, f64::max);
    let share = if total > 0.0 { regs.iter().filter(|x| x.siege > 0.0).map(|x| x.strength).sum::<f64>() / total } else { 0.0 };
    if share < r.engineer_unit_ratio {
        v *= share / r.engineer_unit_ratio;
    }
    v
}

/// Strength and organisation damage one regiment deals to another (3.3).
#[allow(clippy::too_many_arguments)]
pub fn regiment_damage(r: &BattleRules, die_plus_mods: f64, attacker_side: bool, multiplier: f64, shooter: &Fighter, target: &Fighter, fort_level: f64, target_tactics_modifier: f64) -> (f64, f64) {
    let stat = if attacker_side { shooter.attack } else { shooter.defence };
    let mut x = r.table_factor(die_plus_mods) * multiplier * (1.0 + r.stat_step * stat) * shooter.strength;
    x /= 1.0 + r.fort_step * fort_level;
    x /= 1.0 + r.experience_step * target.experience;
    let org = if target.discipline > 0.0 { x * r.org_damage_mult / target.discipline } else { x * r.org_damage_mult };
    // Tactics divide only for a target whose country has some: a country with none takes the whole.
    let strength = if target_tactics_modifier > 0.0 { x / (r.base_tactics + target_tactics_modifier) } else { x };
    (strength * r.damage_scale, org * r.damage_scale)
}

fn tactics_of(ctx: &LandContext, country: &str) -> f64 {
    ctx.attacker.countries.iter().chain(&ctx.defender.countries).find(|c| c.tag == country).map_or(0.0, |c| c.tactics_modifier)
}

fn strike(r: &BattleRules, c: &Combat, who: BattleSide, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, w: usize, die_mod: f64) {
    let mine = c.side(who);
    let enemy = c.side(who.other());
    let allowed: BTreeSet<u32> = inside(w).into_iter().collect();
    let info = ctx.info(who);
    let mut shots: Vec<(u64, u64, f64, f64)> = Vec::new();
    let mut siege_cache: BTreeMap<u64, f64> = BTreeMap::new();
    for (line, is_front) in [(&mine.front, true), (&mine.back, false)] {
        for (&p, &id) in line {
            if !allowed.contains(&p) {
                continue;
            }
            let Some(x) = fighters.get(&id) else { continue };
            if !x.fit(r) || (!is_front && x.support <= 0.0) {
                continue;
            }
            let mult = if is_front { 1.0 } else { x.support };
            let Some(t) = find_target(r, enemy, fighters, p, x.maneuver.floor().max(0.0) as u32) else { continue };
            let fort = if info.fort_applies {
                let s = *siege_cache.entry(x.army).or_insert_with(|| army_siege(r, fighters, x.army));
                (ctx.fort - s).max(0.0).floor()
            } else {
                0.0
            };
            shots.push((id, t, mult, fort));
        }
    }
    for (id, t, multiplier, fort) in shots {
        let (sd, od) = {
            let (a, b) = (&fighters[&id], &fighters[&t]);
            regiment_damage(r, die_mod, who == BattleSide::Attacker, multiplier, a, b, fort, tactics_of(ctx, &b.country))
        };
        let target = fighters.get_mut(&t).expect("target exists");
        target.pending_strength += sd;
        target.pending_organisation += od;
    }
}

/// One person-count of a pop to take away.
#[derive(Debug, Clone, PartialEq)]
pub struct Casualty {
    pub pop: u64,
    pub men: f64,
}

/// Applies pending damage of one side, returns (strength lost, casualties by pop, by-army losses).
fn apply_pending(r: &BattleRules, s: &CombatSide, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, casualties: &mut BTreeMap<u64, f64>) -> f64 {
    let mut lost = 0.0;
    let list: Vec<u64> = ids(s).collect();
    for id in list {
        let Some(x) = fighters.get_mut(&id) else { continue };
        let dstr = x.pending_strength.min(x.strength);
        x.strength -= dstr;
        x.organisation = (x.organisation - x.pending_organisation).max(0.0);
        x.pending_strength = 0.0;
        x.pending_organisation = 0.0;
        lost += dstr;
        if dstr > 0.0 && x.pop.is_some() && x.max_strength > 0.0 {
            let cas = ctx.attacker.countries.iter().chain(&ctx.defender.countries).find(|c| c.tag == x.country).map_or(0.0, |c| c.casualty_modifier);
            let men = dstr / x.max_strength * r.pop_size_per_regiment * r.soldier_to_pop_damage / (1.0 + cas);
            *casualties.entry(x.pop.unwrap_or_default()).or_default() += men;
        }
    }
    lost
}

// ---------------------------------------------------------------- end of a battle

/// What a finished battle gives (3.6), for the caller to apply.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleResult {
    pub score: f64,
    /// Prestige change per country (winner positive, loser the same negative; the caller floors at 0).
    pub prestige: Vec<(String, f64)>,
    /// Prestige of the leaders (score / 100, negative for the loser).
    pub leader_prestige: Vec<(u64, f64)>,
    /// War exhaustion added per country (land only).
    pub exhaustion: Vec<(String, f64)>,
    pub record: BattleRecord,
}

/// The battle record the war keeps for its score.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleRecord {
    pub location: u16,
    pub naval: bool,
    pub attacker_won: bool,
    /// Men lost (land) or value of ships lost (naval) per side.
    pub attacker_losses: f64,
    pub defender_losses: f64,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub winner: BattleSide,
    /// The loser was wiped out at once (under the destroy ratio, overrun, or nobody left).
    pub loser_destroyed: bool,
    /// Loser armies that must leave: the caller picks `retreat_target`, none means deleted.
    pub retreating: Vec<u64>,
    /// Armies deleted by the battle.
    pub destroyed: Vec<u64>,
    pub result: BattleResult,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LandReport {
    pub dice: [f64; 2],
    /// What was added to each die (attacker, defender): leader, terrain, crossing, dig-in.
    pub modifiers: [f64; 2],
    pub rerolled: bool,
    /// The defender's die came out lower: every defending unit loses 1 dig-in (already done on the
    /// fighters, the caller applies it to the armies).
    pub dig_in_loss: bool,
    /// Men to take from soldier pops (pop id, men), merged per pop.
    pub casualties: Vec<Casualty>,
    pub outcome: Option<Outcome>,
}

fn score_term(r: &BattleRules, loss_men: f64, scale: f64) -> f64 {
    (loss_men / (r.score_men_divisor * (scale + 1.0))).max(r.score_min)
}

#[allow(clippy::too_many_arguments)]
fn make_result(r: &BattleRules, c: &Combat, ctx: &LandContext, winner: BattleSide, naval: bool, a_loss: f64, d_loss: f64, a_value: f64, d_value: f64) -> BattleResult {
    let scale = |s: &SideInfo| s.countries.first().map_or(0.0, |x| x.score_scale);
    let (a, d) = if naval {
        (score_term(r, a_value * r.naval.score_factor * r.score_men_divisor, scale(&ctx.attacker)), score_term(r, d_value * r.naval.score_factor * r.score_men_divisor, scale(&ctx.defender)))
    } else {
        (score_term(r, a_loss, scale(&ctx.attacker)), score_term(r, d_loss, scale(&ctx.defender)))
    };
    let score = ((a + d) / r.score_divisor).min(r.score_cap);
    let gain = if naval { r.prestige_naval_gain } else { r.prestige_land_gain };
    let mut res = BattleResult { score, ..BattleResult::default() };
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        let info = ctx.info(who);
        let other_rebel = ctx.info(who.other()).rebel;
        let sign = if who == winner { 1.0 } else { -1.0 };
        if !other_rebel {
            for cn in &info.countries {
                let mut p = score * (1.0 + gain);
                if cn.prestige_modifier > 0.0 {
                    p *= 1.0 + cn.prestige_modifier;
                }
                res.prestige.push((cn.tag.clone(), sign * p));
            }
            if let Some(l) = info.leader {
                res.leader_prestige.push((l, sign * score * r.leader_prestige_share));
            }
        }
        // War exhaustion from own losses, land only, not against rebels.
        if !naval && !other_rebel {
            let own = if who == BattleSide::Attacker { a_loss } else { d_loss };
            let mut q = r.combatloss_war_exhaustion * (own / r.men_per_strength * r.men_per_strength / r.pop_size_per_regiment);
            for cn in &info.countries {
                q /= cn.exhaustion_count.max(1.0);
                res.exhaustion.push((cn.tag.clone(), (q * (1.0 + cn.exhaustion_modifier)).clamp(0.0, r.war_exhaustion_max)));
            }
        }
    }
    res.record = BattleRecord { location: c.location, naval, attacker_won: winner == BattleSide::Attacker, attacker_losses: if naval { a_value } else { a_loss }, defender_losses: if naval { d_value } else { d_loss }, score };
    res
}

fn lost_land(r: &BattleRules, s: &CombatSide, f: &BTreeMap<u64, Fighter>, locked: bool) -> bool {
    if s.wiped || side_strength(s, f) < 0.01 {
        return true;
    }
    if !locked {
        return false;
    }
    if s.front.is_empty() {
        return true;
    }
    let orgs: Vec<f64> = s.front.values().chain(&s.broken).filter_map(|i| f.get(i)).map(|x| x.organisation).collect();
    !orgs.is_empty() && orgs.iter().sum::<f64>() / orgs.len() as f64 <= r.fit_organisation
}

/// Destroys a side: all its strength becomes losses and pop damage.
fn destroy_side(r: &BattleRules, c: &mut Combat, who: BattleSide, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, casualties: &mut BTreeMap<u64, f64>) -> f64 {
    let snapshot = c.side(who).clone();
    for id in ids(&snapshot).collect::<Vec<_>>() {
        if let Some(x) = fighters.get_mut(&id) {
            x.pending_strength = x.strength;
        }
    }
    let lost = apply_pending(r, &snapshot, fighters, ctx, casualties);
    let s = c.side_mut(who);
    s.wiped = true;
    s.losses += lost * r.men_per_strength;
    lost * r.men_per_strength
}

fn finish_land(r: &BattleRules, c: &mut Combat, ctx: &LandContext, winner: BattleSide, destroyed: bool, a_loss: f64, d_loss: f64) -> Outcome {
    let loser = c.side(winner.other()).units.clone();
    let result = make_result(r, c, ctx, winner, false, a_loss, d_loss, 0.0, 0.0);
    Outcome { winner, loser_destroyed: destroyed, retreating: if destroyed { Vec::new() } else { loser.clone() }, destroyed: if destroyed { loser } else { Vec::new() }, result }
}

/// Overrun test (3.5): a side with no front regiment counts as zero on the first day or when
/// below `1/ratio` of the enemy; exactly one zero side is destroyed.
fn overrun(r: &BattleRules, c: &Combat, f: &BTreeMap<u64, Fighter>, first_day: bool, ratio: f64) -> Option<BattleSide> {
    let zero = |me: &CombatSide, other: &CombatSide| {
        me.front.is_empty() && (first_day || side_strength(me, f) * ratio < side_strength(other, f)) && (first_day || {
            let orgs: Vec<f64> = me.front.values().filter_map(|i| f.get(i)).map(|x| x.organisation).collect();
            orgs.is_empty() || (orgs.iter().sum::<f64>() / orgs.len() as f64) < r.fit_organisation
        })
    };
    match (zero(&c.attacker, &c.defender), zero(&c.defender, &c.attacker)) {
        (true, false) => Some(BattleSide::Attacker),
        (false, true) => Some(BattleSide::Defender),
        _ => None,
    }
}

fn merge(m: BTreeMap<u64, f64>) -> Vec<Casualty> {
    m.into_iter().filter(|(_, v)| *v > 0.0).map(|(pop, men)| Casualty { pop, men }).collect()
}

fn roll(r: &BattleRules, rng: &mut BattleRng) -> f64 {
    rng.below(r.dice_sides) as f64
}

/// The daily round of a land battle. `fighters` holds every regiment of both
/// sides by id; deploy newcomers with `join_battle` first.
pub fn battle_day(r: &BattleRules, c: &mut Combat, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, rng: &mut BattleRng) -> LandReport {
    land_round(r, c, fighters, ctx, &mut || roll(r, rng))
}

/// The same round with the dice handed in (attacker, defender) for the day the rules would roll, so
/// a day can be replayed against a save of the original without chance (`check-battle`).
pub fn battle_day_with_dice(r: &BattleRules, c: &mut Combat, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, dice: [f64; 2]) -> LandReport {
    let mut given = dice.into_iter();
    land_round(r, c, fighters, ctx, &mut || given.next().unwrap_or(0.0))
}

fn land_round(r: &BattleRules, c: &mut Combat, fighters: &mut BTreeMap<u64, Fighter>, ctx: &LandContext, die: &mut dyn FnMut() -> f64) -> LandReport {
    let mut rep = LandReport::default();
    if c.attacker.units.is_empty() || c.defender.units.is_empty() {
        let winner = if c.attacker.units.is_empty() { BattleSide::Defender } else { BattleSide::Attacker };
        let (a, d) = (c.attacker.losses, c.defender.losses);
        rep.outcome = Some(finish_land(r, c, ctx, winner, false, a, d));
        return rep;
    }
    c.duration += 1.0;
    let w = width(r, ctx);
    let first = c.day < 0.0;
    let mut cas: BTreeMap<u64, f64> = BTreeMap::new();
    if first {
        c.attacker.dice = die();
        c.defender.dice = die();
        let en = c.defender.front.clone();
        deploy(r, &mut c.attacker, &en, w, fighters);
        let en = c.attacker.front.clone();
        deploy(r, &mut c.defender, &en, w, fighters);
        c.day = 0.0;
        // Nothing stands in front on either side on the first day: the empty side is overrun.
        if let Some(victim) = overrun(r, c, fighters, true, r.overrun_first_day_ratio) {
            let winner = victim.other();
            let loss = destroy_side(r, c, victim, fighters, ctx, &mut cas);
            let (a, d) = if victim == BattleSide::Attacker { (loss, c.defender.losses) } else { (c.attacker.losses, loss) };
            rep.casualties = merge(cas);
            rep.outcome = Some(finish_land(r, c, ctx, winner, true, a, d));
            return rep;
        }
    }
    if c.day > r.dice_period_days - 1.0 {
        c.attacker.dice = die();
        c.defender.dice = die();
        c.day = 0.0;
        rep.rerolled = true;
        if c.duration > 2.0 && c.defender.dice < c.attacker.dice {
            rep.dig_in_loss = true;
            for id in ids(&c.defender).collect::<Vec<_>>() {
                if let Some(x) = fighters.get_mut(&id) {
                    x.dig_in = (x.dig_in - 1.0).max(0.0);
                }
            }
        }
    }
    rep.dice = [c.attacker.dice, c.defender.dice];
    let mods = [side_modifier(r, c, fighters, ctx, BattleSide::Attacker), side_modifier(r, c, fighters, ctx, BattleSide::Defender)];
    rep.modifiers = mods;
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        tidy(r, c, who, fighters, w);
    }
    // A side tidied against the enemy line as it stood; close up again once both have moved.
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        refill(r, c, who, fighters, w);
    }
    for (i, who) in [BattleSide::Attacker, BattleSide::Defender].into_iter().enumerate() {
        let die = c.side(who).dice + mods[i];
        strike(r, c, who, fighters, ctx, w, die);
    }
    // Everyone outside the width goes back to the reserve.
    let allowed: BTreeSet<u32> = inside(w).into_iter().collect();
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        let s = c.side_mut(who);
        let out: Vec<u32> = s.front.keys().chain(s.back.keys()).copied().filter(|p| !allowed.contains(p)).collect();
        for p in out {
            if let Some(id) = s.front.remove(&p) {
                s.reserve.push(id);
            }
            if let Some(id) = s.back.remove(&p) {
                s.reserve.push(id);
            }
        }
    }
    let sa = c.attacker.clone();
    let sd = c.defender.clone();
    let la = apply_pending(r, &sa, fighters, ctx, &mut cas) * r.men_per_strength;
    let ld = apply_pending(r, &sd, fighters, ctx, &mut cas) * r.men_per_strength;
    c.attacker.losses += la;
    c.defender.losses += ld;
    // End tests, attacker first: both losing counts as the attacker losing.
    let locked = c.duration > r.lock_days;
    let loser = if lost_land(r, &c.attacker, fighters, locked) {
        Some(BattleSide::Attacker)
    } else if lost_land(r, &c.defender, fighters, locked) {
        Some(BattleSide::Defender)
    } else {
        None
    };
    if let Some(l) = loser {
        let winner = l.other();
        let destroyed = side_strength(c.side(l), fighters) < r.destroy_ratio * side_strength(c.side(winner), fighters) || c.side(l).wiped || side_strength(c.side(l), fighters) < 0.01;
        if destroyed {
            let lost = destroy_side(r, c, l, fighters, ctx, &mut cas);
            let _ = lost;
        }
        let (a, d) = (c.attacker.losses, c.defender.losses);
        rep.outcome = Some(finish_land(r, c, ctx, winner, destroyed, a, d));
    } else {
        if c.duration <= r.lock_days
            && let Some(victim) = overrun(r, c, fighters, false, r.overrun_ratio)
        {
            let winner = victim.other();
            destroy_side(r, c, victim, fighters, ctx, &mut cas);
            let (a, d) = (c.attacker.losses, c.defender.losses);
            rep.outcome = Some(finish_land(r, c, ctx, winner, true, a, d));
        }
        c.day += 1.0;
    }
    rep.casualties = merge(cas);
    rep
}

// ---------------------------------------------------------------- join and leave

/// An army (or regiments of it) joining a battle in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinUnit {
    pub army: u64,
    pub regiments: Vec<u64>,
}

/// Units join a battle in progress ("dogpile"): the army is added to the side and its regiments go
/// to the reserve, taking front slots as they free up on the following days. Joining twice does
/// nothing.
pub fn join_battle(c: &mut Combat, side: BattleSide, units: &[JoinUnit]) {
    let s = c.side_mut(side);
    for u in units {
        if s.units.contains(&u.army) {
            continue;
        }
        s.units.push(u.army);
        for id in &u.regiments {
            if !ids(s).any(|x| x == *id) {
                s.reserve.push(*id);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeaveError {
    NotInBattle,
    /// Still inside the lock: the army can leave once total days exceed the movement lock.
    Locked { days_left: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Left {
    pub side: BattleSide,
    /// The enemy still has units: the leaver is flagged retreating.
    pub retreating: bool,
    pub regiments: Vec<u64>,
}

/// An army is ordered out of a land battle (D17): refused until the battle has lasted
/// more than movement's `order_battle_min_days` (7); afterwards its regiments leave every line and
/// the battle goes on without it. The battle's own morale lock (`lock_days`) is a separate rule.
pub fn leave_battle(mv: &MovementRules, c: &mut Combat, fighters: &BTreeMap<u64, Fighter>, army: u64) -> Result<Left, LeaveError> {
    let side = [BattleSide::Attacker, BattleSide::Defender].into_iter().find(|s| c.side(*s).units.contains(&army)).ok_or(LeaveError::NotInBattle)?;
    if c.kind == CombatKind::Land && !may_take_order(mv, true, c.duration) {
        return Err(LeaveError::Locked { days_left: (mv.order_battle_min_days + 1.0 - c.duration).max(1.0) });
    }
    let mine: BTreeSet<u64> = fighters.values().filter(|x| x.army == army).map(|x| x.id).collect();
    let retreating = !c.side(side.other()).units.is_empty();
    let s = c.side_mut(side);
    s.units.retain(|u| *u != army);
    s.front.retain(|_, id| !mine.contains(id));
    s.back.retain(|_, id| !mine.contains(id));
    s.reserve.retain(|id| !mine.contains(id));
    s.broken.retain(|id| !mine.contains(id));
    Ok(Left { side, retreating, regiments: mine.into_iter().collect() })
}

// ---------------------------------------------------------------- retreat province

/// A retreating army's facts.
#[derive(Debug, Clone, PartialEq)]
pub struct RetreatUnit {
    pub owner: String,
    pub from: ProvinceId,
    /// Mean organisation of its regiments (needed >= 1.0 to enter a hostile-controlled province).
    pub mean_organisation: f64,
    /// The army is on a fleet at sea.
    pub at_sea: bool,
    /// Rebels only retreat when their type allows it.
    pub rebel_can_retreat: bool,
}

/// What the world says about the neighbours of a retreat (filled by the day loop).
pub struct RetreatFacts<'a> {
    pub map: &'a MoveMap,
    pub border_ctx: &'a BorderContext,
    pub relations: &'a Relations,
    pub provinces: &'a BTreeMap<ProvinceId, ProvinceMilitary>,
    /// Provinces where a hostile army with regiments, not itself retreating, stands.
    pub hostile_armies: &'a BTreeSet<ProvinceId>,
    /// Provinces bordering land controlled by someone hostile to the province's controller.
    pub exposed: &'a BTreeSet<ProvinceId>,
    /// Sea provinces the army would fit into on its country's fleets (empty: no sea retreat).
    pub sea_room: &'a BTreeSet<ProvinceId>,
}

/// Where a beaten army retreats to, `None` meaning it is deleted (the "stackwipe").
/// Each neighbour that may be entered is scored (unowned 0.02, owned and controlled 4, controlled
/// only 3, controller an ally 2.5, owned only 2, owner an ally 1.5, else 1, sea 3), plus a random
/// whole 0..3, halved if it borders hostile land; the highest above 0.01 wins, ties go to the
/// first (lowest id).
pub fn retreat_target(u: &RetreatUnit, facts: &RetreatFacts, rng: &mut BattleRng) -> Option<ProvinceId> {
    let rel = facts.relations;
    let rebel = u.owner == rel.rebel_tag;
    if rebel && !u.rebel_can_retreat {
        return None;
    }
    let from_prov = facts.provinces.get(&u.from).cloned().unwrap_or_default();
    let mut best: Option<(f64, ProvinceId)> = None;
    for n in facts.map.neighbours(u.from) {
        let info = facts.map.provinces.get(&n).copied().unwrap_or_default();
        if border_closed(facts.map, facts.border_ctx, false, u.from, n) {
            continue;
        }
        let mut score;
        if info.sea {
            if rebel || u.at_sea || !facts.sea_room.contains(&n) {
                continue;
            }
            score = 3.0;
        } else {
            if u.at_sea && info.port_sea != Some(u.from) {
                continue;
            }
            let to = facts.provinces.get(&n).cloned().unwrap_or_default();
            if !may_enter_land(rel, &u.owner, false, &from_prov, &to) {
                continue;
            }
            if facts.hostile_armies.contains(&n) {
                continue;
            }
            let hostile_controller = to.controller.as_deref().is_some_and(|c| rel.hostile(&u.owner, c));
            if hostile_controller && u.mean_organisation < 1.0 {
                continue;
            }
            let ours = |t: &Option<String>| t.as_deref() == Some(u.owner.as_str());
            let ally = |t: &Option<String>| t.as_deref().is_some_and(|x| x != u.owner && rel.friendly(&u.owner, x));
            score = if to.owner.is_none() {
                0.02
            } else if ours(&to.owner) && ours(&to.controller) {
                4.0
            } else if ours(&to.controller) {
                3.0
            } else if ally(&to.controller) {
                2.5
            } else if ours(&to.owner) {
                2.0
            } else if ally(&to.owner) {
                1.5
            } else {
                1.0
            };
        }
        score += rng.below(4) as f64;
        if facts.exposed.contains(&n) {
            score /= 2.0;
        }
        if score > 0.01 && best.is_none_or(|(b, _)| score > b) {
            best = Some((score, n));
        }
    }
    best.map(|(_, p)| p)
}

// ================================================================== naval battle

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipClass {
    Big,
    Light,
    Transport,
}

/// Where a ship is in its little state machine (4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipState {
    Firing,
    Approaching,
    Seeking,
    Retreating,
    Gone,
    Escaped,
}

impl ShipState {
    fn active(self) -> bool {
        matches!(self, ShipState::Firing | ShipState::Approaching | ShipState::Seeking)
    }

    fn out(self) -> bool {
        matches!(self, ShipState::Gone | ShipState::Escaped)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalShip {
    pub id: u64,
    pub fleet: u64,
    pub side: BattleSide,
    pub class: ShipClass,
    pub strength: f64,
    pub max_strength: f64,
    pub organisation: f64,
    pub max_organisation: f64,
    pub experience: f64,
    pub speed: f64,
    pub fire_range: f64,
    pub evasion: f64,
    pub hull: f64,
    pub gun_power: f64,
    pub torpedo: f64,
    pub state: ShipState,
    pub target: Option<u64>,
    /// 0 close, 1.0 away.
    pub distance: f64,
    pub pending_strength: f64,
    pub pending_organisation: f64,
}

impl NavalShip {
    #[allow(clippy::too_many_arguments)]
    pub fn new(id: u64, fleet: u64, side: BattleSide, class: ShipClass, def: &UnitDef, strength: f64, organisation: f64, experience: f64) -> NavalShip {
        NavalShip {
            id,
            fleet,
            side,
            class,
            strength,
            max_strength: def.max_strength,
            organisation,
            max_organisation: def.organisation.max(organisation),
            experience,
            speed: def.speed,
            fire_range: def.fire_range,
            evasion: def.evasion,
            hull: def.hull,
            gun_power: def.gun_power,
            torpedo: 0.0,
            state: ShipState::Seeking,
            target: None,
            distance: 1.0,
            pending_strength: 0.0,
            pending_organisation: 0.0,
        }
    }

    fn health(&self) -> f64 {
        let s = if self.max_strength > 0.0 { self.strength / self.max_strength } else { 1.0 };
        let o = if self.max_organisation > 0.0 { self.organisation / self.max_organisation } else { 1.0 };
        (s + o) / 2.0
    }
}

/// Leader traits and country figures of one naval side.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NavalSideInfo {
    pub side: Option<SideInfo>,
    /// Leader traits: reconnaissance, experience gain, speed.
    pub leader_recon: f64,
    pub leader_experience: f64,
    pub leader_speed: f64,
}

/// Naval battle state besides the `Combat`: the ships, and whether a side was ordered to flee.
#[derive(Debug, Clone, PartialEq)]
pub struct NavalState {
    pub ships: Vec<NavalShip>,
    pub fleeing: [bool; 2],
    pub attacker: NavalSideInfo,
    pub defender: NavalSideInfo,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NavalReport {
    pub dice: [f64; 2],
    /// Ships sunk today (ship id).
    pub sunk: Vec<u64>,
    pub outcome: Option<Outcome>,
}

fn class_factor(own: ShipClass, target: ShipClass) -> f64 {
    use ShipClass::*;
    match (own, target) {
        (Big, Big) | (Big, Light) | (Light, Light) | (Transport, Transport) => 1.5,
        (Big, Transport) | (Light, Transport) => 0.5,
        (Light, Big) => 1.0,
        (Transport, _) => 0.1,
    }
}

fn idx(s: BattleSide) -> usize {
    if s == BattleSide::Attacker { 0 } else { 1 }
}

fn stacking(ships: &[NavalShip], me: BattleSide) -> f64 {
    let n = ships.iter().filter(|s| s.side == me && s.state.active()).count() as f64;
    let e = ships.iter().filter(|s| s.side != me && s.state.active()).count() as f64;
    if n > e && e > 0.0 { (1.0 - e / n).min(0.9) } else { 0.0 }
}

fn choose_target(r: &NavalRules, ships: &[NavalShip], me: usize, rng: &mut BattleRng) -> Option<u64> {
    let own = &ships[me];
    let mut weights: Vec<(u64, f64)> = Vec::new();
    for e in ships.iter().filter(|e| e.side != own.side) {
        if e.state.out() {
            continue;
        }
        let aimers = ships.iter().filter(|s| s.side == own.side && s.target == Some(e.id)).count();
        if aimers >= r.max_targets {
            continue;
        }
        let mut w = (101.0 - 100.0 * (own.distance + e.distance) / 2.0).max(1.0);
        w *= class_factor(own.class, e.class);
        w *= 1.0 + r.damaged_target * (1.0 - e.health());
        if e.target == Some(own.id) {
            w *= r.self_defence;
        }
        weights.push((e.id, w));
    }
    let total: f64 = weights.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return None;
    }
    let mut pick = rng.unit() * total;
    for (id, w) in &weights {
        if pick < *w {
            return Some(*id);
        }
        pick -= w;
    }
    weights.last().map(|(id, _)| *id)
}

fn sink_value(r: &NavalRules, c: ShipClass) -> f64 {
    match c {
        ShipClass::Big => r.sink_value_big,
        ShipClass::Light => r.sink_value_light,
        ShipClass::Transport => r.sink_value_transport,
    }
}

fn naval_lost(r: &NavalRules, st: &NavalState, who: BattleSide) -> bool {
    let mine: Vec<&NavalShip> = st.ships.iter().filter(|s| s.side == who).collect();
    if mine.iter().map(|s| s.strength).sum::<f64>() < 0.01 || mine.iter().all(|s| s.state.out()) {
        return true;
    }
    let active: Vec<&&NavalShip> = mine.iter().filter(|s| s.state.active()).collect();
    if active.is_empty() {
        return true;
    }
    let dist = active.iter().map(|s| s.distance).sum::<f64>() / active.len() as f64;
    let org = active.iter().map(|s| s.organisation).sum::<f64>() / active.len() as f64;
    dist <= r.retreat_min_distance && org < 1.0
}

/// The daily round of a naval battle. Sides come from `Combat::attacker` and
/// `defender` (their `units` are fleet ids); the ships and their state live in `NavalState`.
pub fn naval_day(r: &BattleRules, c: &mut Combat, st: &mut NavalState, ctx: &LandContext, rng: &mut BattleRng) -> NavalReport {
    let nr = &r.naval;
    let mut rep = NavalReport::default();
    let total = |st: &NavalState, w: BattleSide| st.ships.iter().filter(|s| s.side == w).map(|s| s.strength).sum::<f64>();
    let finish = |c: &mut Combat, st: &NavalState, winner: BattleSide, destroyed: bool| {
        let loser = winner.other();
        let value = |w: BattleSide| st.ships.iter().filter(|s| s.side == w && s.state == ShipState::Gone).map(|s| sink_value(nr, s.class)).sum::<f64>();
        let (a_v, d_v) = (value(BattleSide::Attacker), value(BattleSide::Defender));
        let result = make_result(r, c, ctx, winner, true, 0.0, 0.0, a_v, d_v);
        let fleets = c.side(loser).units.clone();
        let escaped: BTreeSet<u64> = st.ships.iter().filter(|s| s.side == loser && s.state == ShipState::Escaped).map(|s| s.fleet).collect();
        Outcome { winner, loser_destroyed: destroyed, retreating: if destroyed { Vec::new() } else { fleets.iter().copied().filter(|f| escaped.contains(f)).collect() }, destroyed: if destroyed { fleets } else { Vec::new() }, result }
    };
    if c.attacker.units.is_empty() || c.defender.units.is_empty() {
        let winner = if c.attacker.units.is_empty() { BattleSide::Defender } else { BattleSide::Attacker };
        rep.outcome = Some(finish(c, st, winner, false));
        return rep;
    }
    c.duration += 1.0;
    let first = c.day < 0.0;
    if first || c.day > r.dice_period_days - 1.0 {
        c.attacker.dice = roll(r, rng);
        c.defender.dice = roll(r, rng);
        c.day = 0.0;
    }
    rep.dice = [c.attacker.dice, c.defender.dice];
    if first {
        // First-day overrun: under a tenth of the other side is destroyed at once.
        let (a, d) = (total(st, BattleSide::Attacker), total(st, BattleSide::Defender));
        let victim = if a * r.overrun_first_day_ratio < d { Some(BattleSide::Attacker) } else if d * r.overrun_first_day_ratio < a { Some(BattleSide::Defender) } else { None };
        if let Some(v) = victim {
            for s in st.ships.iter_mut().filter(|s| s.side == v) {
                s.strength = 0.0;
                s.state = ShipState::Gone;
                rep.sunk.push(s.id);
            }
            c.side_mut(v).wiped = true;
            rep.outcome = Some(finish(c, st, v.other(), true));
            return rep;
        }
    }
    let stack = [stacking(&st.ships, BattleSide::Attacker), stacking(&st.ships, BattleSide::Defender)];
    let leader_mod = [ctx.attacker.leader_attack.trunc(), ctx.defender.leader_defence.trunc()];
    for me in 0..st.ships.len() {
        let side = st.ships[me].side;
        let s = idx(side);
        let info = if s == 0 { &st.attacker } else { &st.defender };
        let (l_recon, l_exp, l_speed) = (info.leader_recon, info.leader_experience, info.leader_speed);
        let state = st.ships[me].state;
        if state.out() {
            continue;
        }
        let step = |ship: &NavalShip, rng: &mut BattleRng| (0.5 + 0.5 * rng.unit()) * ship.speed * nr.speed_to_distance;
        if state == ShipState::Retreating || st.fleeing[s] {
            if state != ShipState::Retreating {
                st.ships[me].state = ShipState::Retreating;
                st.ships[me].target = None;
            }
            let d = step(&st.ships[me], rng) * nr.retreat_speed_mod;
            let ship = &mut st.ships[me];
            ship.distance += d;
            if ship.distance >= 1.0 {
                ship.state = ShipState::Escaped;
            }
            continue;
        }
        // Flight on low strength or organisation.
        {
            let ship = &st.ships[me];
            let low = (ship.max_strength > 0.0 && ship.strength / ship.max_strength <= nr.retreat_level) || (ship.max_organisation > 0.0 && ship.organisation / ship.max_organisation <= nr.retreat_level);
            if low {
                let chance = nr.retreat_chance * 100.0 + l_exp * 100.0 + l_speed * 100.0;
                if (rng.below(100) as f64) <= chance {
                    let ship = &mut st.ships[me];
                    ship.target = None;
                    ship.state = ShipState::Retreating;
                    continue;
                }
            }
        }
        if let Some(t) = st.ships[me].target && st.ships.iter().find(|x| x.id == t).is_none_or(|x| x.state.out()) {
            st.ships[me].target = None;
            st.ships[me].state = ShipState::Seeking;
        }
        let mut state = st.ships[me].state;
        if state == ShipState::Seeking {
            if st.ships[me].target.is_none() {
                let mut chance = nr.seeking_chance * 100.0 + l_recon * 100.0;
                if stack[s] > 0.0 {
                    chance *= 1.0 - nr.stacking_target_select * stack[s];
                }
                chance = chance.max(nr.seeking_chance_min * 100.0);
                if (rng.below(100) as f64) <= chance {
                    st.ships[me].target = choose_target(nr, &st.ships, me, rng);
                }
            }
            if st.ships[me].target.is_some() {
                state = ShipState::Approaching;
                let ship = &mut st.ships[me];
                ship.state = state;
                if ship.distance < 1.0 {
                    let scale = nr.shift_back_days.max(1.0);
                    let k = 1.0 - c.duration.min(scale) / scale;
                    ship.distance = (ship.distance + nr.shift_back * k * rng.unit()).min(1.0);
                }
            }
        }
        if state == ShipState::Approaching || state == ShipState::Firing {
            let Some(tid) = st.ships[me].target else { continue };
            let Some(ti) = st.ships.iter().position(|x| x.id == tid) else { continue };
            let range = st.ships[me].distance + st.ships[ti].distance;
            if range <= st.ships[me].fire_range {
                st.ships[me].state = ShipState::Firing;
                // Shoot.
                let evasion = st.ships[ti].evasion.clamp(0.0, 0.99);
                if rng.unit() > 1.0 - evasion {
                    continue;
                }
                let (own, tgt) = (&st.ships[me], &st.ships[ti]);
                let power = own.gun_power + if own.class != ShipClass::Big && tgt.class == ShipClass::Big { own.torpedo } else { 0.0 };
                let mut x = power * r.table_factor(c.side(side).dice + leader_mod[s]);
                if tgt.hull > 0.0 {
                    x /= tgt.hull;
                }
                x /= (1.0 + 0.01 * tgt.experience).clamp(1.0, 2.0);
                let org = x * tgt.strength * nr.damage_org_mult;
                let mut sd = x;
                if tgt.organisation <= 0.0 {
                    sd *= nr.damage_no_org;
                }
                sd *= nr.damage_str_mult * own.strength;
                let t = &mut st.ships[ti];
                t.pending_organisation += org;
                t.pending_strength += sd;
            } else {
                let d = step(&st.ships[me], rng);
                let ship = &mut st.ships[me];
                ship.state = ShipState::Approaching;
                ship.distance = (ship.distance - d).max(0.0);
                let enemies = st.ships.iter().filter(|x| x.side != side && x.state.active()).count();
                if enemies > 1 {
                    let mut chance = nr.change_target_chance * 100.0;
                    if stack[s] > 0.0 {
                        chance = (chance + nr.stacking_target_change * 100.0 * stack[s]).clamp(0.0, 50.0);
                    }
                    if (rng.below(100) as f64) <= chance {
                        st.ships[me].target = None;
                        st.ships[me].state = ShipState::Seeking;
                    }
                }
            }
        }
    }
    // Apply pending damage; a ship whose strength falls below the damage is sunk.
    for ship in st.ships.iter_mut() {
        if ship.state == ShipState::Gone {
            continue;
        }
        if ship.strength < ship.pending_strength || ship.strength - ship.pending_strength < 0.1 && ship.pending_strength > 0.0 {
            ship.strength = 0.0;
            ship.state = ShipState::Gone;
            rep.sunk.push(ship.id);
        } else {
            ship.strength -= ship.pending_strength;
            ship.organisation = (ship.organisation - ship.pending_organisation).max(0.0);
        }
        ship.pending_strength = 0.0;
        ship.pending_organisation = 0.0;
    }
    // End tests: the attacker first. The loser flees; the battle ends when every ship is gone or escaped.
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        if !st.fleeing[idx(who)] && naval_lost(nr, st, who) {
            let (mine, other) = (total(st, who), total(st, who.other()));
            if mine < r.destroy_ratio * other {
                for s in st.ships.iter_mut().filter(|s| s.side == who && s.state != ShipState::Gone) {
                    s.strength = 0.0;
                    s.state = ShipState::Gone;
                    rep.sunk.push(s.id);
                }
                c.side_mut(who).wiped = true;
                rep.outcome = Some(finish(c, st, who.other(), true));
                return rep;
            }
            st.fleeing[idx(who)] = true;
            break;
        }
    }
    for who in [BattleSide::Attacker, BattleSide::Defender] {
        if st.fleeing[idx(who)] && st.ships.iter().filter(|s| s.side == who).all(|s| s.state.out()) {
            let destroyed = st.ships.iter().filter(|s| s.side == who).all(|s| s.state == ShipState::Gone);
            rep.outcome = Some(finish(c, st, who.other(), destroyed));
            return rep;
        }
    }
    c.day += 1.0;
    rep
}
