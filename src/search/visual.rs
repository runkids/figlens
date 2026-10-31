//! Bounded raster fingerprints for visual retrieval (#95). These compare rendered appearance;
//! exact SVG outline lookup stays in `export::SvgIndex` and neither proves component identity.

use std::collections::{BTreeSet, BinaryHeap};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::expand::{Bounds, Stop};

use super::Index;

use resvg::tiny_skia::{Color, FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
use sha2::{Digest, Sha256};

pub const VERSION: &str = "visual-hash-1";
pub const RANKING: &str = "visual-hash-distance-1";
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 4_000_000;
/// White background, aspect-preserving fit, bilinear sampling; the version pins this policy.
pub const SIZE: u32 = 256;
/// Bytes of a normalized raster as `QueryImage::rgb` gives it.
pub const RGB_BYTES: usize = (SIZE * SIZE * 3) as usize;

#[derive(Clone, Copy, Debug)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A normalized image holds no source metadata or path. Callers own its temporary lifetime.
pub struct QueryImage {
    pub fingerprint: Fingerprint,
    pixmap: Pixmap,
}

/// One immutable snapshot's rendered unit, with bounded evidence of known omissions.
pub struct RenderedUnit {
    pub image: QueryImage,
    pub diagnostics: Vec<serde_json::Value>,
    pub diagnostics_truncated: bool,
    pub bounds: serde_json::Map<String, serde_json::Value>,
    /// Pixels the export rendered before normalization, for a caller's job-wide budget.
    pub pixels: u64,
}

/// Render over export bounds at at most 1024 pixels on the longest side, then normalize.
/// The caller decodes `tree` from `snapshot`; image paints never reopen a changing source path.
pub fn render_unit(
    tree: &crate::Tree,
    snapshot: &[u8],
    id: &str,
) -> Result<RenderedUnit, Box<dyn Error>> {
    let prepared = crate::render::Prepared::from_snapshot(tree, snapshot, id)?;
    let (raster, bounds) = crate::export::prepared_export(
        tree,
        &prepared,
        id,
        crate::export::Format::Png,
        false,
        1.0,
        100,
        Some(MAX_BYTES),
        Some(1024),
    )?;
    let image = QueryImage::from_png(&raster.bytes, None)?;
    let pixels = u64::from(raster.width) * u64::from(raster.height);
    let mut diagnostics = Vec::new();
    let mut truncated = false;
    collect_diagnostics(
        prepared.node(),
        &prepared.images,
        &mut diagnostics,
        &mut truncated,
    );
    Ok(RenderedUnit {
        image,
        diagnostics,
        diagnostics_truncated: truncated,
        bounds,
        pixels,
    })
}

fn collect_diagnostics(
    node: &serde_json::Value,
    images: &std::collections::HashMap<String, Vec<u8>>,
    out: &mut Vec<serde_json::Value>,
    truncated: &mut bool,
) {
    let mut add = |item: serde_json::Value| {
        if out.len() < super::MAX_LIMIT {
            out.push(item);
        } else {
            *truncated = true;
        }
    };
    for item in node["_figlens"]["renderDiagnostics"]
        .as_array()
        .into_iter()
        .flatten()
    {
        add(item.clone());
    }
    for paint in ["fills", "strokes"] {
        for fill in node[paint].as_array().into_iter().flatten() {
            if let Some(reference) = fill["imageRef"].as_str()
                && !images.contains_key(reference)
            {
                add(
                    serde_json::json!({"nodeId": node["id"], "feature": "image", "field": paint,
                    "state": "missing", "reason": "original image is not in the snapshot"}),
                );
            }
        }
    }
    for child in node["children"].as_array().into_iter().flatten() {
        collect_diagnostics(child, images, out, truncated);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    /// SHA-256 of the version, reduced aspect ratio and normalized opaque pixels.
    content: [u8; 32],
    /// Horizontal and vertical mean-luma gradients, 64 bits each.
    gradient: [u64; 2],
    mean_rgb: [u8; 3],
    aspect: [u32; 2],
}

/// Distances are features for ranking, not probabilities or proof that two designs are equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Distance {
    pub hamming: u32,
    pub color: u16,
    pub aspect: u16,
}

/// An indexed raster unit; visibility remains a query filter, not an extraction filter.
pub struct Unit<'a> {
    pub rid: u32,
    pub id: &'a str,
    pub hidden: bool,
    pub internal: bool,
}

/// Borrowed units in document order; no copied names, paths or image data.
pub struct Units<'a> {
    pub units: Vec<Unit<'a>>,
    pub complete: bool,
    pub reason: Option<&'static str>,
}

