//! Small DDS reader for the install's interface art: DXT1, DXT3, DXT5 and uncompressed (bit-mask)
//! surfaces, top mip level only. Returns RGBA8. Written from the public DirectDraw Surface / S3TC format
//! description; nothing from the game is embedded.

pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub px: Vec<u8>,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn decode(b: &[u8]) -> Result<Rgba, String> {
    if b.len() < 128 || &b[0..4] != b"DDS " {
        return Err("not a DDS file".into());
    }
    let height = u32le(b, 12) as usize;
    let width = u32le(b, 16) as usize;
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err("bad DDS size".into());
    }
    // Pixel format block at 76.
    let pf_flags = u32le(b, 80);
    let fourcc = &b[84..88];
    let data = &b[128..];
    let mut px = vec![0u8; width * height * 4];
    if pf_flags & 0x4 != 0 {
        let kind = match fourcc {
            b"DXT1" => 1,
            b"DXT2" | b"DXT3" => 3,
            b"DXT4" | b"DXT5" => 5,
            _ => return Err(format!("unsupported DDS fourcc {}", String::from_utf8_lossy(fourcc))),
        };
        let bsize = if kind == 1 { 8 } else { 16 };
        let (bw, bh) = (width.div_ceil(4), height.div_ceil(4));
        if data.len() < bw * bh * bsize {
            return Err("truncated DDS".into());
        }
        for by in 0..bh {
            for bx in 0..bw {
                let blk = &data[(by * bw + bx) * bsize..][..bsize];
                let mut out = [[0u8; 4]; 16];
                match kind {
                    1 => color_block(blk, &mut out, true),
                    3 => {
                        color_block(&blk[8..], &mut out, false);
                        for (i, o) in out.iter_mut().enumerate() {
                            let nib = (blk[i / 2] >> ((i % 2) * 4)) & 0xF;
                            o[3] = nib * 17;
                        }
                    }
                    _ => {
                        color_block(&blk[8..], &mut out, false);
                        alpha_block(&blk[..8], &mut out);
                    }
                }
                for (i, o) in out.iter().enumerate() {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    if x < width && y < height {
                        px[(y * width + x) * 4..][..4].copy_from_slice(o);
                    }
                }
            }
        }
    } else {
        let bits = u32le(b, 88) as usize;
        let masks = [u32le(b, 92), u32le(b, 96), u32le(b, 100), if pf_flags & 0x1 != 0 { u32le(b, 104) } else { 0 }];
        let bpp = bits / 8;
        if !(1..=4).contains(&bpp) {
            return Err("unsupported DDS bit depth".into());
        }
        let pitch = width * bpp;
        if data.len() < pitch * height {
            return Err("truncated DDS".into());
        }
        for y in 0..height {
            for x in 0..width {
                let o = y * pitch + x * bpp;
                let mut v = 0u32;
                for k in 0..bpp {
                    v |= (data[o + k] as u32) << (8 * k);
                }
                let d = &mut px[(y * width + x) * 4..][..4];
                for (c, m) in masks.iter().enumerate() {
                    d[c] = if *m == 0 { if c == 3 { 255 } else { 0 } } else { scale(v & m, *m) };
                }
            }
        }
    }
    Ok(Rgba { width, height, px })
}

fn scale(v: u32, m: u32) -> u8 {
    let shift = m.trailing_zeros();
    let max = m >> shift;
    (((v >> shift) * 255 + max / 2) / max) as u8
}

fn c565(c: u16) -> [u32; 3] {
    let r = (c >> 11) as u32 & 31;
    let g = (c >> 5) as u32 & 63;
    let b = c as u32 & 31;
    [(r * 255 + 15) / 31, (g * 255 + 31) / 63, (b * 255 + 15) / 31]
}

fn color_block(b: &[u8], out: &mut [[u8; 4]; 16], dxt1: bool) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (a, z) = (c565(c0), c565(c1));
    let mut pal = [[0u8; 4]; 4];
    let mix = |p: u32, q: u32, wa: u32, wb: u32, d: u32| [0, 1, 2].map(|i| ((a[i] * p + z[i] * q) * wa / d / wb) as u8);
    pal[0] = [a[0] as u8, a[1] as u8, a[2] as u8, 255];
    pal[1] = [z[0] as u8, z[1] as u8, z[2] as u8, 255];
    if c0 > c1 || !dxt1 {
        let m = mix(2, 1, 1, 1, 3);
        let n = mix(1, 2, 1, 1, 3);
        pal[2] = [m[0], m[1], m[2], 255];
        pal[3] = [n[0], n[1], n[2], 255];
    } else {
        let m = mix(1, 1, 1, 1, 2);
        pal[2] = [m[0], m[1], m[2], 255];
        pal[3] = [0, 0, 0, 0];
    }
    let idx = u32le(b, 4);
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((idx >> (2 * i)) & 3) as usize];
    }
}

fn alpha_block(b: &[u8], out: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = 0u64;
    for k in 0..6 {
        bits |= (b[2 + k] as u64) << (8 * k);
    }
    for (i, o) in out.iter_mut().enumerate() {
        o[3] = pal[((bits >> (3 * i)) & 7) as usize];
    }
}

/// Decode any picture the UI maker can show: `.dds` here, everything else through `image` (tga, png).
pub fn load_any(path: &std::path::Path) -> Option<Rgba> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if ext == "dds" {
        return decode(&std::fs::read(path).ok()?).ok();
    }
    let img = image::open(path).ok()?.to_rgba8();
    Some(Rgba { width: img.width() as usize, height: img.height() as usize, px: img.into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32, flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[0..4].copy_from_slice(b"DDS ");
        v[12..16].copy_from_slice(&h.to_le_bytes());
        v[16..20].copy_from_slice(&w.to_le_bytes());
        v[80..84].copy_from_slice(&flags.to_le_bytes());
        v[84..88].copy_from_slice(fourcc);
        v[88..92].copy_from_slice(&bits.to_le_bytes());
        for (i, m) in masks.iter().enumerate() {
            v[92 + i * 4..96 + i * 4].copy_from_slice(&m.to_le_bytes());
        }
        v
    }

    #[test]
    fn dxt1_solid_red() {
        let mut f = header(4, 4, 4, b"DXT1", 0, [0; 4]);
        f.extend_from_slice(&[0x00, 0xF8, 0x00, 0x00, 0, 0, 0, 0]);
        let r = decode(&f).unwrap();
        assert_eq!(&r.px[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn dxt5_alpha() {
        let mut f = header(4, 4, 4, b"DXT5", 0, [0; 4]);
        f.extend_from_slice(&[128, 0, 0, 0, 0, 0, 0, 0]);
        f.extend_from_slice(&[0xE0, 0x07, 0, 0, 0, 0, 0, 0]);
        let r = decode(&f).unwrap();
        assert_eq!(&r.px[0..4], &[0, 255, 0, 128]);
    }

    #[test]
    fn uncompressed_bgra() {
        let mut f = header(1, 1, 0x41, b"\0\0\0\0", 32, [0xFF0000, 0xFF00, 0xFF, 0xFF000000]);
        f.extend_from_slice(&[10, 20, 30, 40]);
        let r = decode(&f).unwrap();
        assert_eq!(r.px, vec![30, 20, 10, 40]);
    }
}
