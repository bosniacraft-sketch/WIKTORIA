//! Elections and the upper house.
//!
//! A country whose government holds elections (`election = yes` in the governments file) and
//! where somebody may vote ends an election `duration` + CAMPAIGN_DURATION months after the last
//! one, on the 1st of the month (measured: 1836.1.1, 1840.7.1, 1845.1.1, 1849.7.1 ...). The
//! party with the most votes becomes the ruling party.
//!
//! Votes, counted province by province outside colonies:
//! * a pop's weight is its size x the vote of its strata under the country's voting franchise
//!   (`rich_vote` / `middle_vote` / `poor_vote` of the reform); slaves have none. The ruling
//!   party's citizenship policy cuts it for pops that are neither of the primary nor of an
//!   accepted culture: to nothing under `primary_culture_voting`, to half under `culture_voting`
//!   (the description is unsure which of the two rules does which);
//! * the pop's support for a party = the average of the pop's issue shares for the party's own
//!   issue positions + its consciousness x its share for the party's ideology / 20; for the
//!   ruling party x (1 + `ruling_party_support` of the country's and the province's modifiers);
//!   never below 0. The description does not settle the scales of the two shares: the port takes
//!   the issue shares as fractions and the ideology share in percent, the pairing that
//!   reproduces most of the original's election winners (176 of 209 in the saves of 1836-65,
//!   against 155 with both as fractions and 160 for the old rule). The original also multiplies
//!   by 1 + the province's party loyalty for the ideology; the port keeps no party loyalty and
//!   uses 1;
//! * the pop's supports are rescaled to add up to 1 and its weight is split by them;
//! * the country's voting system decides what a province hands on: under `largest_share` the
//!   province's winner gets all its votes, under `dhont` only parties above
//!   JEFFERSON_THRESHOLD of the province keep theirs, under `sainte_laque` every party does;
//! * when the country's average militancy is below CALM_MILITANCY the votes are first added up
//!   by ideology, the largest ideology wins and its largest party rules; otherwise the party
//!   with the most votes does.
//!
//! Parties that count: those of the country that exist at the date and whose ideology the
//! government allows.
//!
//! The upper house is filled on 1 January by the rule of the country's upper house reform:
//! the ruling party's ideology alone, the ideologies of the rich, the average of the states'
//! votes, or the country's votes (size x strata vote x ideology share, as before: exact on the
//! original's saves).
//!
//! The ruling party's policies limit the sliders: taxes stay within `min_tax` .. `max_tax` of its
//! economic policy, military spending below `max_military_spending` of its war policy and the
//! tariff within `min_tariff` .. `max_tariff` of its trade policy (the party issues of the issues
//! file). The port pulls the sliders back inside the limits on the 1st of each month.
//!
//! Not included (D15): campaign events moving the issues, party loyalty and the loyalty a province's
//! winner gains, the province modifier for the number of voters, the rule that a party with a
//! stricter citizenship policy than the ruling party gets nothing from pops of other cultures
//! (inferred only in the description), the player appointing a party.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::script::{Block, Value};
use wiktoria_data::world::World;

use crate::script::State;

#[derive(Debug, Clone, PartialEq)]
pub struct PartyInfo {
    /// Number of the party in a save (position among all parties of all countries + 1).
    pub id: u32,
    pub ideology: String,
    pub start: Option<Date>,
    pub end: Option<Date>,
    /// Lowest and highest tax and highest military spending its policies allow.
    pub min_tax: f64,
    pub max_tax: f64,
    pub max_military: f64,
    /// Lowest and highest tariff its trade policy allows.
    pub min_tariff: f64,
    pub max_tariff: f64,
    /// Its issue positions: the option it stands for in each party issue group.
    pub positions: Vec<String>,
    /// Who votes in full while it rules.
    pub citizenship: Citizenship,
}

