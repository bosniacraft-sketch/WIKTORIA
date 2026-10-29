//! Minimal DirectDraw Surface reader: the first picture of an uncompressed 24- or 32-bit RGB file.
//! Compressed (DXT) files are refused. Victoria 2 keeps its map colour pictures in this form.

use crate::bmp::Image;

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// The first picture as RGB, rows in file order (alpha, if any, is dropped).
pub fn decode(b: &[u8]) -> Result<Image, String> {
    if b.get(0..4) != Some(b"DDS ") {
        return Err("not a DDS file".into());
    }
    let field = |o: usize| u32_at(b, o).ok_or_else(|| "DDS header cut short".to_string());
    let (height, width) = (field(12)? as usize, field(16)? as usize);
    let (flags, bits) = (field(80)?, field(88)? as usize);
    if flags & 0x40 == 0 {
        return Err("compressed DDS files are not read".into());
    }
    if bits != 24 && bits != 32 {
        return Err(format!("{bits}-bit DDS files are not read"));
    }
    let masks = [field(92)?, field(96)?, field(100)?];
    if masks.contains(&0) {
        return Err("DDS colour masks missing".into());
    }
    let step = bits / 8;
    let data = b.get(128..128 + width * height * step).ok_or("DDS picture cut short")?;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for px in data.chunks_exact(step) {
        let v = px.iter().rev().fold(0u32, |v, &x| v << 8 | u32::from(x));
        rgb.extend(masks.map(|m| ((v & m) >> m.trailing_zeros()) as u8));
    }
    Ok(Image { width, height, rgb })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(bits: u32, px: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[12..16].copy_from_slice(&1u32.to_le_bytes());
        b[16..20].copy_from_slice(&2u32.to_le_bytes());
        b[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        b[88..92].copy_from_slice(&bits.to_le_bytes());
        for (o, m) in [(92, 0xff0000u32), (96, 0xff00), (100, 0xff)] {
            b[o..o + 4].copy_from_slice(&m.to_le_bytes());
        }
        b.extend_from_slice(px);
        b
    }

    #[test]
    fn reads_bgra_and_bgr() {
        let img = decode(&file(32, &[1, 2, 3, 255, 4, 5, 6, 255])).unwrap();
        assert_eq!((img.width, img.height, img.rgb), (2, 1, vec![3, 2, 1, 6, 5, 4]));
        let img = decode(&file(24, &[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!(img.rgb, vec![3, 2, 1, 6, 5, 4]);
    }

    #[test]
    fn refuses_compressed() {
        let mut b = file(32, &[0; 8]);
        b[80..84].copy_from_slice(&4u32.to_le_bytes());
        assert!(decode(&b).is_err());
    }
}
