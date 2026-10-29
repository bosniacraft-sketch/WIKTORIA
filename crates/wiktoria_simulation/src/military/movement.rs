//! Movement rules: speed, distance, closed borders, may-enter, path finding, embarking, arrival.
//! Pure functions: the day loop owns the state and
//! acts on what these return.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use wiktoria_data::defines::Defines;
use wiktoria_data::geography::ExtraAdjacency;
use wiktoria_data::map::ProvinceId;
use wiktoria_data::engine_rules::EngineRules;

use super::siege::ProvinceMilitary;

/// Engine constants of movement, from the rules file (`FLD_MIL_MOVE_*`, hypotheses).
#[derive(Debug, Clone, PartialEq)]
pub struct MovementRules {
    /// Distance of one step of an army between land and sea.
    pub sea_land_distance: f64,
    /// Speed of an army stepping between land and sea.
    pub sea_land_speed: f64,
    /// Lowest factor on a border distance from the movement cost of the province left.
    pub min_distance_factor: f64,
    /// LAND_SPEED_MODIFIER of the defines.
    pub land_speed_modifier: f64,
    /// NAVAL_SPEED_MODIFIER of the game's defines.
    pub naval_speed_modifier: f64,
    /// A unit in a land battle takes orders only when the battle has run MORE days than this.
    pub order_battle_min_days: f64,
    /// Days of the "just moved" lock after a step: no arrival check while it runs.
    /// The map gives no length, a rules file stand-in.
    pub just_moved_lock_days: f64,
    /// Regiments one transport ship carries (`FLD_MIL_MOVE_REGIMENTS_PER_TRANSPORT`). The rules file
    /// has no row yet: until it does, [`REGIMENTS_PER_TRANSPORT_FALLBACK`] stands in (one regiment per
    /// transport, as `logistics::fleet_room` already counts). A stand-in, not observed.
    pub regiments_per_transport: f64,
}

/// Stand-in for `FLD_MIL_MOVE_REGIMENTS_PER_TRANSPORT` while the rules file has no such row.
pub const REGIMENTS_PER_TRANSPORT_FALLBACK: f64 = 1.0;

impl MovementRules {
    pub fn new(defines: &Defines, m: &EngineRules) -> Result<MovementRules, String> {
        let n = |id: &str| crate::num_field(m, id);
        Ok(MovementRules {
            sea_land_distance: n("FLD_MIL_MOVE_SEA_LAND_DISTANCE")?,
            sea_land_speed: n("FLD_MIL_MOVE_SEA_LAND_SPEED")?,
            min_distance_factor: n("FLD_MIL_MOVE_MIN_DISTANCE_FACTOR")?,
            land_speed_modifier: defines.get("military", "LAND_SPEED_MODIFIER").ok_or("defines.military.LAND_SPEED_MODIFIER missing")?,
            naval_speed_modifier: defines.get("military", "NAVAL_SPEED_MODIFIER").ok_or("defines.military.NAVAL_SPEED_MODIFIER missing")?,
            order_battle_min_days: n("FLD_MIL_MOVE_ORDER_BATTLE_MIN_DAYS")?,
            just_moved_lock_days: n("FLD_MIL_MOVE_JUST_MOVED_LOCK_DAYS")?,
            regiments_per_transport: m.field("FLD_MIL_MOVE_REGIMENTS_PER_TRANSPORT").and_then(|v| v.as_f64()).unwrap_or(REGIMENTS_PER_TRANSPORT_FALLBACK),
        })
    }
}

// ---------------------------------------------------------------- speed and distance

/// What decides a unit's speed on one step (3.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedInput {
    pub is_army: bool,
    pub from_sea: bool,
    pub to_sea: bool,
    /// Speed of the unit's speed-setting type (the slowest regiment or ship).
    pub unit_speed: f64,
    /// The type's own entry for the terrain of the NEXT province.
    pub terrain_entry: f64,
    /// The border is a strait or a river and this is the type's crossing entry, else 0.
    pub crossing_entry: f64,
    /// Infrastructure bonus of the next province.
    pub infrastructure: f64,
    pub leader_speed: f64,
}

