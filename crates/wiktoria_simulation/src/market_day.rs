//! One day of the economy in the live world (phase C, D15): production, incomes, taxes, state
//! spending, buying, and the market that ties them together.
//!
//! Home markets and the world market (`economy::clear_good`): a sphere
//! member hands a share of what it makes to its leader's pool (CIV_BASE_SHARE_FACTOR or
//! UNCIV_BASE_SHARE_FACTOR; the leader's investment and the secondary powers' own share are not
//! in the port) and buys from a full copy of that pool plus what it kept. Every country buys at
//! home first. What the countries without a leader have left is the world market, and the unmet
//! demand is served from it in yesterday's rank order (`State::rank_order`), first come, first
//! served. `State::home_market[country][good]` holds, from yesterday, the share of its paid
//! demand a country could buy, the share of its goods that sold, and the share of its purchases
//! that were imports. Pops, artisans (inputs) and factories (inputs) pay their country's duty
//! (`economy::tariff_rate`) on the imported share; the state's own purchases do not. A negative
//! slider makes the buyers pay less and the treasury pay the difference.
//!
//! The ranking (`Farms::ranking`) is prestige + industrial score, civilised
//! countries first. THE MILITARY SCORE IS MISSING (the port has no armies): it counts as 0.
//!
//! The price is one per good for the whole world, cleared with a day's delay:
//! * `State::market_fill[good]` is the share of paid demand the market could serve yesterday
//!   (supply / demand, at most 1). Today nobody can buy more than that share of what it wants.
//! * `State::market_sold[good]` is the share of supply that found a buyer yesterday
//!   (demand / supply, at most 1). Today sellers are paid for that share; the rest is gone
//!   (measured: unsold goods are not carried over).
//! * At the end of the day both are set from today's supply and paid demand, half-way from the
//!   old value, and every price takes its daily step (`market::GoodMarket`).
//!
//! Where money goes (the rules are small functions in
//! `economy`, the books are `State::money`):
//! * The state, in the morning: interest on its debts (`loan_interest`), then the pay of its
//!   soldiers, officers, clergymen and bureaucrats and, where the reforms give them, pensions and
//!   unemployment pay (life needs cost x people / 100,000 x `pension_level` or
//!   `unemployment_benefit` x administrative efficiency x the social slider). The treasury is
//!   charged what the pops receive x (2 - administrative efficiency).
//! * The national stockpile: army and navy eat one day of their need x their
//!   slider; the state's building sites take what they lack of what is above the reserve for army
//!   and navy and hand its value back to the treasury out of the price paid at the order; then
//!   the state buys up to one day's target (`stockpile_target`), duty free, as far as the market
//!   and its money go. The army's and navy's need is the one the save had (the port has no
//!   armies), moved with (2 - administrative efficiency).
//! * A state that cannot pay borrows at once: from its own bank, then from
//!   the banks it owes already, then from the other countries' banks in rank order (the port's
//!   reading of an order the description leaves open), last from the private investors. It goes
//!   bankrupt when a day's interest cannot be paid (4.4, 4.5), and its debts are wiped
//!   BANKRUPCY_DURATION years later at the lenders' cost.
//! * A pop: artisans' inputs, life needs, everyday needs, luxury needs, then the
//!   bank (`pop_scale`, `tier_factor`, `buy_tier`, `savings_step`). Each tier is ordered at the
//!   price of the whole tier and counts as demand as ordered; short of everyday needs a pop buys
//!   luxury goods with them (`everyday_cash`). In a civilised country what it orders is met and
//!   paid, whatever the market delivers; elsewhere it pays for and gets only what can be had and
//!   keeps the rest. It pays the price x the world's discount (`pop_discount`), the sellers are
//!   paid in full for what they sell. Investor pops put half their cash and their savings into the country's best placed
//!   project (`investment_state_score`), before life needs when their own state has no factory,
//!   else before luxury needs.
//!
//! The port's own in this, or left out: overseas maintenance, units and province buildings under
//! construction (so the stockpile's construction part is the state's factories only), selling
//! from the stockpile, war subsidies and reparations, the technology part of a loan's rate, the
//! on_action events of an unpaid debt, the enforced repayment after a lost war; a bankrupt
//! country gets its sliders back when its debts are wiped; the computer player's stockpile
//! sliders (at most 0.3 / 0.35 / 0.75 in peace, full at war), its paying back of loans and its
//! budget (taxes, tariff, education and administration, set again every day:
//! `economy::ai_budget_solve`) are here because nothing else would move them. Without the last
//! the treasuries take in half as much again as they pay out, for ever, and end up with all the
//! money; with a tax-only stand-in they lived on tariffs with taxes at 0.
//!
//! Left out on purpose (D15): the blockade surcharge on imports, the one day goods
//! take to reach the world market (the shares of yesterday stand in for it), the purchase
//! factor of factories whose exports stay unsold, deleting shut factories.
//!
//! Artisans (measured on the saves): a pop makes throttle x size / workforce
//! x value of its good from yesterday's inputs, buys today's inputs for tomorrow before its own
//! needs, lowers its throttle by FLD_ARTISAN_THROTTLE_DOWN while it cannot pay for them and
//! raises it by FLD_ARTISAN_THROTTLE_UP when fully supplied (between FLD_ARTISAN_THROTTLE_MIN
//! and 1; pops of FLD_ARTISAN_SMALL_POP people or fewer never throttle), and at the floor and
//! broke switches to another good with chance FLD_ARTISAN_SWITCH_CHANCE a day, picked at random.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use wiktoria_data::date::Date;
use wiktoria_data::world::World;

use crate::economy::{self, FarmMoney, Farms, Loan, MoneyState};
use crate::fixed::Fixed;
use crate::market::GoodMarket;
use crate::script::{State, TimedModifier};

