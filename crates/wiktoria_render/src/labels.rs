//! Country names laid along each country's shape, in map pixels (the page draws them over the map
//! picture, so they grow and shrink with it, as on the original's political map).
//!
//! Per owner, provinces are grouped into connected land masses (touching pixels). A country's largest
//! mass gets a label, plus the mass holding its capital and any other mass of at least
//! `SECONDARY_SHARE` of the largest. The principal axis of the mass's sample points gives the
//! direction, and a quadratic least-squares fit of the perpendicular offset bends the baseline. A
//! label is kept only if its letters sit inside the country; it is shrunk until it fits, else
//! dropped, and it gives way to a bigger label it would cross. The same method as the desktop
//! viewer's labels, done once for the whole map instead of per view. The sizes are our choices.

use wiktoria_data::map::ProvinceMap;

/// Largest and smallest letter height, in map pixels.
const MAX_SIZE: f64 = 70.0;
const MIN_SIZE: f64 = 5.0;
/// Smallest land mass (map pixels) that gets a label at all.
const MIN_PIXELS: u32 = 120;
/// Other masses of a country need at least this share of its largest mass.
const SECONDARY_SHARE: f64 = 0.4;
/// Sampling step (map pixels) of the mass points.
const STEP: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// The owner key (`Palette::owner_key`: owner index + 1).
    pub key: u32,
    pub text: String,
    /// Centre (map pixels) and angle (radians, y down) of each letter.
    pub glyphs: Vec<(f32, f32, f32)>,
    /// Letter height in map pixels.
    pub size: f32,
}

struct Mass {
    key: u32,
    pixels: u32,
    capital: bool,
    pts: Vec<(u16, u16)>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The connected land masses per owner, biggest first, without the ones too small to name.
fn masses(map: &ProvinceMap, is_land: &[bool], owner_key: &[u32], capital_of: &dyn Fn(u32) -> Option<u16>) -> Vec<Mass> {
    let n = owner_key.len();
    let (w, h) = (map.width, map.height);
    let key = |id: u16| if is_land.get(id as usize).copied().unwrap_or(false) { owner_key.get(id as usize).copied().unwrap_or(0) } else { 0 };
    let mut parent: Vec<usize> = (0..n).collect();
    for y in 0..h {
        for x in 0..w {
            let id = map.ids[y * w + x];
            let k = key(id);
            if k == 0 {
                continue;
            }
            for j in [(x + 1 < w).then(|| y * w + x + 1), (y + 1 < h).then(|| (y + 1) * w + x)].into_iter().flatten() {
                let other = map.ids[j];
                if other != id && key(other) == k {
                    let (a, b) = (find(&mut parent, id as usize), find(&mut parent, other as usize));
                    parent[a] = b;
                }
            }
        }
    }
    let mut index = vec![usize::MAX; n];
    let mut out: Vec<Mass> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let id = map.ids[y * w + x];
            let k = key(id);
            if k == 0 {
                continue;
            }
            let root = find(&mut parent, id as usize);
            if index[root] == usize::MAX {
                index[root] = out.len();
                out.push(Mass { key: k, pixels: 0, capital: false, pts: Vec::new() });
            }
            let m = &mut out[index[root]];
            m.pixels += 1;
            if x % STEP == 0 && y % STEP == 0 {
                m.pts.push((x as u16, y as u16));
            }
        }
    }
    // The capital's mass always counts (the home islands of a country are small next to its colonies).
    let keys: std::collections::BTreeSet<u32> = out.iter().map(|m| m.key).collect();
    for k in keys {
        if let Some(cap) = capital_of(k).map(usize::from).filter(|&c| c < n && key(c as u16) == k) {
            let i = index[find(&mut parent, cap)];
            if let Some(m) = out.get_mut(i) {
                m.capital = true;
            }
        }
    }
    let mut largest = std::collections::HashMap::new();
    for m in &out {
        let e = largest.entry(m.key).or_insert(0u32);
        *e = (*e).max(m.pixels);
    }
    out.retain(|m| m.pixels >= MIN_PIXELS && (m.capital || m.pixels as f64 >= SECONDARY_SHARE * largest[&m.key] as f64));
    out.sort_by_key(|m| std::cmp::Reverse(m.pixels));
    out
}

