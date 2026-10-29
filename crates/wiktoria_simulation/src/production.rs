//! Resource-gathering (RGO) production.
//!
//! Spec (rules file FLD_RGO_*, all values fixed point unless noted; "thousandths" values are
//! modifier totals and production bonuses, converted to fixed point truncating toward zero):
//! * Size = max(base x (1 + bonus size + province size), FLD_RGO_MIN_SIZE_RAW).
//!   Bonus size: the owner's technology/invention `rgo_size` for the good, plus its mine size
//!   (mine types) or farm size (farm types). Province size: the province total of
//!   `<good>` + FLD_RGO_GOOD_SIZE_SUFFIX plus its farm size (farm types) or mine size (others).
//! * Base: fixed when the game is set up, before technologies apply: the smallest n >= 1 whose
//!   size employs every worker pop of the province (workforce x size, whole workers), then
//!   n x FLD_RGO_BASE_SIZE_FACTOR, truncated. A province whose controller is not its owner stops
//!   after one step.
//! * Workers: each employee entry adds employed / (workforce x size) to its effect (input, output
//!   or throughput), times its multiplier; the share is capped at `amount` only when the
//!   multiplier is not 1. Input and output start at 1, throughput at 0.
//! * Each factor is multiplied by 1 + (national modifier + national bonus, only for a real
//!   country) + local modifier (+ efficiency for output: province `<good>` eff + country and
//!   province farm/mine eff) + bonus for the good + bonus farm/mine eff (FLD_RGO_TECH_EFF_ALL_FACTORS:
//!   added to every factor) + owner bonus (for the owner's effect: owner pop share of the state x
//!   multiplier). Throughput is then multiplied by (mobilisation factor - overseas penalty when
//!   the province is overseas), at least 0. Negative factors become 0.
//! * Daily output = throughput x output x value x size.

use std::collections::BTreeMap;

use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::production::{JobEffect, ProductionEffects, RgoType};

use crate::fixed::Fixed;
use crate::modifiers::Totals;

fn list(m: &EngineRules, id: &str, n: usize) -> Result<Vec<String>, String> {
    let v: Vec<String> = m.require_str(id)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if v.len() != n {
        return Err(format!("rules field {id} must list {n} keys, found {}", v.len()));
    }
    Ok(v)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgoRules {
    /// Modifier storage scale (FLD_MOD_STORAGE_SCALE), 1000.
    pub scale: i64,
    pub min_size: Fixed,
    /// Base size factor in thousandths.
    pub base_factor: i64,
    /// National modifier / bonus keys for input, output, throughput.
    pub national_keys: Vec<String>,
    pub local_keys: Vec<String>,
    /// Farm, mine.
    pub size_keys: Vec<String>,
    pub eff_keys: Vec<String>,
    pub good_size_suffix: String,
    pub good_eff_suffix: String,
    /// Per-good bonus keys: size, input, output, throughput.
    pub good_bonus_keys: Vec<String>,
    pub tech_eff_all_factors: bool,
}

impl RgoRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<RgoRules, String> {
        Ok(RgoRules {
            scale: crate::int_field(m, "FLD_MOD_STORAGE_SCALE")?,
            min_size: Fixed(crate::int_field(m, "FLD_RGO_MIN_SIZE_RAW")?),
            base_factor: (crate::num_field(m, "FLD_RGO_BASE_SIZE_FACTOR")? * 1000.0).round() as i64,
            national_keys: list(m, "FLD_RGO_NATIONAL_KEYS", 3)?,
            local_keys: list(m, "FLD_RGO_LOCAL_KEYS", 3)?,
            size_keys: list(m, "FLD_RGO_SIZE_KEYS", 2)?,
            eff_keys: list(m, "FLD_RGO_EFF_KEYS", 2)?,
            good_size_suffix: m.require_str("FLD_RGO_GOOD_SIZE_SUFFIX")?.to_string(),
            good_eff_suffix: m.require_str("FLD_RGO_GOOD_EFF_SUFFIX")?.to_string(),
            good_bonus_keys: list(m, "FLD_PROD_GOOD_KEYS", 4)?,
            tech_eff_all_factors: m.field("FLD_RGO_TECH_EFF_ALL_FACTORS").and_then(|v| v.as_bool()).ok_or("rules field FLD_RGO_TECH_EFF_ALL_FACTORS missing")?,
        })
    }

    /// Thousandths to fixed point, truncating toward zero.
    pub fn fx(&self, thousandths: i64) -> Fixed {
        Fixed((thousandths << 15) / self.scale)
    }

    fn thousandths(&self, x: f64) -> i64 {
        (x * self.scale as f64).round() as i64
    }

    /// Farm key for farm types, mine key for the rest (the game's province-side choice).
    fn farm_or_mine<'a>(&self, keys: &'a [String], rgo: &RgoType) -> &'a str {
        if rgo.farm { &keys[0] } else { &keys[1] }
    }
}

