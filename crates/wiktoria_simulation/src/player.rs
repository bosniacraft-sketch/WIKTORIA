//! The player layer (docs/DECISIONS.md D20): which countries a person plays, and the orders a
//! person gives them.
//!
//! **One flag per country.** A country is played by a person when its `CountryState::human` is
//! set (the save's `human = yes`, the same flag the computer player already reads to leave a
//! country alone); every other country is run by the computer. Zero, one or several countries may
//! be played at once: nothing here assumes a single player. [`set_player`] only moves that flag
//! (and its copy in the diplomacy state), and drops the computer's standing plans for the country
//! (its war target and the targets of its armies), so a country handed back to the computer plans
//! again from the state as it is, without a reload, and a country taken over is not steered by
//! plans the computer made for it.
//!
//! **Orders.** A person's orders wait in [`Players`]' queue and are carried out at the start of
//! the next day, before anything else of that day ([`apply`], called by `Game::day`). Carrying
//! them out between two days, as a server may do so the page sees the answer at once, is the
//! same thing: nothing happens between the end of one day and the start of the next. Every order
//! is checked again when it is carried out (the country must still be played, the technology still
//! open, the army still there) and answers with an [`Outcome`].
//!
//! The orders of this first slice:
//! * **budget sliders** ([`Slider`]): the three taxes, the four spending sliders and the tariff.
//!   A value is put inside the limits the engine already keeps: 0..1 for every slider, -1..1 for
//!   the tariff, and the ruling party's own limits (lowest and highest tax, highest military
//!   spending, lowest and highest tariff), which `politics::politics_day` also enforces after an
//!   election. A value outside them is moved to the nearest allowed one, as a slider stops at its
//!   end;
//! * **research** ([`choose_research`]): the next technology, among those the computer could pick
//!   (`research::may_research`). Points already put into another technology go back to the
//!   country's stored points (PORT CHOICE: nothing is lost by changing one's mind; how the original
//!   treats a switch is not observed);
//! * **army movement** ([`move_army`]): an army to a land province, over land when there is a way
//!   (`movement::army_path`, land borders that are not impassable), else with sea legs that a fleet
//!   of the country sails (the day step boards, sails and lands it, `military::day`). A sea route
//!   is refused when no fleet of the country can carry all the army's regiments. Refused while the
//!   army is held in a land battle (`movement::may_take_order`), or is on a fleet that is under way;
//!   an army on a standing fleet is routed from the fleet's sea province.
//!
//!
//! The orders of the second slice (events, decisions, war and peace):
//! * **events** ([`Order::ChooseEvent`]): an event with several options fired for a played country
//!   waits in `EventRunner::held` for the player's pick (events.rs has the rules and the time-out);
//! * **decisions** ([`Order::Decide`]): the computer takes none for a played country;
//! * **justify a casus belli** ([`Order::Justify`]) on the computer's own terms: one at a time,
//!   not during a truce or a war with the target, a type that is built (`constructing`); progress,
//!   detection and its infamy are the computer's (`Core::cb_day`);
//! * **declare war** ([`Order::DeclareWar`]) with a casus belli held against the target, through
//!   the computer's own declaration (`Core::start_war`: infamy, prestige, a truce broken, the
//!   allies of both leaders called);
//! * **offer peace** ([`Order::OfferPeace`]): a white peace or a treaty of the war's goals, all
//!   demanded of the enemy or all conceded to it, between two who may negotiate
//!   (`war::peace_allowed`). The computer answers at once with its own peace answer; a played
//!   enemy is asked;
//! * **answer** ([`Order::Answer`]) a question the computer put: a peace offer it sends (its peace
//!   logic shapes the offer as for any enemy, then asks instead of signing for the player) or a call
//!   to arms of an ally. PORT CHOICE: an unanswered question takes the computer's own answer after
//!   `events::ANSWER_DAYS` days, or at once when the country goes back to the computer; after a
//!   refused peace the same enemy offers again in that war only after as many days.
//!
//! What the computer leaves alone for a played country: research picks (research.rs), tax changes
//! and loan repayment (market_day.rs), national focus (ai.rs), factory building (industry.rs),
//! railways, mobilisation, army orders, war goals, peace offers, war declarations, justifications
//! and calls to arms (game.rs), event choices and decisions (events.rs).
//! NOT WIRED: the penalty for refusing a good peace (`war::decline_penalty` is used nowhere yet),
//! war goals added by a player mid-war, the casus belli types that need no justification (their
//! `can_use` triggers are not evaluated, so the computer never uses them either).

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;