pub fn unit_speed(r: &MovementRules, s: &SpeedInput) -> f64 {
    if s.is_army && s.from_sea != s.to_sea {
        return r.sea_land_speed;
    }
    let modifier = if s.is_army { r.land_speed_modifier } else { r.naval_speed_modifier };
    s.unit_speed * (1.0 + s.terrain_entry) * (1.0 + s.crossing_entry) * (1.0 + s.infrastructure) * modifier * (1.0 + s.leader_speed)
}

/// Distance of one step (3.2). `left_cost_modifier` is the movement cost modifier of the province
/// being LEFT.
pub fn step_distance(r: &MovementRules, is_army: bool, from_sea: bool, to_sea: bool, border_distance: f64, left_cost_modifier: f64) -> f64 {
    if is_army && from_sea != to_sea {
        r.sea_land_distance
    } else {
        border_distance * (1.0 + left_cost_modifier).max(r.min_distance_factor)
    }
}

/// What one daily call does to a moving unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DailyStep {
    /// The border closed: the whole order is cancelled.
    Cancel,
    /// No step; progress as it stands now.
    Wait { progress: f64 },
    /// One step; progress keeps the surplus, the caller pops the path node and places the unit.
    /// Attrition is applied at once after it (3.2).
    Step { progress: f64 },
}

/// One day of travel. The step is taken once per call, the surplus is kept. In a battle progress
/// grows but is capped at the distance and no step is taken.
pub fn daily_step(progress: f64, speed: f64, distance: f64, in_battle: bool, hour: i64, earliest_arrival: i64, border_closed: bool) -> DailyStep {
    let grown = progress + speed.max(0.0);
    if in_battle {
        return DailyStep::Wait { progress: grown.min(distance) };
    }
    if grown >= distance && hour >= earliest_arrival {
        if border_closed {
            return DailyStep::Cancel;
        }
        return DailyStep::Step { progress: grown - distance };
    }
    DailyStep::Wait { progress: grown }
}

/// Hour a unit is expected to arrive: `now + ceil((distance - progress) / speed)` days, not
/// before `earliest`. A unit that cannot move never arrives (`None`).
pub fn arrival_hour(now_hour: i64, distance: f64, progress: f64, speed: f64, earliest: i64) -> Option<i64> {
    if speed <= 0.0 {
        return None;
    }
    let days = ((distance - progress) / speed).ceil().max(0.0) as i64;
    Some((now_hour + days * 24).max(earliest))
}

/// Units ordered together arrive together: all take the latest arrival hour of the group.
pub fn group_arrival(hours: &[Option<i64>]) -> Option<i64> {
    hours.iter().copied().collect::<Option<Vec<_>>>()?.into_iter().max()
}

/// A new order resets progress unless its first step is the first step of the old path.
pub fn progress_after_order(old_path: &[ProvinceId], new_path: &[ProvinceId], progress: f64) -> f64 {
    match (old_path.first(), new_path.first()) {
        (Some(a), Some(b)) if a == b => progress,
        _ => 0.0,
    }
}

/// Orders are refused for a unit in a land battle that has run no more than the limit (3.4).
pub fn may_take_order(r: &MovementRules, in_land_battle: bool, battle_days: f64) -> bool {
    !in_land_battle || battle_days > r.order_battle_min_days
}

/// The "just moved" lock after a step, and its daily decay.
pub fn lock_after_step(r: &MovementRules) -> f64 {
    r.just_moved_lock_days
}

pub fn lock_tick(days: f64) -> f64 {
    (days - 1.0).max(0.0)
}

// ---------------------------------------------------------------- map and borders

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderKind {
    Land,
    Strait,
    Canal,
    Impassable,
    River,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub kind: BorderKind,
    pub distance: f64,
    /// Sea province of a strait, or province of a canal.
    pub through: Option<ProvinceId>,
}

/// Terrain facts of a province the rules need.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MoveProvince {
    pub sea: bool,
    pub infrastructure: f64,
    pub cost_modifier: f64,
    /// For a land province on the coast: the sea its port lies on.
    pub port_sea: Option<ProvinceId>,
}

/// Borders (both directions) and province facts, built once from the world.
#[derive(Debug, Clone, Default)]
pub struct MoveMap {
    pub borders: BTreeMap<(ProvinceId, ProvinceId), Border>,
    pub provinces: BTreeMap<ProvinceId, MoveProvince>,
    /// Whether canals of the world are open.
    pub canals_open: bool,
}