/// A country's accumulated production bonuses from technologies and inventions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductionBonus {
    /// Scalar bonuses in thousandths (farm/mine size and eff).
    pub values: BTreeMap<String, i64>,
    /// National input/output/throughput bonuses, fixed point.
    pub national: BTreeMap<String, Fixed>,
    /// Per-good bonuses in thousandths, by bonus key.
    pub per_good: BTreeMap<String, BTreeMap<String, i64>>,
}

impl ProductionBonus {
    pub fn add(&mut self, rules: &RgoRules, e: &ProductionEffects) {
        for (k, v) in &e.values {
            if rules.national_keys.contains(k) {
                let slot = self.national.entry(k.clone()).or_default();
                *slot = *slot + Fixed::from_f64(*v);
            } else {
                *self.values.entry(k.clone()).or_default() += rules.thousandths(*v);
            }
        }
        for (k, goods) in &e.per_good {
            let slot = self.per_good.entry(k.clone()).or_default();
            for (g, v) in goods {
                *slot.entry(g.clone()).or_default() += rules.thousandths(*v);
            }
        }
    }

    fn value(&self, k: &str) -> i64 {
        self.values.get(k).copied().unwrap_or(0)
    }

    fn good(&self, key: &str, good: &str) -> i64 {
        self.per_good.get(key).and_then(|m| m.get(good)).copied().unwrap_or(0)
    }

    /// Mine bonus for mine types, farm bonus for farm types, nothing otherwise.
    fn kind_bonus(&self, keys: &[String], rgo: &RgoType) -> i64 {
        if rgo.mine {
            self.value(&keys[1])
        } else if rgo.farm {
            self.value(&keys[0])
        } else {
            0
        }
    }
}

fn get(t: &Totals, k: &str) -> i64 {
    t.get(k).copied().unwrap_or(0)
}

/// Everything one province's RGO depends on for a day.
#[derive(Debug, Clone)]
pub struct RgoInputs<'a> {
    pub rgo: &'a RgoType,
    pub base: i64,
    /// The owner's bonuses; `None` for the rebel/no-country owner (no national terms).
    pub bonus: &'a ProductionBonus,
    pub real_country: bool,
    pub country: &'a Totals,
    pub province: &'a Totals,
    /// Employed workers by pop type.
    pub employed: &'a BTreeMap<String, i64>,
    /// (owner pop type size in the state, state population).
    pub owner_share: (i64, i64),
    pub mobilisation: Fixed,
    /// Overseas penalty when the province is overseas, else 0.
    pub overseas_penalty: Fixed,
}

pub fn rgo_size(rules: &RgoRules, inp: &RgoInputs) -> Fixed {
    size_for_base(rules, inp.rgo, inp.base, inp.bonus, inp.province)
}

fn size_for_base(rules: &RgoRules, rgo: &RgoType, base: i64, bonus: &ProductionBonus, province: &Totals) -> Fixed {
    let tech = bonus.good(&rules.good_bonus_keys[0], &rgo.good) + bonus.kind_bonus(&rules.size_keys, rgo);
    let local = get(province, &format!("{}{}", rgo.good, rules.good_size_suffix)) + get(province, rules.farm_or_mine(&rules.size_keys, rgo));
    let size = (rules.fx(tech) + rules.fx(local) + Fixed::ONE) * Fixed(base << 15);
    size.max(rules.min_size)
}

/// Whole workers a size employs.
pub fn max_workers(rgo: &RgoType, size: Fixed) -> i64 {
    (Fixed((rgo.workforce as i64) << 15) * size).0 >> 15
}