use crate::calendar::{Calendar, GameDate};
use crate::events::{EventLog, EventRunner};
use crate::game::Core;
use crate::military::battle::CombatKind;
use crate::military::movement::{army_path_entering, may_take_order, progress_after_order, MoveMap, MovementRules};
use crate::military::units::Navy;
use crate::military::war::WarGoal;
use crate::military::MilitaryState;
use crate::script::EffectRules;
use crate::politics::PoliticsRules;
use crate::research::{may_research, ResearchRules};
use crate::script::State;

/// A budget slider a player can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slider {
    PoorTax,
    MiddleTax,
    RichTax,
    Education,
    Administration,
    Social,
    Military,
    Tariff,
}

impl Slider {
    pub const ALL: [Slider; 8] = [Slider::PoorTax, Slider::MiddleTax, Slider::RichTax, Slider::Education, Slider::Administration, Slider::Social, Slider::Military, Slider::Tariff];

    /// Our own short name (what the server's API takes).
    pub fn name(self) -> &'static str {
        match self {
            Slider::PoorTax => "poor",
            Slider::MiddleTax => "middle",
            Slider::RichTax => "rich",
            Slider::Education => "education",
            Slider::Administration => "administration",
            Slider::Social => "social",
            Slider::Military => "military",
            Slider::Tariff => "tariff",
        }
    }

    pub fn parse(s: &str) -> Option<Slider> {
        Slider::ALL.into_iter().find(|x| x.name() == s)
    }

    /// The key of `CountryState::spending` the market and the pops read.
    pub fn key(self) -> &'static str {
        match self {
            Slider::PoorTax => "poor_tax",
            Slider::MiddleTax => "middle_tax",
            Slider::RichTax => "rich_tax",
            Slider::Education => "education_spending",
            Slider::Administration => "crime_fighting",
            Slider::Social => "social_spending",
            Slider::Military => "military_spending",
            Slider::Tariff => "tariffs",
        }
    }

    fn is_tax(self) -> bool {
        matches!(self, Slider::PoorTax | Slider::MiddleTax | Slider::RichTax)
    }
}

/// One order of a person to a country they play.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    /// Set a budget slider (0..1; the tariff -1..1).
    Budget { slider: Slider, value: f64 },
    /// Research this technology next.
    Research { tech: String },
    /// Send this army to this province.
    MoveArmy { army: u64, to: u16 },
    /// Pick option `option` (from 0) of the held event `key`.
    ChooseEvent { key: u64, option: usize },
    /// Take this decision.
    Decide { name: String },
    /// Start justifying a casus belli of type `cb` against `target`.
    Justify { target: String, cb: String },
    /// Declare war on `target` with the held casus belli `cb` as the goal.
    DeclareWar { target: String, cb: String },
    /// Offer `to` peace in the war named `war`, enforcing these goals (indices into the war's
    /// goals; none: a white peace).
    OfferPeace { war: String, to: String, goals: Vec<usize> },
    /// Answer the question `key` (a peace offer or a call to arms).
    Answer { key: u64, accept: bool },
}

/// What the computer asks a played country.
#[derive(Debug, Clone, PartialEq)]
pub enum AskKind {
    /// Peace in the war, enforcing these goals (none: a white peace).
    Peace { goals: Vec<WarGoal> },
    /// Join the war on the side of `from` (the attackers' side when `attackers`).
    Call { attackers: bool },
}

/// A question to a played country, waiting for its answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    /// Our own number for it, unique in the run (what the player's answer names).
    pub key: u64,
    /// The played country asked.
    pub to: String,
    /// Who asks: the sender of the offer, the ally that calls.
    pub from: String,
    /// The war's name.
    pub war: String,
    pub kind: AskKind,
    pub made: Date,
    /// On this day (or later) the computer answers for the player.
    pub expires: Date,
}

/// The questions waiting for players, and the pauses after a refused peace.
#[derive(Debug, Clone, PartialEq)]
pub struct Asks {
    pub list: Vec<Ask>,
    next: u64,
    /// (war, sender, receiver) -> no new peace offer before this day.
    pub quiet: BTreeMap<(String, String, String), Date>,
    /// Days a question waits (default `events::ANSWER_DAYS`).
    pub answer_days: i64,
}

