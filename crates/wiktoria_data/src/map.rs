//! The province map: `map/default.map`, `map/definition.csv` and `map/provinces.bmp`.

use std::collections::HashMap;

use crate::bmp::Image;
use crate::script::{self, Value};

pub type ProvinceId = u16;

#[derive(Debug, Clone, PartialEq)]
pub struct ProvinceDef {
    pub id: ProvinceId,
    pub rgb: [u8; 3],
    /// Name column of definition.csv (localisation may override it).
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct DefaultMap {
    pub max_provinces: u32,
    pub sea_starts: Vec<ProvinceId>,
    pub definitions: String,
    pub provinces: String,
    pub region: String,
    /// Optional geography files named in default.map.
    pub terrain: Option<String>,
    pub terrain_definition: Option<String>,
    pub continent: Option<String>,
    pub climate: Option<String>,
    pub adjacencies: Option<String>,
    pub rivers: Option<String>,
}

pub fn parse_default_map(src: &str) -> Result<DefaultMap, String> {
    let doc = script::parse(src).map_err(|e| e.to_string())?;
    let r = &doc.root;
    let text = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string).ok_or(format!("default.map: `{k}` missing"));
    let sea_starts = match r.get("sea_starts").and_then(Value::as_block) {
        Some(b) => b
            .bare_scalars()
            .iter()
            .map(|s| s.parse().map_err(|_| format!("default.map: bad sea province id '{s}'")))
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    Ok(DefaultMap {
        max_provinces: r.get("max_provinces").and_then(Value::as_f64).unwrap_or(0.0) as u32,
        sea_starts,
        definitions: text("definitions")?,
        provinces: text("provinces")?,
        region: text("region")?,
        terrain: text("terrain").ok(),
        terrain_definition: text("terrain_definition").ok(),
        continent: text("continent").ok(),
        climate: text("climate").ok(),
        adjacencies: text("adjacencies").ok(),
        rivers: text("rivers").ok(),
    })
}

/// Parses `definition.csv` (`id;r;g;b;name;x`). Rows without a numeric id are unused
/// colours in the shipped file and are skipped.
pub fn parse_definitions(src: &str) -> Result<(Vec<ProvinceDef>, Vec<String>), String> {
    let mut defs = Vec::new();
    let mut warnings = Vec::new();
    let mut skipped = 0;
    for (n, line) in src.lines().enumerate() {
        let line_no = n + 1;
        let cols: Vec<&str> = line.split(';').collect();
        let Ok(id) = cols[0].trim().parse::<ProvinceId>() else {
            if n > 0 && !line.trim().is_empty() && !line.starts_with('#') {
                skipped += 1;
            }
            continue;
        };
        if id == 0 {
            return Err(format!("definition.csv line {line_no}: province id 0 is reserved"));
        }
        // Read leading digits like C atoi: the shipped file has e.g. "223.".
        let mut channel = |i: usize| -> Result<u8, String> {
            let raw = cols.get(i).map(|s| s.trim()).unwrap_or("");
            let digits: String = raw.chars().take_while(|c| c.is_ascii_digit()).collect();
            let v = digits.parse::<u8>().map_err(|_| format!("definition.csv line {line_no}: bad colour column {i} '{raw}'"))?;
            if digits.len() != raw.len() {
                warnings.push(format!("definition.csv line {line_no}: colour '{raw}' read as {v}"));
            }
            Ok(v)
        };
        let rgb = [channel(1)?, channel(2)?, channel(3)?];
        let name = cols.get(4).map(|s| s.trim().to_string()).unwrap_or_default();
        defs.push(ProvinceDef { id, rgb, name });
    }
    if skipped > 0 {
        warnings.push(format!("definition.csv: {skipped} rows without a province id skipped"));
    }
    let mut by_rgb: HashMap<[u8; 3], ProvinceId> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for d in &defs {
        if !seen.insert(d.id) {
            return Err(format!("definition.csv: duplicate province id {}", d.id));
        }
        if let Some(other) = by_rgb.insert(d.rgb, d.id) {
            warnings.push(format!("definition.csv: provinces {other} and {} share colour {:?}", d.id, d.rgb));
        }
    }
    Ok((defs, warnings))
}

/// Province id for every pixel, plus per-province pixel statistics.
#[derive(Debug, Clone)]
pub struct ProvinceMap {
    pub width: usize,
    pub height: usize,
    /// Row-major, top-down. 0 = colour not in definition.csv.
    pub ids: Vec<ProvinceId>,
    /// Indexed by province id.
    pub pixel_count: Vec<u32>,
    /// Pixel centroid (x, y) per province id; (0, 0) if the province has no pixels.
    pub centroid: Vec<(f32, f32)>,
}

impl ProvinceMap {
    pub fn build(img: &Image, defs: &[ProvinceDef]) -> (Self, Vec<String>) {
        let max_id = defs.iter().map(|d| d.id).max().unwrap_or(0) as usize;
        let lookup: HashMap<[u8; 3], ProvinceId> = defs.iter().map(|d| (d.rgb, d.id)).collect();
        let mut ids = Vec::with_capacity(img.width * img.height);
        let mut pixel_count = vec![0u32; max_id + 1];
        let mut sum = vec![(0f64, 0f64); max_id + 1];
        // colour -> (pixel count, first x, first y)
        let mut unknown: HashMap<[u8; 3], (u32, usize, usize)> = HashMap::new();
        let mut last = ([0u8; 3], None::<ProvinceId>);
        for y in 0..img.height {
            for x in 0..img.width {
                let i = (y * img.width + x) * 3;
                let c = [img.rgb[i], img.rgb[i + 1], img.rgb[i + 2]];
                let id = if last.1.is_some() && last.0 == c { last.1 } else { lookup.get(&c).copied() };
                last = (c, id);
                match id {
                    Some(id) => {
                        ids.push(id);
                        pixel_count[id as usize] += 1;
                        let s = &mut sum[id as usize];
                        s.0 += x as f64;
                        s.1 += y as f64;
                    }
                    None => {
                        ids.push(0);
                        unknown.entry(c).or_insert((0, x, y)).0 += 1;
                    }
                }
            }
        }
        let centroid = sum
            .iter()
            .zip(&pixel_count)
            .map(|(s, &n)| if n == 0 { (0.0, 0.0) } else { ((s.0 / n as f64) as f32, (s.1 / n as f64) as f32) })
            .collect();
        let mut warnings = Vec::new();
        if !unknown.is_empty() {
            let px: u32 = unknown.values().map(|v| v.0).sum();
            let mut top: Vec<_> = unknown.iter().collect();
            top.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
            let detail: Vec<String> = top
                .iter()
                .take(6)
                .map(|(c, (n, x, y))| format!("rgb({},{},{}) {n}px first at ({x},{y})", c[0], c[1], c[2]))
                .collect();
            warnings.push(format!(
                "provinces.bmp: {px} pixels in {} colours not in definition.csv; largest: {}",
                unknown.len(),
                detail.join("; ")
            ));
        }
        let missing = defs.iter().filter(|d| pixel_count[d.id as usize] == 0).count();
        if missing > 0 {
            warnings.push(format!("provinces.bmp: {missing} defined provinces have no pixels"));
        }
        (ProvinceMap { width: img.width, height: img.height, ids, pixel_count, centroid }, warnings)
    }

