//! Province geography from the player's install: terrain (terrain.txt + terrain.bmp), climates,
//! continents, adjacency and crimes. These feed the province modifier totals.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::bmp::IndexedImage;
use crate::map::{ProvinceId, ProvinceMap};
use crate::modifiers::{modifier_values, ModifierValues};
use crate::script::{Block, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainCategory {
    pub is_water: bool,
    /// `color = { r g b }`: the category's colour, used to tint the terrain layer of the map picture.
    pub color: Option<[u8; 3]>,
    /// Every numeric entry of the category (movement cost, defence, rgo size/eff, supply limit...).
    pub values: ModifierValues,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainDefs {
    pub categories: BTreeMap<String, TerrainCategory>,
    /// terrain.bmp palette index -> category name (`text_N = { type = x color = { N } }`).
    pub index_category: BTreeMap<u8, String>,
}

pub fn parse_terrain(root: &Block) -> TerrainDefs {
    let mut d = TerrainDefs::default();
    if let Some(c) = root.get("categories").and_then(Value::as_block) {
        for (name, v, _) in c.entries() {
            if let Some(b) = v.as_block() {
                d.categories.insert(
                    name.to_string(),
                    TerrainCategory {
                        is_water: b.get("is_water").and_then(Value::as_bool).unwrap_or(false),
                        color: b.get("color").and_then(Value::as_block).and_then(rgb_of),
                        values: modifier_values(b),
                    },
                );
            }
        }
    }
    for (name, v, _) in root.entries() {
        if name == "categories" {
            continue;
        }
        let Some(b) = v.as_block() else { continue };
        let (Some(ty), Some(color)) = (b.get("type").and_then(Value::as_str), b.get("color").and_then(Value::as_block)) else { continue };
        for c in color.bare_scalars() {
            if let Ok(i) = c.parse::<u8>() {
                d.index_category.insert(i, ty.to_string());
            }
        }
    }
    d
}

/// Three numbers 0..255 of a `color = { r g b }` block.
fn rgb_of(b: &Block) -> Option<[u8; 3]> {
    let v: Vec<u8> = b.bare_scalars().into_iter().filter_map(|c| c.parse::<f64>().ok()).map(|x| x.clamp(0.0, 255.0) as u8).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// Each province's terrain: the category covering most of its pixels. Ties go to
/// the category name that sorts first.
pub fn province_terrain(map: &ProvinceMap, img: &IndexedImage, defs: &TerrainDefs) -> Result<Vec<Option<String>>, String> {
    if (img.width, img.height) != (map.width, map.height) {
        return Err(format!("terrain map is {}x{}, province map {}x{}", img.width, img.height, map.width, map.height));
    }
    let mut counts: Vec<BTreeMap<&str, u32>> = vec![BTreeMap::new(); map.pixel_count.len()];
    for (id, idx) in map.ids.iter().zip(&img.index) {
        if *id == 0 {
            continue;
        }
        if let Some(cat) = defs.index_category.get(idx) {
            *counts[*id as usize].entry(cat.as_str()).or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|c| c.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(k, _)| k.to_string()))
        .collect())
}

/// climate.txt / continent.txt style files: blocks with modifiers and province lists.
/// climate.txt names each climate twice (once with modifiers, once with bare province ids);
/// continent.txt has `name = { provinces = { ... } modifier = x ... }`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProvinceGroups {
    pub modifiers: BTreeMap<String, ModifierValues>,
    pub member: HashMap<ProvinceId, String>,
}

pub fn parse_province_groups(root: &Block) -> ProvinceGroups {
    let mut g = ProvinceGroups::default();
    for (name, v, _) in root.entries() {
        let Some(b) = v.as_block() else { continue };
        let ids = b.get("provinces").and_then(Value::as_block).map(|p| p.bare_scalars()).unwrap_or_else(|| b.bare_scalars());
        for id in ids {
            if let Ok(id) = id.parse() {
                g.member.entry(id).or_insert_with(|| name.to_string());
            }
        }
        let values = modifier_values(b);
        if !values.is_empty() || !g.modifiers.contains_key(name) {
            let e = g.modifiers.entry(name.to_string()).or_default();
            if e.is_empty() {
                *e = values;
            }
        }
    }
    g
}

/// Neighbouring provinces: pixels touching horizontally (the map wraps east-west) or vertically.
pub fn adjacency(map: &ProvinceMap) -> Vec<BTreeSet<ProvinceId>> {
    let mut adj = vec![BTreeSet::new(); map.pixel_count.len()];
    let (w, h) = (map.width, map.height);
    let mut link = |a: ProvinceId, b: ProvinceId| {
        if a != b && a != 0 && b != 0 {
            adj[a as usize].insert(b);
            adj[b as usize].insert(a);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let a = map.ids[y * w + x];
            link(a, map.ids[y * w + (x + 1) % w]);
            if y + 1 < h {
                link(a, map.ids[(y + 1) * w + x]);
            }
        }
    }
    adj
}

/// River pixels on each border, for telling which borders a river runs along: every pair of
/// neighbouring pixels that lie in two different provinces counts when one of the two is a river
/// pixel of the river picture (palette index at most `river_max`; the indices above are the land
/// and sea backgrounds). Key: the lower province id first.
pub fn river_borders(map: &ProvinceMap, img: &IndexedImage, river_max: u8) -> Result<BTreeMap<(ProvinceId, ProvinceId), u32>, String> {
    if (img.width, img.height) != (map.width, map.height) {
        return Err(format!("river map is {}x{}, province map {}x{}", img.width, img.height, map.width, map.height));
    }
    let (w, h) = (map.width, map.height);
    let mut out = BTreeMap::new();
    let mut pair = |i: usize, j: usize| {
        let (a, b) = (map.ids[i], map.ids[j]);
        if a != b && a != 0 && b != 0 && (img.index[i] <= river_max || img.index[j] <= river_max) {
            *out.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    };
    for y in 0..h {
        for x in 0..w {
            pair(y * w + x, y * w + (x + 1) % w);
            if y + 1 < h {
                pair(y * w + x, (y + 1) * w + x);
            }
        }
    }
    Ok(out)
}

/// One line of adjacencies.csv: `from;to;type;through;data;comment` (`#` lines are comments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraAdjacency {
    pub from: ProvinceId,
    pub to: ProvinceId,
    pub kind: String,
    pub through: ProvinceId,
}

pub fn parse_adjacencies(src: &str) -> Vec<ExtraAdjacency> {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let c: Vec<&str> = l.split(';').map(str::trim).collect();
            Some(ExtraAdjacency { from: c.first()?.parse().ok()?, to: c.get(1)?.parse().ok()?, kind: c.get(2)?.to_string(), through: c.get(3).and_then(|t| t.parse().ok()).unwrap_or(0) })
        })
        .collect()
}

