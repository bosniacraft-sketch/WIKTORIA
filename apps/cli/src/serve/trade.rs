//! The Trade window (apps/web/API.md: Trade): every good with its price and trend, and one good in
//! detail for the seat. The engine keeps today's price only, so this module keeps a price book of
//! its own: one row of prices per game day (`after_day`, called by the simulation thread), at most
//! `KEPT_DAYS`, with the last day's market report (supply and paid demand, who made and bought
//! what). Nothing here changes the simulation.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use wiktoria_data::date::Date;
use wiktoria_simulation::economy::FactoryFlow;
use wiktoria_simulation::game::{DayLog, Game};

use super::{country_name, json_num, json_opt, json_opt_str, json_str, lock, region_name, Shared};

/// Days of prices kept (three years).
pub(super) const KEPT_DAYS: usize = 3 * 365;
/// Days of history sent when the page does not say.
const SENT_DAYS: usize = 365;
/// Countries listed as the largest makers and buyers of a good.
const TOP: usize = 8;

/// The price book and the last day's market report.
#[derive(Default)]
pub(super) struct Book {
    /// The date of the newest row; rows are consecutive days.
    last: Option<Date>,
    rows: usize,
    prices: BTreeMap<String, VecDeque<f64>>,
    supply: BTreeMap<String, f64>,
    demand: BTreeMap<String, f64>,
    made_by: BTreeMap<(String, String), [f64; 3]>,
    wanted_by: BTreeMap<(String, String), [f64; 3]>,
    factories: Vec<FactoryFlow>,
}

impl Book {
    /// Adds the prices of `date` (replacing that date's row if it is there already).
    pub(super) fn sample(&mut self, date: Date, prices: &BTreeMap<String, f64>) {
        let again = self.last == Some(date);
        if !again {
            self.rows += 1;
        }
        self.last = Some(date);
        for (good, p) in prices {
            let row = self.prices.entry(good.clone()).or_default();
            if again {
                row.pop_back();
            }
            // A good that appears late is filled with its first price, so all rows line up.
            while row.len() + 1 < self.rows {
                row.push_front(*p);
            }
            row.push_back(*p);
        }
        if self.rows > KEPT_DAYS {
            self.rows = KEPT_DAYS;
        }
        for row in self.prices.values_mut() {
            while row.len() > self.rows {
                row.pop_front();
            }
        }
    }

    /// The price `days` days before the newest row, if the book reaches back that far.
    pub(super) fn ago(&self, good: &str, days: usize) -> Option<f64> {
        let row = self.prices.get(good)?;
        row.len().checked_sub(days + 1).and_then(|i| row.get(i)).copied()
    }

    /// The last `days` prices, oldest first, and the date of the first one.
    fn history(&self, good: &str, days: usize) -> (Option<Date>, Vec<f64>) {
        let Some(row) = self.prices.get(good) else { return (None, Vec::new()) };
        let n = row.len().min(days);
        let first = self.last.map(|d| days_back(d, n.saturating_sub(1)));
        (first, row.iter().skip(row.len() - n).copied().collect())
    }
}

/// The date `n` days before `d` (the game's calendar has no leap days).
fn days_back(mut d: Date, n: usize) -> Date {
    const LEN: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for _ in 0..n {
        if d.day > 1 {
            d.day -= 1;
        } else if d.month > 1 {
            d.month -= 1;
            d.day = LEN[d.month as usize - 1];
        } else {
            (d.year, d.month, d.day) = (d.year - 1, 12, 31);
        }
    }
    d
}

static BOOK: Mutex<Option<Book>> = Mutex::new(None);

/// After each day (the simulation thread): the day's prices and market report.
pub(super) fn after_day(g: &Game, log: &DayLog) {
    let mut b = lock(&BOOK);
    let b = b.get_or_insert_with(Book::default);
    b.sample(g.state.date, &g.state.prices);
    let m = &log.money;
    (b.supply, b.demand) = (m.supply.clone(), m.demand.clone());
    (b.made_by, b.wanted_by, b.factories) = (m.made_by.clone(), m.wanted_by.clone(), m.factories.clone());
}

