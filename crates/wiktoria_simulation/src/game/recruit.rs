//! The day of the units under construction and the computer's unit orders (the rules and the
//! measurements are in military/recruit.rs).

use std::collections::BTreeMap;

use super::{Core, CoreLog};
use crate::calendar::GameDate;
use crate::military::logistics::{self, PopStatus};
use crate::military::recruit::{self, UnitUnlocks};
use crate::military::units::{Army, Construction, Navy, Regiment, Ship, UnitDef};
use crate::script::State;

impl Core {
    /// Sites take their goods, finished units join the map, then every computer country may
    /// order one unit. Runs after the province building sites: it adds its asks to theirs.
    pub(super) fn unit_building_day(&mut self, state: &mut State, log: &mut CoreLog) {
        let Some(share) = self.buildings.as_ref().map(|b| b.n.daily_ask_share) else { return };
        let today = self.day_number(state.date);
        let cal = &self.day.calendar;
        let day_of = |d: wiktoria_data::date::Date| cal.to_game(d).map_or(today, |g| g.0.div_euclid(24));
        let mut need: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        let mut done = Vec::new();
        // Orders queue per province and branch: the first one met is built, every later one
        // waits a day (start and end move on), so N regiments in one province take N build times.
        let mut busy: std::collections::BTreeSet<(u16, bool)> = std::collections::BTreeSet::new();
        let units = &self.day.units;
        for (i, site) in self.mil.constructions.iter_mut().enumerate() {
            let Some(m) = state.money.countries.get_mut(&site.owner) else { continue };
            let land = units.get(&site.kind).is_none_or(|u| u.land);
            let queued = !busy.insert((site.location, land));
            if queued && site.lacks.is_empty() {
                let length = match (site.start, site.end) {
                    (Some(s), Some(e)) => (day_of(e) - day_of(s)).max(1),
                    _ => 1,
                };
                site.start = Some(state.date);
                site.end = Some(cal.to_date(GameDate((today + length) * 24)));
                continue;
            }
            if !site.lacks.is_empty() {
                // Still waiting for goods: the build time has not begun.
                let length = match (site.start, site.end) {
                    (Some(s), Some(e)) => (day_of(e) - day_of(s)).max(1),
                    _ => 1,
                };
                site.start = Some(state.date);
                site.end = Some(cal.to_date(GameDate((today + length) * 24)));
                if !recruit::site_takes(&mut site.lacks, &mut m.stockpile) {
                    let e = need.entry(site.owner.clone()).or_default();
                    for (g, x) in recruit::sites_ask(std::iter::once((&site.lacks, &site.full)), share) {
                        let v = e.entry(g).or_default();
                        *v = v.max(x);
                    }
                    continue;
                }
            }
            if site.end.is_none_or(|e| day_of(e) <= today) {
                done.push(i);
            }
        }
        for (tag, asks) in need {
            let m = state.money.country(&tag, None);
            for (g, x) in asks {
                let v = m.site_need.entry(g).or_default();
                *v = v.max(x);
            }
        }
        for i in done.into_iter().rev() {
            let site = self.mil.constructions.remove(i);
            self.finish_unit(site);
            log.units_built += 1;
        }
        log.units_ordered += self.order_units(state);
    }

    fn finish_unit(&mut self, site: Construction) {
        let unit = self.day.units.get(&site.kind).cloned().unwrap_or_default();
        let id = self.new_id();
        let part = self.new_id();
        if unit.land {
            let regiment = Regiment { id: part, name: site.name.clone(), kind: site.kind.clone(), pop: site.pop, pop_type: Some(self.day.soldier_type.clone()), organisation: unit.organisation, strength: unit.max_strength, experience: 0.0, name_count: 0 };
            self.mil.armies.push(Army { id, name: site.name, owner: site.owner, location: site.location, supplies: 1.0, regiments: vec![regiment], ..Army::default() });
        } else {
            let ship = Ship { id: part, name: site.name.clone(), kind: site.kind.clone(), organisation: unit.organisation, strength: unit.max_strength, experience: 0.0, name_count: 0 };
            self.mil.navies.push(Navy { id, name: site.name, owner: site.owner, location: site.location, supplies: 1.0, ships: vec![ship], ..Navy::default() });
        }
    }