/// Base size at game setup (no technology bonuses yet). `workers` = the province's pops of the
/// type's employee pop types.
pub fn setup_base_size(rules: &RgoRules, rgo: &RgoType, province: &Totals, workers: i64, owner_controls: bool) -> i64 {
    let none = ProductionBonus::default();
    let mut n = 1;
    while max_workers(rgo, size_for_base(rules, rgo, n, &none, province)) < workers {
        n += 1;
        if !owner_controls || n > 100_000 {
            break;
        }
    }
    n * rules.base_factor / 1000
}

/// Input, output and throughput factors.
pub fn rgo_factors(rules: &RgoRules, inp: &RgoInputs, size: Fixed) -> [Fixed; 3] {
    let rgo = inp.rgo;
    let mut f = [Fixed::ONE, Fixed::ONE, Fixed::ZERO];
    let capacity = Fixed((rgo.workforce as i64) << 15) * size;
    if capacity.0 != 0 {
        for job in &rgo.employees {
            let n = inp.employed.get(&job.pop_type).copied().unwrap_or(0);
            let mut share = Fixed((n << 30) / capacity.0);
            let mult = Fixed::from_f64(job.multiplier);
            if mult != Fixed::ONE {
                share = share.min(rules.fx(rules.thousandths(job.amount)));
            }
            let i = job.effect.index();
            f[i] = f[i] + share * mult;
        }
    }
    let kind_eff = inp.bonus.kind_bonus(&rules.eff_keys, rgo);
    for effect in JobEffect::ALL {
        let i = effect.index();
        let mut sum = Fixed::ZERO;
        if inp.real_country {
            let k = &rules.national_keys[i];
            sum = sum + rules.fx(get(inp.country, k)) + inp.bonus.national.get(k).copied().unwrap_or_default();
        }
        sum = sum + rules.fx(get(inp.province, &rules.local_keys[i]));
        if effect == JobEffect::Output {
            let eff_key = rules.farm_or_mine(&rules.eff_keys, rgo);
            let eff = get(inp.province, &format!("{}{}", rgo.good, rules.good_eff_suffix)) + get(inp.country, eff_key) + get(inp.province, eff_key);
            sum = sum + rules.fx(eff);
        }
        // The country's own farm/mine efficiency (inventions, timed modifiers) also counts for
        // input and throughput, like the technology bonus (world output over 64 years).
        let country_eff = if rules.tech_eff_all_factors && effect != JobEffect::Output { get(inp.country, rules.farm_or_mine(&rules.eff_keys, rgo)) } else { 0 };
        let kind = country_eff + if rules.tech_eff_all_factors || effect == JobEffect::Output { kind_eff } else { 0 };
        sum = sum + rules.fx(inp.bonus.good(&rules.good_bonus_keys[1 + i], &rgo.good) + kind);
        if let Some(owner) = rgo.owner.as_ref().filter(|o| o.effect == effect) {
            let (n, total) = inp.owner_share;
            if total > 0 {
                sum = sum + Fixed((n << 30) / (total << 15)) * Fixed::from_f64(owner.multiplier);
            }
        }
        f[i] = (sum + Fixed::ONE) * f[i];
        if effect == JobEffect::Throughput {
            let factor = (inp.mobilisation - inp.overseas_penalty).max(Fixed::ZERO);
            f[i] = f[i] * factor;
        }
        f[i] = f[i].max(Fixed::ZERO);
    }
    f
}

/// Units produced per day.
pub fn rgo_output(rgo: &RgoType, factors: [Fixed; 3], size: Fixed) -> Fixed {
    factors[2] * factors[1] * Fixed::from_f64(rgo.value) * size
}


// ---------------------------------------------------------------- hiring and firing

/// Daily hiring/firing of an RGO (rules file FLD_RGO_HIRE_*, rates from the install's defines).
///
/// Spec: ratio = yesterday's world supply of the good / yesterday's real demand.
/// * No demand figure (0 or less) or a fixed-price good: move to full size at rate 1.
/// * ratio above FLD_RGO_FIRE_ABOVE: move toward 0 at fire rate x (ratio - 1).
/// * ratio below FLD_RGO_HIRE_HI_BELOW: toward full size at the high hire rate; below
///   FLD_RGO_HIRE_LO_BELOW: the low hire rate. Hire rates are x (1 - ratio), except in the first
///   FLD_RGO_RECENT_DAYS days after a controller change, when they are x FLD_RGO_RECENT_FACTOR and
///   not scaled by the ratio.
/// * Otherwise no change.
///
/// One day's step = (target - employed) x rate, truncated; firing steps are divided by
/// FLD_RGO_FIRE_DIVISOR. The step is at least the lowest share of current employment from the
/// defines, never fewer than FLD_RGO_MIN_STEP workers, and never past the target.
#[derive(Debug, Clone, PartialEq)]
pub struct HireRules {
    pub fire_above: Fixed,
    pub hire_hi_below: Fixed,
    pub hire_lo_below: Fixed,
    /// Rates in thousandths (defines).
    pub hire_hi: i64,
    pub hire_lo: i64,
    pub fire: i64,
    pub hire_lowest: i64,
    pub fire_lowest: i64,
    pub min_step: i64,
    pub fire_divisor: i64,
    pub recent_days: i64,
    pub recent_factor: i64,
    pub scale: i64,
}