/// The port's own dice for the market: a number from 0 up to, not including, 1.
fn roll(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

/// The same day some years later.
fn years_on(d: Date, years: f64) -> Date {
    Date::new(d.year + years as i32, d.month, d.day.min(28))
}

/// A dense stand-in for a `BTreeMap<_, f64>` summed with `*entry().or_default() += x`: it keeps
/// whether a slot was ever touched, so the map built back from it has the same keys.
struct Acc {
    x: Vec<f64>,
    seen: Vec<bool>,
}

impl Acc {
    fn new(n: usize) -> Acc {
        Acc { x: vec![0.0; n], seen: vec![false; n] }
    }
    fn load(&mut self, i: usize, x: f64) {
        (self.x[i], self.seen[i]) = (x, true);
    }
    fn add(&mut self, i: usize, x: f64) {
        self.x[i] += x;
        self.seen[i] = true;
    }
    fn get(&self, i: usize) -> Option<f64> {
        self.seen[i].then_some(self.x[i])
    }
}

/// What the needs and the social pay of one country's pops hang on.
#[derive(Default, Clone)]
struct CountryNeeds {
    /// `economy::tier_factor` by strata (poor, middle, rich) and tier.
    factor: [[f64; 3]; 3],
    pension: f64,
    unemployment: f64,
}

impl Farms {
    /// A loan of `amount` from `lender` (a tag, or the private investors) to `tag`.
    fn lend(&self, state: &mut State, tag: &str, lender: &str, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        if lender != economy::PRIVATE_INVESTORS {
            state.money.country(lender, None).lent += amount;
        }
        let rate = self.loan_interest.max(self.numbers.loan_min_interest);
        let loans = &mut state.money.country(tag, None).loans;
        match loans.iter_mut().find(|l| l.lender == lender) {
            Some(l) => l.debt += amount,
            None => loans.push(Loan { lender: lender.to_string(), rate, debt: amount, paid: true }),
        }
        state.money.totals.borrowed += amount;
        if let Some(c) = state.countries.get_mut(tag) {
            c.money += amount;
        }
    }

    /// Borrows up to `gap` for a country that cannot pay: from its own
    /// bank, then from those it owes already, then from the other countries' banks in rank
    /// order, last from the private investors. A lender whose interest was not paid lends
    /// nothing; nobody lends to an enemy; in the years after a bankruptcy only the own bank lends.
    fn borrow(&self, world: &World, state: &mut State, tag: &str, mut gap: f64) {
        let today = state.date;
        let Some(c) = state.countries.get(tag) else { return };
        let modifier = crate::pops::country_modifier(c, &world.defs.modifiers, "max_loan_modifier");
        let free_of = |state: &State, lender: &str| state.money.countries.get(lender).map_or(0.0, |m| (m.bank - m.lent).max(0.0));
        let own = (free_of(state, tag) * (1.0 + modifier).clamp(0.0, 1.0)).min(gap);
        self.lend(state, tag, tag, own);
        gap -= own;
        let (banned, tax_base, known) = {
            let m = state.money.country(tag, None);
            (m.last_bankrupt.is_some_and(|d| today < years_on(d, self.bankruptcy_loan_years)), m.tax_base, m.loans.iter().map(|l| (l.lender.clone(), l.paid)).collect::<Vec<_>>())
        };
        if gap <= 1e-9 || banned {
            return;
        }
        let enemy = |state: &State, other: &str| state.wars.iter().any(|(a, d)| (a.contains(tag) && d.contains(other)) || (d.contains(tag) && a.contains(other)));
        let mut lenders: Vec<String> = known.iter().filter(|(l, paid)| *paid && l != tag).map(|(l, _)| l.clone()).collect();
        lenders.extend(state.rank_order.iter().filter(|l| l.as_str() != tag && !known.iter().any(|(k, _)| k == *l)).cloned());
        if !known.iter().any(|(l, _)| l == economy::PRIVATE_INVESTORS) {
            lenders.push(economy::PRIVATE_INVESTORS.to_string());
        }
        for lender in lenders {
            if gap <= 1e-9 {
                break;
            }
            let owed = state.money.countries.get(tag).and_then(|m| m.loans.iter().find(|l| l.lender == lender)).map_or(0.0, |l| l.debt);
            let offer = if lender == economy::PRIVATE_INVESTORS {
                (self.loan_private_max - owed).max(0.0)
            } else if enemy(state, &lender) || lender == "REB" {
                0.0
            } else {
                economy::foreign_bank_offer(free_of(state, &lender), self.loan_bank_cap, modifier, tax_base, owed)
            };
            let x = offer.min(gap);
            self.lend(state, tag, &lender, x);
            gap -= x;
        }
    }

    /// The one way a state pays: short of money it borrows the gap; if that
    /// fails it pays what it has. Returns what was paid.
    fn spend(&self, world: &World, state: &mut State, tag: &str, amount: f64) -> f64 {
        if amount <= 0.0 {
            return 0.0;
        }
        let have = state.countries.get(tag).map_or(0.0, |c| c.money);
        if have < amount {
            self.borrow(world, state, tag, amount - have.max(0.0));
        }
        let Some(c) = state.countries.get_mut(tag) else { return 0.0 };
        let paid = amount.min(c.money.max(0.0));
        c.money -= paid;
        paid
    }

    /// Bankruptcy: prestige x BANKRUPTCY_FACTOR, +2 militancy for every pop
    /// with savings, the state's building sites cancelled and their money handed back, subsidies
    /// off, spending sliders to nothing, taxes to the top, no negative tariff, stockpile sliders
    /// to their lowest. The debts stay. The port's own: the factories being built stay, they
    /// only need no more goods.
    fn bankrupt(&self, state: &mut State, tag: &str) {
        let today = state.date;
        let window = state.money.country(tag, None).last_bankrupt.is_some_and(|d| today < years_on(d, self.bankruptcy_loan_years));
        let Some(c) = state.countries.get_mut(tag) else { return };
        if window {
            return;
        }
        c.modifiers.push(TimedModifier { name: "bad_debter".into(), expires: Some(years_on(today, self.bankruptcy_loan_years)) });
        c.modifiers.push(TimedModifier { name: "in_bankrupcy".into(), expires: Some(years_on(today, self.bankruptcy_years)) });
        c.prestige *= self.bankruptcy_prestige;
        let before = c.spending.clone();
        for k in ["education_spending", "crime_fighting", "social_spending", "military_spending"] {
            c.spending.insert(k.to_string(), 0.0);
        }
        for k in ["poor_tax", "middle_tax", "rich_tax"] {
            c.spending.insert(k.to_string(), 1.0);
        }
        if let Some(t) = c.spending.get_mut("tariffs") {
            *t = t.max(0.0);
        }
        let m = state.money.country(tag, None);
        let refund: f64 = m.constructions.drain(..).map(|x| x.money).sum();
        (m.last_bankrupt, m.sliders, m.sliders_before) = (Some(today), self.stockpile_low, before);
        m.construction_need.clear();
        c.money += refund;
        for f in state.factories.iter_mut().filter(|f| f.owner == tag) {
            f.subsidised = false;
        }
        let savings = &state.money.savings;
        for p in state.provinces.values_mut().filter(|p| p.owner.as_deref() == Some(tag)) {
            for q in p.pops.iter_mut().filter(|q| savings.contains_key(&q.id)) {
                q.militancy = (q.militancy + self.numbers.bankruptcy_militancy).min(self.numbers.bankruptcy_militancy_cap.max(q.militancy));
            }
        }
        state.money.totals.bankruptcies.push(format!("{tag} {today}"));
    }

    /// The state's morning: debts wiped once BANKRUPCY_DURATION years
    /// have passed since a bankruptcy (the lending banks and their pops' savings lose them),
    /// then interest (none while the bankruptcy lasts), then `bill`. Returns the share of the
    /// bill that was paid.
    fn state_morning(&self, world: &World, state: &mut State, tag: &str, bill: f64) -> f64 {
        let today = state.date;
        let m = state.money.country(tag, None);
        if m.last_bankrupt.is_some_and(|d| today >= years_on(d, self.bankruptcy_years)) && !m.sliders_before.is_empty() {
            let loans = std::mem::take(&mut m.loans);
            let before = std::mem::take(&mut m.sliders_before);
            if let Some(c) = state.countries.get_mut(tag) {
                c.spending = before;
            }
            for l in loans.iter().filter(|l| l.lender != economy::PRIVATE_INVESTORS) {
                let bank = state.money.country(&l.lender, None);
                (bank.lent, bank.bank) = ((bank.lent - l.debt).max(0.0), (bank.bank - l.debt).max(0.0));
                let mut left = l.debt;
                for (at, saved) in state.money.savings.values_mut() {
                    if *at != l.lender || left <= 0.0 {
                        continue;
                    }
                    let lost = saved.min(left);
                    (*saved, left) = (*saved - lost, left - lost);
                }
            }
        }
        let bankrupt_now = state.countries.get(tag).is_some_and(|c| c.modifiers.iter().any(|m| m.name == "in_bankrupcy"));
        let loans = state.money.country(tag, None).loans.clone();
        if !bankrupt_now && !loans.is_empty() {
            let modifier = state.countries.get(tag).map_or(0.0, |c| crate::pops::country_modifier(c, &world.defs.modifiers, "loan_interest"));
            let mut short = 0.0;
            for l in &loans {
                let due = economy::loan_interest(&self.numbers, l.debt, l.rate, modifier, l.lender == tag);
                let paid = self.spend(world, state, tag, due);
                state.money.totals.interest += paid;
                if l.lender != economy::PRIVATE_INVESTORS {
                    state.money.country(&l.lender, None).interest_received += paid;
                }
                if let Some(x) = state.money.country(tag, None).loans.iter_mut().find(|x| x.lender == l.lender) {
                    x.paid = paid >= due - 1e-9;
                }
                short += due - paid;
            }
            if short > self.numbers.bankruptcy_shortfall {
                self.bankrupt(state, tag);
                return 0.0;
            }
        }
        if bill <= 0.0 { 1.0 } else { self.spend(world, state, tag, bill) / bill }
    }

    /// The national stockpile's day: army and navy eat, the building sites take
    /// and hand money back, the state buys up to its target. `can_buy` is the share of a wanted
    /// good the market can give this country. Returns what was bought, by good, as (wanted,
    /// paid for), and what was spent.
    fn stockpile_day(&self, world: &World, state: &mut State, tag: &str, efficiency: f64, prices: &BTreeMap<String, f64>, can_buy: &dyn Fn(&str) -> f64) -> (BTreeMap<String, f64>, f64) {
        let price = |good: &str| prices.get(good).copied().unwrap_or(0.0);
        let (human, at_war) = (state.countries.get(tag).is_some_and(|c| c.human), state.wars.iter().any(|(a, d)| a.contains(tag) || d.contains(tag)));
        let low = self.stockpile_low;
        let m = state.money.country(tag, None);
        // The computer player's sliders (ai_home, sliders 1): full at war, in peace only lowered.
        if !human && m.last_bankrupt.is_none() {
            for (i, slider) in m.sliders.iter_mut().enumerate() {
                *slider = if at_war { 1.0 } else { slider.min(self.numbers.ai_stockpile_peace[i]) }.max(low[i]);
            }
        }
        // The need the save had, moved with 2 - administrative efficiency.
        let moved = (2.0 - efficiency) / (2.0 - m.need_efficiency);
        if (moved - 1.0).abs() > 1e-9 {
            for x in m.land_need.values_mut().chain(m.naval_need.values_mut()) {
                *x *= moved;
            }
            m.need_efficiency = efficiency;
        }
        let supply = |m: &economy::CountryMoney, good: &str| m.sliders[0] * m.land_need.get(good).copied().unwrap_or(0.0) + m.sliders[1] * m.naval_need.get(good).copied().unwrap_or(0.0);
        let goods: BTreeSet<String> = m.land_need.keys().chain(m.naval_need.keys()).chain(m.construction_need.keys()).chain(m.site_need.keys()).chain(m.stockpile.keys()).chain(m.constructions.iter().flat_map(|x| x.lacks.keys())).cloned().collect();
        // Army and navy eat; what is left up to one more day of their need is kept for them.
        let mut open: BTreeMap<String, f64> = BTreeMap::new();
        let mut reserve: BTreeMap<String, f64> = BTreeMap::new();
        for good in &goods {
            let eaten = supply(m, good);
            let stock = (m.stockpile.get(good).copied().unwrap_or(0.0) - eaten).max(0.0);
            let kept = stock.min(eaten);
            reserve.insert(good.clone(), kept);
            open.insert(good.clone(), stock - kept);
        }
        // The building sites, in order: goods out of the open stock, their value back to the treasury.
        // A finished site hands back what money it has left.
        let (mut back, mut left_over) = (0.0, 0.0);
        for site in &mut m.constructions {
            let value = economy::construction_takes(&mut site.lacks, &mut open, site.money, price);
            site.money -= value;
            back += value;
            if site.lacks.is_empty() {
                left_over += site.money;
                site.money = 0.0;
            }
        }
        m.constructions.retain(|x| !x.lacks.is_empty());
        // Tomorrow's need of the building work: per good the largest lack of any one site.
        m.construction_need.clear();
        for site in &m.constructions {
            for (good, lack) in &site.lacks {
                let e = m.construction_need.entry(good.clone()).or_default();
                *e = e.max(*lack);
            }
        }
        // Buying up to one day's target.
        let mut wants: Vec<(String, f64, f64)> = Vec::new();
        let mut cost = 0.0;
        for good in &goods {
            let stock = open[good] + reserve[good];
            m.stockpile.insert(good.clone(), stock);
            let target = economy::stockpile_target(m.sliders, m.land_need.get(good).copied().unwrap_or(0.0), m.naval_need.get(good).copied().unwrap_or(0.0), 0.0, m.construction_need.get(good).copied().unwrap_or(0.0).max(m.site_need.get(good).copied().unwrap_or(0.0)));
            let want = (target - stock).max(0.0);
            if want > 0.0 && price(good) > 0.0 {
                let had = want * can_buy(good).clamp(0.0, 1.0);
                cost += had * price(good);
                wants.push((good.clone(), want, had));
            }
        }
        m.stockpile.retain(|_, x| *x > 1e-9);
        if let Some(c) = state.countries.get_mut(tag) {
            c.money += back + left_over;
        }
        state.money.totals.repaid_by_sites += back;
        let paid = self.spend(world, state, tag, cost);
        let share = if cost > 0.0 { paid / cost } else { 0.0 };
        let m = state.money.country(tag, None);
        let mut wanted = BTreeMap::new();
        for (good, want, had) in wants {
            *m.stockpile.entry(good.clone()).or_default() += had * share;
            wanted.insert(good, want * share);
        }
        state.money.totals.stockpile += paid;
        (wanted, paid - back)
    }

    /// Runs one day. See the module text for the rules.
    pub fn market_day(&self, world: &World, state: &mut State) -> FarmMoney {
        let mut day = FarmMoney::default();
        let mut supply: BTreeMap<String, f64> = BTreeMap::new();
        let mut demand: BTreeMap<String, f64> = BTreeMap::new();
        let mut seed = state.seed ^ 0x6D61_726B_6574;
        let lib = &world.defs.modifiers;
        let discount = if state.money.discount > 0.0 { state.money.discount } else { 1.0 };
        crate::prof::start();

        // Farms and mines.
        let work = self.province_work(world, state);
        let output: Vec<(u16, String, String, f64)> = work.iter().map(|w| (w.id, w.owner.clone(), w.good.clone(), w.units)).collect();
        crate::prof::mark("market: farms and mines");
        // The same per country: what it made and what it wanted and could pay for.
        let mut made: BTreeMap<(String, String), f64> = BTreeMap::new();
        let mut wanted: BTreeMap<(String, String), f64> = BTreeMap::new();
        for (_, owner, good, units) in &output {
            *supply.entry(good.clone()).or_default() += units;
            *made.entry((owner.clone(), good.clone())).or_default() += units;
        }
        // For the reports only (`FarmMoney::made_by`, `wanted_by`): made here by farms and mines.
        let made_by_farms = made.clone();
        let mut wanted_by_states: BTreeMap<(String, String), f64> = BTreeMap::new();
        let (_, _, _, factory_days) = self.workshops_and_factories(world, state);

        crate::prof::mark("market: per country made/wanted");
        // Per country: people, owner people, the owners' income of the day.
        let mut people: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
        let owner_types: BTreeSet<&str> = world.defs.rgo_types.iter().filter_map(|r| r.owner.as_ref()).map(|o| o.pop_type.as_str()).collect();
        let worker_types: BTreeSet<&str> = world.defs.rgo_types.iter().flat_map(|r| r.employees.iter()).map(|j| j.pop_type.as_str()).collect();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_ref() else { continue };
            let e = people.entry(owner.clone()).or_default();
            for q in &p.pops {
                e.0 += q.size;
                if owner_types.contains(q.pop_type.as_str()) {
                    e.1 += q.size;
                }
            }
        }
        let landed: Vec<String> = people.keys().cloned().collect();
        let prices = state.prices.clone();
        let price = |good: &str| prices.get(good).copied().unwrap_or(0.0);
        let (markets, no_market) = (state.home_market.clone(), BTreeMap::new());
        let market = |tag: &str, good: &str, i: usize, default: f64| markets.get(tag).unwrap_or(&no_market).get(good).map_or(default, |m| m[i]);
        let can_buy = |tag: &str, good: &str| market(tag, good, 0, 1.0);
        let was_sold = |tag: &str, good: &str| market(tag, good, 1, 1.0);
        let imported = |tag: &str, good: &str| market(tag, good, 2, 0.0);
        crate::prof::mark("market: people and owners");
        // Tariff rate of each country (`economy::tariff_rate`): slider x efficiency, where the
        // efficiency is the base constant + the country's tariff efficiency modifier +
        // administrative efficiency, at most 1.
        let admin = crate::pops::admin_efficiency_shares(state, world.defines.get("country", "MAX_BUREAUCRACY_PERCENTAGE").unwrap_or(0.01));
        // The efficiency alone is the rate at a full slider.
        let tariff_efficiency: BTreeMap<String, f64> = state
            .countries
            .iter()
            .map(|(tag, c)| {
                let tech: f64 = c.technologies.iter().filter_map(|n| world.defs.technologies.iter().find(|t| t.name == *n)).filter_map(|t| t.values.get("tariff_efficiency_modifier")).sum();
                let modifier = crate::pops::country_modifier(c, lib, "tariff_efficiency_modifier") + tech;
                (tag.clone(), economy::tariff_rate(1.0, self.tariff_efficiency, modifier, admin.get(tag).copied().unwrap_or(0.0)))
            })
            .collect();
        let tariffs: BTreeMap<String, f64> = tariff_efficiency.iter().map(|(tag, e)| (tag.clone(), e * state.countries[tag].spending.get("tariffs").copied().unwrap_or(0.0))).collect();
        let mut per_worker: BTreeMap<u16, f64> = BTreeMap::new();
        let slave_types: BTreeSet<&str> = world.defs.pop_types.iter().filter(|t| t.is_slave).map(|t| t.name.as_str()).collect();
        // Money made from nothing today: what the mines of the money good pay out, to the pops and
        // to the states.
        let (mut gold, mut gold_cash) = (0.0, BTreeMap::<String, f64>::new());
        let places_of: BTreeMap<u16, (f64, f64)> = work.iter().map(|w| (w.id, (w.employed, w.capacity))).collect();
        for (id, owner, good, units) in &output {
            // A farm or mine is paid for all it produces (measured: `last_income` = output x price
            // within 1% of the world total, sold or not). A mine of the money good pays its pops
            // GOLD_TO_WORKER_PAY_RATE times that, and its state GOLD_TO_CASH_RATE a unit (measured:
            // France's gold mine 225.4 = 3.5 x 8.05 x 8; the budget's gold line 4.03 = 0.5 x 8.05).
            let money_good = self.goods.get(good).is_some_and(|g| g.1);
            let income = units * price(good) * if money_good { self.gold_to_pay } else { 1.0 };
            let (employed, places) = places_of.get(id).copied().unwrap_or_default();
            day.rgo.insert(*id, (good.clone(), *units, income, employed, places));
            let Some(e) = people.get_mut(owner) else { continue };
            let owners_share = if e.0 > 0.0 { (self.owner_income_factor * e.1 / e.0).min(self.owner_income_max) } else { 0.0 };
            e.2 += income * owners_share;
            day.income += income;
            if money_good {
                gold += income + units * self.gold_to_cash;
                *gold_cash.entry(owner.clone()).or_default() += units * self.gold_to_cash;
            }
            // Slaves work but are not paid: the workers' share goes to the free workers, or to the
            // owners where only slaves work (measured: the original's slaves hold no money and have
            // no tax base; America's farmers and labourers have the slaves' share on top).
            let workers: f64 = state.provinces.get(id).map_or(0.0, |p| p.pops.iter().filter(|q| worker_types.contains(q.pop_type.as_str()) && !slave_types.contains(q.pop_type.as_str())).map(|q| q.size).sum());
            if workers > 0.0 {
                per_worker.insert(*id, income * (1.0 - owners_share) / workers);
            } else {
                e.2 += income * (1.0 - owners_share);
            }
        }
        crate::prof::mark("market: tariffs, gold");
        // Factories' day (the rules are in `economy`).
        // * Inputs: what it wants at today's staffing, bought as far as yesterday's market share
        //   of each good and its budget go (`factory_input_budget`), with the duty on the imported
        //   share. It makes the share of its output that its scarcest recipe input allows (the
        //   original works from the stock bought the day before; the port from today's purchase).
        // * A day on which nothing was bought counts towards shutting (`no_input_day`).
        // * Pay: `factory_payout`. The minimum wage is the reform's `minimum_wage` x administrative
        //   efficiency x the cost of the workers' life needs (as measured).
        // * A subsidised factory is topped up by the state to a fifth of that base wage sum plus
        //   a full day's inputs.
        // * Staffing for tomorrow (`staffing_step`): it wants all its places filled when it sold
        //   for more than its inputs cost, when it is subsidised, or when it bought nothing (the
        //   last is the port's reading of a test the description leaves open), else nobody.
        crate::prof::mark("market: factories");
        // Stand-ins: every craftsman and clerk of the state shares the workers' pay, employed
        // or not; the owners' pay goes to the capitalists of the state by head.
        let mut heads: BTreeMap<(String, Option<u16>), (f64, f64)> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_ref() else { continue };
            let e = heads.entry((owner.clone(), p.region)).or_default();
            for q in &p.pops {
                if self.factory_workers.contains(&q.pop_type) {
                    e.0 += q.size;
                } else if self.factory_owners.contains(&q.pop_type) {
                    e.1 += q.size;
                }
            }
        }
        let worker_life_cost: f64 = world.defs.pop_types.iter().find(|t| self.factory_workers.contains(&t.name)).map_or(0.0, |t| t.life_needs.iter().map(|(good, amount)| amount * price(good)).sum::<f64>() / self.needs_per_people);
        // (owner, state) -> pay per worker and per owner; and the people at work in its factories.
        let mut factory_pay: BTreeMap<(String, Option<u16>), (f64, f64)> = BTreeMap::new();
        let mut staffed: BTreeMap<(String, Option<u16>), f64> = BTreeMap::new();
        let mut factory_changes: Vec<(usize, FactoryChange)> = Vec::new();
        let mut factory_treasury: BTreeMap<String, f64> = BTreeMap::new();
        let mut factory_imports: BTreeMap<String, f64> = BTreeMap::new();
        let mut factory_wanted: BTreeMap<(String, String), f64> = BTreeMap::new();
        for fd in &factory_days {
            let Some(f) = state.factories.get(fd.index) else { continue };
            let cash = f.money / self.pop_money_scale;
            let rate = tariffs.get(&f.owner).copied().unwrap_or(0.0);
            let full_inputs: f64 = fd.inputs.iter().map(|(good, amount)| amount * price(good)).sum();
            // A factory without a yesterday (just loaded or just opened) may spend its cash.
            let budget = if f.spending <= 0.0 && f.income <= 0.0 && f.no_input_days == 0 { cash.max(0.0) } else { economy::factory_input_budget(&self.numbers, cash, f.spending) };
            let (mut cost, mut duty, mut from_abroad) = (0.0, 0.0, 0.0);
            for (good, amount) in &fd.inputs {
                let value = amount * can_buy(&f.owner, good) * price(good);
                cost += value;
                duty += value * (rate * imported(&f.owner, good)).max(-1.0);
                from_abroad += value * imported(&f.owner, good);
            }
            let scale = if cost + duty > budget { budget / (cost + duty) } else { 1.0 };
            let asked = if full_inputs > budget { budget / full_inputs } else { 1.0 };
            for (good, amount) in &fd.inputs {
                *factory_wanted.entry((f.owner.clone(), good.clone())).or_default() += amount * asked;
            }
            let (cost, duty) = (cost * scale, duty * scale);
            let spending = cost + duty;
            *factory_treasury.entry(f.owner.clone()).or_default() += duty;
            *day.duties.entry(f.owner.clone()).or_default() += duty;
            *factory_imports.entry(f.owner.clone()).or_default() += from_abroad * scale;
            day.tariffs += duty;
            let have = fd.inputs.iter().take(fd.recipe).map(|(good, _)| can_buy(&f.owner, good)).fold(1.0, f64::min) * scale;
            let units = fd.units * have;
            day.factories.push(economy::FactoryFlow { index: fd.index, good: fd.good.clone(), made: units, wanted: fd.inputs.iter().map(|(good, amount)| (good.clone(), amount * asked)).collect() });
            *supply.entry(fd.good.clone()).or_default() += units;
            *made.entry((f.owner.clone(), fd.good.clone())).or_default() += units;
            let income = units * price(&fd.good) * was_sold(&f.owner, &fd.good);
            let (level, no_input_days) = economy::no_input_day(&self.numbers, f.level, f.no_input_days, spending > 1e-9, f.subsidised);
            let loss_days = if income >= spending { 0 } else { f.loss_days + 1 };
            let mut cash = cash - spending + income;
            let base_wages = worker_life_cost * fd.workers;
            let country = state.countries.get(&f.owner);
            if f.subsidised {
                let short = self.numbers.factory_subsidy_wage_share * base_wages + full_inputs - cash;
                if short > 0.0 && country.is_some_and(|c| c.money > 0.0) {
                    cash += short;
                    *factory_treasury.entry(f.owner.clone()).or_default() -= short;
                    day.subsidies += short;
                }
            }
            let minimum_wage = country.map_or(0.0, |c| crate::pops::country_modifier(c, lib, "minimum_wage")) * admin.get(&f.owner).copied().unwrap_or(0.0) * base_wages;
            let key = (f.owner.clone(), f.region);
            *staffed.entry(key.clone()).or_default() += fd.workers;
            let (workers, owners) = heads.get(&key).copied().unwrap_or_default();
            let out = economy::factory_payout(&self.numbers, cash, income, spending, full_inputs, minimum_wage, level, self.factory_pay_factor, self.factory_max_money, owners > 0.0);
            let e = factory_pay.entry(key).or_default();
            if workers > 0.0 {
                e.0 += out.workers / workers;
            }
            if owners > 0.0 {
                e.1 += out.owners / owners;
            }
            day.factory_income += income;
            day.factory_inputs += spending;
            day.factory_pay += out.workers + out.owners;
            day.factories_profitable += usize::from(income > spending);
            let wants = income > spending || f.subsidised || spending <= 0.0;
            let shut = level <= 1.0 && no_input_days >= self.numbers.factory_no_input_days;
            let staff = if shut { 0.0 } else { economy::staffing_step(fd.workers, if wants { fd.places } else { 0.0 }, self.hiring.speed_factory, &self.hiring) };
            factory_changes.push((fd.index, FactoryChange { money: out.cash * self.pop_money_scale, income, spending, pay: out.workers + out.owners, staff, level, no_input_days, loss_days }));
        }
        crate::prof::mark("market: factory pay to pops");
        // Tax rate of each country by strata (poor, middle, rich), its tax efficiency, and its
        // budget sliders.
        let efficiency: BTreeMap<String, f64> = state
            .countries
            .iter()
            .map(|(tag, c)| {
                let tech: f64 = c.technologies.iter().filter_map(|n| world.defs.technologies.iter().find(|t| t.name == *n)).filter_map(|t| t.values.get("tax_eff")).sum();
                let invented: f64 = c.inventions.iter().filter_map(|n| lib.inventions.get((*n as usize).wrapping_sub(1))).filter_map(|(_, v)| v.get("tax_eff")).sum();
                (tag.clone(), self.tax_efficiency + (tech + invented) * self.tax_eff_scale)
            })
            .collect();
        let types: BTreeMap<&str, &wiktoria_data::defs::PopType> = world.defs.pop_types.iter().map(|t| (t.name.as_str(), t)).collect();
        let on_sale = state.goods_on_sale.clone();
        let sells = |good: &str| on_sale.is_empty() || on_sale.contains(good);
        let salaries: BTreeMap<&str, (&str, f64)> = self
            .state_paid
            .iter()
            .filter_map(|(kind, (slider, tiers))| {
                let t = types.get(kind.as_str())?;
                let cost: f64 = [&t.life_needs, &t.everyday_needs, &t.luxury_needs].into_iter().take(*tiers).flat_map(|tier| tier.iter()).filter(|(good, _)| sells(good)).map(|(good, amount)| amount * price(good)).sum();
                Some((kind.as_str(), (slider.as_str(), cost)))
            })
            .collect();
        crate::prof::mark("market: tax rates");
        // What the life needs of each pop type cost today, per line of its file.
        let life_cost: BTreeMap<&str, f64> = types.iter().map(|(kind, t)| (*kind, t.life_needs.iter().filter(|(good, _)| sells(good)).map(|(good, amount)| amount * price(good)).sum())).collect();
        // Needs factors and social pay of each country.
        let needs_of: BTreeMap<String, CountryNeeds> = landed
            .iter()
            .filter_map(|tag| {
                let c = state.countries.get(tag)?;
                let modifier = |key: &str| crate::pops::country_modifier(c, lib, key);
                let (wide, inventions) = (modifier("goods_demand"), c.inventions.len() as f64);
                let mut factor = [[0.0; 3]; 3];
                for (s, strata) in ["poor", "middle", "rich"].into_iter().enumerate() {
                    for (i, tier) in ["life_needs", "everyday_needs", "luxury_needs"].into_iter().enumerate() {
                        factor[s][i] = economy::tier_factor(&self.numbers, i, self.base_goods_demand, wide, modifier(&format!("{strata}_{tier}")), inventions, self.invention_impact);
                    }
                }
                Some((tag.clone(), CountryNeeds { factor, pension: modifier("pension_level"), unemployment: modifier("unemployment_benefit") }))
            })
            .collect();
        let slider = |state: &State, tag: &str, name: &str| state.countries.get(tag).and_then(|c| c.spending.get(name)).copied().unwrap_or(0.0);
        crate::prof::mark("market: needs cost and factors");
        // The share of a province's farm or mine workers without work, and of a state's factory workers.
        let rgo_idle = |p: &crate::script::ProvinceState| p.rgo_staff.map_or(0.0, |staff| {
            let workers: f64 = p.pops.iter().filter(|q| worker_types.contains(q.pop_type.as_str())).map(|q| q.size).sum();
            if workers > 0.0 { (1.0 - staff / workers).clamp(0.0, 1.0) } else { 0.0 }
        });
        let factory_idle = |owner: &String, region: Option<u16>| {
            let key = (owner.clone(), region);
            let workers = heads.get(&key).map_or(0.0, |x| x.0);
            if workers > 0.0 { (1.0 - staffed.get(&key).copied().unwrap_or(0.0) / workers).clamp(0.0, 1.0) } else { 0.0 }
        };
        crate::prof::mark("market: unemployment shares");
        // What each state would pay its pops today in full: salaries and social pay.
        let mut bills: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        // The same at full sliders, per (country, slider).
        let mut full_pay: BTreeMap<(String, &str), f64> = BTreeMap::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_ref() else { continue };
            let Some(needs) = needs_of.get(owner) else { continue };
            let e = admin.get(owner).copied().unwrap_or(0.0);
            let (idle_rgo, idle_factory) = (rgo_idle(p), factory_idle(owner, p.region));
            let bill = bills.entry(owner.clone()).or_default();
            for q in &p.pops {
                let kind = q.pop_type.as_str();
                let heads = self.state_pay_factor * q.size / self.needs_per_people;
                if let Some((name, cost)) = salaries.get(kind) {
                    bill.0 += slider(state, owner, name) * heads * cost;
                    *full_pay.entry((owner.clone(), *name)).or_default() += heads * cost * (2.0 - e);
                    *day.spent_on.entry((owner.clone(), name.to_string())).or_default() += slider(state, owner, name) * heads * cost * (2.0 - e);
                }
                let idle = if self.factory_workers.contains(kind) { idle_factory } else if worker_types.contains(kind) { idle_rgo } else { 0.0 };
                let social = slider(state, owner, "social_spending") * heads * life_cost.get(kind).copied().unwrap_or(0.0) * e * (needs.pension + needs.unemployment * idle);
                bill.1 += social;
                *day.spent_on.entry((owner.clone(), "social_spending".to_string())).or_default() += social * (2.0 - e);
            }
        }
        crate::prof::mark("market: state pay");
        // The states' morning and their stockpiles. `spent_by`: what each treasury paid out.
        let mut can_pay: BTreeMap<String, f64> = BTreeMap::new();
        let mut spent_by: BTreeMap<String, f64> = BTreeMap::new();
        for tag in &landed {
            if !state.countries.contains_key(tag) {
                continue;
            }
            let _ = state.money.country(tag, state.countries.get(tag));
            let e = admin.get(tag).copied().unwrap_or(0.0);
            let (pay, social) = bills.get(tag).copied().unwrap_or_default();
            let before = state.countries[tag].money;
            let borrowed = state.money.totals.borrowed;
            let share = self.state_morning(world, state, tag, (pay + social) * (2.0 - e));
            can_pay.insert(tag.clone(), share);
            (state.money.totals.salaries_charged, state.money.totals.social) = (state.money.totals.salaries_charged + pay * (2.0 - e) * share, state.money.totals.social + social * (2.0 - e) * share);
            day.salary_cost += (pay + social) * (2.0 - e) * share;
            let (bought, _) = self.stockpile_day(world, state, tag, e, &prices, &|good| can_buy(tag, good));
            for (good, x) in bought {
                day.state_goods += x * can_buy(tag, &good).clamp(0.0, 1.0) * price(&good);
                *day.spent_on.entry((tag.clone(), "stockpile".to_string())).or_default() += x * can_buy(tag, &good).clamp(0.0, 1.0) * price(&good);
                *demand.entry(good.clone()).or_default() += x;
                *wanted_by_states.entry((tag.clone(), good.clone())).or_default() += x;
                *wanted.entry((tag.clone(), good)).or_default() += x;
            }
            spent_by.insert(tag.clone(), before - state.countries[tag].money + (state.money.totals.borrowed - borrowed));
        }
        crate::prof::mark("market: state morning, stockpiles");
        // Artisan types a pop can turn to: those whose good is on sale.
        let crafts: Vec<&wiktoria_data::production::ArtisanType> = world.defs.artisan_types.iter().filter(|t| t.workforce > 0.0 && sells(&t.output_good)).collect();
        // Countries whose capitalists have a project to pay into.
        let with_project: BTreeSet<String> = state.projects.keys().map(|k| k.0.clone()).collect();

        crate::prof::mark("market: artisan types, projects");
        // Dense indices for the pop loop: goods and countries by number instead of by name. The
        // sums below run in the same order as with the named maps, so the results are the same.
        let mut gidx: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &world.defs.pop_types {
            for good in t.life_needs.keys().chain(t.everyday_needs.keys()).chain(t.luxury_needs.keys()) {
                let n = gidx.len();
                gidx.entry(good.as_str()).or_insert(n);
            }
        }
        for c in &world.defs.artisan_types {
            for good in c.input_goods.keys().chain(std::iter::once(&c.output_good)) {
                let n = gidx.len();
                gidx.entry(good.as_str()).or_insert(n);
            }
        }
        let ng = gidx.len();
        let mut gname = vec![""; ng];
        for (good, g) in &gidx {
            gname[*g] = good;
        }
        let gprice: Vec<f64> = gname.iter().map(|good| price(good)).collect();
        let otags: Vec<String> = state.countries.keys().cloned().collect();
        let oidx: BTreeMap<&str, usize> = otags.iter().enumerate().map(|(i, t)| (t.as_str(), i)).collect();
        let no = otags.len();
        crate::prof::mark("market: dense indices");
        // Yesterday's home market of each country and good: can buy, was sold, imported.
        let mrow: Vec<[f64; 3]> = otags.iter().flat_map(|tag| gname.iter().map(|good| markets.get(tag).and_then(|m| m.get(*good)).copied().unwrap_or([1.0, 1.0, 0.0]))).collect();
        // What the loop needs of each pop type, looked up once.
        struct Kind<'a> {
            t: &'a wiktoria_data::defs::PopType,
            tiers: [Vec<(usize, f64)>; 3],
            owner: bool,
            rgo_worker: bool,
            factory_worker: bool,
            factory_owner: bool,
            salary: Option<(&'a str, f64)>,
            life_cost: f64,
        }
        let kinds: BTreeMap<&str, Kind> = types
            .iter()
            .map(|(kind, t)| {
                let tiers = [&t.life_needs, &t.everyday_needs, &t.luxury_needs].map(|tier| tier.iter().filter(|(good, _)| sells(good)).map(|(good, amount)| (gidx[good.as_str()], *amount)).collect());
                let k = Kind {
                    t,
                    tiers,
                    owner: owner_types.contains(kind),
                    rgo_worker: worker_types.contains(kind),
                    factory_worker: self.factory_workers.contains(*kind),
                    factory_owner: self.factory_owners.contains(*kind),
                    salary: salaries.get(kind).copied(),
                    life_cost: life_cost.get(kind).copied().unwrap_or(0.0),
                };
                (*kind, k)
            })
            .collect();
        let mut wanted_d = Acc::new(no * ng);
        for ((tag, good), x) in &wanted {
            if let (Some(o), Some(g)) = (oidx.get(tag.as_str()), gidx.get(good.as_str())) {
                wanted_d.load(o * ng + g, *x);
            }
        }
        let (mut demand_d, mut supply_d) = (Acc::new(ng), Acc::new(ng));
        for (good, g) in &gidx {
            if let Some(x) = demand.get(*good) {
                demand_d.load(*g, *x);
            }
            if let Some(x) = supply.get(*good) {
                supply_d.load(*g, *x);
            }
        }
        let mut made_by_artisans = Acc::new(no * ng);
        let mut pop_wanted = vec![[0.0; 5]; ng];
        let (mut treasury_d, mut invested, mut taxed, mut tax_base) = (Acc::new(no), Acc::new(no), Acc::new(no), Acc::new(no));
        let mut treasury: BTreeMap<String, f64> = factory_treasury;
        let mut duty_d = Acc::new(no);
        // For the computer player's budget: the value pops and artisans bought abroad, and each
        // strata's tax base.
        let (mut imports_d, mut base_by_strata) = (Acc::new(no), Acc::new(no * 3));
        for (tag, x) in gold_cash {
            *treasury.entry(tag).or_default() += x;
        }
        for (tag, x) in &treasury {
            if let Some(o) = oidx.get(tag.as_str()) {
                treasury_d.load(*o, *x);
            }
        }
        let mut seen: HashSet<u64> = HashSet::new();
        let mut pops_cash = 0.0;
        let MoneyState { countries: banks, savings, totals, .. } = &mut state.money;
        let sliders: BTreeMap<&String, &BTreeMap<String, f64>> = state.countries.iter().map(|(tag, c)| (tag, &c.spending)).collect();
        let civilised: HashSet<&String> = state.countries.iter().filter(|(_, c)| c.civilized).map(|(tag, _)| tag).collect();
        for (id, p) in &mut state.provinces {
            let Some(owner) = p.owner.clone() else { continue };
            let (Some(needs), Some(spending)) = (needs_of.get(&owner), sliders.get(&owner)) else { continue };
            let o = oidx[owner.as_str()];
            let row = &mrow[o * ng..(o + 1) * ng];
            let civilised = civilised.contains(&owner);
            let (_, owners, owner_pool) = people.get(&owner).copied().unwrap_or_default();
            let tax_efficiency = efficiency.get(&owner).copied().unwrap_or(0.0);
            let tariff = tariffs.get(&owner).copied().unwrap_or(0.0);
            let (e, paid_share) = (admin.get(&owner).copied().unwrap_or(0.0), can_pay.get(&owner).copied().unwrap_or(1.0));
            let (idle_rgo, idle_factory) = (rgo_idle(p), factory_idle(&owner, p.region));
            let social_slider = spending.get("social_spending").copied().unwrap_or(0.0);
            let has_project = with_project.contains(&owner);
            // The bank's unlent money, and what this province's pops add to its deposits.
            let mut free = banks.get(&owner).map_or(0.0, |b| (b.bank - b.lent).max(0.0));
            let mut deposits = 0.0;
            for q in &mut p.pops {
                let kind = q.pop_type.as_str();
                let Some(k) = kinds.get(kind) else { continue };
                let (t, tiers) = (k.t, &k.tiers);
                let heads = self.state_pay_factor * q.size / self.needs_per_people;
                let mut craft = None;
                let mut gross = if kind == "artisans" {
                    if crafts.is_empty() {
                        continue;
                    }
                    // A pop without a good (new, or its good is gone) takes one at random.
                    if !q.production.as_deref().is_some_and(|k| crafts.iter().any(|c| c.name == k)) {
                        q.production = Some(crafts[(roll(&mut seed) * crafts.len() as f64) as usize % crafts.len()].name.clone());
                        (q.throttle, q.stock) = (1.0, 0.0);
                        q.held.clear();
                    }
                    let Some(c) = crafts.iter().find(|c| Some(c.name.as_str()) == q.production.as_deref()) else { continue };
                    craft = Some(*c);
                    let made = economy::artisan_units(q.throttle, q.size, c.workforce, q.stock, c.value);
                    // The work uses up that share of every input; the rest stays in hand.
                    if q.held.len() != c.input_goods.len() {
                        q.held = vec![q.stock; c.input_goods.len()];
                    }
                    let used = q.stock;
                    q.held.iter_mut().for_each(|h| *h = (*h - used).max(0.0));
                    let g = gidx[c.output_good.as_str()];
                    supply_d.add(g, made);
                    made_by_artisans.add(o * ng + g, made);
                    day.artisan_made += made * price(&c.output_good);
                    day.artisan_capacity += q.size / c.workforce * c.value * price(&c.output_good);
                    let income = made * price(&c.output_good) * was_sold(&owner, &c.output_good);
                    day.artisan_income += income;
                    let row = day.artisans_by.entry((owner.clone(), c.name.clone())).or_default();
                    (row[0], row[2], row[3]) = (row[0] + income, row[2] + made, row[3] + q.size);
                    income
                } else if k.owner && owners > 0.0 {
                    owner_pool * q.size / owners
                } else if k.rgo_worker && !t.is_slave {
                    per_worker.get(id).copied().unwrap_or(0.0) * q.size
                } else if k.factory_worker {
                    factory_pay.get(&(owner.clone(), p.region)).map_or(0.0, |x| x.0) * q.size
                } else if k.factory_owner {
                    factory_pay.get(&(owner.clone(), p.region)).map_or(0.0, |x| x.1) * q.size
                } else if let Some((name, cost)) = k.salary {
                    spending.get(name).copied().unwrap_or(0.0) * heads * cost * paid_share
                } else {
                    0.0
                };
                // Pension and unemployment pay, where the reforms give them.
                let idle = if k.factory_worker { idle_factory } else if k.rgo_worker { idle_rgo } else { 0.0 };
                let social = social_slider * heads * k.life_cost * e * (needs.pension + needs.unemployment * idle) * paid_share;
                if k.salary.is_some() {
                    day.salaries += gross;
                }
                day.salaries += social;
                gross += social;
                let mut cash = q.money / self.pop_money_scale + gross;
                // What an artisan pays for its inputs is not taxed.
                let mut inputs_paid = 0.0;

                // An artisan first buys tomorrow's inputs.
                if let Some(c) = craft {
                    let scale = q.throttle * q.size / c.workforce;
                    // It orders only what it lacks of each good: a good it still holds a full day
                    // of is not asked for while another input keeps it from working (measured: the
                    // original's artisans list as need only the goods short in their stockpile,
                    //).
                    let missing: Vec<f64> = q.held.iter().map(|h| (1.0 - h).max(0.0)).collect();
                    let with_duty: f64 = c.input_goods.iter().zip(&missing).map(|((good, amount), m)| amount * scale * m * price(good) * (1.0 + (tariff * imported(&owner, good)).max(-1.0))).sum();
                    let afford = if with_duty <= 0.0 { 1.0 } else { (cash / with_duty).clamp(0.0, 1.0) };
                    let (mut cost, mut duty, mut imports) = (0.0, 0.0, 0.0);
                    for (((good, amount), m), h) in c.input_goods.iter().zip(&missing).zip(q.held.iter_mut()) {
                        let g = gidx[good.as_str()];
                        let want = amount * scale * m * afford;
                        wanted_d.add(o * ng + g, want);
                        demand_d.add(g, want);
                        pop_wanted[g][4] += want;
                        let got = m * afford * can_buy(&owner, good);
                        *h += got;
                        let value = amount * scale * got * price(good);
                        cost += value;
                        // The duty on the imported share (a negative one is paid by the state).
                        duty += value * (tariff * imported(&owner, good)).max(-1.0);
                        imports += value * imported(&owner, good);
                    }
                    q.stock = q.held.iter().copied().fold(1.0, f64::min);
                    inputs_paid = cost + duty;
                    imports_d.add(o, imports);
                    if let Some(row) = day.artisans_by.get_mut(&(owner.clone(), c.name.clone())) {
                        row[1] += inputs_paid;
                    }
                    cash -= inputs_paid;
                    treasury_d.add(o, duty);
                    duty_d.add(o, duty);
                    day.tariffs += duty;
                    day.artisan_inputs += cost;
                    if q.size > self.artisan_small_pop {
                        let bought_at = q.throttle;
                        if afford < 1.0 {
                            q.throttle = (q.throttle * self.artisan_throttle_down).max(self.artisan_throttle_min);
                        } else if q.stock >= 0.999 {
                            q.throttle = (q.throttle * self.artisan_throttle_up).min(1.0);
                        }
                        // What it bought is what it makes tomorrow, whatever the new throttle.
                        let now = q.throttle;
                        q.held.iter_mut().for_each(|h| *h = economy::artisan_stock_after_throttle(*h, bought_at, now));
                        q.stock = q.held.iter().copied().fold(f64::INFINITY, f64::min).min(economy::artisan_stock_after_throttle(1.0, bought_at, now));
                        if afford < 1.0 && q.throttle <= self.artisan_throttle_min * 1.07 && roll(&mut seed) < self.artisan_switch_chance {
                            q.production = Some(crafts[(roll(&mut seed) * crafts.len() as f64) as usize % crafts.len()].name.clone());
                            // The original hands a switching pop a first stock of inputs.
                            (q.throttle, q.stock) = (self.artisan_throttle_down, 1.0);
                            q.held.clear();
                        }
                    }
                }
                // The tax, on the day's income less what the inputs cost.
                let strata = match t.strata.as_str() { "rich" => 2, "middle" => 1, _ => 0 };
                let (base, tax) = economy::pop_tax(gross, inputs_paid, tax_efficiency, spending.get(["poor_tax", "middle_tax", "rich_tax"][strata]).copied().unwrap_or(0.0));
                treasury_d.add(o, tax);
                taxed.add(o, tax);
                tax_base.add(o, base);
                base_by_strata.add(o * 3 + strata, base);
                day.tax += tax;
                cash -= tax;
                // Today's earnings come in after the shopping: a pop buys its needs with what it
                // held since yesterday, and goes to bed with the day's net income (measured on the
                // test saves of 1836.2.9-10: farmers of Kent hold 44.1 at night, their farm paid
                // 54.3 less tax, 44.6; Amiens' labourers 21.3 against 23.1). A loss is paid at once.
                // The state's pay is the exception: it comes before the shopping (soldiers and
                // clergymen hold 0.03-0.26 of a day's pay at night, farmers and labourers 1.0).
                let pay = (cash - q.money / self.pop_money_scale).max(0.0);
                let earned = if k.salary.is_some() { 0.0 } else { pay };
                cash -= earned;
                // A state-paid pop holds back a little of the day's pay from its shopping (test save
                // 1836.2.9: civilised soldiers, clergymen and bureaucrats short of everyday needs
                // hold 0.034-0.039 of it at night).
                let held = if k.salary.is_some() { self.numbers.pop_hold_pay_share * pay } else { 0.0 };
                cash -= held;
                let mut flows = [gross, base, tax, inputs_paid, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

                // Its savings; they lie in the bank of the country it lives in.
                seen.insert(q.id);
                let mut saved = match savings.get_mut(&q.id) {
                    Some((at, x)) => {
                        if *at != owner {
                            if let Some(b) = banks.get_mut(at) {
                                b.bank = (b.bank - *x).max(0.0);
                            }
                            deposits += *x;
                            *at = owner.clone();
                        }
                        *x
                    }
                    None => 0.0,
                };
                let was_saved = saved;
                // Then its own needs, tier by tier, with all the cash it has.
                let scale = economy::pop_scale(q.size, q.consciousness, self.needs_per_people, self.base_con);
                let investor = has_project && k.factory_owner;
                let mut needs_cost = self.numbers.pop_needs_cost_floor;
                let mut got = [0.0; 3];
                // Of the luxury tier: the price of what can be had, of it without the duty, the
                // tier's factor and the price of the whole tier.
                let mut luxury = (0.0, 0.0, 0.0, 0.0);
                let mut luxury_imported = 0.0;
                let price_of = |tier: &[(usize, f64)], fac: f64| -> f64 { tier.iter().map(|&(g, amount)| amount * fac * gprice[g] * (1.0 + (tariff * row[g][2]).max(-1.0))).sum() };
                let luxury_whole = price_of(&tiers[2], scale * needs.factor[strata][2]);
                for (i, tier) in tiers.iter().enumerate() {
                    // Investors: half the cash and all the savings the bank can hand out.
                    if investor && i == if p.has_factory { 2 } else { 0 } {
                        let mut put = 0.0;
                        if cash > self.numbers.invest_min_cash {
                            put += self.numbers.invest_cash_share * cash;
                            cash -= self.numbers.invest_cash_share * cash;
                        }
                        if saved > self.numbers.invest_min_cash {
                            let out = saved.min(free);
                            (saved, free, put) = (saved - out, free - out, put + out);
                        }
                        invested.add(o, put);
                        totals.invested += put;
                        flows[5] += put;
                    }
                    let fac = scale * needs.factor[strata][i];
                    // With the duty on the imported share of each good.
                    let (mut all, mut had, mut plain, mut cost, mut from_abroad) = (0.0, 0.0, 0.0, 0.0, 0.0);
                    for &(g, amount) in tier {
                        let can = amount * fac * row[g][0].clamp(0.0, 1.0);
                        (all, had, plain, cost) = (all + amount * fac, had + can, plain + can * gprice[g], cost + can * gprice[g] * (1.0 + (tariff * row[g][2]).max(-1.0)));
                        from_abroad += can * gprice[g] * row[g][2];
                    }
                    if all <= 0.0 {
                        got[i] = 1.0;
                        continue;
                    }
                    needs_cost += cost;
                    flows[7 + i] += cost;
                    let whole = price_of(tier, fac);
                    if i == 2 {
                        luxury = (cost, plain, fac, whole);
                        luxury_imported = from_abroad;
                    }
                    if cost <= 0.0 {
                        continue;
                    }
                    if cash < whole * discount && saved > 0.0 {
                        let out = (whole * discount - cash).min(saved).min(free);
                        (cash, saved, free) = (cash + out, saved - out, free - out);
                    }
                    // Short of everyday needs, a pop buys luxury goods with them; an artisan
                    // (luxury at 0.01 of everyday met) keeps its cash as working capital.
                    let spend = if i == 1 && craft.is_none() && civilised { economy::everyday_cash(cash, whole * discount, luxury_whole * discount, self.numbers.pop_luxury_with_everyday[strata]) } else { cash };
                    let bought = economy::buy_tier(spend, whole, cost, had / all, discount, civilised);
                    cash -= bought.paid;
                    let duty = (cost - plain) * bought.share;
                    treasury_d.add(o, duty);
                    duty_d.add(o, duty);
                    imports_d.add(o, from_abroad * bought.share);
                    day.tariffs += duty;
                    day.spent += bought.paid;
                    flows[4] += bought.paid;
                    (totals.needs_paid, totals.needs_at_full_price) = (totals.needs_paid + bought.paid, totals.needs_at_full_price + cost * bought.share);
                    for &(g, amount) in tier {
                        wanted_d.add(o * ng + g, amount * fac * bought.share);
                        demand_d.add(g, amount * fac * bought.share);
                        pop_wanted[g][i] += amount * fac * bought.share;
                    }
                    got[i] = bought.met;
                }
                (q.life_needs, q.everyday_needs, q.luxury_needs) = (got[0], got[1], got[2]);
                cash += held;
                // The bank. What is not spent stays with the pop.
                let step = economy::savings_step(&self.numbers, cash, saved, needs_cost, got[2], self.pop_savings, free);
                (cash, saved, free) = (cash - step, saved + step, free + step);
                // What is left above a quarter of the day's pay buys more luxury goods (paid in full
                // in a civilised country, as the needs); an artisan's cash is its working capital
                // (it holds about a day's sales at night).
                if step >= 0.0 && craft.is_none() {
                    let (cost, plain, fac, whole) = luxury;
                    let (share, paid) = economy::luxury_overbuy(cash, self.numbers.pop_keep_pay_share * pay, if civilised { whole } else { cost }, whole, discount);
                    if paid > 0.0 {
                        cash -= paid;
                        let duty = (cost - plain) * share;
                        treasury_d.add(o, duty);
                        duty_d.add(o, duty);
                        imports_d.add(o, luxury_imported * share);
                        day.tariffs += duty;
                        day.spent += paid;
                        flows[4] += paid;
                        (totals.needs_paid, totals.needs_at_full_price) = (totals.needs_paid + paid, totals.needs_at_full_price + cost * share);
                        for &(g, amount) in &tiers[2] {
                            wanted_d.add(o * ng + g, amount * fac * share);
                            demand_d.add(g, amount * fac * share);
                            pop_wanted[g][3] += amount * fac * share;
                        }
                    }
                }
                cash += earned;
                let change = saved - was_saved;
                deposits += change;
                flows[6] = change;
                let split = day.taxed_by.entry((owner.clone(), q.pop_type.clone())).or_default();
                for (x, f) in split.iter_mut().zip(flows) {
                    *x += f;
                }
                day.banked += change;
                if change > 0.0 {
                    totals.deposited += change;
                } else {
                    totals.withdrawn -= change;
                }
                if saved > 1e-9 {
                    // The entry, if any, is already in this country's bank (see above).
                    match savings.get_mut(&q.id) {
                        Some(e) => e.1 = saved,
                        None => {
                            savings.insert(q.id, (owner.clone(), saved));
                        }
                    }
                } else {
                    savings.remove(&q.id);
                }
                pops_cash += cash.max(0.0);
                q.money = cash.max(0.0) * self.pop_money_scale;
            }
            if let Some(b) = banks.get_mut(&owner) {
                b.bank = (b.bank + deposits).max(0.0);
            }
        }
        crate::prof::mark("market: POP LOOP");
        // Back to the named maps.
        for (o, tag) in otags.iter().enumerate() {
            for (g, good) in gname.iter().enumerate() {
                if let Some(x) = wanted_d.get(o * ng + g) {
                    wanted.insert((tag.clone(), good.to_string()), x);
                }
            }
        }
        for (g, good) in gname.iter().enumerate() {
            if pop_wanted[g].iter().any(|x| *x > 0.0) {
                day.pop_wanted.insert(good.to_string(), pop_wanted[g]);
            }
            if let Some(x) = demand_d.get(g) {
                demand.insert(good.to_string(), x);
            }
            if let Some(x) = supply_d.get(g) {
                supply.insert(good.to_string(), x);
            }
        }
        let mba = &made_by_artisans;
        let made_by_artisans: BTreeMap<(String, String), f64> = otags.iter().enumerate().flat_map(|(o, tag)| gname.iter().enumerate().filter_map(move |(g, good)| Some(((tag.clone(), good.to_string()), mba.get(o * ng + g)?)))).collect();
        let named = |acc: &Acc| -> BTreeMap<String, f64> { otags.iter().enumerate().filter_map(|(o, tag)| Some((tag.clone(), acc.get(o)?))).collect() };
        let (invested, taxed, tax_base) = (named(&invested), named(&taxed), named(&tax_base));
        treasury.extend(named(&treasury_d));
        for (tag, x) in named(&duty_d) {
            *day.duties.entry(tag).or_default() += x;
        }
        crate::prof::mark("market: back to named maps");
        // Savings of pops that are gone leave their bank.
        savings.retain(|id, (at, x)| {
            let stays = seen.contains(id);
            if let Some(b) = banks.get_mut(at).filter(|_| !stays) {
                b.bank = (b.bank - *x).max(0.0);
            }
            stays
        });
        for (tag, x) in &tax_base {
            if let Some(b) = banks.get_mut(tag) {
                b.tax_base = *x;
            }
        }

        crate::prof::mark("market: bank cleanup");
        // Factories' inputs count as demand as far as their budgets went.
        for ((owner, good), x) in &factory_wanted {
            *demand.entry(good.clone()).or_default() += x;
            *wanted.entry((owner.clone(), good.clone())).or_default() += x;
        }
        for (key, x) in &made_by_artisans {
            *made.entry(key.clone()).or_default() += x;
        }
        // The reports' split: farms and mines, factories, artisans; states, factories, pops.
        for (key, x) in &made {
            let (farms, artisans) = (made_by_farms.get(key).copied().unwrap_or(0.0), made_by_artisans.get(key).copied().unwrap_or(0.0));
            day.made_by.insert(key.clone(), [farms, (x - farms - artisans).max(0.0), artisans]);
        }
        for (key, x) in &wanted {
            let (states, factories) = (wanted_by_states.get(key).copied().unwrap_or(0.0), factory_wanted.get(key).copied().unwrap_or(0.0));
            day.wanted_by.insert(key.clone(), [states, factories, (x - states - factories).max(0.0)]);
        }
        let at_war: BTreeSet<&String> = state.wars.iter().flat_map(|(a, d)| a.iter().chain(d.iter())).collect();
        for (tag, change) in treasury {
            let balance = change - spent_by.get(&tag).copied().unwrap_or(0.0);
            day.balance.insert(tag.clone(), balance);
            let Some(c) = state.countries.get_mut(&tag) else { continue };
            c.money += change;
            c.tax_income = taxed.get(&tag).copied().unwrap_or(0.0);
            // The computer player sets its budget again (`economy::ai_budget_solve`).
            if !c.human && !c.modifiers.iter().any(|m| m.name == "in_bankrupcy") {
                let o = oidx.get(tag.as_str()).copied();
                let strata = |k: usize| o.and_then(|o| base_by_strata.get(o * 3 + k)).unwrap_or(0.0);
                let pay = |name: &str| full_pay.get(&(tag.clone(), name)).copied().unwrap_or(0.0);
                let (education, administration) = (pay("education_spending"), pay("crime_fighting"));
                let now = |k: &str| c.spending.get(k).copied().unwrap_or(0.0);
                let duties = day.duties.get(&tag).copied().unwrap_or(0.0);
                let imports = o.and_then(|o| imports_d.get(o)).unwrap_or(0.0) + factory_imports.get(&tag).copied().unwrap_or(0.0);
                let d = economy::BudgetDay {
                    tax_base: [strata(0), strata(1), strata(2)],
                    tariff_base: imports * tariff_efficiency.get(&tag).copied().unwrap_or(0.0),
                    education,
                    administration,
                    other: balance - c.tax_income - duties + now("education_spending") * education + now("crime_fighting") * administration,
                    spending: spent_by.get(&tag).copied().unwrap_or(0.0),
                    treasury: c.money,
                };
                let [min_tax, max_tax, _, _, max_tariff] = c.party_limits.unwrap_or([0.0, 1.0, 1.0, -1.0, 1.0]);
                let x = economy::ai_budget_solve(&self.ai_budget, &d, [min_tax, max_tax, max_tariff], at_war.contains(&tag));
                for (k, v) in [("poor_tax", x.taxes[0]), ("middle_tax", x.taxes[1]), ("rich_tax", x.taxes[2]), ("tariffs", x.tariff), ("education_spending", x.education), ("crime_fighting", x.administration)] {
                    c.spending.insert(k.to_string(), v);
                }
            }
            // The computer player pays loans back out of a surplus (ai_home, sliders 11).
            let Some(m) = state.money.countries.get_mut(&tag) else { continue };
            let b = &self.ai_budget;
            let mut back = (balance * b.repay_share).min(b.repay_max);
            if c.human || at_war.contains(&tag) || m.loans.is_empty() || c.money <= b.repay_treasury || back <= b.repay_min {
                continue;
            }
            let mut returned: Vec<(String, f64)> = Vec::new();
            for l in &mut m.loans {
                let x = l.debt.min(back);
                (l.debt, back) = (l.debt - x, back - x);
                returned.push((l.lender.clone(), x));
            }
            m.loans.retain(|l| l.debt > 1e-9);
            for (lender, x) in returned {
                c.money -= x;
                state.money.totals.paid_back += x;
                if let Some(bank) = state.money.countries.get_mut(&lender) {
                    bank.lent = (bank.lent - x).max(0.0);
                }
            }
        }
        (state.money.totals.days, state.money.totals.tax, state.money.totals.tariffs) = (state.money.totals.days + 1.0, state.money.totals.tax + day.tax, state.money.totals.tariffs + day.tariffs);

        crate::prof::mark("market: factory input demand");
        // Home markets for tomorrow (`economy::clear_good`), with the ranking as it stood this
        // morning, i.e. yesterday's.
        let mut leader: BTreeMap<String, (String, f64)> = BTreeMap::new();
        for (tag, c) in &state.countries {
            for member in &c.sphere {
                let civilised = state.countries.get(member).is_some_and(|m| m.civilized);
                leader.insert(member.clone(), (tag.clone(), if civilised { self.sphere_share.0 } else { self.sphere_share.1 }));
            }
        }
        type ByCountry = BTreeMap<String, f64>;
        let mut by_good: BTreeMap<&str, (ByCountry, ByCountry)> = BTreeMap::new();
        for ((tag, good), x) in &made {
            by_good.entry(good.as_str()).or_default().0.insert(tag.clone(), *x);
        }
        for ((tag, good), x) in &wanted {
            by_good.entry(good.as_str()).or_default().1.insert(tag.clone(), *x);
        }
        // Uncivilised countries import whole goods or none, and have what they got today (not
        // half-way from yesterday): their life needs met is made of whole goods.
        let whole: BTreeSet<String> = state.countries.iter().filter(|(_, c)| !c.civilized).map(|(t, _)| t.clone()).collect();
        for (good, (made, wanted)) in &by_good {
            for (tag, x) in economy::clear_good_whole(&state.rank_order, made, wanted, &leader, &whole) {
                let at_once = whole.contains(&tag);
                let m = state.home_market.entry(tag).or_default().entry(good.to_string()).or_insert([1.0, 1.0, 0.0]);
                for (i, new) in [x.fill, x.sold, x.imported].into_iter().enumerate() {
                    if let Some(new) = new {
                        m[i] = if at_once && i == 0 { new } else { (m[i] + new) / 2.0 };
                    }
                }
            }
        }

        crate::prof::mark("market: home markets");
        // Clear the market for tomorrow and move the prices.
        let goods: BTreeSet<String> = supply.keys().chain(demand.keys()).cloned().collect();
        for good in goods {
            let (s, d) = (supply.get(&good).copied().unwrap_or(0.0), demand.get(&good).copied().unwrap_or(0.0));
            let (new_fill, new_sold) = (if d > 0.0 { (s / d).min(1.0) } else { 1.0 }, if s > 0.0 { (d / s).min(1.0) } else { 1.0 });
            let f = state.market_fill.entry(good.clone()).or_insert(1.0);
            *f = (*f + new_fill) / 2.0;
            let x = state.market_sold.entry(good.clone()).or_insert(1.0);
            *x = (*x + new_sold) / 2.0;
            if let (Some((base, fixed)), Some(p)) = (self.goods.get(&good), state.prices.get_mut(&good)) {
                let mut m = GoodMarket { base: Fixed::from_f64(*base), price: Fixed::from_f64(*p), fixed_price: *fixed, supply: Fixed::from_f64(s), real_demand: Fixed::from_f64(d), price_change: Fixed::ZERO };
                m.update_price(&self.price_rules);
                *p = m.price.to_f64();
            }
        }
        crate::prof::mark("market: clear market, prices");
        // The capitalists' money of the day goes into one project per country: the one of the
        // state with the best score; ties by its capitalists, then its people.
        for (tag, sum) in invested {
            let capital = state.countries.get(&tag).and_then(|c| c.capital);
            let best = state
                .projects
                .keys()
                .filter(|k| k.0 == tag)
                .map(|key| {
                    let here: Vec<(&u16, &crate::script::ProvinceState)> = state.provinces.iter().filter(|(_, p)| p.owner.as_ref() == Some(&tag) && p.region == key.1).collect();
                    let score = economy::investment_state_score(&self.numbers, &here.iter().map(|(id, p)| (Some(**id) == capital, p.cores.contains(&tag))).collect::<Vec<_>>(), state.factories.iter().filter(|f| f.owner == tag && f.region == key.1).count(), here.iter().any(|(_, p)| p.colonial != 0));
                    let (investors, all) = here.iter().flat_map(|(_, p)| &p.pops).fold((0.0, 0.0), |a, q| (a.0 + if self.factory_owners.contains(&q.pop_type) { q.size } else { 0.0 }, a.1 + q.size));
                    (score, investors, all, key.clone())
                })
                .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)));
            if let Some((_, _, _, key)) = best {
                *state.investment.entry(key).or_default() += sum;
            }
        }
        for (index, x) in factory_changes {
            if let Some(f) = state.factories.get_mut(index) {
                (f.money, f.income, f.spending, f.pay, f.staff) = (x.money, x.income, x.spending, x.pay, Some(x.staff));
                (f.level, f.no_input_days, f.loss_days, f.closed) = (x.level, x.no_input_days, x.loss_days, x.level <= 1.0 && x.no_input_days >= self.numbers.factory_no_input_days);
            }
        }
        crate::prof::mark("market: capitalist projects");
        // Farms and mines. Where the owner lost control everybody is let go;
        // when it is back, hiring is fast for some days. Hiring and firing by today's supply and
        // paid demand of the good (`rgo_staff_step`): see `RGO_FOLLOWS_MARKET`.
        let work: BTreeMap<u16, (f64, f64)> = work.into_iter().map(|w| (w.id, (w.employed, w.capacity))).collect();
        for (id, p) in &mut state.provinces {
            let Some(good) = p.trade_goods.as_deref() else { continue };
            if !economy::owner_controls(p) {
                (p.rgo_staff, p.rgo_fast_days) = (Some(0.0), self.numbers.rgo_fast_hire_days);
                continue;
            }
            let Some((employed, capacity)) = work.get(id).copied() else { continue };
            let fixed = self.goods.get(good).is_some_and(|g| g.1);
            let wanted = if fixed { None } else { demand.get(good).copied() };
            let staff = economy::rgo_staff_step(employed, capacity, supply.get(good).copied().unwrap_or(0.0), wanted, p.rgo_fast_days > 0, &self.hiring);
            p.rgo_staff = if staff >= capacity || !RGO_FOLLOWS_MARKET { None } else { Some(staff) };
            p.rgo_fast_days = p.rgo_fast_days.saturating_sub(1);
        }
        crate::prof::mark("market: farm hiring");
        // All the world's money tonight, and from its change the discount pops get tomorrow.
        let total = landed.iter().filter_map(|tag| state.countries.get(tag)).map(|c| c.money).sum::<f64>()
            + state.money.countries.values().map(|m| (m.bank - m.lent).max(0.0) + m.constructions.iter().map(|x| x.money).sum::<f64>()).sum::<f64>()
            + state.investment.values().sum::<f64>()
            + state.factories.iter().map(|f| f.money).sum::<f64>() / self.pop_money_scale
            + pops_cash;
        (state.money.discount, state.money.world_money) = (economy::pop_discount(&self.numbers, total, state.money.world_money, gold), Some(total));
        crate::prof::mark("market: world money");
        state.seed = seed;
        day.supply = supply;
        day.demand = demand;
        day
    }
}