/// A mass's frame: centre, axis direction, the bend v(u) = a + b u + c u^2 across it, and extents.
struct Frame {
    mx: f64,
    my: f64,
    dx: f64,
    dy: f64,
    q: (f64, f64, f64),
    umin: f64,
    umax: f64,
    vmin: f64,
    vmax: f64,
    thick: f64,
}

fn frame(m: &Mass) -> Frame {
    let pts: Vec<(f64, f64)> = m.pts.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let n = pts.len().max(1) as f64;
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
        (dx, dy) = (-dx, -dy);
    }
    let (mut su, mut sv) = ([0.0f64; 5], [0.0f64; 3]);
    let (mut umin, mut umax, mut vmin, mut vmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in &pts {
        let (rx, ry) = (x - mx, y - my);
        let (u, v) = (rx * dx + ry * dy, -rx * dy + ry * dx);
        (umin, umax, vmin, vmax) = (umin.min(u), umax.max(u), vmin.min(v), vmax.max(v));
        let u2 = u * u;
        su = [su[0] + 1.0, su[1] + u, su[2] + u2, su[3] + u2 * u, su[4] + u2 * u2];
        sv = [sv[0] + v, sv[1] + v * u, sv[2] + v * u2];
    }
    let mm = [[su[0], su[1], su[2]], [su[1], su[2], su[3]], [su[2], su[3], su[4]]];
    let det3 = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det3(mm);
    let mut q = (0.0, 0.0, 0.0);
    if d.abs() > 1e-9 {
        let col = |k: usize| {
            let mut m2 = mm;
            for (r, row) in m2.iter_mut().enumerate() {
                row[k] = sv[r];
            }
            det3(m2) / d
        };
        q = (col(0), col(1), col(2));
    }
    let len = (umax - umin).max(1.0);
    let half = 0.4 * len;
    let sag = (q.2 * half * half).abs();
    let max_sag = 0.25 * (vmax - vmin).min(len * 0.3);
    if sag > max_sag && sag > 0.0 {
        q.2 *= max_sag / sag;
    }
    let area = m.pixels as f64;
    Frame { mx, my, dx, dy, q, umin, umax, vmin, vmax, thick: (area / len).max(1.0) }
}

fn place(f: &Frame, n: usize, size: f64, shift: f64) -> Vec<(f32, f32, f32)> {
    let (qa, qb, qc) = f.q;
    let nf = n as f64;
    let span = 0.9 * (f.umax - f.umin);
    // Letter spacing: airy when there is room, down to tight for long names in small countries.
    let used = (size * nf * 1.25).min(span).max(size * nf * 0.8);
    let step = if n > 1 { used / nf } else { 0.0 };
    let uc = (f.umin + f.umax) / 2.0 + shift * (f.umax - f.umin - used).max(0.0);
    let u0 = uc - step * (nf - 1.0) / 2.0;
    (0..n)
        .map(|i| {
            let u = u0 + step * i as f64;
            let v = (qa + qb * u + qc * u * u).clamp(f.vmin * 0.6, f.vmax * 0.6);
            let dv = qb + 2.0 * qc * u;
            let (tx, ty) = (f.dx - dv * f.dy, f.dy + dv * f.dx);
            ((f.mx + u * f.dx - v * f.dy) as f32, (f.my + u * f.dy + v * f.dx) as f32, ty.atan2(tx) as f32)
        })
        .collect()
}

/// A letter's box, as (x0, y0, x1, y1).
type Bx = (f32, f32, f32, f32);

