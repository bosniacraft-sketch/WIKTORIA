//! Our own layered map picture, drawn from the content files and composed in layers:
//! (1) land tinted by terrain class, softened; (2) water, deep with a lighter band along coasts;
//! (3) owners (or the map mode's colour) laid over the land at partial opacity; (4) rivers;
//! (5) borders derived from province edges, darker and thicker where the owner changes;
//! (6) a soft glow of one country's colour outside its border, under the borders.
//!
//! Layers 1, 2, 4 and the province edges do not change during a game: `Ground` holds them at full
//! map resolution and is built once. `compose` adds the owner-dependent layers and scales the
//! result down to the requested width (area average), row band by row band on every core.
//! When the install's map colour pictures are given (`GroundSource::colormap` and friends), land and
//! sea take their colours from them instead, owners are shaded by the land's light and dark, and
//! the sea gets a light band along the coasts: closer to the original's look. The mixing amounts
//! are our presentation choices, not the original's.

use std::time::{Duration, Instant};

use wiktoria_data::bmp::{Image, IndexedImage};
use wiktoria_data::map::ProvinceMap;

use crate::{Palette, NO_DATA, UNKNOWN, UNOWNED};

/// Open sea, away from the coast.
pub const DEEP_SEA: u32 = 0x1d3c60;
/// Sea 1, 2 and 3 pixels from land.
pub const SHORE: [u32; 3] = [0x6b98bd, 0x4c7ba5, 0x30608e];
/// Land when the terrain layer is off, and terrain indices no class names.
pub const LAND: u32 = 0xcdbd98;
pub const RIVER: u32 = 0x2f6fb0;
pub const OWNER_LINE: u32 = 0x18120c;

/// Share of the terrain class colour in the painted land (the rest is `LAND`).
const TERRAIN_STRENGTH: f64 = 0.7;
/// Radius of the box that softens the terrain colours, in map pixels.
const TERRAIN_SOFTEN: usize = 2;
/// Owner colour over terrain, and over plain land.
const OWNER_OVER_TERRAIN: f64 = 0.62;
const OWNER_OVER_LAND: f64 = 0.9;
/// With the colour pictures: light band along the coasts (colour, strength at the coast, reach in
/// map pixels), the share of the land colour left under an owner, how strongly the land's light and
/// dark shade the owner colour (and its limits), the wave picture's strength, the province lines.
const SHORE_GLOW: u32 = 0xaac8d2;
const SHORE_GLOW_STRENGTH: f64 = 0.35;
const SHORE_GLOW_REACH: u8 = 6;
const LAND_UNDER_OWNER: f64 = 0.14;
const RELIEF: f64 = 1.1;
const RELIEF_RANGE: (f64, f64) = (0.72, 1.18);
const WAVES: f64 = 0.1;
const PROVINCE_LINE_TEXTURED: f64 = 0.78;
const RIVER_TEXTURED: f64 = 0.55;
const SEA_EDGE_TEXTURED: f64 = 0.06;
/// How far the glow reaches outside the country, in map pixels.
pub const GLOW_RADIUS: usize = 9;

// Per-pixel flags of the ground.
const WATER: u8 = 1;
const RIVER_PX: u8 = 2;
/// Land pixel whose right or lower neighbour is land of another province.
const PROV_RD: u8 = 4;
/// Land pixel with any land neighbour (four directions) of another province.
const PROV_ANY: u8 = 8;
/// Land pixel next to water.
const COAST: u8 = 16;
/// Water pixel whose right or lower neighbour is water of another province.
const SEA_EDGE: u8 = 32;