/// A good as the goods file describes it.
struct GoodDef {
    key: String,
    category: String,
    cost: f64,
    color: [u8; 3],
}

/// The goods file in its order, read once; empty when it cannot be read (the goods are then
/// those that have a price, without a group).
fn goods_file(sh: &Shared) -> &'static [GoodDef] {
    static GOODS: std::sync::OnceLock<Vec<GoodDef>> = std::sync::OnceLock::new();
    GOODS.get_or_init(|| {
        let read = || -> Result<Vec<GoodDef>, String> {
            let engine_rules = crate::config::load_engine_rules(&crate::root())?;
            let mods: Vec<std::path::PathBuf> = sh.mods.iter().map(std::path::PathBuf::from).collect();
            let path = wiktoria_data::install_fs::InstallFs::with_mods(&sh.install, &mods)?.file(engine_rules.require_str("FLD_GOODS_FILE")?);
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let doc = wiktoria_data::script::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(wiktoria_data::goods::load_goods(&doc)?.goods.into_iter().map(|g| GoodDef { key: g.id, category: g.category, cost: g.cost, color: g.color }).collect())
        };
        read().unwrap_or_else(|e| {
            eprintln!("trade: goods file not read ({e}); goods are listed without groups");
            Vec::new()
        })
    })
}

fn opt_num(v: Option<f64>) -> String {
    json_opt(v.filter(|x| x.is_finite()))
}

fn triple(x: Option<&[f64; 3]>, names: [&str; 3]) -> String {
    match x {
        Some(x) => format!("{{\"{}\":{},\"{}\":{},\"{}\":{},\"total\":{}}}", names[0], json_num(x[0]), names[1], json_num(x[1]), names[2], json_num(x[2]), json_num(x[0] + x[1] + x[2])),
        None => format!("{{\"{}\":null,\"{}\":null,\"{}\":null,\"total\":null}}", names[0], names[1], names[2]),
    }
}

/// The sphere the seat is in (its leader, or itself when it leads one), leader first.
fn sphere_of(g: &Game, tag: &str) -> Option<Vec<String>> {
    let s = &g.state;
    let leader = if s.countries.get(tag).is_some_and(|c| !c.sphere.is_empty()) { tag.to_string() } else { s.countries.iter().find(|(_, c)| c.sphere.contains(tag))?.0.clone() };
    let members = &s.countries.get(&leader)?.sphere;
    Some(std::iter::once(leader.clone()).chain(members.iter().filter(|m| **m != leader).cloned()).collect())
}

