//! Minimal uncompressed BMP reader/writer (BITMAPINFOHEADER, 24-bit or 8-bit paletted).
//! Victoria 2's `provinces.bmp` is 24-bit; `terrain.bmp` and `rivers.bmp` are 8-bit.

/// Top-down RGB image (3 bytes per pixel, row 0 is the top of the picture).
#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl Image {
    /// Mirrors the image top-to-bottom.
    pub fn flip_vertical(&mut self) {
        let row = self.width * 3;
        for y in 0..self.height / 2 {
            let (top, bottom) = self.rgb.split_at_mut((self.height - 1 - y) * row);
            top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn i32_at(b: &[u8], o: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if b.get(0..2) != Some(b"BM") {
        return Err("not a BMP file".into());
    }
    let short = || "truncated BMP header".to_string();
    let data_off = u32_at(b, 10).ok_or_else(short)? as usize;
    let header_size = u32_at(b, 14).ok_or_else(short)? as usize;
    let width = i32_at(b, 18).ok_or_else(short)?;
    let raw_height = i32_at(b, 22).ok_or_else(short)?;
    let bpp = u16_at(b, 28).ok_or_else(short)?;
    let compression = u32_at(b, 30).ok_or_else(short)?;
    if header_size < 40 {
        return Err(format!("unsupported BMP header size {header_size}"));
    }
    if compression != 0 {
        return Err(format!("compressed BMP (type {compression}) not supported"));
    }
    if width <= 0 || raw_height == 0 {
        return Err("invalid BMP dimensions".into());
    }
    let (width, height) = (width as usize, raw_height.unsigned_abs() as usize);
    let bottom_up = raw_height > 0;
    let row_bytes = (width * bpp as usize).div_ceil(32) * 4;
    if b.len() < data_off + row_bytes * height {
        return Err("BMP pixel data is truncated".into());
    }
    let palette: Vec<[u8; 3]> = if bpp == 8 { palette(b)? } else { Vec::new() };
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let src_row = if bottom_up { height - 1 - y } else { y };
        let row = &b[data_off + src_row * row_bytes..];
        let dst = &mut rgb[y * width * 3..(y + 1) * width * 3];
        match bpp {
            24 => {
                for x in 0..width {
                    dst[x * 3] = row[x * 3 + 2];
                    dst[x * 3 + 1] = row[x * 3 + 1];
                    dst[x * 3 + 2] = row[x * 3];
                }
            }
            8 => {
                for x in 0..width {
                    let c = palette.get(row[x] as usize).copied().unwrap_or([0, 0, 0]);
                    dst[x * 3..x * 3 + 3].copy_from_slice(&c);
                }
            }
            _ => return Err(format!("{bpp}-bit BMP not supported")),
        }
    }
    Ok(Image { width, height, rgb })
}

/// Palette indices of an 8-bit BMP, row-major in the same orientation `decode` produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: usize,
    pub height: usize,
    pub index: Vec<u8>,
}

impl IndexedImage {
    /// The game treats the first stored row as north (like `Image::flip_vertical`).
    pub fn flip_vertical(&mut self) {
        let w = self.width;
        for y in 0..self.height / 2 {
            let (top, bottom) = self.index.split_at_mut((self.height - 1 - y) * w);
            top[y * w..(y + 1) * w].swap_with_slice(&mut bottom[..w]);
        }
    }
}

/// The colour table of an 8-bit BMP (RGB per palette index).
pub fn palette(b: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    let short = || "truncated BMP header".to_string();
    let header_size = u32_at(b, 14).ok_or_else(short)? as usize;
    let colors = match u32_at(b, 46).ok_or_else(short)? {
        0 => 256,
        n => n as usize,
    };
    let p = 14 + header_size;
    (0..colors).map(|i| b.get(p + i * 4..p + i * 4 + 3).map(|c| [c[2], c[1], c[0]]).ok_or_else(short)).collect()
}

pub fn decode_indexed(b: &[u8]) -> Result<IndexedImage, String> {
    if b.get(0..2) != Some(b"BM") {
        return Err("not a BMP file".into());
    }
    let short = || "truncated BMP header".to_string();
    let data_off = u32_at(b, 10).ok_or_else(short)? as usize;
    let width = i32_at(b, 18).ok_or_else(short)?;
    let raw_height = i32_at(b, 22).ok_or_else(short)?;
    if u16_at(b, 28).ok_or_else(short)? != 8 || u32_at(b, 30).ok_or_else(short)? != 0 {
        return Err("expected an uncompressed 8-bit BMP".into());
    }
    if width <= 0 || raw_height == 0 {
        return Err("invalid BMP dimensions".into());
    }
    let (width, height) = (width as usize, raw_height.unsigned_abs() as usize);
    let row_bytes = width.div_ceil(4) * 4;
    if b.len() < data_off + row_bytes * height {
        return Err("BMP pixel data is truncated".into());
    }
    let mut index = vec![0u8; width * height];
    for y in 0..height {
        let src_row = if raw_height > 0 { height - 1 - y } else { y };
        let row = &b[data_off + src_row * row_bytes..data_off + src_row * row_bytes + width];
        index[y * width..(y + 1) * width].copy_from_slice(row);
    }
    Ok(IndexedImage { width, height, index })
}

/// Encodes a 24-bit bottom-up BMP from top-down RGB.
pub fn encode(img: &Image) -> Vec<u8> {
    let row_bytes = (img.width * 3).div_ceil(4) * 4;
    let size = 54 + row_bytes * img.height;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(img.width as i32).to_le_bytes());
    out.extend_from_slice(&(img.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..img.height).rev() {
        let start = out.len();
        for x in 0..img.width {
            let [r, g, b] = img.pixel(x, y);
            out.extend_from_slice(&[b, g, r]);
        }
        out.resize(start + row_bytes, 0);
    }
    out
}