impl HireRules {
    pub fn from_engine_rules(m: &EngineRules, defines: &wiktoria_data::defines::Defines) -> Result<HireRules, String> {
        let scale = crate::int_field(m, "FLD_MOD_STORAGE_SCALE")?;
        let section = m.require_str("FLD_RGO_HIRE_DEFINES_SECTION")?;
        let keys = list(m, "FLD_RGO_HIRE_DEFINES", 5)?;
        let d = |k: &str| -> Result<i64, String> {
            defines.get(section, k).map(|v| (v * scale as f64).round() as i64).ok_or(format!("defines: {section}.{k} missing"))
        };
        let float = |id: &str| -> Result<Fixed, String> { Ok(Fixed::from_f32(crate::num_field(m, id)? as f32)) };
        Ok(HireRules {
            fire_above: float("FLD_RGO_FIRE_ABOVE")?,
            hire_hi_below: float("FLD_RGO_HIRE_HI_BELOW")?,
            hire_lo_below: float("FLD_RGO_HIRE_LO_BELOW")?,
            hire_hi: d(&keys[0])?,
            hire_lo: d(&keys[1])?,
            fire: d(&keys[2])?,
            hire_lowest: d(&keys[3])?,
            fire_lowest: d(&keys[4])?,
            min_step: crate::int_field(m, "FLD_RGO_MIN_STEP")?,
            fire_divisor: crate::int_field(m, "FLD_RGO_FIRE_DIVISOR")?,
            recent_days: crate::int_field(m, "FLD_RGO_RECENT_DAYS")?,
            recent_factor: crate::int_field(m, "FLD_RGO_RECENT_FACTOR")?,
            scale,
        })
    }

    fn fx(&self, thousandths: i64) -> Fixed {
        Fixed((thousandths << 15) / self.scale)
    }
}

/// What an RGO aims for today: hire up to full size (`true`) or fire toward zero, and how fast.
pub fn hiring_order(rules: &HireRules, supply: Fixed, real_demand: Fixed, fixed_price: bool, days_since_control_change: Option<i64>) -> (bool, Fixed) {
    if real_demand.0 <= 0 || fixed_price {
        return (true, Fixed::ONE);
    }
    let ratio = supply.checked_div(real_demand).unwrap_or(Fixed::ZERO);
    if ratio > rules.fire_above {
        return (false, rules.fx(rules.fire) * (ratio - Fixed::ONE));
    }
    let base = if ratio < rules.hire_hi_below {
        rules.hire_hi
    } else if ratio < rules.hire_lo_below {
        rules.hire_lo
    } else {
        return (true, Fixed::ZERO);
    };
    let recent = days_since_control_change.is_some_and(|d| d < rules.recent_days);
    if recent {
        (true, rules.fx(base * rules.recent_factor))
    } else {
        (true, rules.fx(base) * (Fixed::ONE - ratio))
    }
}

/// One day's change in employed workers toward `target` at `rate` (before pop availability).
pub fn employment_step(rules: &HireRules, employed: i64, target: i64, rate: Fixed) -> i64 {
    let delta = target - employed;
    if delta == 0 || rate.0 == 0 {
        return 0;
    }
    let mut step = (Fixed(delta << 15) * rate).0 >> 15;
    if delta < 0 {
        step /= rules.fire_divisor;
        let lowest = ((Fixed(employed << 15) * rules.fx(rules.fire_lowest)).0 >> 15).max(rules.min_step);
        step = step.min(-lowest).max(delta);
    } else {
        let lowest = ((Fixed(employed << 15) * rules.fx(rules.hire_lowest)).0 >> 15).max(rules.min_step);
        step = step.max(lowest).min(delta);
    }
    step
}
