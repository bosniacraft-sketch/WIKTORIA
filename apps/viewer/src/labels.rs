//! Country names laid along each country's shape (political map).
//!
//! Per owner, provinces are grouped into connected land masses (province adjacency). A country's
//! largest mass gets a label, plus the mass holding its capital and any other mass of at least 40% of the largest (big detached
//! territories yes, islands and small colonies no). Labels are laid out per view: only the part of a
//! mass that is visible on screen counts, so a name never runs off the map or under a panel. The
//! principal axis of the visible sample points gives the direction, and a quadratic least-squares fit
//! of the perpendicular offset bends the baseline. A label is kept only if its letters sit inside the
//! country; it is shrunk until it fits, else dropped. Labels that collide with a bigger one are shrunk
//! or dropped, and two labels of one country never sit close together.

use std::collections::HashMap;

use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, Vec2};
use wiktoria_data::world::World;

/// Font family for map labels (the serif face, see `ui::install_fonts`).
pub const LABEL_FAMILY: &str = crate::theme::SERIF;

/// Largest letter height on screen (points): huge countries get a capped font, not no label.
const MAX_PX: f64 = 46.0;
/// Smallest readable letter height (points). Below this a label is dropped.
const MIN_PX: f64 = 8.0;
/// Smallest land mass (map pixels) that gets a label at all.
const MIN_PIXELS: u32 = 120;
/// Other masses of a country need at least this share of its largest mass.
const SECONDARY_SHARE: f64 = 0.4;
/// Sampling step (map pixels) of the stored mass points.
const STEP: usize = 2;

#[derive(Debug, Clone)]
pub struct Label {
    pub country: u16,
    pub text: Vec<char>,
    /// One centre point (screen points) and tangent angle (radians, y down) per letter.
    pub glyphs: Vec<(Pos2, f32)>,
    /// Letter height in screen points.
    pub size: f32,
}

struct Mass {
    country: u16,
    pixels: u32,
    capital: bool,
    /// Sample points (map pixels), every STEP-th pixel in x and y.
    pts: Vec<(u16, u16)>,
}

/// View the labels are laid out for.
#[derive(Clone, Copy, PartialEq)]
pub struct View {
    pub rect: Rect,
    /// Map coordinates of the screen's top-left corner.
    pub cam_x: f32,
    pub cam_y: f32,
    /// Screen points per map pixel.
    pub scale: f32,
}

impl View {
    fn to_screen(&self, mw: f32, mx: f32, my: f32) -> Pos2 {
        let dx = (mx - self.cam_x).rem_euclid(mw);
        egui::pos2(self.rect.min.x + dx * self.scale, self.rect.min.y + (my - self.cam_y) * self.scale)
    }
    fn to_map(&self, mw: f32, p: Pos2) -> (f32, f32) {
        ((self.cam_x + (p.x - self.rect.min.x) / self.scale).rem_euclid(mw), self.cam_y + (p.y - self.rect.min.y) / self.scale)
    }
}

#[derive(Default)]
pub struct Labels {
    key: Vec<u32>,
    masses: Vec<Mass>,
    view: Option<View>,
    pub labels: Vec<Label>,
}

impl Labels {
    /// Rebuild masses if the ownership key (palette `owner_key`: owner index + 1 per province)
    /// changed, and re-lay out labels if the view changed.
    pub fn update(&mut self, world: &World, owner_key: &[u32], view: View, name: impl Fn(u16) -> String) {
        if self.key.as_slice() != owner_key || self.key.is_empty() {
            self.key = owner_key.to_vec();
            self.masses = masses(world, owner_key);
            self.view = None;
        }
        if self.view != Some(view) {
            self.view = Some(view);
            self.labels = layout(world, &self.key, &self.masses, view, name);
        }
    }
}

