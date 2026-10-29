// `header { icon := "counter.svg" }`: the program's icon, read once by `kova` (never by the
// executable). Whatever its format (PNG, SVG, JPEG, Windows' .ico, the Mac's .icns), it becomes
// one square PNG, which every system can take; a .ico or a .icns is also kept as it is for its
// own system, with all its sizes (`original`).

use resvg::{tiny_skia, usvg};

pub struct Icon {
    /// The image as a square PNG: the largest one of a .ico or a .icns.
    pub png: Vec<u8>,
    /// The file as read.
    pub original: Vec<u8>,
}

/// The side an SVG is drawn at: the largest size the Dock shows without blurring on a Retina screen.
const DRAWN: u32 = 512;

/// The icon in `bytes` (`written`: its name, for its extension); else the message to show.
pub fn read(written: &str, bytes: Vec<u8>) -> Result<Icon, String> {
    use crate::locale::t;
    let extension = written.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let bad = || t!("icon_unreadable", file = written, error = t!("icon_damaged"));
    let png = match extension.as_str() {
        "png" => Some(bytes.clone()).filter(|b| is_png(b)).ok_or_else(bad)?,
        "svg" => svg(&bytes).ok_or_else(bad)?,
        "jpg" | "jpeg" => jpeg(&bytes).ok_or_else(bad)?,
        "ico" => ico(&bytes).ok_or_else(bad)?,
        "icns" => icns(&bytes).ok_or_else(bad)?,
        _ => return Err(t!("icon_not_format", file = written, format = extension)),
    };
    // Always 8 bits per channel, with transparency: what every system reads.
    let png = tiny_skia::Pixmap::decode_png(&png).ok().and_then(|p| p.encode_png().ok()).ok_or_else(bad)?;
    let (width, height) = side(&png);
    if width != height {
        return Err(t!("icon_not_square", file = written, width = width, height = height));
    }
    Ok(Icon { png, original: bytes })
}

fn is_png(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() > 24
}

/// A PNG's width and height (its first chunk, `IHDR`).
pub fn side(png: &[u8]) -> (u32, u32) {
    let number = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (number(16), number(20))
}

/// Drawn on a square of `DRAWN` pixels, centered if it is not square itself.
fn svg(bytes: &[u8]) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = DRAWN as f32 / size.width().max(size.height());
    let (dx, dy) = ((DRAWN as f32 - size.width() * scale) / 2.0, (DRAWN as f32 - size.height() * scale) / 2.0);
    let mut pixmap = tiny_skia::Pixmap::new(DRAWN, DRAWN)?;
    resvg::render(&tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

fn jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    encode(pixels, info.width as u32, info.height as u32)
}

/// RGBA pixels (not premultiplied) as a PNG.
fn encode(mut rgba: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    tiny_skia::Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(width, height)?)?.encode_png().ok()
}

/// A .ico's images: (width, height, bits per pixel, its bytes: a PNG or a Windows bitmap).
pub fn ico_images(b: &[u8]) -> Option<Vec<(u32, u32, u16, &[u8])>> {
    let le16 = |at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
    let le32 = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
    if le16(0)? != 0 || le16(2)? != 1 {
        return None;
    }
    let mut images = vec![];
    for k in 0..le16(4)? as usize {
        let e = 6 + 16 * k;
        let side = |v: u8| if v == 0 { 256 } else { v as u32 };
        let (size, at) = (le32(e + 8)? as usize, le32(e + 12)? as usize);
        images.push((side(*b.get(e)?), side(*b.get(e + 1)?), le16(e + 6)?, b.get(at..at + size)?));
    }
    (!images.is_empty()).then_some(images)
}

/// A .ico's largest image.
fn ico(b: &[u8]) -> Option<Vec<u8>> {
    let (.., data) = ico_images(b)?.into_iter().max_by_key(|(w, h, bits, _)| (w * h, *bits))?;
    if is_png(data) {
        return Some(data.to_vec());
    }
    // A Windows bitmap: its header, its palette (1, 4 or 8 bits per pixel), then the rows from
    // the bottom up (BGRA, BGR or a color's number), then a mask of 1 bit per pixel saying
    // which are transparent (unless 32 bits give their own).
    let le16 = |at: usize| Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let le32 = |at: usize| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let (header, width, height, bits) = (le32(0)? as usize, le32(4)? as usize, le32(8)? as usize / 2, le16(14)? as usize);
    if !matches!(bits, 1 | 4 | 8 | 24 | 32) || width == 0 || width > 1024 || height != width {
        return None;
    }
    let colors = match (bits, le32(32)? as usize) {
        (24 | 32, _) => 0,
        (_, 0) => 1 << bits,
        (_, n) => n.min(256),
    };
    let pixels = header + 4 * colors;
    let row = (width * bits).div_ceil(32) * 4;
    let mask_row = width.div_ceil(32) * 4;
    let bit = |at: usize, x: usize, n: usize| Some(*data.get(at + x * n / 8)? as usize >> (8 - n - x * n % 8) & ((1 << n) - 1));
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        let from = pixels + (height - 1 - y) * row;
        let mask = pixels + row * height + (height - 1 - y) * mask_row;
        for x in 0..width {
            let p = match bits {
                24 | 32 => data.get(from + x * bits / 8..from + x * bits / 8 + bits / 8)?,
                _ => data.get(header + 4 * bit(from, x, bits)?..)?.get(..4)?,
            };
            let alpha = match bits {
                32 => p[3],
                _ if bit(mask, x, 1)? == 1 => 0,
                _ => 255,
            };
            rgba[(y * width + x) * 4..][..4].copy_from_slice(&[p[2], p[1], p[0], alpha]);
        }
    }
    encode(rgba, width as u32, height as u32)
}

/// A .icns's largest PNG (the sizes of 128 pixels and more are PNGs since macOS 10.7).
fn icns(b: &[u8]) -> Option<Vec<u8>> {
    if !b.starts_with(b"icns") {
        return None;
    }
    let mut at = 8;
    let mut best: Option<&[u8]> = None;
    while at + 8 <= b.len() {
        let size = u32::from_be_bytes(b[at + 4..at + 8].try_into().ok()?) as usize;
        let data = b.get(at + 8..at + size.max(8))?;
        if is_png(data) && best.is_none_or(|old| side(old).0 < side(data).0) {
            best = Some(data);
        }
        at += size.max(8);
    }
    best.map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#;
        let icon = super::read("a.svg", svg.to_vec()).unwrap();
        assert_eq!(super::side(&icon.png), (512, 512));
        // A .ico holding one bitmap of 2×2 pixels, 32 bits.
        let mut ico = vec![0, 0, 1, 0, 1, 0, 2, 2, 0, 0, 1, 0, 32, 0];
        let bitmap: Vec<u8> = [&40u32.to_le_bytes()[..], &2u32.to_le_bytes(), &4u32.to_le_bytes(), &[1, 0, 32, 0], &[0; 24], &[9; 16], &[0; 8]].concat();
        ico.extend((bitmap.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&bitmap);
        let icon = super::read("a.ico", ico).unwrap();
        assert_eq!(super::side(&icon.png), (2, 2));
        // A .icns holding that PNG.
        let png = icon.png;
        let icns = [&b"icns"[..], &(16 + png.len() as u32).to_be_bytes(), b"ic07", &(8 + png.len() as u32).to_be_bytes(), &png].concat();
        assert_eq!(super::read("a.icns", icns).unwrap().png, png);
        assert!(super::read("a.jpg", b"not a jpeg".to_vec()).is_err());
    }
}