    /// Every computer country orders at most one unit: a regiment where a soldier pop supports
    /// more than it has and raises, otherwise a ship while the naval supply has room.
    fn order_units(&mut self, state: &State) -> usize {
        let Some(unlocks) = self.unlocks.clone() else { return 0 };
        let soldier = self.day.soldier_type.clone();
        // Regiments standing and being raised per pop.
        let mut raised: BTreeMap<u64, u32> = BTreeMap::new();
        for pop in self.mil.armies.iter().flat_map(|a| &a.regiments).filter_map(|r| r.pop).chain(self.mil.constructions.iter().filter_map(|c| c.pop)) {
            *raised.entry(pop).or_default() += 1;
        }
        let tags: Vec<String> = state.countries.iter().filter(|(t, c)| !c.human && **t != self.day.rebel_tag && !crate::player::is_player(state, t)).map(|(t, _)| t.clone()).collect();
        let mut owned: BTreeMap<&str, Vec<u16>> = BTreeMap::new();
        for (id, p) in &state.provinces {
            if let Some(o) = p.owner.as_deref() && p.controller == p.owner {
                owned.entry(o).or_default().push(*id);
            }
        }
        let mut orders = Vec::new();
        for tag in tags {
            let Some(c) = state.countries.get(&tag) else { continue };
            let inventions = c.inventions.iter().filter_map(|n| state.inventions.get((*n as usize).wrapping_sub(1))).cloned().collect();
            let unlocked = unlocks.unlocked(&self.day.units, &c.technologies, &inventions);
            let efficiency = state.money.countries.get(&tag).map_or(0.0, |m| m.need_efficiency);
            // No regiment while the army's supply goods, at today's prices, cost more than its
            // share of the tax base.
            // Ships likewise, with their own share.
            let supply = |kind: &str, land: bool| self.day.units.get(kind).filter(|u| u.land == land).map_or(0.0, |u| u.supply_cost.iter().map(|(g, x)| x * state.prices.get(g).copied().unwrap_or(0.0)).sum::<f64>());
            let building = |land: bool| self.mil.constructions.iter().filter(|x| x.owner == tag).map(|x| supply(&x.kind, land)).sum::<f64>();
            let upkeep: f64 = self.mil.armies_of(&tag).flat_map(|a| &a.regiments).map(|r| supply(&r.kind, true)).sum::<f64>() + building(true);
            let fleet_upkeep: f64 = self.mil.navies_of(&tag).flat_map(|n| &n.ships).map(|s| supply(&s.kind, false)).sum::<f64>() + building(false);
            let tax_base = state.money.countries.get(&tag).map_or(0.0, |m| m.tax_base);
            let affordable = upkeep <= self.day.logistics.ai_army_taxbase_fraction * tax_base;
            let fleet_affordable = fleet_upkeep <= self.day.logistics.ai_navy_taxbase_fraction * tax_base;
            // The first soldier pop, in province order, that supports another regiment.
            let mut room = None;
            let mine = owned.get(tag.as_str()).map_or(&[][..], |v| v.as_slice());
            for (id, p) in mine.iter().filter(|_| affordable).filter_map(|id| Some((id, state.provinces.get(id)?))) {
                let status = if p.colonial > 0 { PopStatus::Colony } else if p.cores.contains(&tag) { PopStatus::Normal } else { PopStatus::NonCore };
                if let Some(q) = p.pops.iter().find(|q| q.pop_type == soldier && logistics::regiments_supported(&self.day.logistics, q.size, status, false) > raised.get(&q.id).copied().unwrap_or(0)) {
                    room = Some((*id, q.id));
                    break;
                }
            }
            let kind_of = |name: &str| self.day.units.get(name).map(|u| (u.land, u.kind.clone()));
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for name in self.mil.armies_of(&tag).flat_map(|a| a.regiments.iter().map(|r| r.kind.as_str())).chain(self.mil.navies_of(&tag).flat_map(|n| n.ships.iter().map(|s| s.kind.as_str()))).chain(self.mil.constructions.iter().filter(|x| x.owner == tag).map(|x| x.kind.as_str())) {
                if let Some((_, k)) = kind_of(name) {
                    *counts.entry(k).or_default() += 1;
                }
            }
            let n = |k: &str| counts.get(k).copied().unwrap_or(0);
            let pick: Option<(&UnitDef, u16, Option<u64>)> = if let Some((at, pop)) = room {
                let has_support = recruit::best_unit(&self.day.units, &unlocked, "support").is_some();
                let class = recruit::land_class(n("infantry") + n("special"), n("support"), n("cavalry"), has_support);
                recruit::best_unit(&self.day.units, &unlocked, class).or_else(|| recruit::best_unit(&self.day.units, &unlocked, "infantry")).map(|u| (u, at, Some(pop)))
            } else if fleet_affordable {
                self.ship_order(state, &tag, mine, &unlocked, [n("big_ship"), n("light_ship"), n("transport")])
            } else {
                None
            };
            if let Some((u, at, pop)) = pick {
                let count = n(&u.kind) + 1;
                orders.push(Construction { owner: tag.clone(), location: at, start: Some(state.date), name: format!("{count}. {}", u.name), kind: u.name.clone(), pop, rally_point: -1, lacks: recruit::order_goods(u, efficiency), full: recruit::order_goods(u, efficiency), end: Some(self.day.calendar.to_date(GameDate((self.day_number(state.date) + i64::from(logistics::build_days(u.build_time, 0.0))) * 24))), ..Construction::default() });
            }
        }
        let n = orders.len();
        for mut o in orders {
            o.id = self.new_id();
            self.mil.constructions.push(o);
        }
        n
    }