fn masses(world: &World, owner_key: &[u32]) -> Vec<Mass> {
    let n = owner_key.len();
    let land = |id: usize| world.provinces.get(id).and_then(|p| p.as_ref()).is_some_and(|p| !p.is_sea);
    let mut comp = vec![u32::MAX; n];
    let mut comp_owner: Vec<u16> = Vec::new();
    for start in 0..n {
        if comp[start] != u32::MAX || owner_key[start] == 0 || !land(start) {
            continue;
        }
        let c = comp_owner.len() as u32;
        comp_owner.push((owner_key[start] - 1) as u16);
        comp[start] = c;
        let mut stack = vec![start];
        while let Some(p) = stack.pop() {
            if let Some(adj) = world.adjacency.get(p) {
                for &q in adj {
                    let q = q as usize;
                    if q < n && comp[q] == u32::MAX && owner_key[q] == owner_key[start] && land(q) {
                        comp[q] = c;
                        stack.push(q);
                    }
                }
            }
        }
    }
    let map = &world.map;
    let (w, h) = (map.width, map.height);
    let mut out: Vec<Mass> = comp_owner.iter().map(|&country| Mass { country, pixels: 0, capital: false, pts: Vec::new() }).collect();
    for y in 0..h {
        let row = &map.ids[y * w..(y + 1) * w];
        for (x, &id) in row.iter().enumerate() {
            let Some(&c) = comp.get(id as usize) else { continue };
            if c == u32::MAX {
                continue;
            }
            let m = &mut out[c as usize];
            m.pixels += 1;
            if x % STEP == 0 && y % STEP == 0 {
                m.pts.push((x as u16, y as u16));
            }
        }
    }
    let mut largest: HashMap<u16, u32> = HashMap::new();
    for m in &out {
        let e = largest.entry(m.country).or_default();
        *e = (*e).max(m.pixels);
    }
    // The capital's mass always counts (the UK's home islands are small next to Canada).
    let capital_comp: Vec<bool> = comp_owner
        .iter()
        .enumerate()
        .map(|(i, &o)| world.countries.get(o as usize).and_then(|c| c.history.capital).is_some_and(|cap| comp.get(cap as usize) == Some(&(i as u32))))
        .collect();
    for (m, &cap) in out.iter_mut().zip(&capital_comp) {
        m.capital = cap;
    }
    out.retain(|m| m.pixels >= MIN_PIXELS && (m.capital || m.pixels as f64 >= SECONDARY_SHARE * largest[&m.country] as f64));
    out.sort_by(|a, b| b.pixels.cmp(&a.pixels));
    out
}

/// Candidate label of one mass in the current view, before collision handling.
struct Cand {
    country: u16,
    area: f64,
    texts: Vec<Vec<char>>,
    // Screen-space frame: centre, axis, quadratic v(u) = a + b u + c u^2, extents.
    mx: f64,
    my: f64,
    dx: f64,
    dy: f64,
    q: (f64, f64, f64),
    umin: f64,
    umax: f64,
    vmin: f64,
    vmax: f64,
    size: f64,
}