/// What the ground is built from: the province map and the content's pictures.
pub struct GroundSource<'a> {
    pub map: &'a ProvinceMap,
    /// Per province id: a sea province. Pixels of id 0 (colours definition.csv does not name:
    /// lakes and the map's edge) count as water too.
    pub is_sea: &'a [bool],
    /// Terrain index per pixel (terrain.bmp), and the colour of the class of each index.
    pub terrain: Option<&'a IndexedImage>,
    pub terrain_colors: &'a [Option<u32>; 256],
    /// River code per pixel (rivers.bmp): codes at most `river_max` are river pixels; `wide[code]`
    /// draws that code two pixels wide.
    pub rivers: Option<&'a IndexedImage>,
    pub river_max: u8,
    pub wide: &'a [bool; 256],
    /// The install's colour pictures, north at the top, any size (stretched over the map): land
    /// colours with their light and shade, sea colours, and a small wave picture repeated over the sea.
    pub colormap: Option<&'a Image>,
    pub water_colormap: Option<&'a Image>,
    pub water_tile: Option<&'a Image>,
}

/// The static layers at full map resolution.
pub struct Ground {
    pub width: usize,
    pub height: usize,
    /// Softened terrain on land, water with its coastal band at sea.
    color: Vec<u32>,
    flags: Vec<u8>,
    /// Land's light and dark as a factor x 128 for the owner colour (128 = none), when the land
    /// colour picture was given.
    shade: Option<Vec<u8>>,
}

/// Which layers a picture shows. Water is always drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layers {
    pub terrain: bool,
    pub owners: bool,
    pub rivers: bool,
    pub borders: bool,
    pub glow: bool,
}

impl Layers {
    pub const NAMES: [&'static str; 5] = ["terrain", "owners", "rivers", "borders", "glow"];
    pub const ALL: Layers = Layers { terrain: true, owners: true, rivers: true, borders: true, glow: true };

    /// A comma-separated list of layer names ("" = none); None when a name is unknown.
    pub fn parse(s: &str) -> Option<Layers> {
        let mut l = Layers { terrain: false, owners: false, rivers: false, borders: false, glow: false };
        for name in s.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match name {
                "terrain" => l.terrain = true,
                "owners" => l.owners = true,
                "rivers" => l.rivers = true,
                "borders" => l.borders = true,
                "glow" => l.glow = true,
                _ => return None,
            }
        }
        Some(l)
    }

    /// One bit per layer, for cache keys.
    pub fn bits(&self) -> u8 {
        [self.terrain, self.owners, self.rivers, self.borders, self.glow].iter().enumerate().fold(0, |b, (i, on)| b | (u8::from(*on) << i))
    }
}

fn lerp(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s & 0xff) as f64, (b >> s & 0xff) as f64);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

fn scale(c: u32, f: f64) -> u32 {
    lerp(0, c, f)
}

/// `c` times `f`, which may be above 1 (each channel stops at 255).
fn mul(c: u32, f: f64) -> u32 {
    let ch = |s: u32| (((c >> s & 0xff) as f64 * f).round().clamp(0.0, 255.0) as u32) << s;
    ch(16) | ch(8) | ch(0)
}

fn luma(c: u32) -> f64 {
    0.3 * (c >> 16 & 0xff) as f64 + 0.59 * (c >> 8 & 0xff) as f64 + 0.11 * (c & 0xff) as f64
}

/// The colour of `img` stretched over a `w` x `h` map, at map pixel (x, y), mixed from the four
/// nearest picture pixels.
fn stretched(img: &Image, w: usize, h: usize, x: usize, y: usize) -> u32 {
    let (iw, ih) = (img.width, img.height);
    let fx = ((x as f64 + 0.5) * iw as f64 / w as f64 - 0.5).clamp(0.0, (iw - 1) as f64);
    let fy = ((y as f64 + 0.5) * ih as f64 / h as f64 - 0.5).clamp(0.0, (ih - 1) as f64);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(iw - 1), (y0 + 1).min(ih - 1));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    let px = |x, y| {
        let [r, g, b] = img.pixel(x, y).map(u32::from);
        r << 16 | g << 8 | b
    };
    lerp(lerp(px(x0, y0), px(x1, y0), tx), lerp(px(x0, y1), px(x1, y1), tx), ty)
}