/// The rule of a citizenship policy.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Citizenship {
    /// `primary_culture_voting`: pops that are not of the primary or an accepted culture do not vote.
    Primary,
    /// `culture_voting`: their votes count half.
    Accepted,
    /// `all_voting`, or no rule.
    #[default]
    All,
}

/// How a province's votes enter the country's count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VotingSystem {
    /// `largest_share` (first past the post): the province's winner takes all its votes.
    LargestShare,
    /// `dhont` (Jefferson method): parties under a threshold share of the province lose their votes.
    Jefferson,
    /// `sainte_laque` (proportional): every party keeps its votes.
    Proportional,
}

// rules file: FLD_VOTE_IDEOLOGY_DIVISOR - a pop's support for a party gets consciousness x ideology share / 20 on top of its issue shares.
const IDEOLOGY_DIVISOR: f64 = 20.0;
// rules file: FLD_VOTE_IDEOLOGY_SHARE_SCALE - the ideology share counts in percent (x100) against issue shares as fractions; the port's choice, fitted to the original's election winners.
const IDEOLOGY_SHARE_SCALE: f64 = 100.0;
// rules file: FLD_VOTE_CALM_MILITANCY - under this average militancy the largest ideology wins the election first.
const CALM_MILITANCY: f64 = 5.0;
// rules file: FLD_VOTE_JEFFERSON_THRESHOLD - share of a province's votes a party needs under the Jefferson method; the original's constant was not read, 5% is the port's guess.
const JEFFERSON_THRESHOLD: f64 = 0.05;
// rules file: FLD_VOTE_HALF_CITIZEN - weight of a pop of another culture under `culture_voting`.
const HALF_CITIZEN: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Government {
    pub election: bool,
    /// Months between the end of one election and the start of the next campaign.
    pub duration: i32,
    pub ideologies: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HouseRule {
    RulingParty,
    RichOnly,
    StateVote,
    PopulationVote,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PoliticsRules {
    pub campaign_months: i32,
    pub governments: BTreeMap<String, Government>,
    /// Voting franchise option -> votes of a poor, middle and rich person.
    pub franchise: BTreeMap<String, [f64; 3]>,
    /// Name of the franchise reform and of the upper house reform.
    pub franchise_reform: String,
    pub house_reform: String,
    pub house: BTreeMap<String, HouseRule>,
    pub parties: BTreeMap<String, Vec<PartyInfo>>,
    /// Name of the voting system reform and the rule of each of its options.
    pub voting_reform: String,
    pub systems: BTreeMap<String, VotingSystem>,
    /// Reform option, national value or timed modifier -> its `ruling_party_support`.
    pub ruling_support: BTreeMap<String, f64>,
}

impl PoliticsRules {
    /// `governments` and `issues` are the roots of the two files.
    pub fn new(world: &World, governments: &Block, issues: &Block) -> Result<PoliticsRules, String> {
        let mut rules = PoliticsRules { campaign_months: world.defines.get("country", "CAMPAIGN_DURATION").ok_or("defines.country.CAMPAIGN_DURATION missing")? as i32, ..Default::default() };
        for (name, v, _) in governments.entries() {
            let Some(b) = v.as_block() else { continue };
            let ideologies = b.entries().filter(|(k, v, _)| v.as_bool() == Some(true) && !matches!(*k, "election" | "appoint_ruling_party")).map(|(k, _, _)| k.to_string()).collect();
            rules.governments.insert(name.to_string(), Government { election: b.get("election").and_then(Value::as_bool).unwrap_or(false), duration: b.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as i32, ideologies });
        }
        for (_, category, _) in issues.entries() {
            let Some(category) = category.as_block() else { continue };
            for (reform, options, _) in category.entries() {
                let Some(options) = options.as_block() else { continue };
                for (option, b, _) in options.entries() {
                    let Some(b) = b.as_block() else { continue };
                    if let Some(rich) = b.get("rich_vote").and_then(Value::as_f64) {
                        let vote = |k: &str| b.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                        rules.franchise.insert(option.to_string(), [vote("poor_vote"), vote("middle_vote"), rich]);
                        rules.franchise_reform = reform.to_string();
                    }
                    let Some(r) = b.get("rules").and_then(Value::as_block) else { continue };
                    let yes = |k: &str| r.get(k).and_then(Value::as_bool) == Some(true);
                    let system = if yes("largest_share") {
                        Some(VotingSystem::LargestShare)
                    } else if yes("dhont") {
                        Some(VotingSystem::Jefferson)
                    } else if yes("sainte_laque") {
                        Some(VotingSystem::Proportional)
                    } else {
                        None
                    };
                    if let Some(system) = system {
                        rules.systems.insert(option.to_string(), system);
                        rules.voting_reform = reform.to_string();
                    }
                    let rule = if yes("same_as_ruling_party") {
                        Some(HouseRule::RulingParty)
                    } else if yes("rich_only") {
                        Some(HouseRule::RichOnly)
                    } else if yes("state_vote") {
                        Some(HouseRule::StateVote)
                    } else if yes("population_vote") {
                        Some(HouseRule::PopulationVote)
                    } else {
                        None
                    };
                    if let Some(rule) = rule {
                        rules.house.insert(option.to_string(), rule);
                        rules.house_reform = reform.to_string();
                    }
                }
            }
        }
        let lib = &world.defs.modifiers;
        rules.ruling_support = lib.issue_options.iter().chain(&lib.national_values).chain(&lib.event_modifiers).filter_map(|(o, v)| Some((o.clone(), *v.get("ruling_party_support")?))).collect();
        // Party issue option -> the numbers written in it, and its citizenship rule if it has one.
        let mut policy: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        let mut citizens: BTreeMap<String, Citizenship> = BTreeMap::new();
        for (_, groups, _) in issues.entries().filter(|(k, _, _)| *k == "party_issues") {
            for (_, options, _) in groups.as_block().into_iter().flat_map(|b| b.entries()) {
                for (option, b, _) in options.as_block().into_iter().flat_map(|b| b.entries()) {
                    let voting = |k: &str| b.as_block().and_then(|b| b.get("rules")).and_then(Value::as_block).and_then(|r| r.get(k)).and_then(Value::as_bool) == Some(true);
                    if voting("primary_culture_voting") {
                        citizens.insert(option.to_string(), Citizenship::Primary);
                    } else if voting("culture_voting") {
                        citizens.insert(option.to_string(), Citizenship::Accepted);
                    }
                    policy.insert(option.to_string(), b.as_block().map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect()).unwrap_or_default());
                }
            }
        }
        let mut id = 0;
        for c in &world.countries {
            let list = c.country.parties.iter().map(|p| {
                id += 1;
                let limit = |key: &str, default: f64| p.policies.iter().filter_map(|(_, option)| policy.get(option)?.get(key).copied()).next().unwrap_or(default);
                PartyInfo { id, ideology: p.ideology.clone().unwrap_or_default(), start: p.start_date, end: p.end_date, min_tax: limit("min_tax", 0.0), max_tax: limit("max_tax", 1.0), max_military: limit("max_military_spending", 1.0), min_tariff: limit("min_tariff", -1.0), max_tariff: limit("max_tariff", 1.0), positions: p.policies.iter().map(|(_, option)| option.clone()).collect(), citizenship: p.policies.iter().filter_map(|(_, option)| citizens.get(option).copied()).next().unwrap_or_default() }
            });
            rules.parties.insert(c.country.tag.clone(), list.collect());
        }
        Ok(rules)
    }
}

fn months(d: Date) -> i32 {
    d.year * 12 + i32::from(d.month)
}

fn strata_index(s: Option<&String>) -> usize {
    match s.map(String::as_str) {
        Some("rich") => 2,
        Some("middle") => 1,
        _ => 0,
    }
}

/// Votes by ideology of the country's pops outside colonies, per state (region number).
/// `weights` are the votes of a poor, middle and rich person.
fn votes(state: &State, tag: &str, weights: [f64; 3]) -> BTreeMap<u16, BTreeMap<String, f64>> {
    let mut by_state: BTreeMap<u16, BTreeMap<String, f64>> = BTreeMap::new();
    for p in state.provinces.values() {
        if p.owner.as_deref() != Some(tag) || p.colonial != 0 {
            continue;
        }
        let tally = by_state.entry(p.region.unwrap_or(u16::MAX)).or_default();
        for q in p.pops.iter().filter(|q| !state.slave_types.contains(&q.pop_type)) {
            let w = q.size * weights[strata_index(state.strata.get(&q.pop_type))];
            if w <= 0.0 {
                continue;
            }
            for (n, share) in &q.ideology {
                if let Some(name) = n.checked_sub(1).and_then(|i| state.ideologies.get(i as usize)) {
                    *tally.entry(name.clone()).or_default() += w * share / 100.0;
                }
            }
        }
    }
    by_state
}

fn total(by_state: &BTreeMap<u16, BTreeMap<String, f64>>) -> BTreeMap<String, f64> {
    let mut all = BTreeMap::new();
    for tally in by_state.values() {
        for (k, v) in tally {
            *all.entry(k.clone()).or_default() += v;
        }
    }
    all
}

/// The votes of each party of the country that counts today (by party number), after the
/// voting system's rule per province, and the country's average militancy.
pub fn party_votes(state: &State, rules: &PoliticsRules, tag: &str) -> Option<(BTreeMap<u32, f64>, f64)> {
    let c = state.countries.get(tag)?;
    let government = rules.governments.get(c.government.as_deref()?)?;
    let weights = *rules.franchise.get(c.reforms.get(&rules.franchise_reform)?)?;
    let today = state.date;
    let all = rules.parties.get(tag)?;
    let parties: Vec<&PartyInfo> = all.iter().filter(|p| government.ideologies.contains(&p.ideology) && p.start.is_none_or(|d| d <= today) && p.end.is_none_or(|d| today < d)).collect();
    let ruling = c.ruling_party.and_then(|id| all.iter().find(|p| p.id == id));
    let citizenship = ruling.map_or(Citizenship::All, |p| p.citizenship);
    let system = c.reforms.get(&rules.voting_reform).and_then(|o| rules.systems.get(o)).copied().unwrap_or(VotingSystem::Proportional);
    let number = |name: &str| state.issues.iter().position(|(o, _)| o == name).map(|i| i as u32 + 1);
    // Per party: the numbers of its issue positions, of its ideology, and whether it rules.
    let known: Vec<(Vec<u32>, Option<u32>, bool)> = parties.iter().map(|p| (p.positions.iter().filter_map(|o| number(o)).collect(), state.ideologies.iter().position(|i| *i == p.ideology).map(|i| i as u32 + 1), ruling.is_some_and(|r| r.id == p.id))).collect();
    let support_of = |names: &mut dyn Iterator<Item = &String>| names.filter_map(|n| rules.ruling_support.get(n)).sum::<f64>();
    let country_support = support_of(&mut c.reforms.values().chain(&c.national_value).chain(c.modifiers.iter().map(|m| &m.name)));
    let mut votes: BTreeMap<u32, f64> = BTreeMap::new();
    let (mut people, mut militancy) = (0.0, 0.0);
    for p in state.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
        for q in &p.pops {
            people += q.size;
            militancy += q.size * q.militancy;
        }
        if p.colonial != 0 {
            continue;
        }
        let incumbent = 1.0 + country_support + support_of(&mut p.modifiers.iter().map(|m| &m.name));
        let mut tally = vec![0.0; parties.len()];
        for q in p.pops.iter().filter(|q| !state.slave_types.contains(&q.pop_type)) {
            let citizen = c.primary_culture.as_deref() == Some(q.culture.as_str()) || c.accepted_cultures.contains(&q.culture);
            let cut = match citizenship {
                _ if citizen => 1.0,
                Citizenship::Primary => 0.0,
                Citizenship::Accepted => HALF_CITIZEN,
                Citizenship::All => 1.0,
            };
            let w = q.size * weights[strata_index(state.strata.get(&q.pop_type))] * cut;
            if w <= 0.0 {
                continue;
            }
            let share = |map: &BTreeMap<u32, f64>, n: &u32| map.get(n).copied().unwrap_or(0.0) / 100.0;
            let support: Vec<f64> = known
                .iter()
                .map(|(positions, ideology, rules_now)| {
                    let s = positions.iter().map(|n| share(&q.issues, n)).sum::<f64>() / positions.len().max(1) as f64 + q.consciousness * IDEOLOGY_SHARE_SCALE * ideology.as_ref().map_or(0.0, |n| share(&q.ideology, n)) / IDEOLOGY_DIVISOR;
                    (s * if *rules_now { incumbent } else { 1.0 }).max(0.0)
                })
                .collect();
            let sum: f64 = support.iter().sum();
            if sum > 0.0 {
                tally.iter_mut().zip(&support).for_each(|(t, s)| *t += w * s / sum);
            }
        }
        let cast: f64 = tally.iter().sum();
        if cast <= 0.0 {
            continue;
        }
        match system {
            VotingSystem::LargestShare => {
                let best = (0..tally.len()).max_by(|a, b| tally[*a].total_cmp(&tally[*b]))?;
                *votes.entry(parties[best].id).or_default() += cast;
            }
            VotingSystem::Jefferson | VotingSystem::Proportional => {
                for (party, v) in parties.iter().zip(&tally).filter(|(_, v)| system == VotingSystem::Proportional || **v > JEFFERSON_THRESHOLD * cast) {
                    *votes.entry(party.id).or_default() += v;
                }
            }
        }
    }
    Some((votes, if people > 0.0 { militancy / people } else { 0.0 }))
}