fn layout(world: &World, owner_key: &[u32], masses: &[Mass], view: View, name: impl Fn(u16) -> String) -> Vec<Label> {
    let map = &world.map;
    let mw = map.width as f32;
    let inner = view.rect.shrink(4.0);
    let owner_at = |p: Pos2| -> u32 {
        if !inner.contains(p) {
            return 0;
        }
        let (x, y) = view.to_map(mw, p);
        if y < 0.0 || y >= map.height as f32 {
            return 0;
        }
        let id = map.ids[y as usize * map.width + (x as usize).min(map.width - 1)];
        owner_key.get(id as usize).copied().unwrap_or(0)
    };
    let cell = (STEP as f64 * view.scale as f64).powi(2); // screen area per sample
    let mut cands = Vec::new();
    for m in masses {
        // Visible sample points in screen space (wrap handled by the camera transform).
        let pts: Vec<(f64, f64)> = m
            .pts
            .iter()
            .map(|&(x, y)| view.to_screen(mw, x as f32, y as f32))
            .filter(|p| inner.contains(*p))
            .map(|p| (p.x as f64, p.y as f64))
            .collect();
        let n = pts.len() as f64;
        if n * cell < (MIN_PX * MIN_PX * 3.0) {
            continue;
        }
        let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
        let (mut cxx, mut cxy, mut cyy) = (0.0, 0.0, 0.0);
        for &(x, y) in &pts {
            let (rx, ry) = (x - mx, y - my);
            cxx += rx * rx;
            cxy += rx * ry;
            cyy += ry * ry;
        }
        let th = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
        let (mut dx, mut dy) = (th.cos(), th.sin());
        if dx < 0.0 {
            dx = -dx;
            dy = -dy;
        }
        let (mut su, mut sv) = ([0.0f64; 5], [0.0f64; 3]);
        let (mut umin, mut umax, mut vmin, mut vmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, y) in &pts {
            let (rx, ry) = (x - mx, y - my);
            let u = rx * dx + ry * dy;
            let v = -rx * dy + ry * dx;
            umin = umin.min(u);
            umax = umax.max(u);
            vmin = vmin.min(v);
            vmax = vmax.max(v);
            let u2 = u * u;
            su[0] += 1.0;
            su[1] += u;
            su[2] += u2;
            su[3] += u2 * u;
            su[4] += u2 * u2;
            sv[0] += v;
            sv[1] += v * u;
            sv[2] += v * u2;
        }
        let mm = [[su[0], su[1], su[2]], [su[1], su[2], su[3]], [su[2], su[3], su[4]]];
        let det3 = |m: [[f64; 3]; 3]| {
            m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
        };
        let d = det3(mm);
        let (mut qa, mut qb, mut qc) = (0.0, 0.0, 0.0);
        if d.abs() > 1e-9 {
            let col = |k: usize| {
                let mut m2 = mm;
                for (r, row) in m2.iter_mut().enumerate() {
                    row[k] = sv[r];
                }
                det3(m2) / d
            };
            (qa, qb, qc) = (col(0), col(1), col(2));
        }
        let len = (umax - umin).max(1.0);
        let half = 0.4 * len;
        let sag = (qc * half * half).abs();
        let max_sag = 0.25 * (vmax - vmin).min(len * 0.3);
        if sag > max_sag && sag > 0.0 {
            qc *= max_sag / sag;
        }
        let area = n * cell;
        let thick = (area / len).max(1.0);
        let full = name(m.country).to_uppercase();
        let mut texts: Vec<Vec<char>> = vec![full.chars().collect()];
        if let Some(short) = full.strip_prefix("THE ") {
            texts.push(short.chars().collect());
        }
        if texts[0].is_empty() {
            continue;
        }
        let size = (thick * 0.6).min(MAX_PX);
        cands.push(Cand { country: m.country, area, texts, mx, my, dx, dy, q: (qa, qb, qc), umin, umax, vmin, vmax, size });
    }
    cands.sort_by(|a, b| b.area.total_cmp(&a.area));

    let mut out: Vec<Label> = Vec::new();
    let mut boxes: Vec<Vec<Rect>> = Vec::new();
    for c in &cands {
        let key = c.country as u32 + 1;
        'texts: for text in &c.texts {
            let n = text.len() as f64;
            let span = 0.9 * (c.umax - c.umin);
            let mut size = (span / (n * 0.8)).min(c.size);
            while size >= MIN_PX {
              for shift in [0.0, -0.15, 0.15, -0.3, 0.3] {
                let label = place(c, text, size, shift);
                let rects: Vec<Rect> = label.glyphs.iter().map(|(p, _)| Rect::from_center_size(*p, Vec2::splat(size as f32 * 0.9))).collect();
                // Every letter centre inside the visible country; letter corners mostly inside.
                // Sea or unowned land under a letter is tolerated (straits between islands); another
                // country's land is not.
                let (mut own, mut foreign, mut total) = (0, 0, 0);
                for ((p, a), ch) in label.glyphs.iter().zip(text) {
                    if ch.is_whitespace() {
                        continue;
                    }
                    let nrm = Vec2::new(-a.sin(), a.cos()) * (size as f32 * 0.35);
                    let tan = Vec2::new(a.cos(), a.sin()) * (size as f32 * 0.3);
                    for o in [Vec2::ZERO, Vec2::ZERO, nrm, -nrm, tan, -tan] {
                        let k = owner_at(*p + o);
                        total += 1;
                        own += (k == key) as usize;
                        foreign += (k != key && k != 0) as usize;
                    }
                }
                let fits = own * 10 >= total * 7 && foreign * 10 <= total && label.glyphs.iter().all(|(p, _)| inner.contains(*p));
                let ours = rects.iter().fold(Rect::NOTHING, |a, r| a.union(*r));
                let clash = out.iter().zip(&boxes).any(|(o, bs)| {
                    let near = if o.country == c.country { ours.expand(ours.height().max(60.0)) } else { ours };
                    bs.iter().any(|b| near.intersects(*b)) && (o.country == c.country || rects.iter().any(|r| bs.iter().any(|b| r.intersects(*b))))
                });
                if fits && !clash {
                    boxes.push(rects);
                    out.push(label);
                    break 'texts;
                }
                if clash && out.iter().zip(&boxes).any(|(o, bs)| o.country == c.country && bs.iter().any(|b| ours.expand(ours.height().max(60.0)).intersects(*b))) {
                    break 'texts; // a second label of the same country nearby: drop
                }
              }
                size *= 0.85;
            }
        }
    }
    out
}