impl MoveMap {
    /// From the world's plain adjacency plus `adjacencies.csv`. `distance` gives the length of a
    /// border from the two provinces (e.g. between their positions). Extra rows of kind
    /// strait, canal or impassable override the plain border; other kinds are plain borders.
    pub fn from_adjacency(adjacency: &[BTreeSet<ProvinceId>], extra: &[ExtraAdjacency], distance: &dyn Fn(ProvinceId, ProvinceId) -> f64) -> MoveMap {
        let mut map = MoveMap { canals_open: true, ..MoveMap::default() };
        for (a, set) in adjacency.iter().enumerate() {
            for &b in set {
                let a = a as ProvinceId;
                map.borders.insert((a, b), Border { kind: BorderKind::Land, distance: distance(a, b), through: None });
            }
        }
        for e in extra {
            let kind = match e.kind.to_ascii_lowercase().as_str() {
                "strait" | "sea" => BorderKind::Strait,
                "canal" => BorderKind::Canal,
                "impassable" => BorderKind::Impassable,
                _ => BorderKind::Land,
            };
            let through = (e.through != 0).then_some(e.through);
            let border = Border { kind, distance: distance(e.from, e.to), through };
            map.borders.insert((e.from, e.to), border);
            map.borders.insert((e.to, e.from), border);
        }
        map
    }

    /// Marks plain land borders as river borders, both directions; a strait, canal or closed
    /// border keeps its kind.
    pub fn set_rivers(&mut self, borders: impl IntoIterator<Item = (ProvinceId, ProvinceId)>) {
        for (a, b) in borders {
            for key in [(a, b), (b, a)] {
                if let Some(x) = self.borders.get_mut(&key).filter(|x| x.kind == BorderKind::Land) {
                    x.kind = BorderKind::River;
                }
            }
        }
    }

    pub fn border(&self, from: ProvinceId, to: ProvinceId) -> Option<&Border> {
        self.borders.get(&(from, to))
    }

    fn info(&self, p: ProvinceId) -> MoveProvince {
        self.provinces.get(&p).copied().unwrap_or_default()
    }

    pub fn neighbours(&self, from: ProvinceId) -> impl Iterator<Item = ProvinceId> + '_ {
        self.borders.range((from, 0)..=(from, ProvinceId::MAX)).map(|((_, b), _)| *b)
    }
}

/// The day's state that closes borders (3.3).
#[derive(Debug, Clone, Default)]
pub struct BorderContext {
    /// Sea provinces where a hostile fleet with ships, not retreating, stands.
    pub hostile_fleet_seas: BTreeSet<ProvinceId>,
    /// Provinces controlled by a country hostile to the mover.
    pub hostile_controlled: BTreeSet<ProvinceId>,
}

/// Is the border closed to this kind of unit, before the unit's own may-enter test (3.3)?
pub fn border_closed(map: &MoveMap, ctx: &BorderContext, is_fleet: bool, from: ProvinceId, to: ProvinceId) -> bool {
    let Some(b) = map.border(from, to) else { return true };
    match b.kind {
        BorderKind::Impassable => return true,
        BorderKind::Canal => {
            if !map.canals_open || b.through.is_some_and(|p| ctx.hostile_controlled.contains(&p)) {
                return true;
            }
        }
        BorderKind::Strait => {
            if b.through.is_some_and(|p| ctx.hostile_fleet_seas.contains(&p)) {
                return true;
            }
        }
        BorderKind::Land | BorderKind::River => {}
    }
    if is_fleet {
        let (f, t) = (map.info(from), map.info(to));
        return match (f.sea, t.sea) {
            (false, false) => true,
            (true, false) => t.port_sea != Some(from),
            (false, true) => f.port_sea != Some(to),
            (true, true) => false,
        };
    }
    false
}

/// The stricter path finders (3.3): `Safe` also avoids hostile strength above the unit's limit,
/// `VerySafe` also hostile-controlled targets.
pub fn safe_closed(hostile_strength_in_target: f64, limit: f64, hostile_rebels_in_foreign_target: bool) -> bool {
    hostile_strength_in_target > limit || hostile_rebels_in_foreign_target
}

