//! Day-to-day comparison of pops across a series of saves (for example one autosave per day).
//!
//! This is an analysis tool: it measures what the original game changes each day so the
//! findings can go into the rules file. It contains no engine rules itself.
//!
//! Pops are matched across saves by their save `id`.

use std::collections::{BTreeMap, HashMap};

use crate::date::Date;
use crate::map::ProvinceId;
use crate::save::{SaveGame, SavePop};

/// The pop fields whose changes are counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Size,
    Ideology,
    Issues,
    Consciousness,
    Militancy,
    Literacy,
    Money,
}

impl Field {
    pub const ALL: [Field; 7] =
        [Field::Size, Field::Ideology, Field::Issues, Field::Consciousness, Field::Militancy, Field::Literacy, Field::Money];

    pub fn label(self) -> &'static str {
        match self {
            Field::Size => "size",
            Field::Ideology => "ideology",
            Field::Issues => "issues",
            Field::Consciousness => "con",
            Field::Militancy => "mil",
            Field::Literacy => "literacy",
            Field::Money => "money",
        }
    }

    fn changed(self, a: &SavePop, b: &SavePop) -> bool {
        match self {
            Field::Size => a.size != b.size,
            Field::Ideology => a.ideology != b.ideology,
            Field::Issues => a.issues != b.issues,
            Field::Consciousness => a.consciousness != b.consciousness,
            Field::Militancy => a.militancy != b.militancy,
            Field::Literacy => a.literacy != b.literacy,
            Field::Money => a.money != b.money,
        }
    }
}

/// All pops of one save, by id, with the province each one lives in.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub date: Date,
    pub pops: HashMap<u64, (ProvinceId, SavePop)>,
}

impl Snapshot {
    pub fn from_save(save: &SaveGame) -> Snapshot {
        let mut pops = HashMap::new();
        for (&prov, p) in &save.provinces {
            for pop in &p.pops {
                pops.insert(pop.id, (prov, pop.clone()));
            }
        }
        Snapshot { date: save.date, pops }
    }

    pub fn population(&self) -> f64 {
        self.pops.values().map(|(_, p)| p.size).sum()
    }
}

/// What changed between two consecutive snapshots.
#[derive(Debug, Clone, Default)]
pub struct DayDiff {
    pub pops: usize,
    pub population: f64,
    pub new: usize,
    pub gone: usize,
    pub grew: usize,
    pub shrank: usize,
    /// Number of pops present on both days whose field changed.
    pub changed: BTreeMap<Field, usize>,
}

pub fn diff(a: &Snapshot, b: &Snapshot) -> DayDiff {
    let mut d = DayDiff { pops: b.pops.len(), population: b.population(), ..Default::default() };
    for (id, (_, pb)) in &b.pops {
        let Some((_, pa)) = a.pops.get(id) else {
            d.new += 1;
            continue;
        };
        if pb.size > pa.size {
            d.grew += 1;
        } else if pb.size < pa.size {
            d.shrank += 1;
        }
        for f in Field::ALL {
            if f.changed(pa, pb) {
                *d.changed.entry(f).or_default() += 1;
            }
        }
    }
    d.gone = a.pops.keys().filter(|id| !b.pops.contains_key(id)).count();
    d
}

/// Running day number for each snapshot of a consecutive daily series, counted from the first:
/// +difference of the day of month, or +1 when the day of month starts over. (The month length
/// itself is not needed for consecutive saves.)
pub fn running_days(series: &[Snapshot]) -> Vec<i64> {
    let mut out = Vec::with_capacity(series.len());
    let mut n = 0i64;
    for (i, s) in series.iter().enumerate() {
        if i > 0 {
            let prev = series[i - 1].date;
            let same_month = (prev.year, prev.month) == (s.date.year, s.date.month);
            n += if same_month { i64::from(s.date.day) - i64::from(prev.day) } else { 1 };
        }
        out.push(n);
    }
    out
}

/// How a field's daily changes line up with `(day - pop id) mod m`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleFit {
    pub modulus: u64,
    pub residue: u64,
    /// Share of all changes that fall on the best residue.
    pub share: f64,
    pub changes: usize,
}

