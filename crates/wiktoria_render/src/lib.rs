//! Window-independent map rendering: camera, map modes, borders and picking.
//! Renders into a `u32` framebuffer (0x00RRGGBB), so any window layer or image
//! writer can display it.

pub mod labels;
pub mod layers;

use wiktoria_data::map::{ProvinceId, ProvinceMap};
use wiktoria_data::world::World;

pub const SEA: u32 = 0x2a4d74;
pub const UNOWNED: u32 = 0x8c8577;
/// Pixels whose colour is not in definition.csv (lakes, map edge). Presentation choice.
pub const UNKNOWN: u32 = 0x1f3550;
pub const BACKGROUND: u32 = 0x10141a;
pub const COUNTRY_BORDER: u32 = 0x161616;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    /// Land coloured by owner at the start date.
    Political,
    /// Raw province colours from definition.csv.
    Provinces,
    /// Total population per province, log scale.
    Population,
    /// Largest culture by population.
    Culture,
    /// Largest religion by population.
    Religion,
    /// State regions from region.txt.
    States,
}

impl MapMode {
    pub const ALL: [MapMode; 6] =
        [MapMode::Political, MapMode::Provinces, MapMode::Population, MapMode::Culture, MapMode::Religion, MapMode::States];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            MapMode::Political => "political",
            MapMode::Provinces => "provinces",
            MapMode::Population => "population",
            MapMode::Culture => "culture",
            MapMode::Religion => "religion",
            MapMode::States => "states",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label() == s)
    }
}

pub fn rgb(c: [u8; 3]) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

fn darken(c: u32, num: u32, den: u32) -> u32 {
    let ch = |s: u32| ((c >> s & 0xff) * num / den) << s;
    ch(16) | ch(8) | ch(0)
}