/// River codes drawn two pixels wide: the darkest blues of the river picture's colour table (the
/// table runs from light cyan to dark navy as the codes rise; reading them as widths is ours).
pub fn wide_river_codes(palette: &[[u8; 3]], river_max: u8) -> [bool; 256] {
    let mut wide = [false; 256];
    for (i, c) in palette.iter().enumerate().take(river_max as usize + 1) {
        let [r, g, b] = c.map(u32::from);
        wide[i] = b > r + g && r + g + b < 200;
    }
    wide
}

impl Ground {
    /// Builds the static layers; returns the time each took.
    pub fn build(src: &GroundSource) -> (Ground, Vec<(&'static str, Duration)>) {
        let map = src.map;
        let (w, h) = (map.width, map.height);
        let n = w * h;
        let mut times = Vec::new();
        let same_size = |img: &IndexedImage| img.width == w && img.height == h;
        let terrain = src.terrain.filter(|i| same_size(i));
        let rivers = src.rivers.filter(|i| same_size(i));

        // Water and province edges.
        let t = Instant::now();
        let water_id = |id: u16| id == 0 || src.is_sea.get(id as usize).copied().unwrap_or(false);
        let mut flags: Vec<u8> = map.ids.iter().map(|&id| if water_id(id) { WATER } else { 0 }).collect();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (id, wet) = (map.ids[i], flags[i] & WATER != 0);
                let r = y * w + (x + 1) % w;
                let l = y * w + (x + w - 1) % w;
                let mut f = 0;
                for (j, right_or_down) in [(Some(r), true), (Some(l), false), (y.checked_sub(1).map(|u| u * w + x), false), ((y + 1 < h).then(|| (y + 1) * w + x), true)] {
                    let Some(j) = j else { continue };
                    let (other, other_wet) = (map.ids[j], flags[j] & WATER != 0);
                    match (wet, other_wet) {
                        (false, false) if other != id => f |= PROV_ANY | if right_or_down { PROV_RD } else { 0 },
                        (false, true) => f |= COAST,
                        (true, true) if other != id && right_or_down => f |= SEA_EDGE,
                        _ => {}
                    }
                }
                flags[i] |= f;
            }
        }
        times.push(("water and province edges", t.elapsed()));

        // Terrain: class colour per pixel, mixed with plain land, then a box average over land only.
        let t = Instant::now();
        let colormap = src.colormap.filter(|i| i.width > 0 && i.height > 0);
        let mut shade = None;
        let mut color: Vec<u32> = match (colormap, terrain) {
            (Some(img), _) => (0..n).map(|i| stretched(img, w, h, i % w, i / w)).collect(),
            (None, Some(img)) => img.index.iter().map(|&k| src.terrain_colors[k as usize].map_or(LAND, |c| lerp(LAND, c, TERRAIN_STRENGTH))).collect(),
            (None, None) => vec![LAND; n],
        };
        if colormap.is_some() {
            // The owner colour is shaded by how much lighter or darker the land is than its average.
            let land: Vec<f64> = (0..n).filter(|&i| flags[i] & WATER == 0).map(|i| luma(color[i])).collect();
            let mean = (land.iter().sum::<f64>() / land.len().max(1) as f64).max(1.0);
            shade = Some(color.iter().map(|&c| ((1.0 + RELIEF * (luma(c) / mean - 1.0)).clamp(RELIEF_RANGE.0, RELIEF_RANGE.1) * 128.0).round() as u8).collect());
        } else {
            soften_land(&mut color, &flags, w, h, TERRAIN_SOFTEN);
        }
        times.push(("terrain", t.elapsed()));

