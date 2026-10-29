//! The war and peace orders of a played country, and the questions the computer puts to one
//! (player.rs has the rules in words). Everything goes through the computer's own war code:
//! `Core::start_war` declares, `Core::peace_answer` answers, `Core::execute_peace` signs.

use std::collections::BTreeSet;

use super::{Core, CoreLog, PeaceCtx};
use crate::military::ai::Sides;
use crate::military::war::{self, PeaceRequest, WarEvent, WarEventKind, WarGoal};
use crate::military::war_ai;
use crate::player::{days_after, is_player, Ask, AskKind};
use crate::script::State;

impl Core {
    fn check_target(&self, state: &State, tag: &str, target: &str) -> Result<(), String> {
        if target == tag {
            return Err("a country cannot go to war with itself".into());
        }
        if target == self.day.rebel_tag || !state.countries.contains_key(target) {
            return Err(format!("no country {target}"));
        }
        if !state.provinces.values().any(|p| p.owner.as_deref() == Some(target)) {
            return Err(format!("{target} owns no land"));
        }
        Ok(())
    }

    fn at_war_with(&self, a: &str, b: &str) -> bool {
        Sides::from_wars(&self.mil.wars, &self.day.rebel_tag).at_war_with(a, b)
    }

    /// The casus belli types a country may start to justify: those that are built (the same
    /// list the computer justifies from).
    pub fn justifiable_cbs(&self) -> Vec<&war::CbType> {
        self.day.cb_types.values().filter(|c| c.constructing && !c.needs_no_justification).collect()
    }

    /// `tag` starts justifying a casus belli of type `cb` against `target`.
    pub(crate) fn player_justify(&mut self, state: &mut State, tag: &str, target: &str, cb: &str) -> Result<String, String> {
        if self.war_ai.is_none() {
            return Err("wars are switched off in this game".into());
        }
        self.check_target(state, tag, target)?;
        if !self.justifiable_cbs().iter().any(|c| c.name == cb) {
            return Err(format!("{cb} is not a casus belli that can be justified"));
        }
        if let Some(((_, t), (k, _, _))) = self.justifying.iter().find(|((a, _), _)| a == tag) {
            return Err(format!("{tag} is already justifying {k} against {t}"));
        }
        if self.at_war_with(tag, target) {
            return Err(format!("{tag} is at war with {target}"));
        }
        if self.dip.as_ref().is_some_and(|(_, d)| d.truce_active(tag, target, state.date)) {
            return Err(format!("{tag} has a truce with {target}"));
        }
        if self.held_cbs.get(&(tag.to_string(), target.to_string())).is_some_and(|v| v.iter().any(|h| h.cb_type == cb)) {
            return Err(format!("{tag} already holds {cb} against {target}"));
        }
        self.justifying.insert((tag.to_string(), target.to_string()), (cb.to_string(), 0.0, false));
        Ok(format!("justifying {cb} against {target}"))
    }

    /// `tag` declares war on `target` with the casus belli `cb` it holds against it.
    pub(crate) fn player_declare(&mut self, state: &mut State, tag: &str, target: &str, cb: &str) -> Result<String, String> {
        let Some(r) = self.war_ai.clone() else { return Err("wars are switched off in this game".into()) };
        self.check_target(state, tag, target)?;
        if self.at_war_with(tag, target) {
            return Err(format!("{tag} is already at war with {target}"));
        }
        if !self.held_cbs.get(&(tag.to_string(), target.to_string())).is_some_and(|v| v.iter().any(|h| h.cb_type == cb)) {
            return Err(format!("{tag} holds no {cb} against {target}: justify one first"));
        }
        let Some((rules, mut dip)) = self.dip.take() else { return Err("no diplomacy in this game".into()) };
        let refusal = if dip.allied(tag, target) { Some(format!("{tag} is allied to {target}")) } else { None };
        let out = match refusal {
            Some(e) => Err(e),
            None => {
                let truce = dip.truce_active(tag, target, state.date);
                let mut at_war: BTreeSet<String> = self.mil.wars.iter().filter(|w| w.active).flat_map(|w| w.attackers.iter().chain(&w.defenders)).cloned().collect();
                self.start_war(state, &rules, &mut dip, &r, tag, target, cb, truce, &mut at_war, &mut CoreLog::default())
            }
        };
        self.dip = Some((rules, dip));
        out.map(|name| format!("war declared: {name}"))
    }

    /// The index of the running war named `name`.
    fn war_index(&self, name: &str) -> Result<usize, String> {
        self.mil.wars.iter().position(|w| w.active && w.name == name).ok_or(format!("no war {name} is running"))
    }