/// Linear blend between two colours, t in 0..=1.
fn lerp(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s & 0xff) as f64, (b >> s & 0xff) as f64);
        ((x + (y - x) * t.clamp(0.0, 1.0)).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Population colour ramp (log scale): pale yellow → orange → dark red.
pub fn population_color(pop: f64, max: f64) -> u32 {
    if pop <= 0.0 || max <= 1.0 {
        return NO_DATA;
    }
    let t = (pop.ln() / max.ln()).clamp(0.0, 1.0);
    if t < 0.5 { lerp(0xfff3c4, 0xf08a24, t * 2.0) } else { lerp(0xf08a24, 0x7a0b0b, (t - 0.5) * 2.0) }
}

/// Stable, well-spread colour for an index (golden-angle hue).
pub fn index_color(i: u32) -> u32 {
    let h = (i as f64 * 137.508).rem_euclid(360.0) / 60.0;
    let (s, v) = (0.55, 0.85);
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    rgb([((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8])
}

/// Land with no data for the current mode (e.g. no pops).
pub const NO_DATA: u32 = 0x5a5a5a;

/// Per-province colours and owner keys for one map mode.
pub struct Palette {
    pub color: Vec<u32>,
    /// Owner index + 1 per province (0 = none / sea), used to draw country borders.
    pub owner_key: Vec<u32>,
}

impl Palette {
    pub fn build(world: &World, mode: MapMode) -> Self {
        let n = world.provinces.len();
        let mut color = vec![UNKNOWN; n];
        let mut owner_key = vec![0; n];
        let max_pop = world.provinces.iter().flatten().map(|p| p.population()).fold(0.0, f64::max);
        for p in world.provinces.iter().flatten() {
            let i = p.id as usize;
            owner_key[i] = p.owner.map_or(0, |o| o as u32 + 1);
            color[i] = if mode != MapMode::Provinces && p.is_sea {
                SEA
            } else {
                match mode {
                    MapMode::Provinces => rgb(p.rgb),
                    MapMode::Political => match p.owner {
                        Some(o) => rgb(world.country(o).country.color),
                        None => UNOWNED,
                    },
                    MapMode::Population => population_color(p.population(), max_pop),
                    MapMode::Culture => p
                        .dominant(|x| x.culture)
                        .map_or(NO_DATA, |c| rgb(world.defs.cultures.cultures[c as usize].color)),
                    MapMode::Religion => p
                        .dominant(|x| x.religion)
                        .map_or(NO_DATA, |r| rgb(world.defs.religions.religions[r as usize].color)),
                    MapMode::States => p.region.map_or(NO_DATA, |r| index_color(r as u32)),
                }
            };
        }
        Palette { color, owner_key }
    }
}

/// View onto the map. `x`, `y` are the map coordinates of the screen's top-left corner;
/// `zoom` is screen pixels per map pixel. The map wraps horizontally.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 16.0;

impl Camera {
    /// Fits the whole map height into a `w` x `h` screen.
    pub fn fit(map: &ProvinceMap, w: usize, h: usize) -> Self {
        let zoom = (h as f64 / map.height as f64).min(w as f64 / map.width as f64).clamp(MIN_ZOOM, MAX_ZOOM);
        let mut c = Camera { x: 0.0, y: 0.0, zoom };
        c.x = (map.width as f64 - w as f64 / zoom) / 2.0;
        c.clamp(map, h);
        c
    }

    /// Map coordinates under a screen point, with x wrapped; None above/below the map.
    pub fn screen_to_map(&self, map: &ProvinceMap, sx: f64, sy: f64) -> Option<(usize, usize)> {
        let my = (self.y + sy / self.zoom).floor();
        if my < 0.0 || my >= map.height as f64 {
            return None;
        }
        let mx = (self.x + sx / self.zoom).floor().rem_euclid(map.width as f64);
        Some((mx as usize, my as usize))
    }

    pub fn pan(&mut self, map: &ProvinceMap, dx_screen: f64, dy_screen: f64, screen_h: usize) {
        self.x = (self.x - dx_screen / self.zoom).rem_euclid(map.width as f64);
        self.y -= dy_screen / self.zoom;
        self.clamp(map, screen_h);
    }

    /// Zooms by `factor`, keeping the map point under (sx, sy) fixed on screen.
    pub fn zoom_at(&mut self, map: &ProvinceMap, sx: f64, sy: f64, factor: f64, screen_h: usize) {
        let (mx, my) = (self.x + sx / self.zoom, self.y + sy / self.zoom);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.x = (mx - sx / self.zoom).rem_euclid(map.width as f64);
        self.y = my - sy / self.zoom;
        self.clamp(map, screen_h);
    }

    /// Centres the view on a map point.
    pub fn center_on(&mut self, map: &ProvinceMap, mx: f64, my: f64, w: usize, h: usize) {
        self.x = (mx - w as f64 / 2.0 / self.zoom).rem_euclid(map.width as f64);
        self.y = my - h as f64 / 2.0 / self.zoom;
        self.clamp(map, h);
    }

    /// Keeps the map vertically on screen (centred if it is shorter than the screen).
    fn clamp(&mut self, map: &ProvinceMap, screen_h: usize) {
        let visible = screen_h as f64 / self.zoom;
        let h = map.height as f64;
        self.y = if visible >= h { (h - visible) / 2.0 } else { self.y.clamp(0.0, h - visible) };
    }
}

pub struct RenderOptions {
    pub borders: bool,
    /// Province drawn highlighted (hover/selection).
    pub highlight: Option<ProvinceId>,
}

/// Draws the map into `buf` (row-major, `w` x `h`, 0x00RRGGBB).
pub fn render(map: &ProvinceMap, pal: &Palette, cam: &Camera, opts: &RenderOptions, buf: &mut [u32], w: usize, h: usize) {
    assert!(buf.len() >= w * h, "framebuffer too small");
    let (mw, mh) = (map.width, map.height);
    // Precompute the map column for every screen column once per frame.
    let cols: Vec<usize> =
        (0..w).map(|sx| (cam.x + sx as f64 / cam.zoom).floor().rem_euclid(mw as f64) as usize).collect();
    let draw_borders = opts.borders && cam.zoom >= 0.5;
    for sy in 0..h {
        let row = &mut buf[sy * w..(sy + 1) * w];
        let my = (cam.y + sy as f64 / cam.zoom).floor();
        if my < 0.0 || my >= mh as f64 {
            row.fill(BACKGROUND);
            continue;
        }
        let my = my as usize;
        let line = &map.ids[my * mw..(my + 1) * mw];
        let below = if my + 1 < mh { Some(&map.ids[(my + 1) * mw..(my + 2) * mw]) } else { None };
        for (sx, px) in row.iter_mut().enumerate() {
            let mx = cols[sx];
            let id = line[mx];
            let mut c = pal.color.get(id as usize).copied().unwrap_or(UNKNOWN);
            if draw_borders {
                let right = line[(mx + 1) % mw];
                let down = below.map_or(id, |b| b[mx]);
                for other in [right, down] {
                    if other != id {
                        let ok = |p: ProvinceId| pal.owner_key.get(p as usize).copied().unwrap_or(0);
                        c = if ok(other) != ok(id) { COUNTRY_BORDER } else { darken(c, 3, 4) };
                        break;
                    }
                }
            }
            if opts.highlight == Some(id) && id != 0 {
                c = (c >> 1 & 0x7f7f7f) + 0x7f7f7f; // blend 50% toward white
            }
            *px = c;
        }
    }
}

/// Province under a screen point, if any.
pub fn pick(map: &ProvinceMap, cam: &Camera, sx: f64, sy: f64) -> Option<ProvinceId> {
    let (mx, my) = cam.screen_to_map(map, sx, sy)?;
    Some(map.id_at(mx, my)).filter(|&id| id != 0)
}

/// Thousands separators for display: 1234567 -> "1,234,567".
pub fn fmt_int(v: f64) -> String {
    let s = format!("{:.0}", v.max(0.0));
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One-line description of a province for titles and console output.
pub fn describe(world: &World, id: ProvinceId) -> String {
    let Some(p) = world.province(id) else { return format!("#{id} (undefined)") };
    let mut s = format!("{} (#{})", p.name, p.id);
    if p.is_sea {
        s.push_str(" - sea");
        return s;
    }
    match p.owner {
        Some(o) => {
            let c = world.country(o);
            s.push_str(&format!(" - {} [{}]", c.name, c.country.tag));
        }
        None => s.push_str(" - unowned"),
    }
    if let Some(g) = &p.trade_goods {
        s.push_str(&format!(" - {g}"));
    }
    if !p.pops.is_empty() {
        s.push_str(&format!(" - pop {}", fmt_int(p.population())));
    }
    s
}