fn overlap(a: &Bx, b: &Bx) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The labels of the whole map. `is_land` and `owner_key` are per province id; `name(key)` is the
/// country's name, `capital_of(key)` its capital province.
pub fn labels(map: &ProvinceMap, is_land: &[bool], owner_key: &[u32], name: &dyn Fn(u32) -> String, capital_of: &dyn Fn(u32) -> Option<u16>) -> Vec<Label> {
    let (w, h) = (map.width, map.height);
    let owner_at = |x: f32, y: f32| -> u32 {
        if x < 0.0 || y < 0.0 || x as usize >= w || y as usize >= h {
            return 0;
        }
        let id = map.ids[y as usize * w + x as usize];
        if !is_land.get(id as usize).copied().unwrap_or(false) {
            return 0;
        }
        owner_key.get(id as usize).copied().unwrap_or(0)
    };
    let mut out: Vec<Label> = Vec::new();
    let mut boxes: Vec<Vec<Bx>> = Vec::new();
    for m in masses(map, is_land, owner_key, capital_of) {
        let full = name(m.key).to_uppercase();
        let mut texts = vec![full.clone()];
        if let Some(short) = full.strip_prefix("THE ") {
            texts.push(short.to_string());
        }
        if full.trim().is_empty() {
            continue;
        }
        let f = frame(&m);
        'texts: for text in &texts {
            let chars: Vec<char> = text.chars().collect();
            let n = chars.len();
            let span = 0.9 * (f.umax - f.umin);
            let mut size = (span / (n as f64 * 0.8)).min(f.thick * 0.6).min(MAX_SIZE);
            while size >= MIN_SIZE {
                for shift in [0.0, -0.15, 0.15, -0.3, 0.3] {
                    let glyphs = place(&f, n, size, shift);
                    let s = size as f32;
                    let rects: Vec<Bx> = glyphs.iter().map(|&(x, y, _)| (x - s * 0.45, y - s * 0.45, x + s * 0.45, y + s * 0.45)).collect();
                    // Letters sit on the country: sea or unowned land under a letter is tolerated
                    // (straits between islands), another country's land is not.
                    let (mut own, mut foreign, mut total) = (0, 0, 0);
                    for (&(x, y, a), ch) in glyphs.iter().zip(&chars) {
                        if ch.is_whitespace() {
                            continue;
                        }
                        let (nx, ny) = (-a.sin() * s * 0.35, a.cos() * s * 0.35);
                        let (tx, ty) = (a.cos() * s * 0.3, a.sin() * s * 0.3);
                        for (ox, oy) in [(0.0, 0.0), (0.0, 0.0), (nx, ny), (-nx, -ny), (tx, ty), (-tx, -ty)] {
                            let k = owner_at(x + ox, y + oy);
                            total += 1;
                            own += usize::from(k == m.key);
                            foreign += usize::from(k != m.key && k != 0);
                        }
                    }
                    let fits = own * 10 >= total * 7 && foreign * 10 <= total;
                    let ours = rects.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |a, r| (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3)));
                    let grow = |b: Bx, d: f32| (b.0 - d, b.1 - d, b.2 + d, b.3 + d);
                    let near_own = out.iter().zip(&boxes).any(|(o, bs)| o.key == m.key && bs.iter().any(|b| overlap(&grow(ours, (ours.3 - ours.1).max(60.0)), b)));
                    if near_own {
                        break 'texts; // a second label of the same country nearby: none
                    }
                    let clash = boxes.iter().any(|bs| rects.iter().any(|r| bs.iter().any(|b| overlap(r, b))));
                    if fits && !clash {
                        boxes.push(rects);
                        out.push(Label { key: m.key, text: text.clone(), glyphs, size: s });
                        break 'texts;
                    }
                }
                size *= 0.85;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiktoria_data::bmp::Image;
    use wiktoria_data::map::ProvinceDef;

    /// A 120 x 40 world: province 1 (left two thirds, land), 2 (right third, land), 3 (sea row).
    #[test]
    fn a_wide_country_gets_a_flat_label_inside_it() {
        let (w, h) = (120, 40);
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend(if y >= 36 { [0, 0, 200] } else if x < 80 { [200, 0, 0] } else { [0, 200, 0] });
            }
        }
        let defs = [([200, 0, 0], 1), ([0, 200, 0], 2), ([0, 0, 200], 3)].map(|(rgb, id)| ProvinceDef { id, rgb, name: format!("p{id}") });
        let map = ProvinceMap::build(&Image { width: w, height: h, rgb }, &defs).0;
        let is_land = [false, true, true, false];
        let owner_key = [0, 1, 2, 0];
        let got = labels(&map, &is_land, &owner_key, &|k| if k == 1 { "Aland".into() } else { "Be".into() }, &|_| None);
        let a = got.iter().find(|l| l.key == 1).expect("label for the wide country");
        assert_eq!(a.text, "ALAND");
        assert!(a.glyphs.iter().all(|&(x, y, ang)| x < 80.0 && y < 36.0 && ang.abs() < 0.2), "{a:?}");
        assert!(a.size >= MIN_SIZE as f32);
    }
}