    /// The facts of war `i` the peace answers read (as the foreign minister makes them).
    fn peace_ctx(&self, state: &State, i: usize) -> PeaceCtx<'_> {
        let w = &self.mil.wars[i];
        let score = self.day.war_scores(&self.mil, state).into_iter().find(|(n, _)| *n == w.name).map_or(0.0, |x| x.1);
        let today = state.date;
        let months = w.started().map_or(0.0, |s| f64::from((today.year - s.year) * 12 + i32::from(today.month) - i32::from(s.month)));
        PeaceCtx { war: w, months, score, direction: self.war_direction.get(&w.name).map_or(0.0, |d| d.value) }
    }

    /// Checks a peace `sender` would offer `target` in war `i` enforcing `goals`: the two may
    /// negotiate, and the goals are all demanded of the target or all conceded to it, as the
    /// computer's own table allows them. Returns the goals sorted.
    pub fn check_offer(&self, i: usize, sender: &str, target: &str, goals: &[usize]) -> Result<Vec<usize>, String> {
        let w = &self.mil.wars[i];
        let mut kept: Vec<usize> = goals.to_vec();
        kept.sort_unstable();
        kept.dedup();
        if let Some(g) = kept.iter().find(|g| **g >= w.goals.len()) {
            return Err(format!("the war has no goal {g}"));
        }
        let req = PeaceRequest { war: w, crisis: false, sender, target, selected: &kept };
        war::peace_allowed(&req, &self.day.cb_types).map_err(|e| format!("{sender} and {target} cannot make this peace ({e:?})"))?;
        let side_of = |t: &str| w.attackers.iter().any(|x| x == t);
        let (demands, concedes) = if kept.iter().all(|&g| side_of(&w.goals[g].actor) == side_of(sender)) {
            (sender, target)
        } else if kept.iter().all(|&g| side_of(&w.goals[g].actor) == side_of(target)) {
            (target, sender)
        } else {
            return Err("a peace either demands goals or gives them, not both".into());
        };
        let (table, forced) = Self::peace_table(w, &self.day.cb_types, demands, concedes).ok_or("a separate peace that ends every goal must annex, and none can")?;
        if let Some(g) = kept.iter().find(|g| !table.contains(g)) {
            return Err(format!("goal {g} is not one {demands} can ask of {concedes}"));
        }
        if let Some(g) = forced.iter().find(|g| !kept.contains(g)) {
            return Err(format!("goal {g} must be in this peace"));
        }
        Ok(kept)
    }

    /// Signs peace in war `i` and brings the list of running wars up to date.
    #[allow(clippy::too_many_arguments)]
    fn sign_peace(&mut self, state: &mut State, i: usize, sender: &str, target: &str, kept: &[usize], why: &str, log: &mut CoreLog) {
        self.execute_peace(state, i, sender, target, kept, why, log);
        state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
    }

    /// `tag` offers `to` peace in the war named `war`. The computer answers at once; a played
    /// country is asked.
    pub(crate) fn player_offer_peace(&mut self, state: &mut State, tag: &str, war_name: &str, to: &str, goals: &[usize]) -> Result<String, String> {
        let i = self.war_index(war_name)?;
        let kept = self.check_offer(i, tag, to, goals)?;
        if is_player(state, to) {
            if !self.asks.may_ask_peace(war_name, tag, to, state.date) {
                return Err(format!("{to} has an offer of this war to answer already"));
            }
            let g: Vec<WarGoal> = kept.iter().map(|&k| self.mil.wars[i].goals[k].clone()).collect();
            self.asks.peace(to, tag, war_name, g, state.date, &self.day.calendar);
            return Ok(format!("peace offered to {to}, who plays it themselves"));
        }
        let accepts = self.peace_answer(state, &self.peace_ctx(state, i), to, tag, &kept).0;
        if !accepts {
            return Err(format!("{to} refuses the peace"));
        }
        self.sign_peace(state, i, tag, to, &kept, "a player's offer", &mut CoreLog::default());
        Ok(format!("{to} accepts: peace in {war_name}"))
    }

    /// The player's answer to a question.
    pub(crate) fn player_answer(&mut self, state: &mut State, tag: &str, key: u64, accept: bool) -> Result<String, String> {
        if !self.asks.list.iter().any(|a| a.key == key && a.to == tag) {
            return Err(format!("no question {key} waits for {tag}"));
        }
        self.resolve_ask(state, key, Some(accept), &mut CoreLog::default())
    }

    /// Answers a question: `accept` as the player says, or with `None` as the computer would.
    /// The question goes unless an acceptance cannot be carried out (then it stays and the
    /// reason is returned); one whose war has ended goes in any case.
    fn resolve_ask(&mut self, state: &mut State, key: u64, accept: Option<bool>, log: &mut CoreLog) -> Result<String, String> {
        let Some(at) = self.asks.list.iter().position(|a| a.key == key) else { return Err(format!("no question {key}")) };
        let ask: Ask = self.asks.list[at].clone();
        let Ok(i) = self.war_index(&ask.war) else {
            self.asks.list.remove(at);
            return Err(format!("{} is over", ask.war));
        };
        let gone = |me: &mut Core, why: String| {
            me.asks.list.retain(|a| a.key != key);
            Err(why)
        };
        match &ask.kind {
            AskKind::Peace { goals } => {
                let w = &self.mil.wars[i];
                if !(w.takes_part(&ask.from) && w.takes_part(&ask.to)) {
                    return gone(self, format!("{} or {} has left {}", ask.from, ask.to, ask.war));
                }
                // The goals as the war holds them now (a separate peace may have taken some out).
                // Every offered goal must still be there: a smaller peace is not what was asked.
                let mut kept: Vec<usize> = Vec::new();
                for g in goals {
                    match (0..w.goals.len()).find(|&k| !kept.contains(&k) && { let x = &w.goals[k]; x.casus_belli == g.casus_belli && x.actor == g.actor && x.receiver == g.receiver && x.state_province == g.state_province }) {
                        Some(k) => kept.push(k),
                        None => return gone(self, "the offer no longer fits the war".into()),
                    }
                }
                let Ok(kept) = self.check_offer(i, &ask.from, &ask.to, &kept) else { return gone(self, "the offer no longer fits the war".into()) };
                let yes = accept.unwrap_or_else(|| self.peace_answer(state, &self.peace_ctx(state, i), &ask.to, &ask.from, &kept).0);
                self.asks.list.retain(|a| a.key != key);
                if yes {
                    let why = if accept.is_some() { "accepted by its player" } else { "accepted for its player by the computer" };
                    self.sign_peace(state, i, &ask.from, &ask.to, &kept, why, log);
                    Ok(format!("peace with {} in {}", ask.from, ask.war))
                } else {
                    let until = days_after(&self.day.calendar, state.date, self.asks.answer_days);
                    self.asks.quiet.insert((ask.war.clone(), ask.from.clone(), ask.to.clone()), until);
                    Ok(format!("peace offer of {} refused", ask.from))
                }
            }
            AskKind::Call { attackers } => {
                let w = &self.mil.wars[i];
                let side = if *attackers { &w.attackers } else { &w.defenders };
                if !side.contains(&ask.from) || w.takes_part(&ask.to) {
                    return gone(self, format!("the call of {} in {} no longer stands", ask.from, ask.war));
                }
                let enemy = if *attackers { w.defender_leader() } else { w.attacker_leader() }.unwrap_or_default().to_string();
                let Some((rules, dip)) = self.dip.as_mut() else { return gone(self, "no diplomacy in this game".into()) };
                if !dip.allied(&ask.to, &ask.from) {
                    return gone(self, format!("{} and {} are no longer allied", ask.to, ask.from));
                }
                let busy = self.mil.wars.iter().any(|x| x.active && x.takes_part(&ask.to));
                let against_ally = dip.allied(&ask.to, &enemy);
                let yes = match (accept, &self.war_ai) {
                    (Some(a), _) => a,
                    (None, Some(r)) => war_ai::ally_joins(r, dip.relation_value(&ask.to, &ask.from), against_ally, busy),
                    (None, None) => false,
                };
                if yes && against_ally {
                    if accept.is_some() {
                        return Err(format!("{} is allied to {enemy} as well: refuse, or end that alliance first", ask.to));
                    }
                    return gone(self, "allied to both sides".into());
                }
                self.asks.list.retain(|a| a.key != key);
                if yes {
                    let w = &mut self.mil.wars[i];
                    let kind = if *attackers { WarEventKind::AddAttacker(ask.to.clone()) } else { WarEventKind::AddDefender(ask.to.clone()) };
                    if *attackers { &mut w.attackers } else { &mut w.defenders }.push(ask.to.clone());
                    w.history.push(WarEvent { date: state.date, kind });
                    state.wars = self.mil.wars.iter().filter(|w| w.active).map(|w| (w.attackers.iter().cloned().collect(), w.defenders.iter().cloned().collect())).collect();
                    Ok(format!("{} joins {} beside {}", ask.to, ask.war, ask.from))
                } else {
                    // A refused call ends the alliance and costs prestige, as for the computer.
                    crate::diplomacy::treaties::end_alliance(dip, &ask.to, &ask.from);
                    if let Some(s) = state.countries.get_mut(&ask.to) {
                        s.prestige = (s.prestige + rules.treaties.call_ally_decline_prestige).max(0.0);
                    }
                    Ok(format!("{} refuses the call of {}: the alliance ends", ask.to, ask.from))
                }
            }
        }
    }

    /// Every day: questions whose time is up, or whose country went back to the computer, get the
    /// computer's answer; questions about wars that ended go.
    pub(crate) fn answer_due_asks(&mut self, state: &mut State, log: &mut CoreLog) {
        if self.asks.list.is_empty() {
            return;
        }
        let today = state.date;
        self.asks.quiet.retain(|_, d| *d > today);
        let due: Vec<u64> = self.asks.list.iter().filter(|a| a.expires <= today || !is_player(state, &a.to) || !self.mil.wars.iter().any(|w| w.active && w.name == a.war)).map(|a| a.key).collect();
        for key in due {
            if self.resolve_ask(state, key, None, log).is_err() {
                self.asks.list.retain(|a| a.key != key);
            }
        }
    }
}
