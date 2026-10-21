//! Raster export of one node, as Figma's Export panel makes it (render bounds, so shadows are
//! included), in PNG, JPG, WebP or AVIF, with a quality setting and an optional size budget.

use std::error::Error;
use std::path::Path;

use resvg::tiny_skia::Pixmap;

use crate::Tree;
use crate::render::{self, Svg};
use crate::rest::render_nodes;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Png,
    Jpg,
    Webp,
    Avif,
}

impl Format {
    /// The format an output path's extension names.
    pub fn of(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => Format::Png,
            "jpg" | "jpeg" => Format::Jpg,
            "webp" => Format::Webp,
            "avif" => Format::Avif,
            _ => return None,
        })
    }
}

/// An encoded image and what it ended up as.
pub struct Image {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The quality used; lower than asked when a size budget forced it down.
    pub quality: u8,
}

/// Renders node `id` of the `.fig` at `fig` at `scale` and encodes it.
pub fn export(
    tree: &Tree,
    fig: &Path,
    id: &str,
    format: Format,
    scale: f64,
    quality: u8,
    max_bytes: Option<usize>,
) -> Result<Image, Box<dyn Error>> {
    let doc = render_nodes(tree, root_of(id));
    let n = crate::bundle::find(&doc["document"], id).ok_or("node not found")?;
    let images = crate::bundle::images(fig, n)?;
    let lookup = |h: &str| images.get(h).cloned();
    let area = render::export_area(n).ok_or("node has nothing to render")?;
    let pixmap = render::pixmap(
        &Svg::new(tree, &lookup)
            .for_raster()
            .backdrop(n, area)
            .document(n, area, scale),
    )?;
    Ok(encode(&pixmap, format, quality, max_bytes)?)
}

/// The real node an id lives in: an instance sublayer `I<instance>;…` exists only once
/// `<instance>` is expanded.
pub fn root_of(id: &str) -> &str {
    id.strip_prefix('I')
        .and_then(|rest| rest.split(';').next())
        .unwrap_or(id)
}

/// Encodes at `quality`, or at the highest lower quality that fits in `max_bytes`.
pub fn encode(
    p: &Pixmap,
    format: Format,
    quality: u8,
    max_bytes: Option<usize>,
) -> Result<Image, String> {
    let image = |bytes, quality| Image {
        bytes,
        width: p.width(),
        height: p.height(),
        quality,
    };
    let first = encode_at(p, format, quality)?;
    let Some(max) = max_bytes.filter(|&max| first.len() > max) else {
        return Ok(image(first, quality));
    };
    if format == Format::Png {
        return Err(format!(
            "PNG is lossless and comes to {} KB; a size budget needs jpg, webp or avif",
            first.len().div_ceil(1024)
        ));
    }
    // Size falls as quality falls, so binary-search the highest quality that fits.
    let (mut lo, mut hi, mut fit) = (1, quality - 1, None);
    while lo <= hi {
        let q = lo + (hi - lo) / 2;
        let bytes = encode_at(p, format, q)?;
        if bytes.len() <= max {
            fit = Some(image(bytes, q));
            lo = q + 1;
        } else {
            hi = q - 1;
        }
    }
    fit.ok_or_else(|| format!("does not fit in {} KB even at quality 1", max / 1024))
}

fn encode_at(p: &Pixmap, format: Format, quality: u8) -> Result<Vec<u8>, String> {
    let (w, h) = (p.width(), p.height());
    // Encoders take straight (not premultiplied) alpha.
    let rgba = || p.pixels().iter().map(|c| c.demultiply());
    match format {
        Format::Png => p.encode_png().map_err(|e| e.to_string()),
        Format::Jpg => {
            // JPG has no alpha: composite over white, as Figma does. Premultiplied makes it c + (255 - a).
            let rgb: Vec<u8> = p
                .pixels()
                .iter()
                .flat_map(|c| {
                    let white = 255 - c.alpha();
                    [c.red() + white, c.green() + white, c.blue() + white]
                })
                .collect();
            let size = |v: u32| u16::try_from(v).map_err(|_| "JPG is limited to 65535 px a side");
            let mut out = Vec::new();
            jpeg_encoder::Encoder::new(&mut out, quality)
                .encode(&rgb, size(w)?, size(h)?, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| e.to_string())?;
            Ok(out)
        }
        Format::Webp => {
            let bytes: Vec<u8> = rgba()
                .flat_map(|c| [c.red(), c.green(), c.blue(), c.alpha()])
                .collect();
            Ok(webp::Encoder::from_rgba(&bytes, w, h)
                .encode(f32::from(quality))
                .to_vec())
        }
        Format::Avif => {
            let pixels: Vec<ravif::RGBA8> = rgba()
                .map(|c| ravif::RGBA8::new(c.red(), c.green(), c.blue(), c.alpha()))
                .collect();
            ravif::Encoder::new()
                .with_quality(f32::from(quality))
                .with_speed(6)
                .encode_rgba(ravif::Img::new(&pixels[..], w as usize, h as usize))
                .map(|e| e.avif_file)
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noisy, half-transparent image, so lossy sizes depend on quality.
    fn noise() -> Pixmap {
        let mut p = Pixmap::new(64, 64).unwrap();
        let mut x: u32 = 1;
        for px in p.pixels_mut() {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let [r, g, b, _] = x.to_le_bytes();
            *px = resvg::tiny_skia::ColorU8::from_rgba(r, g, b, 128).premultiply();
        }
        p
    }

    #[test]
    fn size_budget_lowers_quality_until_it_fits() {
        let p = noise();
        let full = encode(&p, Format::Jpg, 90, None).unwrap().bytes.len();
        let fit = encode(&p, Format::Jpg, 90, Some(full / 2)).unwrap();
        assert!(fit.bytes.len() <= full / 2 && fit.quality < 90);
    }

    #[test]
    fn png_cannot_meet_a_budget_it_exceeds() {
        assert!(encode(&noise(), Format::Png, 80, Some(100)).is_err());
    }

    #[test]
    fn every_format_encodes() {
        let p = noise();
        let magic = |f| encode(&p, f, 80, None).unwrap().bytes[..12].to_vec();
        assert_eq!(
            (
                &magic(Format::Png)[1..4],
                &magic(Format::Jpg)[..2],
                &magic(Format::Webp)[8..12],
                &magic(Format::Avif)[4..12]
            ),
            (
                &b"PNG"[..],
                &[0xff, 0xd8][..],
                &b"WEBP"[..],
                &b"ftypavif"[..]
            )
        );
    }
}
