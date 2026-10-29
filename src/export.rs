//! Export one node as SVG or a raster image, with render bounds so shadows are included, or
//! with its layout box on request. Raster formats support a quality setting and a size budget.

use std::error::Error;
use std::path::Path;

use resvg::tiny_skia::Pixmap;
use serde_json::{Map, Value as Json, json};

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
        Format::from_ext(path.extension()?.to_str()?)
    }

    /// The format a bare extension (`png`, `JPEG`, ...) names.
    pub fn from_ext(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
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

/// Renders node `id` over its render bounds, or its layout box when `layout`, and encodes it.
/// SVG keeps its original dimensions and ignores quality. Also returns `layoutBox`, `renderBox`
/// and `warnings`.
#[allow(clippy::too_many_arguments)]
pub fn export(
    tree: &Tree,
    fig: &Path,
    id: &str,
    format: Format,
    layout: bool,
    scale: f64,
    quality: u8,
    max_bytes: Option<usize>,
    max_px: Option<u32>,
) -> Result<(Image, Map<String, Json>), Box<dyn Error>> {
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
    let n = prepared.node();
    // What a render-bounds export covers: raster adds a SECTION's margin and frames a page.
    let drawn = if format == Format::Svg {
        render::render_bounds(n)
    } else {
        render::export_area(n)
    };
    let area = if layout {
        render::layout_bounds(n).ok_or_else(|| {
            unsupported("it has no layout box (a page has none); export it without bounds layout")
        })?
    } else {
        drawn.ok_or_else(unbounded)?
    };
    let report = bounds_report(n, drawn, layout);
    if format == Format::Svg {
        let lookup = |h: &str| prepared.images.get(h).cloned();
        // Keep original image fills embedded; raster export's downsampling would lose detail.
        let bytes = render::Svg::new(tree, &lookup)
            .clipped(layout)
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
        let image = Image {
            bytes,
            width: px(area[2] - area[0]),
            height: px(area[3] - area[1]),
            quality,
        };
        return Ok((image, report));
    }
    // Fit the longest side into `max_px`, so an agent's inline image stays small.
    let longest = (area[2] - area[0]).max(area[3] - area[1]) * scale;
    let scale = match max_px {
        Some(max) if longest > f64::from(max) => scale * f64::from(max) / longest,
        _ => scale,
    };
    let pixmap = render::pixmap(&prepared.export_svg_in(tree, area, scale, layout))?;
    Ok((encode(&pixmap, format, quality, max_bytes)?, report))
}

/// Both boxes of REST node `n`, and a warning naming what is drawn past the layout box when
/// `drawn` (what a render-bounds export covers) differs from it, since such an asset does not fit
/// the layout slot. Causes follow the renderer's reach: shadows and layer blurs, outside or
/// centered strokes, children past an unclipped box, and a SECTION's margin.
fn bounds_report(n: &Json, drawn: Option<render::Rect>, layout: bool) -> Map<String, Json> {
    let mut warnings = Vec::new();
    if let (Some(l), Some(d)) = (render::layout_bounds(n), drawn) {
        // How far the drawn area reaches past each side of the layout box; negative is inside.
        let past = [l[1] - d[1], d[2] - l[2], d[3] - l[3], l[0] - d[0]];
        if past.iter().any(|d| d.abs() >= 0.01) {
            let r = &n["_r"];
            let mut causes = Vec::new();
            let reaches = |e: &Json| {
                render::visible(e)
                    && matches!(
                        e["type"].as_str(),
                        Some("DROP_SHADOW" | "FOREGROUND_BLUR" | "LAYER_BLUR")
                    )
            };
            if r["effects"]
                .as_array()
                .is_some_and(|e| e.iter().any(reaches))
            {
                causes.push("effects");
            }
            if render::has_paint(&r["strokePaints"])
                && r["strokeWeight"].as_f64().is_some_and(|w| w > 0.0)
                && matches!(r["strokeAlign"].as_str(), Some("OUTSIDE" | "CENTER"))
            {
                causes.push("stroke");
            }
            let outside = |c: &Json| {
                render::visible(c)
                    && render::render_bounds(c)
                        .is_some_and(|b| b[0] < l[0] || b[1] < l[1] || b[2] > l[2] || b[3] > l[3])
            };
            if n["clipsContent"] != true
                && n["children"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(outside))
            {
                causes.push("child geometry");
            }
            if n["type"] == "SECTION" && drawn != render::render_bounds(n) {
                causes.push("section margin");
            }
            let sides = ["top", "right", "bottom", "left"]
                .iter()
                .zip(past)
                .map(|(side, d)| format!("{side} {}", (d * 100.0).round() / 100.0 + 0.0))
                .collect::<Vec<_>>()
                .join(", ");
            let from = if past.iter().any(|&d| d >= 0.01) && !causes.is_empty() {
                format!(" from {}", causes.join(", "))
            } else {
                String::new()
            };
            let used = if layout {
                "exported at the layout box, cutting off what is drawn past it"
            } else {
                "exported over what is drawn; bounds layout exports the layout box"
            };
            warnings.push(format!(
                "the drawn area differs from the layout box by {sides} px (positive reaches past it){from}; {used}"
            ));
        }
    }
    let mut report = Map::new();
    report.insert("layoutBox".into(), n["absoluteBoundingBox"].clone());
    report.insert("renderBox".into(), n["absoluteRenderBounds"].clone());
    report.insert("warnings".into(), json!(warnings));
    report
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
    fn bounds_report_names_only_what_the_renderer_draws_past_the_box() {
        let warning = |n: Json, drawn: render::Rect| {
            bounds_report(&n, Some(drawn), false)["warnings"][0]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let node = |kind: &str, r: Json, children: Json| {
            json!({ "type": kind, "_r": r, "children": children,
                "absoluteBoundingBox": { "x": 0, "y": 0, "width": 10, "height": 10 },
                "absoluteRenderBounds": { "x": 0, "y": 0, "width": 12, "height": 12 } })
        };
        let shadow = node(
            "FRAME",
            json!({ "effects": [{ "type": "DROP_SHADOW" }, { "type": "INNER_SHADOW" }] }),
            json!([]),
        );
        assert!(warning(shadow, [0.0, 0.0, 12.0, 12.0]).contains("from effects;"));
        // An inner shadow and a stroke with no outward alignment stay inside the box.
        let overflow = node(
            "FRAME",
            json!({ "effects": [{ "type": "INNER_SHADOW" }], "strokeWeight": 2,
                "strokePaints": [{ "type": "SOLID" }] }),
            json!([{ "absoluteRenderBounds": { "x": 5, "y": 5, "width": 7, "height": 7 } }]),
        );
        assert!(warning(overflow, [0.0, 0.0, 12.0, 12.0]).contains("from child geometry;"));
        let section = node("SECTION", json!({}), json!([]));
        let margin = warning(section, [-40.0, -40.0, 52.0, 52.0]);
        assert!(
            margin.contains("top 40, right 42, bottom 42, left 40 px"),
            "{margin}"
        );
        assert!(margin.contains("from section margin;"), "{margin}");
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