// ---------------------------------------------------------------- relations and may-enter

/// Diplomatic facts between countries that movement needs. The day loop fills it from wars,
/// alliances, access and subject links.
#[derive(Debug, Clone, Default)]
pub struct Relations {
    pub rebel_tag: String,
    /// Unordered pairs at war.
    pub wars: BTreeSet<(String, String)>,
    /// (mover, host): same country, overlord/vassal, same overlord, ally at war, or access given.
    pub friendly: BTreeSet<(String, String)>,
    /// Unordered pairs: overlord and vassal.
    pub subjects: BTreeSet<(String, String)>,
    /// Overlord of a vassal.
    pub overlord: BTreeMap<String, String>,
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) }
}

impl Relations {
    pub fn at_war(&self, a: &str, b: &str) -> bool {
        self.wars.contains(&pair(a, b))
    }

    pub fn friendly(&self, a: &str, b: &str) -> bool {
        a == b || self.friendly.contains(&(a.to_string(), b.to_string()))
    }

    pub fn tied(&self, a: &str, b: &str) -> bool {
        a == b || self.subjects.contains(&pair(a, b))
    }

    /// Hostile for battle purposes: at war, or one is rebels and the other is not.
    pub fn hostile(&self, a: &str, b: &str) -> bool {
        a != b && (self.at_war(a, b) || ((a == self.rebel_tag) != (b == self.rebel_tag)))
    }
}

/// May an army of `owner` enter a land province? `from` is the
/// province it stands in, `to` the target; `None` controller means unowned/uncontrolled.
pub fn may_enter_land(rel: &Relations, owner: &str, exiled: bool, from: &ProvinceMilitary, to: &ProvinceMilitary) -> bool {
    if exiled {
        return true;
    }
    // Same controller, or overlord and vassal.
    if let (Some(a), Some(b)) = (&from.controller, &to.controller)
        && rel.tied(a, b)
    {
        return true;
    }
    let against = |c: &Option<String>| c.as_deref().is_some_and(|c| rel.at_war(owner, c) || rel.friendly(owner, c) || rel.overlord.get(c).is_some_and(|o| rel.at_war(owner, o)));
    if to.owner.is_none() || against(&to.owner) || against(&to.controller) {
        return true;
    }
    owner == rel.rebel_tag
}

/// May an army enter a sea province: the regiments of its country's armies there plus its own
/// must fit into the room of its country's fleets there.
pub fn may_enter_sea(regiments_already: usize, own_regiments: usize, fleet_room: usize) -> bool {
    regiments_already + own_regiments <= fleet_room
}

// ---------------------------------------------------------------- path finding

/// Cheapest path from `from` to `to` over the open borders, deterministic: costs are the border
/// distances in thousandths, ties go to the lowest province id (neighbours in id order, a
/// shorter route replaces only when strictly cheaper). Returns the provinces after `from`.
pub fn find_path(map: &MoveMap, from: ProvinceId, to: ProvinceId, passable: &dyn Fn(ProvinceId, ProvinceId) -> bool) -> Option<Vec<ProvinceId>> {
    if from == to {
        return Some(Vec::new());
    }
    let mut best: BTreeMap<ProvinceId, u64> = BTreeMap::from([(from, 0)]);
    let mut prev: BTreeMap<ProvinceId, ProvinceId> = BTreeMap::new();
    let mut heap = BinaryHeap::from([Reverse((0u64, from))]);
    while let Some(Reverse((cost, p))) = heap.pop() {
        if p == to {
            break;
        }
        if best.get(&p).is_some_and(|&c| c < cost) {
            continue;
        }
        for n in map.neighbours(p) {
            if !passable(p, n) {
                continue;
            }
            let step = (map.border(p, n).map_or(0.0, |b| b.distance) * 1000.0).round().max(0.0) as u64;
            let c = cost + step;
            if best.get(&n).is_none_or(|&old| c < old) {
                best.insert(n, c);
                prev.insert(n, p);
                heap.push(Reverse((c, n)));
            }
        }
    }
    best.get(&to)?;
    let mut path = vec![to];
    let mut at = to;
    while let Some(&p) = prev.get(&at) {
        if p == from {
            break;
        }
        path.push(p);
        at = p;
    }
    path.reverse();
    Some(path)
}

