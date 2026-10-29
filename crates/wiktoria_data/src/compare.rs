//! Compares the port's loaded world with an original save and groups the differences into
//! patterns, so setup rules the game applies before day 1 show up as data.

use std::collections::BTreeMap;

use crate::map::ProvinceId;
use crate::save::SaveGame;
use crate::world::World;

/// (pop type, culture, religion)
pub type PopKey = (String, String, String);

#[derive(Debug, Default)]
pub struct Report {
    pub provinces_compared: usize,
    pub missing_in_save: Vec<ProvinceId>,
    pub missing_in_world: Vec<ProvinceId>,
    /// (province, ours, save)
    pub owner: Vec<(ProvinceId, String, String)>,
    pub controller: Vec<(ProvinceId, String, String)>,
    pub cores: Vec<(ProvinceId, String, String)>,
    pub pops_ours: (usize, f64),
    pub pops_save: (usize, f64),
    /// Pop count per province that differs: (province, ours, save)
    pub pop_count: Vec<(ProvinceId, usize, usize)>,
    /// "labourers->farmers" → (count, trade goods → count)
    pub type_swaps: BTreeMap<String, (usize, BTreeMap<String, usize>)>,
    /// Pop type → size changes (province, ours, save) for groups present on both sides.
    pub size_changes: BTreeMap<String, Vec<(ProvinceId, f64, f64)>>,
    /// Pop type → number of groups compared with equal size.
    pub size_equal: BTreeMap<String, usize>,
    /// Groups only on one side that no swap explains: (province, key, size)
    pub only_ours: Vec<(ProvinceId, PopKey, f64)>,
    pub only_save: Vec<(ProvinceId, PopKey, f64)>,
}

fn tag_or_dash(world: &World, idx: Option<u16>) -> String {
    idx.map_or("-".into(), |i| world.country(i).country.tag.clone())
}

fn sizes_close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

pub fn compare(world: &World, save: &SaveGame) -> Report {
    let mut r = Report::default();
    for p in world.provinces.iter().flatten() {
        let Some(s) = save.provinces.get(&p.id) else {
            r.missing_in_save.push(p.id);
            continue;
        };
        r.provinces_compared += 1;
        let owner = tag_or_dash(world, p.owner);
        let s_owner = s.owner.clone().unwrap_or("-".into());
        if owner != s_owner {
            r.owner.push((p.id, owner, s_owner));
        }
        let ctrl = tag_or_dash(world, p.controller);
        let s_ctrl = s.controller.clone().unwrap_or("-".into());
        if ctrl != s_ctrl {
            r.controller.push((p.id, ctrl, s_ctrl));
        }
        let mut ours: Vec<String> = p.cores.iter().map(|c| world.country(*c).country.tag.clone()).collect();
        let mut theirs = s.cores.clone();
        ours.sort();
        theirs.sort();
        if ours != theirs {
            r.cores.push((p.id, ours.join(" "), theirs.join(" ")));
        }

        // Pops grouped by (type, culture, religion).
        let mut a: BTreeMap<PopKey, f64> = BTreeMap::new();
        for x in &p.pops {
            let k = (
                world.defs.pop_types[x.pop_type as usize].name.clone(),
                world.defs.cultures.cultures[x.culture as usize].name.clone(),
                world.defs.religions.religions[x.religion as usize].name.clone(),
            );
            *a.entry(k).or_default() += x.size;
        }
        let mut b: BTreeMap<PopKey, f64> = BTreeMap::new();
        for x in &s.pops {
            *b.entry((x.pop_type.clone(), x.culture.clone(), x.religion.clone())).or_default() += x.size;
        }
        r.pops_ours.0 += p.pops.len();
        r.pops_ours.1 += p.population();
        r.pops_save.0 += s.pops.len();
        r.pops_save.1 += s.population();
        if p.pops.len() != s.pops.len() {
            r.pop_count.push((p.id, p.pops.len(), s.pops.len()));
        }

        let mut only_a: Vec<(PopKey, f64)> = Vec::new();
        for (k, &sa) in &a {
            match b.get(k) {
                Some(&sb) if sizes_close(sa, sb) => *r.size_equal.entry(k.0.clone()).or_default() += 1,
                Some(&sb) => r.size_changes.entry(k.0.clone()).or_default().push((p.id, sa, sb)),
                None => only_a.push((k.clone(), sa)),
            }
        }
        let mut only_b: Vec<(PopKey, f64)> = b.iter().filter(|(k, _)| !a.contains_key(*k)).map(|(k, v)| (k.clone(), *v)).collect();

        // A group that vanished and a group that appeared with the same culture, religion and
        // size is a type swap.
        only_a.retain(|(ka, sa)| {
            if let Some(i) = only_b.iter().position(|(kb, sb)| kb.1 == ka.1 && kb.2 == ka.2 && sizes_close(*sa, *sb)) {
                let (kb, _) = only_b.remove(i);
                let e = r.type_swaps.entry(format!("{}->{}", ka.0, kb.0)).or_default();
                e.0 += 1;
                *e.1.entry(p.trade_goods.clone().unwrap_or("-".into())).or_default() += 1;
                false
            } else {
                true
            }
        });
        r.only_ours.extend(only_a.into_iter().map(|(k, v)| (p.id, k, v)));
        r.only_save.extend(only_b.into_iter().map(|(k, v)| (p.id, k, v)));
    }
    let ours: std::collections::HashSet<_> = world.provinces.iter().flatten().map(|p| p.id).collect();
    r.missing_in_world = save.provinces.keys().filter(|id| !ours.contains(id)).copied().collect();
    r
}
