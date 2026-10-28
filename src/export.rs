//! Export one node as SVG or a raster image, with render bounds so shadows are included.
//! Raster formats support a quality setting and an optional size budget.

use std::error::Error;
use std::path::Path;

use resvg::tiny_skia::Pixmap;

use crate::Tree;
use crate::render::{self, Prepared};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Png,
    Jpg,
    Webp,
    Avif,
    Svg,
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
            "svg" => Format::Svg,
            _ => return None,
        })
    }
}

/// An encoded image and what it ended up as.
pub struct Image {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Raster quality used (lowered to meet a budget); the requested value for SVG, which ignores it.
    pub quality: u8,
}

/// Renders node `id` and encodes it. SVG keeps its original dimensions and ignores quality.
#[allow(clippy::too_many_arguments)]
pub fn export(
    tree: &Tree,
    fig: &Path,
    id: &str,
    format: Format,
    scale: f64,
    quality: u8,
    max_bytes: Option<usize>,
    max_px: Option<u32>,
) -> Result<Image, Box<dyn Error>> {
    let prepared = Prepared::new(tree, fig, id)?;
    let unsupported = |reason: &str| {
        crate::FigError::new(
            crate::ErrorKind::UnsupportedNode,
            format!("node {id} is unsupported for export: {reason}"),
        )
    };
    let unbounded =
        || unsupported("no drawable bounds; inspect the node and select a sized drawable layer");
    // A hidden layer draws nothing: say so instead of writing an empty raster.
    if prepared.node()["visible"] == false {
        return Err(unsupported(
            "the layer is hidden; show it in Figma and save a new local copy, or export a visible layer",
        )
        .into());
    }
    if format == Format::Svg {
        let n = prepared.node();
        let area = render::render_bounds(n).ok_or_else(unbounded)?;
        let lookup = |h: &str| prepared.images.get(h).cloned();
        // Keep original image fills embedded; raster export's downsampling would lose detail.
        let bytes = render::Svg::new(tree, &lookup)
            .document(n, area, 1.0)
            .into_bytes();
        if max_bytes.is_some_and(|max| bytes.len() > max) {
            return Err(format!(
                "SVG comes to {} KB; a size budget needs jpg, webp or avif",
                bytes.len().div_ceil(1024)
            )
            .into());
        }
        let px = |len: f64| (len - 1e-3).ceil().max(1.0) as u32;
        return Ok(Image {
            bytes,
            width: px(area[2] - area[0]),
            height: px(area[3] - area[1]),
            quality,
        });
    }
    // Fit the longest side into `max_px`, so an agent's inline image stays small.
    let scale = match (max_px, render::export_area(prepared.node())) {
        (Some(max), Some(area)) => {
            let longest = (area[2] - area[0]).max(area[3] - area[1]) * scale;
            if longest > f64::from(max) {
                scale * f64::from(max) / longest
            } else {
                scale
            }
        }
        _ => scale,
    };
    let svg = prepared.export_svg(tree, scale).ok_or_else(unbounded)?;
    let pixmap = render::pixmap(&svg)?;
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
        Format::Svg => Err("SVG requires vector input; use export instead of encode".into()),
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