/// Distance over the open borders from `from` to every province it can reach (itself at 0).
pub fn distances(map: &MoveMap, from: ProvinceId, passable: &dyn Fn(ProvinceId, ProvinceId) -> bool) -> BTreeMap<ProvinceId, f64> {
    let mut best: BTreeMap<ProvinceId, u64> = BTreeMap::from([(from, 0)]);
    let mut heap = BinaryHeap::from([Reverse((0u64, from))]);
    while let Some(Reverse((cost, p))) = heap.pop() {
        if best.get(&p).is_some_and(|&c| c < cost) {
            continue;
        }
        for n in map.neighbours(p) {
            if !passable(p, n) {
                continue;
            }
            let c = cost + (map.border(p, n).map_or(0.0, |b| b.distance) * 1000.0).round().max(0.0) as u64;
            if best.get(&n).is_none_or(|&old| c < old) {
                best.insert(n, c);
                heap.push(Reverse((c, n)));
            }
        }
    }
    best.into_iter().map(|(p, c)| (p, c as f64 / 1000.0)).collect()
}

// ---------------------------------------------------------------- embark and disembark

/// May an army order step from `a` to `b`? Land borders that are not impassable; with `sea` also
/// onto a sea province from its coast, between sea provinces (as a fleet may sail) and ashore again.
pub fn army_passable(map: &MoveMap, sea: bool, a: ProvinceId, b: ProvinceId) -> bool {
    let Some(border) = map.border(a, b) else { return false };
    if border.kind == BorderKind::Impassable {
        return false;
    }
    match (map.info(a).sea, map.info(b).sea) {
        (false, false) => true,
        (true, true) => sea && !border_closed(map, &BorderContext::default(), true, a, b),
        _ => sea,
    }
}

/// The path of an army order to the land province `to`: over land when there is one, else over
/// land and sea legs (each sea leg is sailed by a fleet of the army's country). `None` when there
/// is no way, or `to` is a sea province. The flag says whether the path has sea legs.
pub fn army_path(map: &MoveMap, from: ProvinceId, to: ProvinceId) -> Option<(Vec<ProvinceId>, bool)> {
    army_path_entering(map, from, to, &|_, _| true)
}

/// As [`army_path`], stepping into a land province only where `may_enter(from, to)` holds (the
/// army may cross the border: `may_enter_land`), so a way through a closed country is not
/// taken when the sea goes round it.
pub fn army_path_entering(map: &MoveMap, from: ProvinceId, to: ProvinceId, may_enter: &dyn Fn(ProvinceId, ProvinceId) -> bool) -> Option<(Vec<ProvinceId>, bool)> {
    if map.info(to).sea {
        return None;
    }
    if !map.info(from).sea
        && let Some(p) = find_path(map, from, to, &|a, b| army_passable(map, false, a, b) && may_enter(a, b))
    {
        return Some((p, false));
    }
    let p = find_path(map, from, to, &|a, b| army_passable(map, true, a, b) && (map.info(b).sea || may_enter(a, b)))?;
    let sea = map.info(from).sea || p.iter().any(|x| map.info(*x).sea);
    Some((p, sea))
}

/// Regiments a fleet can carry: its transport ships times the regiments each carries, whole part.
pub fn transport_capacity(r: &MovementRules, transport_ships: usize) -> usize {
    (transport_ships as f64 * r.regiments_per_transport).floor().max(0.0) as usize
}

/// A fleet of the army's country in a sea province.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetRoom {
    pub id: u64,
    pub owner: String,
    pub free: usize,
}

/// An army stepping onto sea boards the first (lowest id) fleet of its country with room for
/// all its regiments. `None`: the order is cancelled.
pub fn embark(owner: &str, regiments: usize, fleets: &[FleetRoom]) -> Option<u64> {
    fleets.iter().filter(|f| f.owner == owner && f.free >= regiments).map(|f| f.id).min()
}

/// A carried army is put ashore when its fleet arrives next to a land province it may enter.
pub fn may_disembark(rel: &Relations, owner: &str, exiled: bool, from_coast: &ProvinceMilitary, to: &ProvinceMilitary) -> bool {
    may_enter_land(rel, owner, exiled, from_coast, to)
}