/// The party a country's voters would elect today, with its share of the votes that count.
pub fn election_winner(state: &State, rules: &PoliticsRules, tag: &str) -> Option<(u32, f64)> {
    let (votes, militancy) = party_votes(state, rules, tag)?;
    let all: f64 = votes.values().sum();
    let ideology_of = |id: &u32| rules.parties.get(tag).and_then(|l| l.iter().find(|p| p.id == *id)).map(|p| p.ideology.as_str());
    let mut running: Vec<(&u32, &f64)> = votes.iter().filter(|(_, v)| **v > 0.0).collect();
    if militancy < CALM_MILITANCY {
        let mut by_ideology: BTreeMap<&str, f64> = BTreeMap::new();
        for (id, v) in &running {
            *by_ideology.entry(ideology_of(id)?).or_default() += **v;
        }
        let largest = by_ideology.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?.0;
        running.retain(|(id, _)| ideology_of(id) == Some(largest));
    }
    running.into_iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|(id, v)| (*id, v / all))
}

/// The upper house a country would get today, in shares that add up to 1.
pub fn upper_house(state: &State, rules: &PoliticsRules, tag: &str) -> Option<BTreeMap<String, f64>> {
    let c = state.countries.get(tag)?;
    let rule = *rules.house.get(c.reforms.get(&rules.house_reform)?)?;
    let franchise = c.reforms.get(&rules.franchise_reform).and_then(|o| rules.franchise.get(o)).copied().unwrap_or([0.0; 3]);
    let mut house: BTreeMap<String, f64> = BTreeMap::new();
    match rule {
        HouseRule::RulingParty => {
            let ideology = state.party_ideologies.get(c.ruling_party?.checked_sub(1)? as usize)?;
            house.insert(ideology.clone(), 1.0);
        }
        HouseRule::RichOnly => house = total(&votes(state, tag, [0.0, 0.0, 1.0])),
        HouseRule::PopulationVote => house = total(&votes(state, tag, franchise)),
        HouseRule::StateVote => {
            for tally in votes(state, tag, franchise).values() {
                let all: f64 = tally.values().sum();
                if all > 0.0 {
                    for (k, v) in tally {
                        *house.entry(k.clone()).or_default() += v / all;
                    }
                }
            }
        }
    }
    let all: f64 = house.values().sum();
    if all <= 0.0 {
        return None;
    }
    house.values_mut().for_each(|v| *v /= all);
    Some(house)
}