        // Sea colour by distance to land (1..3 pixels), deep beyond.
        let t = Instant::now();
        let water_colormap = src.water_colormap.filter(|i| i.width > 0 && i.height > 0);
        let reach = if water_colormap.is_some() { SHORE_GLOW_REACH } else { SHORE.len() as u8 };
        let mut dist: Vec<u8> = flags.iter().map(|f| if f & WATER != 0 { u8::MAX } else { 0 }).collect();
        for k in 1..=reach {
            let mut next = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if dist[i] != u8::MAX {
                        continue;
                    }
                    let near = [y * w + (x + 1) % w, y * w + (x + w - 1) % w].into_iter().chain(y.checked_sub(1).map(|u| u * w + x)).chain((y + 1 < h).then(|| (y + 1) * w + x));
                    if near.into_iter().any(|j| dist[j] == k - 1) {
                        next.push(i);
                    }
                }
            }
            for i in next {
                dist[i] = k;
            }
        }
        let tile = src.water_tile.filter(|i| i.width > 0 && i.height > 0);
        let tile_mean = tile.map_or(1.0, |t| {
            let sum: f64 = (0..t.width * t.height).map(|i| luma(stretched(t, t.width, t.height, i % t.width, i / t.width))).sum();
            (sum / (t.width * t.height) as f64).max(1.0)
        });
        for (i, d) in dist.iter().enumerate() {
            if flags[i] & WATER == 0 {
                continue;
            }
            color[i] = match water_colormap {
                Some(img) => {
                    let (x, y) = (i % w, i / w);
                    let mut c = stretched(img, w, h, x, y);
                    if let Some(t) = tile {
                        let [r, g, b] = t.pixel(x % t.width, y % t.height).map(u32::from);
                        c = mul(c, 1.0 + WAVES * (luma(r << 16 | g << 8 | b) / tile_mean - 1.0));
                    }
                    let near = 1.0 - (*d as f64 - 1.0) / reach as f64;
                    if *d != u8::MAX && near > 0.0 { lerp(c, SHORE_GLOW, SHORE_GLOW_STRENGTH * near) } else { c }
                }
                None => SHORE.get((*d as usize).wrapping_sub(1)).copied().unwrap_or(DEEP_SEA),
            };
        }
        times.push(("water", t.elapsed()));

        // Rivers, on land only; wide codes also take the pixel to the right.
        let t = Instant::now();
        if let Some(img) = rivers {
            for (i, &code) in img.index.iter().enumerate() {
                if code > src.river_max || flags[i] & WATER != 0 {
                    continue;
                }
                flags[i] |= RIVER_PX;
                if src.wide[code as usize] {
                    let j = i - i % w + (i % w + 1) % w;
                    if flags[j] & WATER == 0 {
                        flags[j] |= RIVER_PX;
                    }
                }
            }
        }
        times.push(("rivers", t.elapsed()));
        (Ground { width: w, height: h, color, flags, shade }, times)
    }
}

/// Box average of the land colours (water pixels neither give nor take colour), done with a
/// ring of horizontally summed rows so it needs no full-size scratch buffer.
fn soften_land(color: &mut [u32], flags: &[u8], w: usize, h: usize, r: usize) {
    let rows = 2 * r + 1;
    let mut ring = vec![[0u32; 4]; rows * w];
    let hsum = |y: usize, color: &[u32], out: &mut [[u32; 4]]| {
        let px = |x: usize| {
            let i = y * w + x % w;
            if flags[i] & WATER != 0 { [0, 0, 0, 0] } else { [color[i] >> 16 & 0xff, color[i] >> 8 & 0xff, color[i] & 0xff, 1] }
        };
        let mut s = [0u32; 4];
        for x in 0..rows {
            let p = px(x + w - r);
            (0..4).for_each(|k| s[k] += p[k]);
        }
        for (x, o) in out.iter_mut().enumerate() {
            *o = s;
            let (add, sub) = (px(x + r + 1), px(x + w - r));
            (0..4).for_each(|k| s[k] = s[k] + add[k] - sub[k]);
        }
    };
    let mut done = 0; // rows summed so far
    for y in 0..h {
        while done < h && done <= y + r {
            let slot = done % rows;
            hsum(done, color, &mut ring[slot * w..(slot + 1) * w]);
            done += 1;
        }
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            let i = y * w + x;
            if flags[i] & WATER != 0 {
                continue;
            }
            let mut s = [0u32; 4];
            for yy in lo..=hi {
                let p = ring[(yy % rows) * w + x];
                (0..4).for_each(|k| s[k] += p[k]);
            }
            if let (Some(r), Some(g), Some(b)) = (s[0].checked_div(s[3]), s[1].checked_div(s[3]), s[2].checked_div(s[3])) {
                color[i] = r << 16 | g << 8 | b;
            }
        }
    }
}