/// The same screen kinds as `tree --top-frames`, plus component roots and explicit subscopes.
/// Preparation uses a full extraction, never the partial record set of a lexical query.
pub fn units<'a>(
    index: &'a Index,
    selected: &[String],
    bounds: &mut Bounds,
) -> Result<Units<'a>, Box<dyn Error>> {
    if index.total != index.records.len() {
        return Err(super::invalid(
            "visual preparation requires a full scope index",
        ));
    }
    if selected.len() > super::MAX_LIMIT {
        return Err(super::invalid(
            "at most 200 visual subscopes may be selected",
        ));
    }
    let mut wanted = BTreeSet::new();
    for id in selected {
        if id.is_empty() || id.len() > super::store::ID_BYTES {
            return Err(super::invalid("invalid visual subscope id"));
        }
        if !bounds.charge(1, 64) {
            break;
        }
        wanted.insert(id.as_str());
    }
    let mut units = Vec::new();
    for (rid, record) in index.records.iter().enumerate() {
        if !bounds.charge(10, 0) {
            break;
        }
        let explicit = wanted.remove(record.id.as_str());
        if explicit && matches!(record.kind.as_str(), "DOCUMENT" | "CANVAS") {
            return Err(super::invalid("select a design scope below the page"));
        }
        let internal = record
            .page
            .is_some_and(|p| index.pages[p as usize].internal);
        let screen = matches!(
            record.kind.as_str(),
            "FRAME" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE"
        );
        let below_container = record
            .parent
            .is_some_and(|p| matches!(index.record(p).kind.as_str(), "CANVAS" | "SECTION"));
        let default = record.kind == "COMPONENT" || screen && (rid == 0 || below_container);
        if explicit || default {
            if !bounds.charge(0, 4 * std::mem::size_of::<Unit<'_>>()) {
                break;
            }
            units.push(Unit {
                rid: rid as u32,
                id: &record.id,
                hidden: record.hidden,
                internal,
            });
        }
    }
    if bounds.stop.is_none() && index.stop.is_none() && !wanted.is_empty() {
        return Err(super::invalid(
            "visual subscope is not in the prepared scope",
        ));
    }
    let reason = stop_reason(bounds.stop).or(index.stop);
    Ok(Units {
        units,
        complete: reason.is_none(),
        reason,
    })
}

/// The units of a stored scope: its default units, plus the scope root itself when it lies
/// below the page, so a visual job never widens a subscope to the whole file.
pub fn scope_units<'a>(index: &'a Index, bounds: &mut Bounds) -> Result<Units<'a>, Box<dyn Error>> {
    let selected: Vec<String> = index
        .records
        .first()
        .filter(|r| !matches!(r.kind.as_str(), "DOCUMENT" | "CANVAS"))
        .map(|r| r.id.clone())
        .into_iter()
        .collect();
    units(index, &selected, bounds)
}

/// One unit in the generation's document order. Raster comparison never proves node identity.
#[derive(Clone, Debug)]
pub struct Hit {
    pub rid: u32,
    pub exact: bool,
    pub distance: Distance,
}

pub struct Matches {
    pub hits: Vec<Hit>,
    pub complete: bool,
    pub reason: Option<&'static str>,
}

/// Exact normalized pixels first, then gradient, color and aspect distance, then document order.
/// The caller supplies only authorized, filtered units of the requested generation.
pub fn nearest(
    query: &Fingerprint,
    candidates: &[(u32, Fingerprint)],
    limit: usize,
    bounds: &mut Bounds,
    cancel: &AtomicBool,
) -> Result<Matches, Box<dyn Error>> {
    if !(1..=super::MAX_LIMIT).contains(&limit) {
        return Err(super::invalid("visual result limit takes 1 to 200"));
    }
    let mut best = BinaryHeap::new();
    // Keep only the requested page, never a sorted copy of every candidate.
    if !bounds.charge(0, limit * 64 + 128) {
        return Ok(Matches {
            hits: Vec::new(),
            complete: false,
            reason: stop_reason(bounds.stop),
        });
    }
    for (rid, fingerprint) in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Err(crate::FigError::new(
                crate::ErrorKind::Cancelled,
                "visual query cancelled",
            )
            .into());
        }
        if !bounds.charge(1, 0) {
            break;
        }
        let d = query.distance(fingerprint);
        let key = (
            !query.same_pixels(fingerprint),
            d.hamming,
            d.color,
            d.aspect,
            *rid,
        );
        if best.len() < limit {
            best.push(key);
        } else if best.peek().is_some_and(|last| key < *last) {
            best.pop();
            best.push(key);
        }
    }
    let hits = best
        .into_sorted_vec()
        .into_iter()
        .map(|(approximate, hamming, color, aspect, rid)| Hit {
            rid,
            exact: !approximate,
            distance: Distance {
                hamming,
                color,
                aspect,
            },
        })
        .collect();
    Ok(Matches {
        hits,
        complete: bounds.stop.is_none(),
        reason: stop_reason(bounds.stop),
    })
}