impl Default for Asks {
    fn default() -> Asks {
        Asks { list: Vec::new(), next: 0, quiet: BTreeMap::new(), answer_days: crate::events::ANSWER_DAYS }
    }
}

/// `today` plus `days` on the game's calendar.
pub fn days_after(cal: &Calendar, today: Date, days: i64) -> Date {
    cal.to_game(today).map_or(today, |g| cal.to_date(GameDate(g.0 + 24 * days.max(0))))
}

impl Asks {
    fn push(&mut self, to: &str, from: &str, war: &str, kind: AskKind, today: Date, cal: &Calendar) -> u64 {
        self.next += 1;
        let expires = days_after(cal, today, self.answer_days);
        self.list.push(Ask { key: self.next, to: to.into(), from: from.into(), war: war.into(), kind, made: today, expires });
        self.next
    }

    /// Asks `to` for peace in `war` on behalf of `from`.
    pub fn peace(&mut self, to: &str, from: &str, war: &str, goals: Vec<WarGoal>, today: Date, cal: &Calendar) -> u64 {
        self.push(to, from, war, AskKind::Peace { goals }, today, cal)
    }

    /// Calls the ally `to` into `war` on the side of `from`.
    pub fn call(&mut self, to: &str, from: &str, war: &str, attackers: bool, today: Date, cal: &Calendar) -> u64 {
        self.push(to, from, war, AskKind::Call { attackers }, today, cal)
    }

    /// Whether `from` may send `to` a new peace offer in `war` today: none is waiting, and no
    /// refusal of the last few days stands.
    pub fn may_ask_peace(&self, war: &str, from: &str, to: &str, today: Date) -> bool {
        !self.list.iter().any(|a| a.war == war && a.to == to && matches!(a.kind, AskKind::Peace { .. }))
            && self.quiet.get(&(war.to_string(), from.to_string(), to.to_string())).is_none_or(|d| today >= *d)
    }

    /// The questions waiting for `tag`, oldest first.
    pub fn of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Ask> + 'a {
        self.list.iter().filter(move |a| a.to == tag)
    }
}

/// What came of one order: `Ok` with what was done, `Err` with why it was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub tag: String,
    pub order: Order,
    pub result: Result<String, String>,
}

/// The orders waiting for the start of the next day.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Players {
    queue: Vec<(String, Order)>,
}

impl Players {
    /// Queues an order of `tag`'s player for the start of the next day.
    pub fn push(&mut self, tag: &str, order: Order) {
        self.queue.push((tag.to_string(), order));
    }

    /// The orders still waiting, oldest first.
    pub fn pending(&self) -> &[(String, Order)] {
        &self.queue
    }

    /// Takes the waiting orders out, oldest first.
    pub fn take(&mut self) -> Vec<(String, Order)> {
        std::mem::take(&mut self.queue)
    }
}

/// Whether a person plays this country.
pub fn is_player(state: &State, tag: &str) -> bool {
    state.countries.get(tag).is_some_and(|c| c.human)
}

/// The countries a person plays, by tag.
pub fn players(state: &State) -> Vec<String> {
    state.countries.iter().filter(|(_, c)| c.human).map(|(t, _)| t.clone()).collect()
}

/// Gives a country to a person (`on`) or back to the computer. Only the flag moves, and the
/// computer's standing plans for the country are dropped (see the module notes). An unknown
/// country is refused.
pub fn set_player(state: &mut State, core: &mut Core, tag: &str, on: bool) -> Result<(), String> {
    if tag == core.day.rebel_tag {
        return Err("rebels cannot be played".into());
    }
    let c = state.countries.get_mut(tag).ok_or_else(|| format!("no country {tag}"))?;
    c.human = on;
    if let Some((_, dip)) = core.dip.as_mut()
        && let Some(d) = dip.countries.get_mut(tag)
    {
        d.human = on;
    }
    core.war_targets.remove(tag);
    let armies: BTreeSet<u64> = core.mil.armies_of(tag).map(|a| a.id).collect();
    if on {
        // The marches the computer ordered stop; those the player ordered go on.
        let planned: BTreeSet<u64> = armies.iter().copied().filter(|id| core.ai_moved.contains(id) || core.ai_targets.contains_key(id) || core.ai_resting.contains(id)).collect();
        for a in core.mil.armies.iter_mut().filter(|a| planned.contains(&a.id)) {
            a.path.clear();
            a.target = None;
            a.movement_progress = 0.0;
        }
    }
    core.ai_targets.retain(|id, _| !armies.contains(id));
    core.ai_resting.retain(|id| !armies.contains(id));
    core.ai_moved.retain(|id| !armies.contains(id));
    Ok(())
}