/// Whether farms and mines hire and fire by supply over demand. It was
/// switched off while the port's pops paid for far less than the original's (most of the money
/// sat in the treasuries; over 90 days from 1836 farm income fell 29%). On again since the pops
/// keep their money: over the year 1836 farm and mine output stays within 1% and farm income
/// rises.
const RGO_FOLLOWS_MARKET: bool = true;

/// What a factory's day changes in its record.
struct FactoryChange {
    money: f64,
    income: f64,
    spending: f64,
    pay: f64,
    staff: f64,
    level: f64,
    no_input_days: u32,
    loss_days: u32,
}

impl Farms {
    /// The countries that own land in overall rank order, best first: civilised
    /// before uncivilised, then by prestige + industrial score. The military score is missing
    /// (no armies in the port) and counts as 0; so does the score for money invested abroad.
    pub fn ranking(&self, world: &World, state: &State) -> Vec<String> {
        // Per (owner, state): levels, places, people who can work in a factory.
        let mut states: BTreeMap<(&str, Option<u16>), (f64, f64, f64)> = BTreeMap::new();
        for f in state.factories.iter().filter(|f| f.level >= 1.0) {
            if let Some(t) = world.defs.factory_types.iter().find(|t| t.name == f.kind) {
                let e = states.entry((f.owner.as_str(), f.region)).or_default();
                (e.0, e.1) = (e.0 + f.level, e.1 + f.level * t.workforce);
            }
        }
        let mut owners: BTreeSet<&str> = BTreeSet::new();
        for p in state.provinces.values() {
            let Some(owner) = p.owner.as_deref() else { continue };
            owners.insert(owner);
            if let Some(e) = states.get_mut(&(owner, p.region)) {
                e.2 += p.pops.iter().filter(|q| self.factory_workers.contains(&q.pop_type)).map(|q| q.size).sum::<f64>();
            }
        }
        let mut by_country: BTreeMap<&str, Vec<(f64, f64, f64)>> = BTreeMap::new();
        for ((owner, _), x) in states {
            by_country.entry(owner).or_default().push(x);
        }
        economy::rank_order(owners.into_iter().filter_map(|tag| state.countries.get(tag).map(|c| (tag.to_string(), c.civilized, c.prestige + economy::industrial_score(&self.numbers, by_country.get(tag).map_or(&[], Vec::as_slice))))).collect())
    }
}