/// Which day number the schedule is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayBasis {
    DayOfMonth,
    Running,
}

/// Finds, for one field, the modulus in `moduli` and the residue r for which the most changes
/// happen when `(day - id) mod modulus == r`. Pairs whose second save is on `skip_day_of_month`
/// (for example the 1st, which can behave differently) are left out when given.
pub fn fit_schedule(series: &[Snapshot], field: Field, basis: DayBasis, moduli: &[u64], skip_day_of_month: Option<u8>) -> Option<ScheduleFit> {
    let running = running_days(series);
    let mut best: Option<ScheduleFit> = None;
    for &m in moduli {
        if m == 0 {
            continue;
        }
        let mut counts = vec![0usize; m as usize];
        let mut total = 0usize;
        for i in 1..series.len() {
            let (a, b) = (&series[i - 1], &series[i]);
            if skip_day_of_month == Some(b.date.day) {
                continue;
            }
            let day = match basis {
                DayBasis::DayOfMonth => i64::from(b.date.day),
                DayBasis::Running => running[i],
            };
            for (id, (_, pb)) in &b.pops {
                if let Some((_, pa)) = a.pops.get(id)
                    && field.changed(pa, pb)
                {
                    let r = (day - (*id % m) as i64).rem_euclid(m as i64) as usize;
                    counts[r] += 1;
                    total += 1;
                }
            }
        }
        if total == 0 {
            continue;
        }
        let (residue, top) = counts.iter().enumerate().max_by_key(|(_, c)| **c).map(|(r, c)| (r as u64, *c)).unwrap_or((0, 0));
        let fit = ScheduleFit { modulus: m, residue, share: top as f64 / total as f64, changes: total };
        if best.as_ref().is_none_or(|b| fit.share > b.share) {
            best = Some(fit);
        }
    }
    best
}

/// Where a pop that is new on day b most likely came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NewPopSource {
    /// A pop in the same province shrank by at least the new pop's size; lists which of
    /// type / culture / religion differ between that pop and the new one.
    SplitFrom { differs: Vec<&'static str> },
    /// No pop in the same province shrank enough: people came from elsewhere.
    NoLocalDonor,
}

impl NewPopSource {
    pub fn label(&self) -> String {
        match self {
            NewPopSource::SplitFrom { differs } if differs.is_empty() => "split, same type/culture/religion".into(),
            NewPopSource::SplitFrom { differs } => format!("split, new {}", differs.join(" + ")),
            NewPopSource::NoLocalDonor => "no shrinking pop in the province (moved in)".into(),
        }
    }
}

/// Classifies every pop new on day b. The donor is the same-province pop with the smallest loss
/// that still covers the new pop's size.
pub fn new_pop_sources(a: &Snapshot, b: &Snapshot) -> Vec<(NewPopSource, f64)> {
    let mut losers: HashMap<ProvinceId, Vec<(f64, &SavePop)>> = HashMap::new();
    for (id, (prov, pb)) in &b.pops {
        if let Some((_, pa)) = a.pops.get(id)
            && pb.size < pa.size
        {
            losers.entry(*prov).or_default().push((pa.size - pb.size, pa));
        }
    }
    let mut out = Vec::new();
    for (id, (prov, p)) in &b.pops {
        if a.pops.contains_key(id) {
            continue;
        }
        let donor = losers
            .get(prov)
            .and_then(|l| l.iter().filter(|(loss, _)| *loss >= p.size).min_by(|x, y| x.0.total_cmp(&y.0)));
        let source = match donor {
            None => NewPopSource::NoLocalDonor,
            Some((_, d)) => {
                let mut differs = Vec::new();
                if d.pop_type != p.pop_type {
                    differs.push("type");
                }
                if d.culture != p.culture {
                    differs.push("culture");
                }
                if d.religion != p.religion {
                    differs.push("religion");
                }
                NewPopSource::SplitFrom { differs }
            }
        };
        out.push((source, p.size));
    }
    out
}