fn stop_reason(stop: Option<Stop>) -> Option<&'static str> {
    stop.map(|s| match s {
        Stop::Work => "work_limit",
        Stop::Bytes => "byte_limit",
        Stop::Depth => "depth_limit",
        Stop::Deadline => "deadline",
    })
}

impl QueryImage {
    /// Admission precedes PNG decoding. Entry points also bound the read to `MAX_BYTES + 1`.
    pub fn from_png(bytes: &[u8], crop: Option<Crop>) -> Result<Self, Box<dyn Error>> {
        if bytes.len() > MAX_BYTES {
            return Err(super::invalid(
                "query image exceeds 8 MiB; use a smaller image",
            ));
        }
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(super::invalid("query image must be PNG"));
        }
        let declared =
            imagesize::blob_size(bytes).map_err(|_| super::invalid("invalid PNG query image"))?;
        let pixels = (declared.width as u64).checked_mul(declared.height as u64);
        if declared.width == 0 || declared.height == 0 || pixels.is_none_or(|n| n > MAX_PIXELS) {
            return Err(super::invalid(
                "query image exceeds 4000000 pixels; crop it first",
            ));
        }
        let source =
            Pixmap::decode_png(bytes).map_err(|_| super::invalid("invalid PNG query image"))?;
        if source.width() as usize != declared.width || source.height() as usize != declared.height
        {
            return Err(super::invalid(
                "query image dimensions changed during decode",
            ));
        }
        Self::from_pixmap(&source, crop)
    }

    /// A renderer output follows exactly the same crop, background and sampling policy.
    pub fn from_pixmap(source: &Pixmap, crop: Option<Crop>) -> Result<Self, Box<dyn Error>> {
        if u64::from(source.width()) * u64::from(source.height()) > MAX_PIXELS {
            return Err(super::invalid(
                "query image exceeds 4000000 pixels; crop it first",
            ));
        }
        let crop = crop.unwrap_or(Crop {
            x: 0,
            y: 0,
            width: source.width(),
            height: source.height(),
        });
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|n| n > source.width())
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|n| n > source.height())
        {
            return Err(super::invalid("query image crop is outside the image"));
        }
        // Crop before filtering, so pixels outside a requested crop cannot bleed into it.
        let cropped = if crop.x != 0
            || crop.y != 0
            || crop.width != source.width()
            || crop.height != source.height()
        {
            let mut out = Pixmap::new(crop.width, crop.height)
                .ok_or_else(|| super::invalid("empty query image crop"))?;
            let row = crop.width as usize * 4;
            for y in 0..crop.height as usize {
                let start = ((y + crop.y as usize) * source.width() as usize + crop.x as usize) * 4;
                out.data_mut()[y * row..(y + 1) * row]
                    .copy_from_slice(&source.data()[start..start + row]);
            }
            Some(out)
        } else {
            None
        };
        let source = cropped.as_ref().unwrap_or(source);
        let mut pixmap = Pixmap::new(SIZE, SIZE).unwrap();
        pixmap.fill(Color::WHITE);
        let scale = (SIZE as f32 / source.width() as f32).min(SIZE as f32 / source.height() as f32);
        let transform = Transform::from_row(
            scale,
            0.0,
            0.0,
            scale,
            (SIZE as f32 - source.width() as f32 * scale) / 2.0,
            (SIZE as f32 - source.height() as f32 * scale) / 2.0,
        );
        pixmap.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            },
            transform,
            None,
        );
        let fingerprint = Fingerprint::of(&pixmap, [crop.width, crop.height]);
        Ok(Self {
            fingerprint,
            pixmap,
        })
    }

    /// The normalized raster as `SIZE * SIZE * 3` RGB bytes, the form a vision child receives.
    /// It is opaque: normalization paints every pixel over white.
    pub fn rgb(&self) -> Vec<u8> {
        let rgba = self.pixmap.data();
        rgba.chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect()
    }

    /// Admits exactly the bytes `rgb` gives, as they are: no resampling, crop or metadata. Its
    /// fingerprint describes this raster only, not the source the parent normalized.
    pub fn from_rgb(bytes: &[u8]) -> Result<Self, Box<dyn Error>> {
        if bytes.len() != RGB_BYTES {
            return Err(super::invalid(format!(
                "a normalized image holds {RGB_BYTES} RGB bytes, got {}",
                bytes.len()
            )));
        }
        let rgba = bytes
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        let pixmap = Pixmap::from_vec(rgba, IntSize::from_wh(SIZE, SIZE).unwrap()).unwrap();
        Ok(Self {
            fingerprint: Fingerprint::of(&pixmap, [SIZE, SIZE]),
            pixmap,
        })
    }

    /// Re-encoding the normalized raster strips container metadata, without persisting a file.
    pub fn png(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        self.pixmap
            .encode_png()
            .map_err(|e| super::invalid(e.to_string()))
    }
}