    pub fn id_at(&self, x: usize, y: usize) -> ProvinceId {
        self.ids[y * self.width + x]
    }

    /// The middle of each province's bounding box (x, y), indexed by province id; (0, 0) if the
    /// province has no pixels. One pass over the map: call it once and keep the result.
    pub fn bounds_centres(&self) -> Vec<(f32, f32)> {
        // min x, min y, max x, max y
        let mut bounds = vec![(usize::MAX, usize::MAX, 0usize, 0usize); self.pixel_count.len()];
        for (i, &id) in self.ids.iter().enumerate() {
            let (x, y) = (i % self.width, i / self.width);
            let b = &mut bounds[id as usize];
            *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
        }
        bounds.iter().zip(&self.pixel_count).map(|(b, &n)| if n == 0 { (0.0, 0.0) } else { ((b.0 + b.2) as f32 / 2.0, (b.1 + b.3) as f32 / 2.0) }).collect()
    }
}

/// A state region: its id and member provinces.
pub type RegionDef = (String, Vec<ProvinceId>);

/// `map/region.txt`: `REGION_ID = { province ids }`. Returns regions in file order, plus warnings.
pub fn parse_regions(src: &str) -> Result<(Vec<RegionDef>, Vec<String>), String> {
    let doc = script::parse(src).map_err(|e| format!("region.txt: {e}"))?;
    let mut warnings = doc.warnings.clone();
    let mut out: Vec<RegionDef> = Vec::new();
    let mut owner: HashMap<ProvinceId, String> = HashMap::new();
    for (name, value, line) in doc.root.entries() {
        let Some(b) = value.as_block() else {
            warnings.push(format!("region.txt line {line}: '{name}' is not a block"));
            continue;
        };
        let mut ids = Vec::new();
        for s in b.bare_scalars() {
            match s.parse::<ProvinceId>() {
                Ok(id) => {
                    if let Some(prev) = owner.insert(id, name.to_string()) {
                        warnings.push(format!("region.txt line {line}: province {id} is in both {prev} and {name}"));
                    }
                    ids.push(id);
                }
                Err(_) => warnings.push(format!("region.txt line {line}: bad province id '{s}'")),
            }
        }
        out.push((name.to_string(), ids));
    }
    Ok((out, warnings))
}