/// The votes of the country's voters by ideology, in shares that add up to 1 (read-only, for the
/// interface): its pops outside colonies weighted by their strata's vote under its franchise.
pub fn voter_ideologies(state: &State, rules: &PoliticsRules, tag: &str) -> Option<BTreeMap<String, f64>> {
    let c = state.countries.get(tag)?;
    let franchise = *rules.franchise.get(c.reforms.get(&rules.franchise_reform)?)?;
    let mut all = total(&votes(state, tag, franchise));
    let sum: f64 = all.values().sum();
    if sum <= 0.0 {
        return None;
    }
    all.values_mut().for_each(|v| *v /= sum);
    Some(all)
}

/// The day the country's next election ends (read-only, for the interface); `None` when its
/// government holds none or no election has been held yet.
pub fn next_election(state: &State, rules: &PoliticsRules, tag: &str) -> Option<Date> {
    let c = state.countries.get(tag)?;
    let g = rules.governments.get(c.government.as_deref()?).filter(|g| g.election)?;
    let at = months(c.last_election?) + g.duration + rules.campaign_months - 1;
    Some(Date::new(at.div_euclid(12), at.rem_euclid(12) as u8 + 1, 1))
}

/// What happened on a day: elections held as (country, old ruling party, new ruling party).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PoliticsDay {
    pub elections: Vec<(String, Option<u32>, u32)>,
}