// ---------------------------------------------------------------- arrival

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Attacker,
    Defender,
}

/// The unit whose arrival (or daily check) is looked at.
#[derive(Debug, Clone, PartialEq)]
pub struct Mover {
    pub id: u64,
    pub owner: String,
    pub is_army: bool,
    pub exiled: bool,
    pub retreating: bool,
    pub strength: f64,
    /// Carried by a fleet that is under way, or still at sea.
    pub on_moving_fleet: bool,
    pub at_sea: bool,
    /// The unit has no path left (stands still).
    pub standing: bool,
    /// Days left of the "just moved" lock.
    pub lock_days: f64,
}

/// Another unit in the province.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub id: u64,
    pub owner: String,
    pub is_army: bool,
    pub exiled: bool,
    pub retreating: bool,
    pub at_sea: bool,
    pub carried: bool,
    pub in_port: bool,
}

/// A battle or siege in the province, seen by the countries on each side.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatView {
    pub index: usize,
    pub siege: bool,
    /// Land (armies) or naval (fleets) battle; sieges are land.
    pub land: bool,
    pub attacker_countries: Vec<String>,
    pub defender_countries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arrival {
    Nothing,
    JoinBattle { combat: usize, side: Side },
    JoinSiege { combat: usize, side: Side },
    /// A new battle: the mover attacks, `defender` is the unit found.
    StartBattle { attacker: u64, defender: u64 },
    StartSiege { besieger: u64 },
}

/// Can `a` (checking) treat `b` as an enemy for starting a battle?
fn is_enemy(rel: &Relations, m: &Mover, o: &Standing) -> bool {
    !m.exiled
        && !m.retreating
        && !o.exiled
        && !o.retreating
        && m.owner != o.owner
        && m.is_army == o.is_army
        && !(m.is_army && (m.at_sea || m.on_moving_fleet))
        && !(o.is_army && (o.at_sea || o.carried))
        && !(!o.is_army && o.in_port)
        && rel.hostile(&m.owner, &o.owner)
}

/// What should happen when `m` arrives in a province (or is checked there) (3.6). Offer to a
/// battle in progress first, then to a siege, else a new battle against the first enemy
/// (others listed in the order given), else a siege. `controller` is the province's controller.
pub fn arrival(rel: &Relations, m: &Mover, controller: Option<&str>, others: &[Standing], combats: &[CombatView]) -> Arrival {
    if m.on_moving_fleet && m.is_army || m.lock_days > 0.0 || m.exiled {
        return Arrival::Nothing;
    }
    let join = |c: &CombatView| -> Option<Side> {
        // Armies join land battles and sieges, fleets naval battles.
        if c.land != m.is_army {
            return None;
        }
        let hostile_to = |side: &[String]| side.iter().any(|t| rel.hostile(&m.owner, t));
        let member = |side: &[String]| side.contains(&m.owner);
        if member(&c.attacker_countries) || member(&c.defender_countries) {
            return None;
        }
        match (hostile_to(&c.attacker_countries), hostile_to(&c.defender_countries)) {
            (false, true) => Some(Side::Attacker),
            (true, false) => Some(Side::Defender),
            _ => None,
        }
    };
    if let Some((c, side)) = combats.iter().filter(|c| !c.siege).find_map(|c| join(c).map(|s| (c, s))) {
        return Arrival::JoinBattle { combat: c.index, side };
    }
    if let Some((c, side)) = combats.iter().filter(|c| c.siege).find_map(|c| join(c).map(|s| (c, s))) {
        return Arrival::JoinSiege { combat: c.index, side };
    }
    if let Some(enemy) = others.iter().find(|o| is_enemy(rel, m, o)) {
        return Arrival::StartBattle { attacker: m.id, defender: enemy.id };
    }
    let can_siege = m.is_army && m.standing && !m.exiled && m.strength > 0.001 && combats.iter().all(|c| c.siege) && controller.is_some_and(|c| c != m.owner && rel.hostile(&m.owner, c));
    if can_siege {
        return Arrival::StartSiege { besieger: m.id };
    }
    Arrival::Nothing
}