/// Every country back to the computer (observing). Returns the countries that were played.
pub fn observe_all(state: &mut State, core: &mut Core) -> Vec<String> {
    let was = players(state);
    for tag in &was {
        let _ = set_player(state, core, tag, false);
    }
    // The diplomacy copy of the flag may carry a player the script state does not know.
    if let Some((_, dip)) = core.dip.as_mut() {
        for d in dip.countries.values_mut() {
            d.human = false;
        }
    }
    was
}

/// The lowest and highest value a slider may take for this country: 0..1 (the tariff -1..1),
/// narrowed by its ruling party's limits.
pub fn slider_limits(state: &State, politics: &PoliticsRules, tag: &str, slider: Slider) -> (f64, f64) {
    let (mut lo, mut hi): (f64, f64) = if slider == Slider::Tariff { (-1.0, 1.0) } else { (0.0, 1.0) };
    let party = state.countries.get(tag).and_then(|c| c.ruling_party).and_then(|id| politics.parties.get(tag)?.iter().find(|p| p.id == id));
    if let Some(p) = party {
        if slider.is_tax() {
            (lo, hi) = (lo.max(p.min_tax), hi.min(p.max_tax.max(p.min_tax)));
        } else if slider == Slider::Military {
            hi = hi.min(p.max_military);
        } else if slider == Slider::Tariff {
            (lo, hi) = (lo.max(p.min_tariff), hi.min(p.max_tariff.max(p.min_tariff)));
        }
    }
    (lo, hi.max(lo))
}

/// Sets a slider of a country, moved inside its limits. Returns the value set.
pub fn set_slider(state: &mut State, politics: &PoliticsRules, tag: &str, slider: Slider, value: f64) -> Result<f64, String> {
    if !value.is_finite() {
        return Err("the value must be a number".into());
    }
    let (lo, hi) = slider_limits(state, politics, tag, slider);
    let c = state.countries.get_mut(tag).ok_or_else(|| format!("no country {tag}"))?;
    let v = value.clamp(lo, hi);
    c.spending.insert(slider.key().to_string(), v);
    Ok(v)
}

/// The technologies this country may start now, in the order of the technology files.
pub fn open_technologies<'a>(state: &State, rules: &'a ResearchRules, tag: &str) -> Vec<&'a crate::research::TechInfo> {
    let Some(c) = state.countries.get(tag).filter(|c| c.civilized) else { return Vec::new() };
    rules.techs.iter().filter(|t| may_research(c, t, state.date.year)).collect()
}

/// Makes `tech` the technology the country researches. Points put into another technology go
/// back to its stored points.
pub fn choose_research(state: &mut State, rules: &ResearchRules, tag: &str, tech: &str) -> Result<(), String> {
    let year = state.date.year;
    let c = state.countries.get_mut(tag).ok_or_else(|| format!("no country {tag}"))?;
    if !c.civilized {
        return Err("an uncivilized country does not research".into());
    }
    let t = rules.techs.iter().find(|t| t.name == tech).ok_or_else(|| format!("no technology {tech}"))?;
    if c.research.as_ref().is_some_and(|(name, _)| name == tech) {
        return Ok(());
    }
    if c.technologies.contains(tech) {
        return Err(format!("{tech} is already researched"));
    }
    if t.year > year {
        return Err(format!("{tech} cannot be researched before {}", t.year));
    }
    if !may_research(c, t, year) {
        return Err(format!("{tech} needs {} first", t.previous.as_deref().unwrap_or("another technology")));
    }
    if let Some((_, progress)) = c.research.take() {
        c.research_points += progress.max(0.0);
    }
    c.research = Some((tech.to_string(), 0.0));
    Ok(())
}