fn place(c: &Cand, text: &[char], size: f64, shift: f64) -> Label {
    let (qa, qb, qc) = c.q;
    let n = text.len() as f64;
    let span = 0.9 * (c.umax - c.umin);
    // Letter spacing: airy when there is room, down to tight for long names in small countries.
    let used = (size * n * 1.25).min(span).max(size * n * 0.8);
    let step = if n > 1.0 { used / n } else { 0.0 };
    let uc = (c.umin + c.umax) / 2.0 + shift * (c.umax - c.umin - used).max(0.0);
    let u0 = uc - step * (n - 1.0) / 2.0;
    let glyphs = (0..text.len())
        .map(|i| {
            let u = u0 + step * i as f64;
            let v = (qa + qb * u + qc * u * u).clamp(c.vmin * 0.6, c.vmax * 0.6);
            let dv = qb + 2.0 * qc * u;
            let tx = c.dx - dv * c.dy;
            let ty = c.dy + dv * c.dx;
            let px = c.mx + u * c.dx - v * c.dy;
            let py = c.my + u * c.dy + v * c.dx;
            (egui::pos2(px as f32, py as f32), ty.atan2(tx) as f32)
        })
        .collect();
    Label { country: c.country, text: text.to_vec(), glyphs, size: size as f32 }
}

/// Draw laid-out labels (already in screen points).
pub fn draw(painter: &egui::Painter, labels: &[Label], ink: Color32, halo: Color32) {
    for l in labels {
        let px = l.size;
        let font = FontId::new(px, FontFamily::Name(LABEL_FAMILY.into()));
        let halo = halo.gamma_multiply(0.8);
        let hw = (px * 0.06).clamp(1.0, 3.0);
        for (ch, (p, a)) in l.text.iter().zip(&l.glyphs) {
            if ch.is_whitespace() {
                continue;
            }
            let s = ch.to_string();
            let gal = painter.layout_no_wrap(s.clone(), font.clone(), ink);
            let rot = egui::emath::Rot2::from_angle(*a);
            let pos = *p - rot * (gal.size() / 2.0);
            for off in [Vec2::new(hw, 0.0), Vec2::new(-hw, 0.0), Vec2::new(0.0, hw), Vec2::new(0.0, -hw)] {
                let hg = painter.layout_no_wrap(s.clone(), font.clone(), halo);
                painter.add(egui::epaint::TextShape::new(pos + off, hg, halo).with_angle(*a));
            }
            painter.add(egui::epaint::TextShape::new(pos, gal, ink).with_angle(*a));
        }
    }
}