/// `GET /api/trade?good=<key>&days=<n>`.
pub(super) fn trade_json(sh: &Shared, g: &Game, tag: &str, good: Option<&str>, days: Option<usize>) -> String {
    let s = &g.state;
    let mut guard = lock(&BOOK);
    let book = guard.get_or_insert_with(Book::default);
    if book.last.is_none() {
        book.sample(s.date, &s.prices);
    }
    let book: &Book = book;
    let file = goods_file(sh);
    // Every good: the file's order where it has one, then any other good with a price.
    let mut keys: Vec<(&str, Option<&GoodDef>)> = file.iter().map(|d| (d.key.as_str(), Some(d))).collect();
    keys.extend(s.prices.keys().filter(|k| !file.iter().any(|d| d.key == **k)).map(|k| (k.as_str(), None)));
    let rows: Vec<String> = keys
        .iter()
        .map(|(key, def)| {
            let price = s.prices.get(*key).copied();
            format!(
                "{{\"key\":{},\"name\":{},\"group\":{},\"group_name\":{},\"color\":{},\"price\":{},\"base\":{},\"yesterday\":{},\"month\":{},\"on_sale\":{},\"supply\":{},\"demand\":{}}}",
                json_str(key),
                json_str(&sh.name(key)),
                json_opt_str(def.map(|d| d.category.as_str())),
                json_opt_str(def.map(|d| sh.name(&d.category)).as_deref()),
                def.map_or("null".into(), |d| format!("[{},{},{}]", d.color[0], d.color[1], d.color[2])),
                opt_num(price),
                opt_num(def.map(|d| d.cost)),
                opt_num(book.ago(key, 1)),
                opt_num(book.ago(key, 30)),
                s.goods_on_sale.is_empty() || s.goods_on_sale.contains(*key),
                opt_num(book.supply.get(*key).copied()),
                opt_num(book.demand.get(*key).copied()),
            )
        })
        .collect();
    let chosen = good.filter(|k| keys.iter().any(|(x, _)| x == k)).map(|k| good_json(sh, g, book, tag, k, keys.iter().find(|(x, _)| *x == k).and_then(|x| x.1), days.unwrap_or(SENT_DAYS).clamp(2, KEPT_DAYS)));
    format!(
        "{{\"tag\":{},\"played\":{},\"date\":{},\"days_kept\":{},\"goods\":[{}],\"good\":{}}}",
        json_str(tag),
        s.countries.get(tag).is_some_and(|c| c.human),
        json_str(&s.date.to_string()),
        book.rows,
        rows.join(","),
        chosen.unwrap_or_else(|| "null".into())
    )
}