/// Elections that end today and, on 1 January, the upper houses. Does nothing on other days
/// than the 1st of a month.
pub fn politics_day(state: &mut State, rules: &PoliticsRules) -> PoliticsDay {
    let mut day = PoliticsDay::default();
    if state.date.day != 1 {
        return day;
    }
    let today = state.date;
    let owners: BTreeSet<String> = state.provinces.values().filter_map(|p| p.owner.clone()).collect();
    for tag in owners {
        let Some(c) = state.countries.get(&tag) else { continue };
        let due = c.government.as_deref().and_then(|g| rules.governments.get(g)).filter(|g| g.election).is_some_and(|g| c.last_election.is_some_and(|last| months(today) >= months(last) + g.duration + rules.campaign_months));
        let old = c.ruling_party;
        let winner = if due { election_winner(state, rules, &tag) } else { None };
        let house = if today.month == 1 || winner.is_some() { upper_house(state, rules, &tag) } else { None };
        let Some(c) = state.countries.get_mut(&tag) else { continue };
        if let Some((party, _)) = winner {
            c.ruling_party = Some(party);
            c.last_election = Some(today);
            day.elections.push((tag.clone(), old, party));
        }
        if let Some(house) = house {
            c.upper_house = house;
        }
        if let Some(party) = c.ruling_party.and_then(|id| rules.parties.get(&tag)?.iter().find(|p| p.id == id)) {
            c.party_limits = Some([party.min_tax, party.max_tax, party.max_military, party.min_tariff, party.max_tariff]);
            for tax in ["rich_tax", "middle_tax", "poor_tax"] {
                if let Some(x) = c.spending.get_mut(tax) {
                    *x = x.clamp(party.min_tax, party.max_tax.max(party.min_tax));
                }
            }
            if let Some(x) = c.spending.get_mut("military_spending") {
                *x = x.min(party.max_military);
            }
            if let Some(x) = c.spending.get_mut("tariffs") {
                *x = x.clamp(party.min_tariff, party.max_tariff.max(party.min_tariff));
            }
        }
    }
    day
}