/// A country's glow: alpha 0..255 over a box of the map, outside the country only.
pub struct Glow {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    alpha: Vec<u8>,
    color: u32,
}

impl Glow {
    /// The glow of the provinces whose `owner_key` is `key`, in `color` lightened; None when the
    /// country has no land.
    pub fn build(map: &ProvinceMap, pal: &Palette, key: u32, color: u32, radius: usize) -> Option<Glow> {
        let (mw, mh) = (map.width, map.height);
        let mine = |id: u16| key != 0 && pal.owner_key.get(id as usize) == Some(&key);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, &id) in map.ids.iter().enumerate() {
            if mine(id) {
                let (x, y) = (i % mw, i / mw);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        if x0 == usize::MAX {
            return None;
        }
        let (x0, y0) = (x0.saturating_sub(radius), y0.saturating_sub(radius));
        let (x1, y1) = ((x1 + radius).min(mw - 1), (y1 + radius).min(mh - 1));
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let inside: Vec<bool> = (0..w * h).map(|i| mine(map.ids[(y0 + i / w) * mw + x0 + i % w])).collect();
        // Box blur of the mask, rows then columns: the share of the country within the box.
        let mut rows = vec![0u16; w * h];
        for y in 0..h {
            let line = &inside[y * w..(y + 1) * w];
            let mut s: u16 = line[..radius.min(w)].iter().map(|&b| u16::from(b)).sum();
            for x in 0..w {
                if x + radius < w {
                    s += u16::from(line[x + radius]);
                }
                rows[y * w + x] = s;
                if x >= radius {
                    s -= u16::from(line[x - radius]);
                }
            }
        }
        let full = ((2 * radius + 1) * (2 * radius + 1)) as f64;
        let mut alpha = vec![0u8; w * h];
        for x in 0..w {
            let mut s: u32 = (0..radius.min(h)).map(|y| u32::from(rows[y * w + x])).sum();
            for y in 0..h {
                if y + radius < h {
                    s += u32::from(rows[(y + radius) * w + x]);
                }
                if !inside[y * w + x] && s > 0 {
                    // About one half right at a straight border, falling to nothing at the radius.
                    alpha[y * w + x] = ((2.0 * s as f64 / full).min(1.0) * 180.0) as u8;
                }
                if y >= radius {
                    s -= u32::from(rows[(y - radius) * w + x]);
                }
            }
        }
        Some(Glow { x0, y0, w, h, alpha, color: lerp(color, 0xffffff, 0.3) })
    }

    fn at(&self, x: usize, y: usize) -> u8 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return 0;
        }
        self.alpha[(y - self.y0) * self.w + x - self.x0]
    }
}