/// Sends an army of `tag` to the land province `to`. `capacity` gives the regiments a fleet can
/// carry (`MilitaryDay::fleet_capacity`), `may_enter` whether the army may step from one land
/// province into another (`movement::may_enter_land`). Returns the number of provinces on the
/// way (0: it is there already, and stops) and whether the way has sea legs.
#[allow(clippy::too_many_arguments)]
pub fn move_army(mil: &mut MilitaryState, map: &MoveMap, rules: &MovementRules, capacity: &dyn Fn(&Navy) -> usize, may_enter: &dyn Fn(u16, u16) -> bool, tag: &str, army: u64, to: u16) -> Result<(usize, bool), String> {
    let a = mil.armies.iter().find(|a| a.id == army).ok_or_else(|| format!("no army {army}"))?;
    if a.owner != tag {
        return Err(format!("army {army} is not {tag}'s"));
    }
    if map.provinces.get(&to).is_some_and(|p| p.sea) {
        return Err(format!("province {to} is sea: an army is sent to a land province"));
    }
    let battle = mil.combats.iter().filter(|c| c.kind == CombatKind::Land && (c.attacker.units.contains(&army) || c.defender.units.contains(&army))).map(|c| c.duration).fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.min(d))));
    if !may_take_order(rules, battle.is_some(), battle.unwrap_or(0.0)) {
        return Err("the army is held in a battle".into());
    }
    let fleet = a.carried_by.map(|id| mil.navies.iter().find(|n| n.id == id).ok_or("the army's fleet is gone")).transpose()?;
    if fleet.is_some_and(|n| !n.path.is_empty()) {
        return Err("the army's fleet is under way".into());
    }
    let from = fleet.map_or(a.location, |n| n.location);
    let (path, sea) = army_path_entering(map, from, to, may_enter).ok_or_else(|| format!("no way to province {to}"))?;
    if sea && fleet.is_none() {
        let regiments = a.regiments.len();
        let used = |n: &Navy| mil.armies.iter().filter(|x| x.carried_by == Some(n.id)).map(|x| x.regiments.len()).sum::<usize>();
        if !mil.navies.iter().any(|n| n.owner == tag && capacity(n).saturating_sub(used(n)) >= regiments) {
            return Err(format!("no way over land to province {to}, and no fleet of {tag} can carry {regiments} regiments over the sea"));
        }
    }
    let a = mil.armies.iter_mut().find(|a| a.id == army).expect("found above");
    a.movement_progress = progress_after_order(&a.path, &path, a.movement_progress);
    a.target = if sea { path.last().copied() } else { None };
    a.path = path;
    Ok((a.path.len(), sea))
}

/// What the event and decision orders need: the event runner with its calendar, effect rules
/// and log.
pub struct EventSide<'a> {
    pub runner: &'a mut EventRunner,
    pub cal: &'a Calendar,
    pub rules: &'a EffectRules,
    pub log: &'a mut EventLog,
}

/// Carries out one order of `tag`'s player. Without `events` the event and decision orders are
/// refused.
pub fn apply_one(state: &mut State, core: &mut Core, research: &ResearchRules, politics: &PoliticsRules, events: Option<&mut EventSide>, tag: &str, order: &Order) -> Result<String, String> {
    if !is_player(state, tag) {
        return Err(format!("{tag} is played by the computer"));
    }
    match order {
        Order::Budget { slider, value } => set_slider(state, politics, tag, *slider, *value).map(|v| format!("{} set to {v:.2}", slider.name())),
        Order::Research { tech } => choose_research(state, research, tag, tech).map(|()| format!("researching {tech}")),
        Order::ChooseEvent { key, option } => {
            let e = events.ok_or("no events in this game")?;
            if !e.runner.held.iter().any(|h| h.key == *key && h.tag == tag) {
                return Err(format!("no event {key} waits for {tag}"));
            }
            e.runner.choose(state, e.cal, e.rules, *key, Some(*option), e.log)
        }
        Order::Decide { name } => {
            let e = events.ok_or("no decisions in this game")?;
            e.runner.take_decision(state, e.cal, e.rules, tag, name, e.log)
        }
        Order::Justify { target, cb } => core.player_justify(state, tag, target, cb),
        Order::DeclareWar { target, cb } => core.player_declare(state, tag, target, cb),
        Order::OfferPeace { war, to, goals } => core.player_offer_peace(state, tag, war, to, goals),
        Order::Answer { key, accept } => core.player_answer(state, tag, *key, *accept),
        Order::MoveArmy { army, to } => {
            let day = &core.day;
            let rel = day.relations(&core.mil, state);
            let provinces = core.mil.provinces.clone();
            let none = crate::military::siege::ProvinceMilitary::default();
            let may_enter = |a: u16, b: u16| crate::military::movement::may_enter_land(&rel, tag, false, provinces.get(&a).unwrap_or(&none), provinces.get(&b).unwrap_or(&none));
            let (n, sea) = move_army(&mut core.mil, &day.map, &day.movement, &|f| day.fleet_capacity(f), &may_enter, tag, *army, *to)?;
            core.ai_targets.remove(army);
            core.ai_resting.remove(army);
            core.ai_moved.remove(army);
            Ok(match (n, sea) {
                (0, _) => format!("army {army} stays in province {to}"),
                (n, false) => format!("army {army} marches to province {to}, {n} provinces"),
                (n, true) => format!("army {army} goes to province {to} by sea, {n} provinces"),
            })
        }
    }
}