/// One good in detail for the seat.
fn good_json(sh: &Shared, g: &Game, book: &Book, tag: &str, key: &str, def: Option<&GoodDef>, days: usize) -> String {
    let s = &g.state;
    let at = |k: &(String, String)| (book.made_by.get(k), book.wanted_by.get(k));
    let me = (tag.to_string(), key.to_string());
    let (made, wanted) = at(&me);
    let home = s.home_market.get(tag).and_then(|m| m.get(key));
    let (first, history) = book.history(key, days);
    // Who makes and buys most of it in the world.
    let mut makers: Vec<(&str, f64)> = book.made_by.iter().filter(|((_, gd), _)| gd == key).map(|((t, _), x)| (t.as_str(), x.iter().sum())).filter(|x| x.1 > 0.0).collect();
    let mut buyers: Vec<(&str, f64)> = book.wanted_by.iter().filter(|((_, gd), _)| gd == key).map(|((t, _), x)| (t.as_str(), x.iter().sum())).filter(|x| x.1 > 0.0).collect();
    let world_made: f64 = makers.iter().map(|x| x.1).sum();
    let world_bought: f64 = buyers.iter().map(|x| x.1).sum();
    let top = |list: &mut Vec<(&str, f64)>, total: f64| -> String {
        list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        let mut shown: Vec<(usize, &str, f64)> = list.iter().enumerate().take(TOP).map(|(i, x)| (i + 1, x.0, x.1)).collect();
        if let Some(i) = list.iter().position(|x| x.0 == tag).filter(|i| *i >= TOP) {
            shown.push((i + 1, list[i].0, list[i].1));
        }
        let rows: Vec<String> = shown.iter().map(|(rank, t, x)| format!("{{\"rank\":{rank},\"tag\":{},\"name\":{},\"units\":{},\"share\":{}}}", json_str(t), json_str(&country_name(g, t)), json_num(*x), opt_num((total > 0.0).then(|| x / total)))).collect();
        format!("[{}]", rows.join(","))
    };
    let (makers, buyers) = (top(&mut makers, world_made), top(&mut buyers, world_bought));
    // The seat's factories that make it or use it.
    let flows: BTreeMap<usize, &FactoryFlow> = book.factories.iter().map(|f| (f.index, f)).collect();
    let factories: Vec<String> = s
        .factories
        .iter()
        .enumerate()
        .filter(|(_, f)| f.owner == tag)
        .filter_map(|(i, f)| {
            let flow = flows.get(&i);
            let makes = s.factory_types.get(&f.kind).is_some_and(|t| t.0 == key);
            let uses = flow.and_then(|x| x.wanted.iter().find(|w| w.0 == key)).map(|w| w.1);
            (makes || uses.is_some()).then(|| {
                format!(
                    "{{\"kind\":{},\"name\":{},\"state\":{},\"level\":{},\"makes\":{},\"made\":{},\"used\":{},\"closed\":{},\"building\":{}}}",
                    json_str(&f.kind),
                    json_str(&sh.name(&f.kind)),
                    json_opt_str(region_name(sh, g, f.region).as_deref()),
                    json_num(f.level),
                    makes,
                    opt_num(if makes { flow.map(|x| x.made) } else { None }),
                    opt_num(uses),
                    f.closed,
                    f.building_days.is_some()
                )
            })
        })
        .collect();
    // Government: what army, navy and building work need, the stockpile and its sliders.
    let money = s.money.countries.get(tag);
    let need = |m: Option<&BTreeMap<String, f64>>| opt_num(m.map(|m| m.get(key).copied().unwrap_or(0.0)));
    let sliders = money.map_or("null".into(), |m| format!("{{\"land\":{},\"naval\":{},\"projects\":{}}}", json_num(m.sliders[0]), json_num(m.sliders[1]), json_num(m.sliders[2])));
    // The common market: the sphere's members.
    let common = match sphere_of(g, tag) {
        Some(list) => {
            let rows: Vec<String> = list
                .iter()
                .map(|t| {
                    let k = (t.clone(), key.to_string());
                    let (m, w) = at(&k);
                    format!("{{\"tag\":{},\"name\":{},\"made\":{},\"bought\":{}}}", json_str(t), json_str(&country_name(g, t)), opt_num(m.map(|x| x.iter().sum())), opt_num(w.map(|x| x.iter().sum())))
                })
                .collect();
            format!("{{\"leader\":{},\"members\":[{}]}}", json_str(&list[0]), rows.join(","))
        }
        None => "null".into(),
    };
    let reported = !book.made_by.is_empty() || !book.wanted_by.is_empty();
    let world = format!(
        "{{\"supply\":{},\"demand\":{},\"fill\":{},\"sold\":{}}}",
        opt_num(book.supply.get(key).copied().or(reported.then_some(0.0))),
        opt_num(book.demand.get(key).copied().or(reported.then_some(0.0))),
        opt_num(s.market_fill.get(key).copied()),
        opt_num(s.market_sold.get(key).copied())
    );
    let zero = [0.0; 3];
    let made = made.or(reported.then_some(&zero));
    let wanted = wanted.or(reported.then_some(&zero));
    format!(
        "{{\"key\":{},\"name\":{},\"group_name\":{},\"price\":{},\"base\":{},\"yesterday\":{},\"month\":{},\"history\":{{\"from\":{},\"prices\":[{}]}},\"world\":{world},\
\"seat\":{{\"made\":{},\"bought\":{},\"fill\":{},\"sold\":{},\"imported\":{}}},\
\"government\":{{\"land\":{},\"naval\":{},\"construction\":{},\"bought\":{}}},\
\"stockpile\":{{\"amount\":{},\"sliders\":{sliders}}},\
\"factories\":[{}],\"population\":{{\"bought\":{},\"artisans_made\":{}}},\
\"market\":{{\"makers\":{makers},\"buyers\":{buyers}}},\"common\":{common},\
\"orders\":{{\"buy\":false,\"sell\":false,\"automate\":false,\"stockpile\":false}}}}",
        json_str(key),
        json_str(&sh.name(key)),
        json_opt_str(def.map(|d| sh.name(&d.category)).as_deref()),
        opt_num(s.prices.get(key).copied()),
        opt_num(def.map(|d| d.cost)),
        opt_num(book.ago(key, 1)),
        opt_num(book.ago(key, 30)),
        json_opt_str(first.map(|d| d.to_string()).as_deref()),
        history.iter().map(|p| json_num(*p)).collect::<Vec<_>>().join(","),
        triple(made, ["farms", "factories", "artisans"]),
        triple(wanted, ["state", "factories", "pops"]),
        opt_num(home.map(|x| x[0])),
        opt_num(home.map(|x| x[1])),
        opt_num(home.map(|x| x[2])),
        need(money.map(|m| &m.land_need)),
        need(money.map(|m| &m.naval_need)),
        need(money.map(|m| &m.construction_need)),
        opt_num(wanted.map(|x| x[0])),
        need(money.map(|m| &m.stockpile)),
        factories.join(","),
        opt_num(wanted.map(|x| x[2])),
        opt_num(made.map(|x| x[2])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices(x: f64) -> BTreeMap<String, f64> {
        BTreeMap::from([("grain".to_string(), x), ("iron".to_string(), x * 2.0)])
    }

    #[test]
    fn the_price_book_keeps_one_row_a_day() {
        let mut b = Book::default();
        b.sample(Date::new(1836, 1, 1), &prices(1.0));
        b.sample(Date::new(1836, 1, 1), &prices(1.5));
        assert_eq!(b.rows, 1);
        assert_eq!(b.ago("grain", 0), Some(1.5));
        assert_eq!(b.ago("grain", 1), None);
        b.sample(Date::new(1836, 1, 2), &prices(2.0));
        assert_eq!((b.ago("grain", 1), b.ago("iron", 0)), (Some(1.5), Some(4.0)));
        // A good first priced later lines up with the others.
        let mut late = prices(3.0);
        late.insert("tea".into(), 7.0);
        b.sample(Date::new(1836, 1, 3), &late);
        assert_eq!((b.ago("tea", 2), b.ago("tea", 0), b.ago("tea", 3)), (Some(7.0), Some(7.0), None));
        let (first, h) = b.history("grain", 10);
        assert_eq!((first, h), (Some(Date::new(1836, 1, 1)), vec![1.5, 2.0, 3.0]));
    }

    #[test]
    fn the_price_book_forgets_the_oldest_days() {
        let mut b = Book::default();
        let mut d = Date::new(1836, 1, 1);
        for i in 0..KEPT_DAYS + 10 {
            b.sample(d, &prices(i as f64));
            d = if d.day < 28 { Date::new(d.year, d.month, d.day + 1) } else if d.month < 12 { Date::new(d.year, d.month + 1, 1) } else { Date::new(d.year + 1, 1, 1) };
        }
        assert_eq!(b.rows, KEPT_DAYS);
        assert_eq!(b.prices["grain"].len(), KEPT_DAYS);
        assert_eq!(b.ago("grain", 0), Some((KEPT_DAYS + 9) as f64));
        assert_eq!(b.ago("grain", 30), Some((KEPT_DAYS + 9 - 30) as f64));
        assert_eq!(b.history("grain", 5).1.len(), 5);
    }

    #[test]
    fn dates_count_back_over_months_and_years() {
        assert_eq!(days_back(Date::new(1836, 3, 1), 1), Date::new(1836, 2, 28));
        assert_eq!(days_back(Date::new(1836, 1, 1), 1), Date::new(1835, 12, 31));
        assert_eq!(days_back(Date::new(1836, 1, 10), 0), Date::new(1836, 1, 10));
        assert_eq!(days_back(Date::new(1837, 1, 1), 365), Date::new(1836, 1, 1));
    }

    #[test]
    fn a_split_is_written_with_its_total_or_as_nulls() {
        assert_eq!(triple(Some(&[1.0, 2.0, 0.5]), ["a", "b", "c"]), "{\"a\":1.0,\"b\":2.0,\"c\":0.5,\"total\":3.5}");
        assert_eq!(triple(None, ["a", "b", "c"]), "{\"a\":null,\"b\":null,\"c\":null,\"total\":null}");
    }
}