/// Connected groups (land masses, or one owner's territory): provinces passing `is_land` joined by map adjacency (except pairs listed with a `cut` kind)
/// and by listed adjacencies of a `link` kind. Returns a component number per province id
/// (0 for sea and unknown ids).
pub fn land_masses(adjacency: &[BTreeSet<ProvinceId>], is_land: &dyn Fn(ProvinceId) -> bool, extra: &[ExtraAdjacency], link: &[String], cut: &[String]) -> Vec<u32> {
    let n = adjacency.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let cuts: std::collections::HashSet<(ProvinceId, ProvinceId)> =
        extra.iter().filter(|e| cut.contains(&e.kind)).flat_map(|e| [(e.from, e.to), (e.to, e.from)]).collect();
    let join = |p: &mut Vec<usize>, a: ProvinceId, b: ProvinceId| {
        let (ra, rb) = (find(p, a as usize), find(p, b as usize));
        if ra != rb {
            p[ra.max(rb)] = ra.min(rb);
        }
    };
    for (a, ns) in adjacency.iter().enumerate() {
        let a = a as ProvinceId;
        if !is_land(a) {
            continue;
        }
        for &b in ns {
            if is_land(b) && !cuts.contains(&(a, b)) {
                join(&mut parent, a, b);
            }
        }
    }
    for e in extra.iter().filter(|e| link.contains(&e.kind)) {
        if (e.from as usize) < n && (e.to as usize) < n && is_land(e.from) && is_land(e.to) {
            join(&mut parent, e.from, e.to);
        }
    }
    (0..n).map(|i| if is_land(i as ProvinceId) { find(&mut parent, i) as u32 + 1 } else { 0 }).collect()
}

/// Named blocks in file order (crimes are numbered by their position).
pub fn ordered_blocks(root: &Block) -> Vec<(String, ModifierValues)> {
    root.entries().filter_map(|(k, v, _)| Some((k.to_string(), modifier_values(v.as_block()?)))).collect()
}