/// Carries out every waiting order, oldest first (the start of the day).
pub fn apply(players: &mut Players, state: &mut State, core: &mut Core, research: &ResearchRules, politics: &PoliticsRules) -> Vec<Outcome> {
    apply_all(players, state, core, research, politics, None)
}

/// [`apply`] with the event runner, for the event and decision orders.
pub fn apply_all(players: &mut Players, state: &mut State, core: &mut Core, research: &ResearchRules, politics: &PoliticsRules, mut events: Option<EventSide>) -> Vec<Outcome> {
    players
        .take()
        .into_iter()
        .map(|(tag, order)| {
            let result = apply_one(state, core, research, politics, events.as_mut(), &tag, &order);
            Outcome { tag, order, result }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use wiktoria_data::date::Date;

    use super::*;
    use crate::military::battle::{Combat, CombatSide};
    use crate::military::movement::{Border, BorderKind, MoveProvince};
    use crate::military::units::Army;
    use crate::politics::PartyInfo;
    use crate::research::TechInfo;
    use crate::script::CountryState;

    fn state() -> State {
        let mut s = State::new(Date::new(1840, 1, 1));
        s.countries.insert("AAA".into(), CountryState { civilized: true, ruling_party: Some(7), ..CountryState::default() });
        s.countries.insert("BBB".into(), CountryState { civilized: true, ..CountryState::default() });
        s
    }

    fn politics() -> PoliticsRules {
        let party = PartyInfo { id: 7, ideology: "x".into(), start: None, end: None, min_tax: 0.2, max_tax: 0.8, max_military: 0.6, min_tariff: -0.5, max_tariff: 0.4, positions: Vec::new(), citizenship: Default::default() };
        PoliticsRules { parties: [("AAA".to_string(), vec![party])].into(), ..PoliticsRules::default() }
    }

    fn tech(name: &str, year: i32, previous: Option<&str>) -> TechInfo {
        TechInfo { name: name.into(), folder: "army".into(), year, cost: 100.0, ai_chance: None, previous: previous.map(str::to_string) }
    }

    fn research() -> ResearchRules {
        ResearchRules { techs: vec![tech("a1", 1836, None), tech("a2", 1836, Some("a1")), tech("a3", 1850, Some("a2"))], year_span: 50.0, max_daily: 10.0, min_cost: 1.0, store_days: 365.0, ..ResearchRules::default() }
    }

    #[test]
    fn sliders_map_to_the_market_keys() {
        for s in Slider::ALL {
            assert_eq!(Slider::parse(s.name()), Some(s));
        }
        assert_eq!(Slider::Administration.key(), "crime_fighting");
        assert_eq!(Slider::Tariff.key(), "tariffs");
        assert_eq!(Slider::parse("bogus"), None);
    }

    #[test]
    fn sliders_stay_inside_the_party_limits() {
        let (mut s, p) = (state(), politics());
        assert_eq!(slider_limits(&s, &p, "AAA", Slider::PoorTax), (0.2, 0.8));
        assert_eq!(slider_limits(&s, &p, "AAA", Slider::Military), (0.0, 0.6));
        assert_eq!(slider_limits(&s, &p, "AAA", Slider::Tariff), (-0.5, 0.4));
        assert_eq!(slider_limits(&s, &p, "AAA", Slider::Education), (0.0, 1.0));
        // No ruling party: only the slider's own range.
        assert_eq!(slider_limits(&s, &p, "BBB", Slider::Tariff), (-1.0, 1.0));
        assert_eq!(set_slider(&mut s, &p, "AAA", Slider::RichTax, 0.95), Ok(0.8));
        assert_eq!(set_slider(&mut s, &p, "AAA", Slider::Education, 0.35), Ok(0.35));
        assert_eq!(set_slider(&mut s, &p, "AAA", Slider::Tariff, -1.0), Ok(-0.5));
        assert_eq!(set_slider(&mut s, &p, "BBB", Slider::Social, 7.0), Ok(1.0));
        assert!(set_slider(&mut s, &p, "AAA", Slider::Social, f64::NAN).is_err());
        assert!(set_slider(&mut s, &p, "ZZZ", Slider::Social, 0.5).is_err());
        let c = &s.countries["AAA"];
        assert_eq!((c.spending["rich_tax"], c.spending["education_spending"], c.spending["tariffs"]), (0.8, 0.35, -0.5));
    }

    #[test]
    fn research_choice_follows_the_tree_and_keeps_points() {
        let (mut s, r) = (state(), research());
        let open: Vec<&str> = open_technologies(&s, &r, "AAA").iter().map(|t| t.name.as_str()).collect();
        assert_eq!(open, ["a1"]);
        assert!(choose_research(&mut s, &r, "AAA", "a2").unwrap_err().contains("a1"));
        assert!(choose_research(&mut s, &r, "AAA", "nope").is_err());
        s.countries.get_mut("AAA").unwrap().technologies.insert("a1".into());
        assert!(choose_research(&mut s, &r, "AAA", "a1").unwrap_err().contains("already"));
        assert!(choose_research(&mut s, &r, "AAA", "a3").unwrap_err().contains("1850"));
        // Switching puts the points already spent back into the store.
        s.countries.get_mut("AAA").unwrap().research = Some(("other".into(), 40.0));
        choose_research(&mut s, &r, "AAA", "a2").unwrap();
        let c = &s.countries["AAA"];
        assert_eq!((c.research.clone(), c.research_points), (Some(("a2".to_string(), 0.0)), 40.0));
        // The same choice again changes nothing.
        s.countries.get_mut("AAA").unwrap().research = Some(("a2".into(), 12.0));
        choose_research(&mut s, &r, "AAA", "a2").unwrap();
        assert_eq!(s.countries["AAA"].research, Some(("a2".to_string(), 12.0)));
        s.countries.get_mut("BBB").unwrap().civilized = false;
        assert!(choose_research(&mut s, &r, "BBB", "a1").is_err());
        assert!(open_technologies(&s, &r, "BBB").is_empty());
    }

    #[test]
    fn a_played_country_gets_no_research_from_the_computer() {
        let (mut s, r) = (state(), research());
        for (id, owner) in [(1, "AAA"), (2, "BBB")] {
            s.provinces.insert(id, crate::script::ProvinceState { owner: Some(owner.into()), controller: Some(owner.into()), ..Default::default() });
        }
        for c in s.countries.values_mut() {
            c.research_rate = 5.0;
        }
        s.countries.get_mut("AAA").unwrap().human = true;
        crate::research::research_day(&mut s, &r, &Default::default());
        // The computer picked one for BBB; AAA's player has not chosen, so its points are stored.
        assert!(s.countries["BBB"].research.is_some());
        assert_eq!((s.countries["AAA"].research.clone(), s.countries["AAA"].research_points), (None, 5.0));
        choose_research(&mut s, &r, "AAA", "a1").unwrap();
        crate::research::research_day(&mut s, &r, &Default::default());
        assert_eq!(s.countries["AAA"].research, Some(("a1".to_string(), 10.0)));
    }

    fn map() -> MoveMap {
        // 1 - 2 - 3 land in a row, 4 a sea next to 3, 5 behind an impassable border of 3.
        let mut m = MoveMap::default();
        for (id, sea) in [(1, false), (2, false), (3, false), (4, true), (5, false)] {
            m.provinces.insert(id, MoveProvince { sea, ..MoveProvince::default() });
        }
        for (a, b, kind) in [(1, 2, BorderKind::Land), (2, 3, BorderKind::Land), (3, 4, BorderKind::Land), (3, 5, BorderKind::Impassable)] {
            m.borders.insert((a, b), Border { kind, distance: 100.0, through: None });
            m.borders.insert((b, a), Border { kind, distance: 100.0, through: None });
        }
        m
    }

    fn movement() -> MovementRules {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let m = wiktoria_data::config::load_engine_rules(&root).expect("the repo's engine_rules");
        let defines = wiktoria_data::defines::Defines::parse("defines = { military = { LAND_SPEED_MODIFIER = 1 NAVAL_SPEED_MODIFIER = 1 } }").expect("defines");
        MovementRules::new(&defines, &m).expect("movement rules")
    }

    fn mil(armies: Vec<Army>, navies: Vec<Navy>) -> MilitaryState {
        MilitaryState { date: Date::new(1840, 1, 1), countries: Default::default(), leaders: vec![], armies, navies, constructions: vec![], wars: vec![], combats: vec![], provinces: Default::default(), warnings: vec![] }
    }

    /// Every ship carries one regiment in these tests.
    fn ships(n: &Navy) -> usize {
        n.ships.len()
    }

    #[test]
    fn armies_move_over_land_and_not_out_of_a_young_battle() {
        let (map, rules) = (map(), movement());
        let mut m = mil(vec![Army { id: 9, owner: "AAA".into(), location: 1, ..Army::default() }, Army { id: 10, owner: "BBB".into(), location: 1, ..Army::default() }], vec![]);
        assert_eq!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 3), Ok((2, false)));
        assert_eq!(m.armies[0].path, vec![2, 3]);
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 10, 3).unwrap_err().contains("not AAA"));
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 4).is_err(), "sea");
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 5).is_err(), "impassable");
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 77, 2).is_err(), "unknown army");
        assert_eq!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 1), Ok((0, false)));
        assert!(m.armies[0].path.is_empty());
        let mut battle = Combat { kind: CombatKind::Land, location: 1, day: 0.0, duration: 1.0, total: 0.0, attacker: CombatSide::default(), defender: CombatSide::default() };
        battle.defender.units.push(9);
        m.combats = vec![battle.clone()];
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 2).unwrap_err().contains("battle"));
        battle.duration = rules.order_battle_min_days + 1.0;
        m.combats = vec![battle];
        assert_eq!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 2), Ok((1, false)));
    }

    #[test]
    fn an_order_over_water_needs_a_fleet_with_room() {
        // Island 6 lies off the sea 4, next to province 3.
        let mut map = map();
        map.provinces.insert(6, MoveProvince::default());
        for (a, b) in [(4, 6), (6, 4)] {
            map.borders.insert((a, b), Border { kind: BorderKind::Land, distance: 100.0, through: None });
        }
        let rules = movement();
        let regiment = crate::military::units::Regiment::default;
        let army = Army { id: 9, owner: "AAA".into(), location: 1, regiments: vec![regiment(), regiment()], ..Army::default() };
        let ship = crate::military::units::Ship::default;
        let mut m = mil(vec![army], vec![Navy { id: 50, owner: "AAA".into(), location: 4, ships: vec![ship()], ..Navy::default() }]);
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 6).unwrap_err().contains("2 regiments"), "one ship carries one regiment");
        assert!(m.armies[0].path.is_empty());
        m.navies[0].ships.push(ship());
        assert_eq!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 6), Ok((4, true)));
        assert_eq!((m.armies[0].path.clone(), m.armies[0].target), (vec![2, 3, 4, 6], Some(6)));
        // On board a standing fleet the army is routed from the fleet's sea province; under way it is not.
        m.armies[0].carried_by = Some(50);
        assert_eq!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 3), Ok((1, true)));
        assert_eq!(m.armies[0].path, vec![3]);
        m.navies[0].path = vec![4];
        assert!(move_army(&mut m, &map, &rules, &ships, &|_, _| true, "AAA", 9, 6).unwrap_err().contains("under way"));
    }

    #[test]
    fn the_queue_keeps_orders_until_the_day_starts() {
        let mut p = Players::default();
        p.push("AAA", Order::Budget { slider: Slider::PoorTax, value: 0.5 });
        p.push("BBB", Order::Research { tech: "a1".into() });
        assert_eq!(p.pending().len(), 2);
        let taken = p.take();
        assert_eq!(taken[0].0, "AAA");
        assert_eq!(taken[1].1, Order::Research { tech: "a1".into() });
        assert!(p.pending().is_empty());
    }
}