    /// A ship of the class the fleet lacks most, in the first port whose naval base is big
    /// enough, when no other ship waits for goods and the naval supply has room for it besides
    /// the ships afloat and building.
    fn ship_order(&self, state: &State, tag: &str, mine: &[u16], unlocked: &std::collections::BTreeSet<String>, [big, light, transport]: [usize; 3]) -> Option<(&UnitDef, u16, Option<u64>)> {
        // One ship at a time waits for its goods (measured: on 1836.2.10 no country of the test save has two).
        if self.mil.constructions.iter().any(|x| x.owner == tag && !x.lacks.is_empty() && self.day.units.get(&x.kind).is_some_and(|u| !u.land)) {
            return None;
        }
        let u = recruit::best_unit(&self.day.units, unlocked, recruit::sea_class(big, light, transport))?;
        let level = u.min_port_level.max(1.0);
        let port = mine.iter().copied().find(|id| state.provinces.get(id).is_some_and(|p| p.cores.iter().any(|x| x == tag)) && self.mil.provinces.get(id).is_some_and(|m| m.naval_base >= level))?;
        let points = self.colonial_points_of(state, tag, 1.0)?;
        let building: f64 = self.mil.constructions.iter().filter(|x| x.owner == tag).filter_map(|x| self.day.units.get(&x.kind)).filter(|u| !u.land).map(|u| u.supply_consumption_score).sum();
        if points.supply_used + building + u.supply_consumption_score > points.supply_limit {
            return None;
        }
        Some((u, port, None))
    }
}

/// Built once with the game (the technology and invention files).
pub fn unlocks(install: &std::path::Path, m: &wiktoria_data::engine_rules::EngineRules) -> Result<UnitUnlocks, String> {
    UnitUnlocks::new(install, m)
}