/// The picture: `ground` with the owner layers of `pal` (and `glow`), averaged down (or sampled
/// up) to `out_w` x `out_h`. Row-major 0x00RRGGBB.
pub fn compose(ground: &Ground, map: &ProvinceMap, pal: &Palette, layers: Layers, glow: Option<&Glow>, out_w: usize, out_h: usize) -> Vec<u32> {
    let (mw, mh) = (ground.width, ground.height);
    assert!(map.width == mw && map.height == mh, "ground and province map differ in size");
    // The tint of each province: none for sea, unowned land and land without data.
    let tint: Vec<Option<u32>> = pal.color.iter().map(|&c| Some(c).filter(|c| layers.owners && ![UNOWNED, NO_DATA, UNKNOWN].contains(c))).collect();
    let owner_alpha = if layers.terrain { OWNER_OVER_TERRAIN } else { OWNER_OVER_LAND };
    let glow = glow.filter(|_| layers.glow);
    let key = |id: u16| pal.owner_key.get(id as usize).copied().unwrap_or(0);

    let pixel = |x: usize, y: usize| -> u32 {
        let i = y * mw + x;
        let (f, id) = (ground.flags[i], map.ids[i]);
        let wet = f & WATER != 0;
        let mut c = if wet || layers.terrain { ground.color[i] } else { LAND };
        if !wet && let Some(t) = tint.get(id as usize).copied().flatten() {
            c = match ground.shade.as_ref().filter(|_| layers.terrain) {
                Some(shade) => lerp(mul(t, shade[i] as f64 / 128.0), c, LAND_UNDER_OWNER),
                None => lerp(c, t, owner_alpha),
            };
        }
        if let Some(g) = glow {
            let a = g.at(x, y);
            if a > 0 {
                c = lerp(c, g.color, a as f64 / 255.0);
            }
        }
        if layers.rivers && f & RIVER_PX != 0 {
            c = lerp(c, RIVER, if ground.shade.is_some() { RIVER_TEXTURED } else { 0.9 });
        }
        if layers.borders {
            if f & PROV_ANY != 0 {
                let k = key(id);
                let near = [y * mw + (x + 1) % mw, y * mw + (x + mw - 1) % mw].into_iter().chain(y.checked_sub(1).map(|u| u * mw + x)).chain((y + 1 < mh).then(|| (y + 1) * mw + x));
                let owner_edge = near.into_iter().any(|j| ground.flags[j] & WATER == 0 && key(map.ids[j]) != k);
                if owner_edge {
                    c = lerp(c, OWNER_LINE, 0.85);
                } else if f & PROV_RD != 0 {
                    c = scale(c, if ground.shade.is_some() { PROVINCE_LINE_TEXTURED } else { 0.68 });
                }
            }
            if f & COAST != 0 {
                c = scale(c, 0.55);
            }
            if f & SEA_EDGE != 0 && ((x + y) / 3).is_multiple_of(2) {
                c = lerp(c, 0xffffff, if ground.shade.is_some() { SEA_EDGE_TEXTURED } else { 0.14 });
            }
        }
        c
    };

    let cols: Vec<(usize, usize)> = (0..out_w)
        .map(|ox| {
            let a = ox * mw / out_w;
            (a, (((ox + 1) * mw) / out_w).clamp(a + 1, mw))
        })
        .collect();
    let rows_of = |oy: usize| {
        let a = oy * mh / out_h;
        (a, (((oy + 1) * mh) / out_h).clamp(a + 1, mh))
    };
    let mut out = vec![0u32; out_w * out_h];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let band = out_h.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (b, chunk) in out.chunks_mut(band * out_w).enumerate() {
            let (cols, pixel) = (&cols, &pixel);
            s.spawn(move || {
                let mut acc = vec![[0u32; 4]; out_w];
                for (r, row) in chunk.chunks_mut(out_w).enumerate() {
                    let (y0, y1) = rows_of(b * band + r);
                    acc.iter_mut().for_each(|a| *a = [0; 4]);
                    for y in y0..y1 {
                        for (a, &(x0, x1)) in acc.iter_mut().zip(cols) {
                            for x in x0..x1 {
                                let c = pixel(x, y);
                                a[0] += c >> 16 & 0xff;
                                a[1] += c >> 8 & 0xff;
                                a[2] += c & 0xff;
                                a[3] += 1;
                            }
                        }
                    }
                    for (o, a) in row.iter_mut().zip(&acc) {
                        *o = (a[0] / a[3]) << 16 | (a[1] / a[3]) << 8 | (a[2] / a[3]);
                    }
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiktoria_data::bmp::Image;
    use wiktoria_data::map::ProvinceDef;

    /// A 12 x 6 world: province 1 (left land), 2 (right land), 3 (sea along the bottom two rows).
    fn world() -> ProvinceMap {
        let (w, h) = (12, 6);
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend(if y >= 4 { [0, 0, 200] } else if x < 6 { [200, 0, 0] } else { [0, 200, 0] });
            }
        }
        let defs = [([200, 0, 0], 1), ([0, 200, 0], 2), ([0, 0, 200], 3)].map(|(rgb, id)| ProvinceDef { id, rgb, name: format!("p{id}") });
        ProvinceMap::build(&Image { width: w, height: h, rgb }, &defs).0
    }

    fn ground(map: &ProvinceMap, rivers: Option<&IndexedImage>) -> Ground {
        let is_sea = [false, false, false, true];
        let mut colors = [None; 256];
        colors[5] = Some(0x00ff00);
        let terrain = IndexedImage { width: map.width, height: map.height, index: vec![5; map.width * map.height] };
        let src = GroundSource { map, is_sea: &is_sea, terrain: Some(&terrain), terrain_colors: &colors, rivers, river_max: 11, wide: &[false; 256], colormap: None, water_colormap: None, water_tile: None };
        Ground::build(&src).0
    }

    fn palette(owners: [u32; 4]) -> Palette {
        Palette { color: vec![UNKNOWN, 0xff0000, 0x0000ff, crate::SEA], owner_key: owners.to_vec() }
    }

    #[test]
    fn coast_band_gets_lighter_towards_land() {
        let map = world();
        let g = ground(&map, None);
        let out = compose(&g, &map, &palette([0, 1, 2, 0]), Layers { borders: false, ..Layers::ALL }, None, map.width, map.height);
        assert_eq!(out[4 * 12 + 2], SHORE[0]);
        assert_eq!(out[5 * 12 + 2], SHORE[1]);
    }

    #[test]
    fn owner_borders_are_drawn_on_both_sides_province_borders_once() {
        let map = world();
        let g = ground(&map, None);
        let only_borders = Layers { terrain: false, owners: false, rivers: false, borders: true, glow: false };
        let two_owners = compose(&g, &map, &palette([0, 1, 2, 0]), only_borders, None, 12, 6);
        assert_eq!(two_owners[12 + 5], lerp(LAND, OWNER_LINE, 0.85));
        assert_eq!(two_owners[12 + 6], lerp(LAND, OWNER_LINE, 0.85));
        let one_owner = compose(&g, &map, &palette([0, 1, 1, 0]), only_borders, None, 12, 6);
        assert_eq!(one_owner[12 + 5], scale(LAND, 0.68));
        assert_eq!(one_owner[12 + 6], LAND);
    }

    #[test]
    fn rivers_stay_on_land_and_glow_stays_outside() {
        let map = world();
        let rivers = IndexedImage { width: 12, height: 6, index: (0..72).map(|i| if i % 12 == 3 { 2 } else { 255 }).collect() };
        let g = ground(&map, Some(&rivers));
        let pal = palette([0, 1, 2, 0]);
        let only_rivers = Layers { terrain: false, owners: false, rivers: true, borders: false, glow: false };
        let out = compose(&g, &map, &pal, only_rivers, None, 12, 6);
        assert_eq!(out[12 + 3], lerp(LAND, RIVER, 0.9));
        assert_eq!(out[4 * 12 + 3], SHORE[0], "no river at sea");
        let glow = Glow::build(&map, &pal, 1, 0xff0000, 3).expect("country has land");
        assert_eq!(glow.at(2, 1), 0, "no glow inside the country");
        assert!(glow.at(6, 1) > glow.at(8, 1) && glow.at(8, 1) > 0, "glow fades outwards");
    }

    #[test]
    fn downscale_averages_and_layers_parse() {
        let map = world();
        let g = ground(&map, None);
        let out = compose(&g, &map, &palette([0, 1, 2, 0]), Layers::ALL, None, 6, 3);
        assert_eq!(out.len(), 18);
        assert_eq!(Layers::parse("terrain,glow"), Some(Layers { terrain: true, owners: false, rivers: false, borders: false, glow: true }));
        assert_eq!(Layers::parse("nonsense"), None);
        assert_eq!(Layers::ALL.bits(), 31);
    }
}