impl Fingerprint {
    /// Fixed-size, versioned bytes; stale or malformed cache entries must be rebuilt.
    pub fn encode(&self) -> [u8; 63] {
        let mut out = [0; 63];
        out[..4].copy_from_slice(b"VH1\0");
        out[4..36].copy_from_slice(&self.content);
        for (i, n) in self.gradient.iter().enumerate() {
            out[36 + i * 8..44 + i * 8].copy_from_slice(&n.to_le_bytes());
        }
        out[52..55].copy_from_slice(&self.mean_rgb);
        for (i, n) in self.aspect.iter().enumerate() {
            out[55 + i * 4..59 + i * 4].copy_from_slice(&n.to_le_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Box<dyn Error>> {
        if bytes.len() != 63 || &bytes[..4] != b"VH1\0" {
            return Err(super::invalid(
                "invalid visual fingerprint version or length; rebuild the visual layer",
            ));
        }
        let aspect = [
            u32::from_le_bytes(bytes[55..59].try_into().unwrap()),
            u32::from_le_bytes(bytes[59..63].try_into().unwrap()),
        ];
        if aspect.contains(&0) || u64::from(aspect[0]) * u64::from(aspect[1]) > MAX_PIXELS {
            return Err(super::invalid(
                "invalid visual fingerprint dimensions; rebuild the visual layer",
            ));
        }
        Ok(Self {
            content: bytes[4..36].try_into().unwrap(),
            gradient: [
                u64::from_le_bytes(bytes[36..44].try_into().unwrap()),
                u64::from_le_bytes(bytes[44..52].try_into().unwrap()),
            ],
            mean_rgb: bytes[52..55].try_into().unwrap(),
            aspect,
        })
    }

    pub fn same_pixels(&self, other: &Self) -> bool {
        self.content == other.content
    }

    fn of(pixmap: &Pixmap, size: [u32; 2]) -> Self {
        let (mut a, mut b) = (size[0], size[1]);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let aspect = [size[0] / a, size[1] / a];
        let mut digest = Sha256::new();
        digest.update(VERSION.as_bytes());
        for n in aspect {
            digest.update(n.to_le_bytes());
        }
        digest.update(pixmap.data());
        let mut luma = [[0u32; 9]; 9];
        let mut rgb = [0u64; 3];
        for (gy, row) in luma.iter_mut().enumerate() {
            let (y0, y1) = (gy * SIZE as usize / 9, (gy + 1) * SIZE as usize / 9);
            for (gx, mean) in row.iter_mut().enumerate() {
                let (x0, x1) = (gx * SIZE as usize / 9, (gx + 1) * SIZE as usize / 9);
                let mut sum = 0;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = &pixmap.data()[(y * SIZE as usize + x) * 4..][..3];
                        for (total, n) in rgb.iter_mut().zip(p) {
                            *total += u64::from(*n);
                        }
                        sum += 54 * u32::from(p[0]) + 183 * u32::from(p[1]) + 19 * u32::from(p[2]);
                    }
                }
                *mean = sum / ((x1 - x0) * (y1 - y0)) as u32;
            }
        }
        let mut gradient = [0u64; 2];
        for y in 0..8 {
            for x in 0..8 {
                gradient[0] |= u64::from(luma[y][x] < luma[y][x + 1]) << (y * 8 + x);
                gradient[1] |= u64::from(luma[y][x] < luma[y + 1][x]) << (y * 8 + x);
            }
        }
        Self {
            content: digest.finalize().into(),
            gradient,
            mean_rgb: rgb.map(|n| (n / u64::from(SIZE * SIZE)) as u8),
            aspect,
        }
    }

    pub fn distance(&self, other: &Self) -> Distance {
        let cross = [
            u64::from(self.aspect[0]) * u64::from(other.aspect[1]),
            u64::from(other.aspect[0]) * u64::from(self.aspect[1]),
        ];
        Distance {
            hamming: self
                .gradient
                .iter()
                .zip(other.gradient)
                .map(|(a, b)| (a ^ b).count_ones())
                .sum(),
            color: self
                .mean_rgb
                .iter()
                .zip(other.mean_rgb)
                .map(|(a, b)| u16::from(a.abs_diff(b)))
                .sum(),
            aspect: (cross[0].abs_diff(cross[1]) * 65535 / cross[0].max(cross[1])) as u16,
        }
    }
}

#[cfg(feature = "embed")]
pub mod model;

#[cfg(test)]
mod tests;

pub(crate) mod rank;
