//! Geometry and SVG rendering of REST-shaped nodes built with render info (`rest::render_nodes`).
//!
//! Paths come straight from the `.fig`: shapes carry `fillGeometry` / `strokeGeometry` and text
//! carries glyph outlines, all as command blobs, so no font files are needed. Everything is
//! written in page coordinates; the SVG's viewBox picks the region.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;

use serde_json::Value as Json;

use crate::Tree;
use crate::expand::{Mat, Vec2};

/// Expanded render input and original image bytes for one selected export node.
pub(crate) struct Prepared {
    pub document: Json,
    pub images: HashMap<String, Vec<u8>>,
}

impl Prepared {
    /// Expand the containing instance before selecting the exact target and its images.
    pub fn new(tree: &Tree, fig: &Path, id: &str) -> Result<Self, Box<dyn Error>> {
        let document = crate::rest::resolve_render_nodes(tree, id)?;
        let images = images(fig, &document["document"])?;
        Ok(Self { document, images })
    }

    pub fn node(&self) -> &Json {
        &self.document["document"]
    }

    /// A raster export includes page/section backdrops and uses export bounds, not asset bounds.
    pub fn export_svg(&self, tree: &Tree, scale: f64) -> Option<String> {
        let n = self.node();
        let area = export_area(n)?;
        let lookup = |h: &str| self.images.get(h).cloned();
        Some(
            Svg::new(tree, &lookup)
                .for_raster()
                .backdrop(n, area)
                .document(n, area, scale),
        )
    }
}

pub(crate) fn find<'j>(n: &'j Json, id: &str) -> Option<&'j Json> {
    if n["id"] == id {
        return Some(n);
    }
    n["children"].as_array()?.iter().find_map(|c| find(c, id))
}

/// The `images/<sha1>` bytes of every image fill under `document`, by imageRef.
pub(crate) fn images(
    fig: &Path,
    document: &Json,
) -> Result<HashMap<String, Vec<u8>>, Box<dyn Error>> {
    let mut images = HashMap::new();
    let mut refs = Vec::new();
    collect_refs(document, &mut refs);
    if refs.is_empty() {
        return Ok(images);
    }
    let mut zip = crate::open_zip(fig)?;
    for r in refs {
        let mut bytes = Vec::new();
        if let Ok(mut f) = zip.by_name(&format!("images/{r}")) {
            f.read_to_end(&mut bytes)?;
            images.insert(r, bytes);
        }
    }
    Ok(images)
}

fn collect_refs(v: &Json, out: &mut Vec<String>) {
    match v {
        Json::Object(o) => {
            if let Some(r) = o.get("imageRef").and_then(Json::as_str)
                && !out.iter().any(|x| x == r)
            {
                out.push(r.to_owned());
            }
            o.values().for_each(|x| collect_refs(x, out));
        }
        Json::Array(a) => a.iter().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

/// `[x0, y0, x1, y1]` in page coordinates.
pub type Rect = [f64; 4];

pub fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, b) => a.or(b),
    }
}

/// The overlap of `a` with `b`; a zero-width or zero-height `a` (a hairline) can still overlap.
pub fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    let open = |lo: f64, hi: f64, flat: bool| lo < hi || (flat && lo == hi);
    (open(r[0], r[2], a[0] == a[2]) && open(r[1], r[3], a[1] == a[3])).then_some(r)
}

pub fn rect_json(r: Option<Rect>) -> Json {
    r.map_or(Json::Null, |r| {
        serde_json::json!({ "x": r[0], "y": r[1], "width": r[2] - r[0], "height": r[3] - r[1] })
    })
}

pub fn rect_of(v: &Json) -> Option<Rect> {
    let f = |k: &str| v[k].as_f64();
    let (x, y) = (f("x")?, f("y")?);
    Some([x, y, x + f("width")?, y + f("height")?])
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    M(Vec2),
    L(Vec2),
    Q(Vec2, Vec2),
    C(Vec2, Vec2, Vec2),
    Z,
}

/// Decodes a command blob: a command byte (0 close, 1 move, 2 line, 3 quad, 4 cubic)
/// followed by its points as little-endian f32 pairs.
pub fn commands(blob: &[u8]) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut i = 0;
    let pt = |i: &mut usize| -> Option<Vec2> {
        let f = |o: usize| Some(f32::from_le_bytes(blob.get(o..o + 4)?.try_into().ok()?) as f64);
        let p = Vec2 {
            x: f(*i)?,
            y: f(*i + 4)?,
        };
        *i += 8;
        Some(p)
    };
    while i < blob.len() {
        let c = blob[i];
        i += 1;
        let seg = match c {
            0 => Some(Seg::Z),
            1 => pt(&mut i).map(Seg::M),
            2 => pt(&mut i).map(Seg::L),
            3 => (|| Some(Seg::Q(pt(&mut i)?, pt(&mut i)?)))(),
            4 => (|| Some(Seg::C(pt(&mut i)?, pt(&mut i)?, pt(&mut i)?)))(),
            _ => None,
        };
        match seg {
            Some(s) => out.push(s),
            None => break,
        }
    }
    out
}

/// Tight bounds of `segs` after `m`; curves are sampled, which is exact to well under a pixel.
fn path_bounds(segs: &[Seg], m: Mat) -> Option<Rect> {
    let mut b: Option<Rect> = None;
    let mut add = |p: Vec2| {
        let p = m.apply(p);
        b = union(b, Some([p.x, p.y, p.x, p.y]));
    };
    let mut cur = Vec2 { x: 0.0, y: 0.0 };
    for s in segs {
        match *s {
            Seg::M(p) | Seg::L(p) => {
                add(p);
                cur = p;
            }
            Seg::Q(c, p) => {
                for t in (1..=16).map(|i| i as f64 / 16.0) {
                    let u = 1.0 - t;
                    add(Vec2 {
                        x: u * u * cur.x + 2.0 * u * t * c.x + t * t * p.x,
                        y: u * u * cur.y + 2.0 * u * t * c.y + t * t * p.y,
                    });
                }
                cur = p;
            }
            Seg::C(c1, c2, p) => {
                for t in (1..=16).map(|i| i as f64 / 16.0) {
                    let u = 1.0 - t;
                    let (a, bb, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    add(Vec2 {
                        x: a * cur.x + bb * c1.x + c * c2.x + d * p.x,
                        y: a * cur.y + bb * c1.y + c * c2.y + d * p.y,
                    });
                }
                cur = p;
            }
            Seg::Z => {}
        }
    }
    b
}

fn path_d(segs: &[Seg], m: Mat) -> String {
    let mut d = String::new();
    let p = |v: Vec2| {
        let v = m.apply(v);
        format!("{} {}", num(v.x), num(v.y))
    };
    // Glyph outlines can open with a close command, which SVG path data rejects.
    let start = segs
        .iter()
        .position(|s| matches!(s, Seg::M(_)))
        .unwrap_or(segs.len());
    for s in &segs[start..] {
        let _ = match *s {
            Seg::M(a) => write!(d, "M{}", p(a)),
            Seg::L(a) => write!(d, "L{}", p(a)),
            Seg::Q(a, b) => write!(d, "Q{} {}", p(a), p(b)),
            Seg::C(a, b, c) => write!(d, "C{} {} {}", p(a), p(b), p(c)),
            Seg::Z => write!(d, "Z"),
        };
    }
    d
}

fn num(x: f64) -> String {
    let s = format!("{:.4}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// A rectangle with corner radii `[tl, tr, br, bl]`, as path segments.
fn rect_segs(x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) -> Vec<Seg> {
    let max = (w.min(h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|r| r.clamp(0.0, max));
    let v = |x, y| Vec2 { x, y };
    // Circular arcs as cubics.
    let k = 1.0 - 0.552_284_75;
    vec![
        Seg::M(v(x + tl, y)),
        Seg::L(v(x + w - tr, y)),
        Seg::C(v(x + w - tr * k, y), v(x + w, y + tr * k), v(x + w, y + tr)),
        Seg::L(v(x + w, y + h - br)),
        Seg::C(
            v(x + w, y + h - br * k),
            v(x + w - br * k, y + h),
            v(x + w - br, y + h),
        ),
        Seg::L(v(x + bl, y + h)),
        Seg::C(v(x + bl * k, y + h), v(x, y + h - bl * k), v(x, y + h - bl)),
        Seg::L(v(x, y + tl)),
        Seg::C(v(x, y + tl * k), v(x + tl * k, y), v(x + tl, y)),
        Seg::Z,
    ]
}

fn mat(v: &Json) -> Mat {
    let f = |i: usize| v[i].as_f64().unwrap_or(0.0);
    Mat {
        a: f(0),
        b: f(1),
        c: f(2),
        d: f(3),
        tx: f(4),
        ty: f(5),
    }
}

pub fn mat_json(m: Mat) -> Json {
    serde_json::json!([m.a, m.b, m.c, m.d, m.tx, m.ty])
}

fn kiwi_mat(t: &Json) -> Mat {
    let f = |k: &str| t[k].as_f64().unwrap_or(0.0);
    Mat {
        a: f("m00"),
        b: f("m10"),
        c: f("m01"),
        d: f("m11"),
        tx: f("m02"),
        ty: f("m12"),
    }
}

fn visible(p: &Json) -> bool {
    p["visible"] != Json::Bool(false) && p["opacity"].as_f64().unwrap_or(1.0) > 0.0
}

/// Known renderer fallbacks in the effective paints of one visible node.
/// This is deliberately not a completeness or pixel-fidelity claim.
pub(crate) fn diagnostics(tree: &Tree, n: &Json) -> Vec<Json> {
    let mut out = Vec::new();
    let mut drawn_strokes = None;
    let mut report = |feature: &str, field: String, state: &str, reason: &str| {
        // Decode stroke outlines only when a paint would otherwise produce a warning.
        if field.starts_with("strokePaints[")
            && !*drawn_strokes.get_or_insert_with(|| !stroke_shapes(tree, n).is_empty())
        {
            return;
        }
        out.push(serde_json::json!({
            "nodeId": n["id"], "feature": feature, "field": field,
            "state": state, "reason": reason,
        }));
    };
    let r = &n["_r"];
    let mut lists = Vec::new();
    // Match the renderer's per-region / per-character paint selection; unused styles
    // must not turn into warnings on the node that happens to store them.
    let text = n["type"] == "TEXT" && r["styleOverrideTable"].is_array();
    let (table, ids, table_name) = if text {
        let keep = r["truncationStartIndex"]
            .as_i64()
            .filter(|i| *i >= 0)
            .map_or(usize::MAX, |i| i as usize);
        let ids = r["glyphs"]
            .as_array()
            .into_iter()
            .flatten()
            .take(keep)
            .map(|g| {
                r["characterStyleIDs"][g["firstCharacter"].as_u64().unwrap_or(0) as usize]
                    .as_u64()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>();
        (&r["styleOverrideTable"], ids, "textData.styleOverrideTable")
    } else {
        let ids = r["fillGeometry"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| g["styleID"].as_u64().unwrap_or(0))
            .collect::<Vec<_>>();
        (&r["vectorStyles"], ids, "vectorData.styleOverrideTable")
    };
    let mut base = !text && ids.is_empty();
    for id in ids {
        let selected = table
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .find(|(_, s)| s["styleID"].as_u64() == Some(id) && s["fillPaints"].is_array());
        if let Some((index, style)) = selected {
            let field = format!("{table_name}[{index}].fillPaints");
            if !lists.iter().any(|(_, path)| path == &field) {
                lists.push((&style["fillPaints"], field));
            }
        } else {
            base = true;
        }
    }
    if base {
        lists.push((&r["fillPaints"], "fillPaints".into()));
    }
    if r["strokeWeight"].as_f64().unwrap_or(0.0) > 0.0 && has_paint(&r["strokePaints"]) {
        lists.push((&r["strokePaints"], "strokePaints".into()));
    }
    for (list, path) in lists {
        for (i, paint) in list
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, p)| visible(p))
        {
            let field = format!("{path}[{i}]");
            match paint["type"].as_str() {
                Some("IMAGE") => {
                    if paint["imageScaleMode"] == "TILE" {
                        report(
                            "imageTile",
                            format!("{field}.imageScaleMode"),
                            "approximate",
                            "TILE is rendered as FILL",
                        );
                    }
                    for adjustment in ["contrast", "temperature", "tint", "highlights", "shadows"] {
                        if paint["paintFilter"][adjustment]
                            .as_f64()
                            .is_some_and(|v| v != 0.0)
                        {
                            report(
                                "imageAdjustment",
                                format!("{field}.paintFilter.{adjustment}"),
                                "skipped",
                                "This image adjustment is not rendered",
                            );
                        }
                    }
                }
                Some("GRADIENT_ANGULAR" | "GRADIENT_DIAMOND") => {
                    report(
                        "gradient",
                        format!("{field}.type"),
                        "approximate",
                        "Angular and diamond gradients are rendered as radial gradients",
                    );
                }
                _ => {}
            }
        }
    }
    for (i, effect) in r["effects"].as_array().into_iter().flatten().enumerate() {
        if visible(effect)
            && effect["type"] == "BACKGROUND_BLUR"
            && effect["radius"].as_f64().is_some_and(|v| v > 0.0)
        {
            report(
                "backgroundBlur",
                format!("effects[{i}]"),
                "skipped",
                "Background blur is not rendered",
            );
        }
    }
    out
}

fn size_of(r: &Json) -> Vec2 {
    Vec2 {
        x: r["size"]["x"].as_f64().unwrap_or(0.0),
        y: r["size"]["y"].as_f64().unwrap_or(0.0),
    }
}

/// One geometry list (`fillGeometry` / `strokeGeometry`) as `(segments, evenodd)` per path.
fn geometry(tree: &Tree, list: &Json) -> Vec<(Vec<Seg>, bool)> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let blob = tree.blob(g["commandsBlob"].as_u64()?)?;
            Some((
                commands(&blob),
                matches!(g["windingRule"].as_str(), Some("EVENODD" | "ODD")),
            ))
        })
        .collect()
}

/// Corner radii `[tl, tr, br, bl]` of a REST node.
fn radii(n: &Json) -> [f64; 4] {
    match n["rectangleCornerRadii"].as_array() {
        Some(r) => [0, 1, 2, 3].map(|i| r.get(i).and_then(Json::as_f64).unwrap_or(0.0)),
        None => [n["cornerRadius"].as_f64().unwrap_or(0.0); 4],
    }
}

/// Fill outlines of a node in its own space: its fill geometry, its glyphs, or its box.
fn fill_shapes(tree: &Tree, n: &Json) -> Vec<(Vec<Seg>, bool, Mat)> {
    let r = &n["_r"];
    let m = mat(&r["m"]);
    if n["type"] == "TEXT" {
        return glyphs(tree, r)
            .into_iter()
            .map(|(segs, gm, _)| (segs, false, m * gm))
            .collect();
    }
    let geo = geometry(tree, &r["fillGeometry"]);
    if !geo.is_empty() {
        let gm = m * geometry_scale(r, "fillGeometrySize");
        return geo.into_iter().map(|(s, odd)| (s, odd, gm)).collect();
    }
    let s = size_of(r);
    vec![(rect_segs(0.0, 0.0, s.x, s.y, radii(n)), false, m)]
}

/// Glyph outlines with their transform inside the text box and their character index.
fn glyphs(tree: &Tree, r: &Json) -> Vec<(Vec<Seg>, Mat, usize)> {
    // Truncated text ends with an ellipsis glyph; the glyphs from this list index on are cut.
    let keep = r["truncationStartIndex"]
        .as_i64()
        .filter(|i| *i >= 0)
        .map_or(usize::MAX, |i| i as usize);
    r["glyphs"]
        .as_array()
        .into_iter()
        .flatten()
        .take(keep)
        .filter_map(|g| {
            let blob = tree.blob(g["commandsBlob"].as_u64()?)?;
            let fs = g["fontSize"].as_f64().unwrap_or(0.0);
            let p = &g["position"];
            // Glyphs are in em units with y up.
            let gm = Mat {
                a: fs,
                b: 0.0,
                c: 0.0,
                d: -fs,
                tx: p["x"].as_f64().unwrap_or(0.0),
                ty: p["y"].as_f64().unwrap_or(0.0),
            };
            Some((
                commands(&blob),
                gm,
                g["firstCharacter"].as_u64().unwrap_or(0) as usize,
            ))
        })
        .collect()
}

/// Stroke outlines in page space: stroke geometry when the file has it, else the box's outline
/// (`None` geometry) to be stroked with `stroke-width`.
fn stroke_shapes(tree: &Tree, n: &Json) -> Vec<(Vec<Seg>, bool, Mat)> {
    let r = &n["_r"];
    let gm = mat(&r["m"]) * geometry_scale(r, "strokeGeometrySize");
    geometry(tree, &r["strokeGeometry"])
        .into_iter()
        .map(|(s, odd)| (s, odd, gm))
        .collect()
}

/// Page-space outlines a pointer can hit on a vector-like node (fill and stroke geometry), as SVG
/// path data with its even-odd flag. Read-only: the same decoding rendering uses, for preview picking.
pub fn hit_paths(tree: &Tree, n: &Json) -> Vec<(String, bool)> {
    if !matches!(
        n["type"].as_str(),
        Some("VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "REGULAR_POLYGON" | "LINE" | "ELLIPSE")
    ) {
        return Vec::new();
    }
    let geometry = |list: &Json| !geometry(tree, list).is_empty();
    let mut shapes = if geometry(&n["_r"]["fillGeometry"]) {
        fill_shapes(tree, n)
    } else {
        Vec::new()
    };
    shapes.extend(stroke_shapes(tree, n));
    shapes
        .into_iter()
        .map(|(segs, odd, m)| (path_d(&segs, m), odd))
        .filter(|(d, _)| !d.is_empty())
        .collect()
}

/// Scales outlines drawn at the size under `field` (a resized instance child's source) to the node's size.
fn geometry_scale(r: &Json, field: &str) -> Mat {
    let (s, g) = (
        size_of(r),
        Vec2 {
            x: r[field]["x"].as_f64().unwrap_or(0.0),
            y: r[field]["y"].as_f64().unwrap_or(0.0),
        },
    );
    let k = |a: f64, b: f64| if b > 0.0 { a / b } else { 1.0 };
    Mat {
        a: k(s.x, g.x),
        b: 0.0,
        c: 0.0,
        d: k(s.y, g.y),
        tx: 0.0,
        ty: 0.0,
    }
}

fn has_paint(list: &Json) -> bool {
    list.as_array().into_iter().flatten().any(visible)
}

/// `base` (what a node paints before its effects, `base_bounds`) grown by its shadows and blurs:
/// what the node paints itself, not its children, in page space.
pub fn grow_by_effects(n: &Json, base: Rect) -> Rect {
    let r = &n["_r"];
    let mut out = Some(base);
    for e in r["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| visible(e))
    {
        let radius = e["radius"].as_f64().unwrap_or(0.0);
        let spread = e["spread"].as_f64().unwrap_or(0.0);
        let (dx, dy) = (
            e["offset"]["x"].as_f64().unwrap_or(0.0),
            e["offset"]["y"].as_f64().unwrap_or(0.0),
        );
        let grown = match e["type"].as_str() {
            Some("DROP_SHADOW") => {
                let g = radius + spread;
                [
                    base[0] + dx - g,
                    base[1] + dy - g,
                    base[2] + dx + g,
                    base[3] + dy + g,
                ]
            }
            Some("FOREGROUND_BLUR") | Some("LAYER_BLUR") => [
                base[0] - radius,
                base[1] - radius,
                base[2] + radius,
                base[3] + radius,
            ],
            _ => continue,
        };
        out = union(out, Some(grown));
    }
    out.unwrap_or(base)
}

/// What a node paints itself (not its children) before its effects, in page space; `None` when
/// it paints nothing.
pub fn base_bounds(tree: &Tree, n: &Json) -> Option<Rect> {
    let r = &n["_r"];
    let m = mat(&r["m"]);
    let mut b = None;
    if has_paint(&r["fillPaints"]) {
        for (segs, _, sm) in fill_shapes(tree, n) {
            b = union(b, path_bounds(&segs, sm));
        }
    }
    let fill_box = fill_shapes(tree, n)
        .iter()
        .fold(None, |b, (segs, _, sm)| union(b, path_bounds(segs, *sm)));
    let weight = r["strokeWeight"].as_f64().unwrap_or(0.0);
    if has_paint(&r["strokePaints"]) && weight > 0.0 {
        let shapes = stroke_shapes(tree, n);
        if shapes.is_empty() && n["type"] != "TEXT" {
            let s = size_of(r);
            let grow = match r["strokeAlign"].as_str() {
                Some("OUTSIDE") => weight,
                Some("CENTER") => weight / 2.0,
                _ => 0.0,
            };
            let segs = rect_segs(-grow, -grow, s.x + 2.0 * grow, s.y + 2.0 * grow, [0.0; 4]);
            b = union(b, path_bounds(&segs, m));
        }
        // Stroke geometry is a double-width centered outline; an inside stroke keeps the part within the shape.
        let inside = r["strokeAlign"] == "INSIDE";
        for (segs, _, sm) in shapes {
            let sb = path_bounds(&segs, sm);
            b = union(
                b,
                if inside {
                    sb.zip(fill_box).and_then(|(s, f)| intersect(s, f))
                } else {
                    sb
                },
            );
        }
    }
    // Containers always cover their own box, painted or not.
    if matches!(
        n["type"].as_str(),
        Some("FRAME" | "INSTANCE" | "COMPONENT" | "COMPONENT_SET" | "GROUP")
    ) {
        b = union(b, box_bounds(n));
    }
    b
}

/// The node's own box in page space, used to clip its children.
pub fn box_bounds(n: &Json) -> Option<Rect> {
    let r = &n["_r"];
    let s = size_of(r);
    path_bounds(&rect_segs(0.0, 0.0, s.x, s.y, [0.0; 4]), mat(&r["m"]))
}

/// Pixel size from the encoded image header, without decoding the image.
fn image_size(b: &[u8]) -> Option<(f64, f64)> {
    let size = imagesize::blob_size(b).ok()?;
    (size.width > 0 && size.height > 0).then_some((size.width as f64, size.height as f64))
}

/// Half the size, each pixel the mean of a 2x2 block (premultiplied, so edges stay right).
fn halve(p: &resvg::tiny_skia::Pixmap) -> resvg::tiny_skia::Pixmap {
    let (w, h) = ((p.width() / 2).max(1), (p.height() / 2).max(1));
    let mut out = resvg::tiny_skia::Pixmap::new(w, h).expect("nonzero size");
    let src = p.data();
    let stride = p.width() as usize * 4;
    let dst = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..4 {
                let at = |dx: usize, dy: usize| {
                    let (sx, sy) = (
                        (2 * x + dx).min(p.width() as usize - 1),
                        (2 * y + dy).min(p.height() as usize - 1),
                    );
                    u32::from(src[sy * stride + sx * 4 + c])
                };
                dst[(y * w as usize + x) * 4 + c] =
                    ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

/// The area a node exports as, like Figma's `exportAsync`: its render bounds (else its box);
/// a page is the union of its children. Sections get 40 px of empty margin.
pub fn export_area(n: &Json) -> Option<Rect> {
    let area = if n["type"] == "CANVAS" {
        n["children"]
            .as_array()
            .into_iter()
            .flatten()
            .fold(None, |a, c| union(a, render_bounds(c)))
    } else {
        render_bounds(n).or_else(|| box_bounds(n))
    }?;
    let pad = if n["type"] == "SECTION" { 40.0 } else { 0.0 };
    Some([area[0] - pad, area[1] - pad, area[2] + pad, area[3] + pad])
}

/// Writes SVG for render-built nodes.
pub struct Svg<'t, 'a> {
    tree: &'t Tree<'a>,
    images: &'t dyn Fn(&str) -> Option<Vec<u8>>,
    defs: String,
    ids: usize,
    /// Drawn under the node across the whole export area.
    backdrop: String,
    /// Output pixels per user unit, set by `document`.
    scale: f64,
    /// Downscaled images by hash and halving count; `None` keeps full images (SVG output).
    mips: Option<std::collections::HashMap<(String, u32), Vec<u8>>>,
}

impl<'t, 'a> Svg<'t, 'a> {
    pub fn new(tree: &'t Tree<'a>, images: &'t dyn Fn(&str) -> Option<Vec<u8>>) -> Self {
        Svg {
            tree,
            images,
            defs: String::new(),
            ids: 0,
            backdrop: String::new(),
            scale: 1.0,
            mips: None,
        }
    }

    /// For output that is rasterized at the document's scale: large images are downscaled
    /// first, and pure blurs skip unused shadow intermediates. An SVG kept as a file keeps
    /// full images and the original filter graph, since it may be drawn at any size.
    pub fn for_raster(mut self) -> Self {
        self.mips = Some(std::collections::HashMap::new());
        self
    }

    /// What Figma draws under an exported page or section across the whole area: the page's
    /// background, or the fills of the sections a section is nested in (margin included).
    pub fn backdrop(mut self, n: &Json, area: Rect) -> Self {
        let mut layers = Vec::new();
        let mut id = n["id"].as_str().unwrap_or("").to_owned();
        if n["type"] == "CANVAS" {
            let page = self
                .tree
                .nodes
                .get(&id)
                .map(|k| crate::to_json(k))
                .unwrap_or_default();
            if page["backgroundEnabled"] == true {
                let mut color = page["backgroundColor"].clone();
                color["a"] = serde_json::json!(
                    color["a"].as_f64().unwrap_or(1.0)
                        * page["backgroundOpacity"].as_f64().unwrap_or(1.0)
                );
                layers.push(serde_json::json!([{ "type": "SOLID", "color": color }]));
            }
        } else if n["type"] != "SECTION" {
            return self;
        }
        while n["type"] == "SECTION"
            && let Some(parent) = self
                .tree
                .nodes
                .get(&id)
                .and_then(|k| k.get("parentIndex")?.get("guid"))
                .and_then(crate::guid_key)
        {
            let Some(node) = self.tree.nodes.get(&parent) else {
                break;
            };
            if !matches!(
                node.get("type"),
                Some(kiwi_schema::Value::Enum(_, "SECTION"))
            ) {
                break;
            }
            layers.push(crate::to_json(node)["fillPaints"].clone());
            id = parent;
        }
        for p in layers
            .iter()
            .rev()
            .flat_map(|l| l.as_array().into_iter().flatten())
        {
            if p["type"] == "SOLID" && p["visible"] != false {
                let a =
                    p["color"]["a"].as_f64().unwrap_or(1.0) * p["opacity"].as_f64().unwrap_or(1.0);
                let _ = write!(
                    self.backdrop,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                    num(area[0]),
                    num(area[1]),
                    num(area[2] - area[0]),
                    num(area[3] - area[1]),
                    hex(&p["color"]),
                    alpha_attr("fill-opacity", a)
                );
            }
        }
        self
    }

    /// A ` filter="..."` attribute for an image paint's adjustments, set on the painted shape
    /// (inside a pattern resvg filters at the tile's own size), fitted to the plugin's
    /// renders: exposure scales linear light by 2^(5e), saturation is SVG `saturate` in linear RGB.
    // shortcut: contrast, temperature, tint, highlights and shadows are not drawn; fit them against renders when designs rely on them.
    fn image_filter(&mut self, f: &Json) -> String {
        let get = |k: &str| f[k].as_f64().filter(|v| *v != 0.0);
        let (exposure, saturation) = (get("exposure"), get("vibrance"));
        if exposure.is_none() && saturation.is_none() {
            return String::new();
        }
        let id = self.id("a");
        let mut body = String::new();
        if let Some(e) = exposure {
            let slope = num(2f64.powf(5.0 * e));
            let _ = write!(
                body,
                "<feComponentTransfer><feFuncR type=\"linear\" slope=\"{slope}\"/><feFuncG type=\"linear\" slope=\"{slope}\"/><feFuncB type=\"linear\" slope=\"{slope}\"/></feComponentTransfer>"
            );
        }
        if let Some(s) = saturation {
            let _ = write!(
                body,
                "<feColorMatrix type=\"saturate\" values=\"{}\"/>",
                num((1.0 + s).max(0.0))
            );
        }
        let _ = write!(
            self.defs,
            "<filter id=\"{id}\" color-interpolation-filters=\"linearRGB\">{body}</filter>"
        );
        format!(" filter=\"url(#{id})\"")
    }

    /// The image halved (2x2 averages) while it stays at least twice the pixels it covers:
    /// resvg samples a large downscale without averaging, which aliases; Figma's does not.
    // shortcut: only PNGs are downscaled; decode JPEGs too if photos alias.
    fn mip(&mut self, hash: &str, bytes: Vec<u8>, covered: (f64, f64)) -> Vec<u8> {
        if self.mips.is_none() {
            return bytes;
        }
        let Ok(mut img) = resvg::tiny_skia::Pixmap::decode_png(&bytes) else {
            return bytes;
        };
        let k = (f64::from(img.width()) / covered.0).min(f64::from(img.height()) / covered.1);
        let levels = if k >= 2.0 { k.log2().floor() as u32 } else { 0 };
        if levels == 0 {
            return bytes;
        }
        let key = (hash.to_owned(), levels);
        if let Some(b) = self.mips.as_ref().and_then(|m| m.get(&key)) {
            return b.clone();
        }
        for _ in 0..levels {
            img = halve(&img);
        }
        let out = img.encode_png().unwrap_or(bytes);
        if let Some(m) = self.mips.as_mut() {
            m.insert(key, out.clone());
        }
        out
    }

    fn id(&mut self, prefix: &str) -> String {
        self.ids += 1;
        format!("{prefix}{}", self.ids)
    }

    /// A standalone SVG document of `n` and its children over the page region `area`.
    pub fn document(mut self, n: &Json, area: Rect, scale: f64) -> String {
        self.scale = scale;
        let body = self.node(n);
        // Exports are sized to the scaled box, rounded up, as Figma sizes them.
        // Float noise (26.0000001) must not round up a whole pixel.
        let px = |len: f64| (len * scale - 1e-3).ceil().max(1.0);
        let (w, h) = (px(area[2] - area[0]) / scale, px(area[3] - area[1]) / scale);
        // The origin moves in a transform, not the viewBox: resvg draws filters wrong under a
        // fractional viewBox origin (drop shadows came out a fifth as strong).
        format!(
            "<svg width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">\n<defs>{}</defs>\n<g transform=\"translate({} {})\">{}</g></svg>\n",
            num(w * scale),
            num(h * scale),
            num(w),
            num(h),
            self.defs,
            num(-area[0]),
            num(-area[1]),
            self.backdrop + &body
        )
    }

    fn node(&mut self, n: &Json) -> String {
        if n["visible"] == Json::Bool(false) || n["_r"].is_null() {
            return String::new();
        }
        let mut inner = String::new();
        let r = &n["_r"];
        let fills = match self.region_fills(n) {
            Some(f) => f,
            None => self.fills(n, &r["fillPaints"], fill_shapes(self.tree, n)),
        };
        inner += &fills;
        // A boolean operation's children are its operands, already merged into its geometry.
        if n["type"] != "BOOLEAN_OPERATION" {
            let mut kids = String::new();
            let mut masked: Option<String> = None;
            for c in n["children"].as_array().into_iter().flatten() {
                if c["isMask"] == Json::Bool(true) && c["visible"] != Json::Bool(false) {
                    if let Some(m) = masked.take() {
                        kids += &m;
                        kids += "</g>";
                    }
                    let id = self.id("m");
                    let alpha = c["maskType"] != "LUMINANCE";
                    let body = self.node(c);
                    let _ = write!(
                        self.defs,
                        "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"-1e6\" y=\"-1e6\" width=\"2e6\" height=\"2e6\" style=\"mask-type:{}\">{body}</mask>",
                        if alpha { "alpha" } else { "luminance" }
                    );
                    masked = Some(format!("<g mask=\"url(#{id})\">"));
                    continue;
                }
                let body = self.node(c);
                match masked.as_mut() {
                    Some(m) => m.push_str(&body),
                    None => kids += &body,
                }
            }
            if let Some(m) = masked {
                kids += &m;
                kids += "</g>";
            }
            if n["clipsContent"] == Json::Bool(true) && !kids.is_empty() {
                let id = self.id("c");
                let s = size_of(r);
                let d = path_d(&rect_segs(0.0, 0.0, s.x, s.y, radii(n)), mat(&r["m"]));
                let _ = write!(
                    self.defs,
                    "<clipPath id=\"{id}\"><path d=\"{d}\"/></clipPath>"
                );
                let _ = write!(inner, "<g clip-path=\"url(#{id})\">{kids}</g>");
            } else {
                inner += &kids;
            }
        }
        inner += &self.strokes(n);
        if inner.is_empty() {
            return inner;
        }
        let mut attrs = String::new();
        if let Some(o) = n["opacity"].as_f64() {
            let _ = write!(attrs, " opacity=\"{}\"", num(o));
        }
        // Only the known modes: the enum's names come from the file's own schema, so an
        // arbitrary one could break out of the attribute.
        if let Some(b) = n["blendMode"].as_str().filter(|b| {
            matches!(
                *b,
                "DARKEN"
                    | "MULTIPLY"
                    | "LINEAR_BURN"
                    | "COLOR_BURN"
                    | "LIGHTEN"
                    | "SCREEN"
                    | "LINEAR_DODGE"
                    | "COLOR_DODGE"
                    | "OVERLAY"
                    | "SOFT_LIGHT"
                    | "HARD_LIGHT"
                    | "DIFFERENCE"
                    | "EXCLUSION"
                    | "HUE"
                    | "SATURATION"
                    | "COLOR"
                    | "LUMINOSITY"
            )
        }) {
            let _ = write!(
                attrs,
                " style=\"mix-blend-mode:{}\"",
                b.to_lowercase().replace('_', "-")
            );
        }
        if let Some(f) = self.effects(n) {
            let _ = write!(attrs, " filter=\"url(#{f})\"");
        }
        format!(
            "<g id=\"{}\"{attrs}>{inner}</g>\n",
            xml(n["id"].as_str().unwrap_or(""))
        )
    }

    /// Vector regions with their own fills (`vectorData.styleOverrideTable`), each path painted
    /// with its region's paints; `None` when the node has a single fill.
    fn region_fills(&mut self, n: &Json) -> Option<String> {
        let r = &n["_r"];
        let table = r["vectorStyles"].as_array()?;
        let paints_of = |id: u64| {
            table
                .iter()
                .find(|s| s["styleID"].as_u64() == Some(id) && s["fillPaints"].is_array())
        };
        let list = r["fillGeometry"].as_array()?;
        if !list
            .iter()
            .any(|g| paints_of(g["styleID"].as_u64().unwrap_or(0)).is_some())
        {
            return None;
        }
        let m = mat(&r["m"]) * geometry_scale(r, "fillGeometrySize");
        let mut out = String::new();
        for g in list {
            let shapes: Vec<_> = geometry(self.tree, &Json::Array(vec![g.clone()]))
                .into_iter()
                .map(|(segs, odd)| (segs, odd, m))
                .collect();
            let style = g["styleID"].as_u64().unwrap_or(0);
            let paints = paints_of(style)
                .map_or(&r["fillPaints"], |s| &s["fillPaints"])
                .clone();
            out += &self.fills(n, &paints, shapes);
        }
        Some(out)
    }

    /// Paints `list` (bottom first) over `shapes`.
    fn fills(&mut self, n: &Json, list: &Json, shapes: Vec<(Vec<Seg>, bool, Mat)>) -> String {
        let mut out = String::new();
        if shapes.is_empty() {
            return out;
        }
        let r = &n["_r"];
        let m = mat(&r["m"]);
        let size = size_of(r);
        // Text ranges can override the fill per glyph.
        let per_glyph = n["type"] == "TEXT" && r["styleOverrideTable"].is_array();
        if per_glyph {
            let ids = &r["characterStyleIDs"];
            for (segs, gm, ch) in glyphs(self.tree, r) {
                let style = ids[ch].as_u64().unwrap_or(0);
                let over = r["styleOverrideTable"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["styleID"].as_u64() == Some(style) && s["fillPaints"].is_array());
                let paints = over.map_or(list, |s| &s["fillPaints"]);
                let d = path_d(&segs, m * gm);
                out += &self.paint_path(paints, &d, false, m, size);
            }
            return out;
        }
        let d: String = shapes.iter().map(|(s, _, sm)| path_d(s, *sm)).collect();
        let odd = shapes.iter().any(|(_, odd, _)| *odd);
        self.paint_path(list, &d, odd, m, size)
    }

    fn paint_path(&mut self, list: &Json, d: &str, odd: bool, m: Mat, size: Vec2) -> String {
        let mut out = String::new();
        let rule = if odd { " fill-rule=\"evenodd\"" } else { "" };
        for p in list.as_array().into_iter().flatten().filter(|p| visible(p)) {
            let opacity = p["opacity"].as_f64().unwrap_or(1.0);
            match p["type"].as_str().unwrap_or("") {
                "SOLID" => {
                    let a = p["color"]["a"].as_f64().unwrap_or(1.0) * opacity;
                    let _ = write!(
                        out,
                        "<path d=\"{d}\"{rule} fill=\"{}\"{}/>",
                        hex(&p["color"]),
                        alpha_attr("fill-opacity", a)
                    );
                }
                "IMAGE" => {
                    let Some(bytes) = p["image"]["hash"].as_str().and_then(|h| (self.images)(h))
                    else {
                        continue;
                    };
                    // A pattern keeps the painted bounds to the shape: an <image> clipped to it keeps
                    // the whole image's bounds, and resvg then cuts it off under an effect filter.
                    let ((w, h), t, aspect) = match p["imageScaleMode"].as_str() {
                        // shortcut: STRETCH repeats past the image's edges where Figma leaves them empty; only matters for zoomed-out crops.
                        Some("STRETCH") => (
                            (1.0, 1.0),
                            m * Mat {
                                a: size.x,
                                b: 0.0,
                                c: 0.0,
                                d: size.y,
                                tx: 0.0,
                                ty: 0.0,
                            } * kiwi_mat(&p["transform"]).inverse(),
                            "none",
                        ),
                        // shortcut: TILE is drawn as FILL; tile with the image's natural size when a design needs it.
                        mode => (
                            (size.x, size.y),
                            m,
                            if mode == Some("FIT") {
                                "xMidYMid meet"
                            } else {
                                "xMidYMid slice"
                            },
                        ),
                    };
                    // Device pixels the image covers along each of its axes.
                    let len = |x: f64, y: f64| x.hypot(y) * self.scale;
                    let covered = match p["imageScaleMode"].as_str() {
                        Some("STRETCH") => Some((len(t.a, t.b), len(t.c, t.d))),
                        _ => image_size(&bytes).map(|(iw, ih)| {
                            let fit = (w / iw, h / ih);
                            let s = if aspect.ends_with("meet") {
                                fit.0.min(fit.1)
                            } else {
                                fit.0.max(fit.1)
                            };
                            (iw * s * len(t.a, t.b), ih * s * len(t.c, t.d))
                        }),
                    };
                    let hash = p["image"]["hash"].as_str().unwrap_or("").to_owned();
                    let bytes = match covered {
                        Some(c) => self.mip(&hash, bytes, c),
                        None => bytes,
                    };
                    // The pattern already clips its tile. Explicit cover geometry avoids usvg's
                    // extra slice clip, which resvg truncates to the outer canvas inside patterns.
                    let (iw, ih, x, y, aspect) = if aspect.ends_with("slice") {
                        match image_size(&bytes) {
                            Some((iw, ih)) => {
                                let s = (w / iw).max(h / ih);
                                (
                                    iw * s,
                                    ih * s,
                                    (w - iw * s) / 2.0,
                                    (h - ih * s) / 2.0,
                                    "none",
                                )
                            }
                            None => (w, h, 0.0, 0.0, aspect),
                        }
                    } else {
                        (w, h, 0.0, 0.0, aspect)
                    };
                    let adjust = self.image_filter(&p["paintFilter"]);
                    let pattern = self.id("p");
                    let _ = write!(
                        self.defs,
                        "<pattern id=\"{pattern}\" patternUnits=\"userSpaceOnUse\" width=\"{w}\" height=\"{h}\" patternTransform=\"{}\"><image x=\"{x}\" y=\"{y}\" width=\"{iw}\" height=\"{ih}\" preserveAspectRatio=\"{aspect}\" href=\"data:{};base64,{}\"/></pattern>",
                        matrix(t),
                        media_type(&bytes),
                        base64(&bytes),
                        w = num(w),
                        h = num(h),
                        x = num(x),
                        y = num(y),
                        iw = num(iw),
                        ih = num(ih),
                    );
                    let _ = write!(
                        out,
                        "<path d=\"{d}\"{rule} fill=\"url(#{pattern})\"{}{adjust}/>",
                        alpha_attr("fill-opacity", opacity)
                    );
                }
                kind if kind.starts_with("GRADIENT_") => {
                    let id = self.id("g");
                    let t =
                        m * Mat {
                            a: size.x,
                            b: 0.0,
                            c: 0.0,
                            d: size.y,
                            tx: 0.0,
                            ty: 0.0,
                        } * kiwi_mat(&p["transform"]).inverse();
                    let stops: String = p["stops"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|s| {
                            format!(
                                "<stop offset=\"{}\" stop-color=\"{}\"{}/>",
                                num(s["position"].as_f64().unwrap_or(0.0)),
                                hex(&s["color"]),
                                alpha_attr("stop-opacity", s["color"]["a"].as_f64().unwrap_or(1.0))
                            )
                        })
                        .collect();
                    // shortcut: angular and diamond gradients are drawn as radial ones; needs a mesh or shader to match exactly.
                    let (tag, geom) = if kind == "GRADIENT_LINEAR" {
                        ("linearGradient", "x1=\"0\" y1=\"0.5\" x2=\"1\" y2=\"0.5\"")
                    } else {
                        ("radialGradient", "cx=\"0.5\" cy=\"0.5\" r=\"0.5\"")
                    };
                    let _ = write!(
                        self.defs,
                        "<{tag} id=\"{id}\" gradientUnits=\"userSpaceOnUse\" {geom} gradientTransform=\"{}\">{stops}</{tag}>",
                        matrix(t)
                    );
                    let _ = write!(
                        out,
                        "<path d=\"{d}\"{rule} fill=\"url(#{id})\"{}/>",
                        alpha_attr("fill-opacity", opacity)
                    );
                }
                _ => {}
            }
        }
        out
    }

    fn strokes(&mut self, n: &Json) -> String {
        let r = &n["_r"];
        let weight = r["strokeWeight"].as_f64().unwrap_or(0.0);
        if weight <= 0.0 || !has_paint(&r["strokePaints"]) {
            return String::new();
        }
        let shapes = stroke_shapes(self.tree, n);
        if !shapes.is_empty() {
            let body = self.fills(&strip_text(n), &r["strokePaints"], shapes);
            let shape: String = fill_shapes(self.tree, n)
                .iter()
                .map(|(s, _, sm)| path_d(s, *sm))
                .collect();
            return match r["strokeAlign"].as_str() {
                Some("INSIDE") => {
                    let id = self.id("c");
                    let _ = write!(
                        self.defs,
                        "<clipPath id=\"{id}\"><path d=\"{shape}\"/></clipPath>"
                    );
                    format!("<g clip-path=\"url(#{id})\">{body}</g>")
                }
                Some("OUTSIDE") => {
                    let id = self.id("m");
                    let _ = write!(
                        self.defs,
                        "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"-1e6\" y=\"-1e6\" width=\"2e6\" height=\"2e6\"><rect x=\"-1e6\" y=\"-1e6\" width=\"2e6\" height=\"2e6\" fill=\"white\"/><path d=\"{shape}\" fill=\"black\"/></mask>"
                    );
                    format!("<g mask=\"url(#{id})\">{body}</g>")
                }
                _ => body,
            };
        }
        if n["type"] == "TEXT" {
            return String::new();
        }
        // Boxes without stroke geometry: stroke the box outline, moved by the alignment.
        let m = mat(&r["m"]);
        let s = size_of(r);
        let shift = match r["strokeAlign"].as_str() {
            Some("OUTSIDE") => weight / 2.0,
            Some("CENTER") => 0.0,
            _ => -weight / 2.0,
        };
        let rad = radii(n).map(|x| (x + shift).max(0.0));
        let segs = rect_segs(-shift, -shift, s.x + 2.0 * shift, s.y + 2.0 * shift, rad);
        let d = path_d(&segs, m);
        let scale = (m.a * m.d - m.b * m.c).abs().sqrt();
        let mut out = String::new();
        for p in r["strokePaints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| visible(p))
        {
            if p["type"] != "SOLID" {
                continue;
            }
            let a = p["color"]["a"].as_f64().unwrap_or(1.0) * p["opacity"].as_f64().unwrap_or(1.0);
            let dash = r["dashPattern"]
                .as_array()
                .filter(|d| !d.is_empty())
                .map(|d| {
                    let list: Vec<String> = d
                        .iter()
                        .map(|x| num(x.as_f64().unwrap_or(0.0) * scale))
                        .collect();
                    format!(" stroke-dasharray=\"{}\"", list.join(" "))
                })
                .unwrap_or_default();
            let _ = write!(
                out,
                "<path d=\"{d}\" stroke=\"{}\" stroke-width=\"{}\"{}{dash}/>",
                hex(&p["color"]),
                num(weight * scale),
                alpha_attr("stroke-opacity", a)
            );
        }
        out
    }

    /// A filter for the node's visible shadows and blurs; background blur is not supported.
    fn effects(&mut self, n: &Json) -> Option<String> {
        let list: Vec<&Json> = n["_r"]["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| visible(e) && e["type"] != "BACKGROUND_BLUR")
            .collect();
        if list.is_empty() {
            return None;
        }
        let id = self.id("f");
        let mut f = String::from(
            "<feColorMatrix in=\"SourceAlpha\" type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0\" result=\"hard\"/>",
        );
        let mut under = Vec::new();
        let mut over = Vec::new();
        let mut blur = None;
        for (i, e) in list.iter().enumerate() {
            let sd = num(e["radius"].as_f64().unwrap_or(0.0) / 2.0);
            let (dx, dy) = (
                num(e["offset"]["x"].as_f64().unwrap_or(0.0)),
                num(e["offset"]["y"].as_f64().unwrap_or(0.0)),
            );
            let spread = e["spread"].as_f64().unwrap_or(0.0);
            // Figma's own SVG export: shadows start from the node's alpha made hard (any
            // coverage counts as full), and are colored by a matrix.
            let c = &e["color"];
            let channel = |k: &str| num(c[k].as_f64().unwrap_or(0.0));
            let tint = format!(
                "<feColorMatrix type=\"matrix\" values=\"0 0 0 0 {} 0 0 0 0 {} 0 0 0 0 {} 0 0 0 {} 0\" result=\"e{i}\"/>",
                channel("r"),
                channel("g"),
                channel("b"),
                num(c["a"].as_f64().unwrap_or(1.0))
            );
            match e["type"].as_str() {
                Some("DROP_SHADOW") => {
                    let src = if spread > 0.0 {
                        let _ = write!(
                            f,
                            "<feMorphology in=\"SourceAlpha\" operator=\"dilate\" radius=\"{}\" result=\"s{i}\"/>",
                            num(spread)
                        );
                        format!("s{i}")
                    } else {
                        "hard".into()
                    };
                    let _ = write!(
                        f,
                        "<feOffset in=\"{src}\" dx=\"{dx}\" dy=\"{dy}\"/><feGaussianBlur stdDeviation=\"{sd}\"/>"
                    );
                    // By default the shadow is cut out wherever the node covers it, so it never
                    // shows through translucent fills.
                    if e["showShadowBehindNode"] != true {
                        f += "<feComposite in2=\"hard\" operator=\"out\"/>";
                    }
                    f += &tint;
                    under.push(format!("e{i}"));
                }
                Some("INNER_SHADOW") => {
                    let _ = write!(
                        f,
                        "<feOffset in=\"hard\" dx=\"{dx}\" dy=\"{dy}\"/><feGaussianBlur stdDeviation=\"{sd}\"/><feComposite in2=\"hard\" operator=\"arithmetic\" k2=\"-1\" k3=\"1\"/>{tint}"
                    );
                    over.push(format!("e{i}"));
                }
                _ => blur = Some(sd),
            }
        }
        let input = if self.mips.is_some() && under.is_empty() && over.is_empty() {
            // Skip pure-blur intermediates for rasterization, keeping SVG asset bytes stable.
            f.clear();
            "SourceGraphic"
        } else {
            let merge: String = under
                .iter()
                .map(String::as_str)
                .chain(["SourceGraphic"])
                .chain(over.iter().map(String::as_str))
                .map(|r| format!("<feMergeNode in=\"{r}\"/>"))
                .collect();
            let _ = write!(f, "<feMerge result=\"all\">{merge}</feMerge>");
            "all"
        };
        if let Some(sd) = blur {
            let _ = write!(f, "<feGaussianBlur in=\"{input}\" stdDeviation=\"{sd}\"/>");
        }
        // The filter canvas is what the node draws, uncut by its ancestors' clips, plus each
        // effect's reach: 3 sigma (sigma = radius / 2) of blur, spread and offset. Filter cost
        // grows with this area, so it must not also count the reach that render bounds hold.
        let reach = list
            .iter()
            .map(|e| {
                let o = &e["offset"];
                1.5 * e["radius"].as_f64().unwrap_or(0.0)
                    + e["spread"].as_f64().unwrap_or(0.0).abs()
                    + o["x"]
                        .as_f64()
                        .unwrap_or(0.0)
                        .abs()
                        .max(o["y"].as_f64().unwrap_or(0.0).abs())
            })
            .fold(1.0, f64::max);
        let [x0, y0, x1, y1] = rect_of(&n["_r"]["fb"])
            .or_else(|| box_bounds(n))
            .unwrap_or([0.0; 4]);
        let (x0, y0, x1, y1) = (x0 - reach, y0 - reach, x1 + reach, y1 + reach);
        let _ = write!(
            self.defs,
            "<filter id=\"{id}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" color-interpolation-filters=\"sRGB\">{f}</filter>",
            num(x0),
            num(y0),
            num(x1 - x0),
            num(y1 - y0)
        );
        Some(id)
    }
}

/// A copy of `n` that is not text, so stroke outlines are painted as plain shapes.
fn strip_text(n: &Json) -> Json {
    let mut c = n.clone();
    c["type"] = Json::from("VECTOR");
    c
}

fn matrix(m: Mat) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        num(m.a),
        num(m.b),
        num(m.c),
        num(m.d),
        num(m.tx),
        num(m.ty)
    )
}

fn alpha_attr(name: &str, a: f64) -> String {
    if a < 1.0 {
        format!(" {name}=\"{}\"", num(a))
    } else {
        String::new()
    }
}

fn hex(c: &Json) -> String {
    let ch = |k: &str| (c[k].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", ch("r"), ch("g"), ch("b"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Image type from its signature, as the bundle names files.
pub fn media_type(b: &[u8]) -> &'static str {
    match b {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "image/webp",
        _ => "application/octet-stream",
    }
}

pub fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Rasterizes an SVG document to PNG bytes.
pub fn png(svg: &str) -> Result<Vec<u8>, String> {
    pixmap(svg)?.encode_png().map_err(|e| e.to_string())
}

/// Rasterizes an SVG document to premultiplied RGBA pixels.
pub fn pixmap(svg: &str) -> Result<resvg::tiny_skia::Pixmap, String> {
    // Images are embedded as data: URLs; never let an href read a file from disk.
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("empty image")?;
    let s = |v: u32, f: f32| v as f32 / f;
    let transform = resvg::tiny_skia::Transform::from_scale(
        s(size.width(), tree.size().width()),
        s(size.height(), tree.size().height()),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Render bounds from a REST node (`absoluteRenderBounds`).
pub fn render_bounds(n: &Json) -> Option<Rect> {
    rect_of(&n["absoluteRenderBounds"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn diagnostics_preserve_drawn_stroke_warnings_and_other_sources() {
        use kiwi_schema::Value;
        let mut bytes = Vec::new();
        for (command, x, y) in [(1u8, 0f32, 0f32), (2, 16., 0.), (2, 16., 4.), (2, 0., 4.)] {
            bytes.push(command);
            bytes.extend(x.to_le_bytes());
            bytes.extend(y.to_le_bytes());
        }
        bytes.push(0);
        let blob = Value::Object(
            "Blob",
            std::collections::HashMap::from([("bytes", Value::Bytes(bytes))]),
        );
        let tree = Tree {
            nodes: Default::default(),
            children: Default::default(),
            blobs: vec![&blob],
        };
        for geometry in [false, true] {
            for weight in [0, 2] {
                let mut node = serde_json::json!({"id":"1:1","type":"RECTANGLE","_r":{
                    "fillPaints":[{"type":"GRADIENT_ANGULAR"}],
                    "strokeWeight":weight,
                    "strokePaints":[
                        {"type":"SOLID"},
                        {"type":"IMAGE","imageScaleMode":"FILL","paintFilter":{"exposure":0.2}},
                        {"type":"GRADIENT_LINEAR"},
                        {"type":"GRADIENT_ANGULAR","visible":false},
                        {"type":"GRADIENT_DIAMOND","opacity":0},
                        {"type":"GRADIENT_ANGULAR"},
                        {"type":"IMAGE","imageScaleMode":"TILE","paintFilter":{"contrast":0.2}}
                    ],
                    "effects":[{"type":"BACKGROUND_BLUR","radius":4}]
                }});
                if geometry {
                    node["_r"]["strokeGeometry"] = serde_json::json!([{"commandsBlob":0}]);
                }
                let found = diagnostics(&tree, &node);
                let fields: Vec<_> = found.iter().map(|d| d["field"].as_str().unwrap()).collect();
                let mut expected = vec!["fillPaints[0].type"];
                if geometry && weight > 0 {
                    expected.extend([
                        "strokePaints[5].type",
                        "strokePaints[6].imageScaleMode",
                        "strokePaints[6].paintFilter.contrast",
                    ]);
                }
                expected.push("effects[0]");
                assert_eq!(fields, expected);
            }
        }
    }

    #[test]
    fn diagnostics_report_only_active_paint_fallbacks() {
        let tree = Tree {
            nodes: Default::default(),
            children: Default::default(),
            blobs: Vec::new(),
        };
        let mut node = serde_json::json!({"id":"1:1", "type":"RECTANGLE", "_r": {
            "fillPaints": [
                {"type":"IMAGE", "imageScaleMode":"TILE", "paintFilter":{"contrast":0.2,"exposure":0.2,"vibrance":0.2,"tint":0}},
                {"type":"GRADIENT_ANGULAR"},
                {"type":"GRADIENT_DIAMOND", "visible":false},
                {"type":"IMAGE", "opacity":0, "paintFilter":{"temperature":0.3}}
            ], "effects":[{"type":"BACKGROUND_BLUR","radius":4}, {"type":"BACKGROUND_BLUR","radius":0}]
        }});
        let found = diagnostics(&tree, &node);
        assert_eq!(found.len(), 4);
        assert_eq!(found[0]["state"], "approximate");
        assert_eq!(found[1]["field"], "fillPaints[0].paintFilter.contrast");
        assert_eq!(found[3]["feature"], "backgroundBlur");
        node["_r"]["fillGeometry"] = serde_json::json!([{"styleID":1}]);
        node["_r"]["vectorStyles"] = serde_json::json!([
            {"styleID":1,"fillPaints":[{"type":"SOLID"}]},
            {"styleID":2,"fillPaints":[{"type":"GRADIENT_DIAMOND"}]}
        ]);
        node["_r"]["effects"] = serde_json::json!([]);
        assert!(
            diagnostics(&tree, &node).is_empty(),
            "overridden and unused fills are not rendered"
        );
        node["type"] = serde_json::json!("TEXT");
        node["_r"]["characterStyleIDs"] = serde_json::json!([1, 1]);
        node["_r"]["glyphs"] = serde_json::json!([{"firstCharacter":0}, {"firstCharacter":1}]);
        node["_r"]["styleOverrideTable"] = node["_r"]["vectorStyles"].clone();
        assert!(diagnostics(&tree, &node).is_empty());
        node["_r"]["styleOverrideTable"][0]["fillPaints"][0]["type"] =
            serde_json::json!("GRADIENT_DIAMOND");
        assert_eq!(
            diagnostics(&tree, &node).len(),
            1,
            "one report per effective paint, not per glyph"
        );
        node["_r"]["truncationStartIndex"] = serde_json::json!(0);
        assert!(
            diagnostics(&tree, &node).is_empty(),
            "truncated glyph styles are unused"
        );
    }

    #[test]
    fn preparation_selects_an_expanded_sublayer_and_only_loads_its_images() {
        use kiwi_schema::Value;
        use std::io::Write;

        let obj = |fields: Vec<(&'static str, Value<'static>)>| {
            Value::Object("Test", fields.into_iter().collect())
        };
        let paint = |hash: &'static str| {
            obj(vec![
                ("type", Value::Enum("PaintType", "IMAGE")),
                ("image", obj(vec![("hash", Value::String(hash.into()))])),
            ])
        };
        let main = obj(vec![("type", Value::Enum("NodeType", "FRAME"))]);
        let instance = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            (
                "symbolData",
                obj(vec![(
                    "symbolID",
                    obj(vec![
                        ("sessionID", Value::UInt(2)),
                        ("localID", Value::UInt(1)),
                    ]),
                )]),
            ),
        ]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "RECTANGLE")),
            (
                "size",
                obj(vec![("x", Value::Float(10.0)), ("y", Value::Float(20.0))]),
            ),
            (
                "fillPaints",
                Value::Array(vec![paint("present"), paint("missing"), paint("present")]),
            ),
        ]);
        let sibling = obj(vec![
            ("type", Value::Enum("NodeType", "RECTANGLE")),
            ("fillPaints", Value::Array(vec![paint("unused")])),
        ]);
        let tree = Tree {
            nodes: HashMap::from([
                ("1:1".into(), &instance),
                ("2:1".into(), &main),
                ("2:2".into(), &child),
                ("2:3".into(), &sibling),
            ]),
            children: HashMap::from([("2:1".into(), vec!["2:2".into(), "2:3".into()])]),
            blobs: Vec::new(),
        };
        let dir = crate::test_support::TestDir::new();
        let path = dir.join("input.fig");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for hash in ["present", "unused"] {
            zip.start_file(
                format!("images/{hash}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(hash.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let prepared = Prepared::new(&tree, &path, "I1:1;2:2").unwrap();
        assert_eq!(prepared.node()["id"], "I1:1;2:2");
        assert!(prepared.node()["_r"].is_object());
        assert_eq!(
            prepared.images,
            HashMap::from([("present".into(), b"present".to_vec())])
        );
        let pixels = pixmap(&prepared.export_svg(&tree, 2.0).unwrap()).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (20, 40));
        assert_eq!(
            Prepared::new(&tree, &path, "I1:1;absent")
                .err()
                .unwrap()
                .to_string(),
            "node I1:1;absent not found in expanded instance 1:1; inspect 1:1 for available sublayer IDs"
        );
    }

    #[test]
    fn prepared_page_export_draws_backdrop_across_the_child_union() {
        use kiwi_schema::Value;
        let page = Value::Object(
            "Test",
            vec![
                ("backgroundEnabled", Value::Bool(true)),
                (
                    "backgroundColor",
                    Value::Object(
                        "Color",
                        vec![
                            ("r", Value::Float(1.0)),
                            ("g", Value::Float(0.0)),
                            ("b", Value::Float(0.0)),
                            ("a", Value::Float(1.0)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let tree = Tree {
            nodes: HashMap::from([("page".into(), &page)]),
            children: Default::default(),
            blobs: Vec::new(),
        };
        let prepared = Prepared {
            document: serde_json::json!({ "document": {
                "id": "page", "type": "CANVAS", "children": [{
                    "absoluteRenderBounds": { "x": 5, "y": 7, "width": 10, "height": 20 }
                }]
            }}),
            images: Default::default(),
        };
        let pixels = pixmap(&prepared.export_svg(&tree, 1.0).unwrap()).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (10, 20));
        assert!(pixels.data().chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn filter_covers_the_unclipped_content_plus_three_sigma() {
        let tree = Tree {
            nodes: Default::default(),
            children: Default::default(),
            blobs: Vec::new(),
        };
        let images = |_: &str| None;
        let mut svg = Svg::new(&tree, &images);
        // An ancestor clipped the render bounds to the right half; the blur still needs the left.
        let n = serde_json::json!({
            "absoluteRenderBounds": { "x": 50, "y": 0, "width": 50, "height": 50 },
            "_r": {
                "fb": { "x": 0, "y": 0, "width": 100, "height": 50 },
                "effects": [{ "type": "LAYER_BLUR", "radius": 20, "visible": true }],
            },
        });
        svg.effects(&n);
        assert!(
            svg.defs
                .contains("x=\"-30\" y=\"-30\" width=\"160\" height=\"110\""),
            "{}",
            svg.defs
        );
    }

    #[test]
    fn pure_blur_skips_shadow_intermediates_without_changing_pixels() {
        let tree = Tree {
            nodes: Default::default(),
            children: Default::default(),
            blobs: Vec::new(),
        };
        let images = |_: &str| None;
        for kind in ["LAYER_BLUR", "FOREGROUND_BLUR"] {
            for radius in [0.0, 1.0, 19.5] {
                let mut svg = Svg::new(&tree, &images).for_raster();
                let mut n = serde_json::json!({ "_r": {
                    "fb": { "x": 10.25, "y": 10.75, "width": 30, "height": 20 },
                    "effects": [{ "type": kind, "radius": radius, "visible": true }]
                }});
                let id = svg.effects(&n).unwrap();
                assert!(!svg.defs.contains("feColorMatrix"));
                assert!(!svg.defs.contains("feMerge"));
                assert!(svg.defs.contains("in=\"SourceGraphic\""));
                let mut vector = Svg::new(&tree, &images);
                vector.effects(&n);
                assert!(vector.defs.contains("result=\"hard\""));
                assert!(vector.defs.contains("<feMerge result=\"all\">"));
                assert!(vector.defs.contains("<feGaussianBlur in=\"all\""));
                for scale in [1, 2] {
                    let document = |defs: &str| {
                        format!(
                            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 60 50\"><defs>{defs}</defs><g filter=\"url(#{id})\"><rect x=\"10.25\" y=\"10.75\" width=\"30\" height=\"20\" fill=\"#a54cfe\" opacity=\"0.53\"/></g></svg>",
                            60 * scale,
                            50 * scale
                        )
                    };
                    let pixels = pixmap(&document(&svg.defs)).unwrap();
                    assert!(pixels.data().iter().any(|b| *b != 0));
                    assert_eq!(
                        pixels.data(),
                        pixmap(&document(&vector.defs)).unwrap().data()
                    );
                }
                for shadow in ["DROP_SHADOW", "INNER_SHADOW"] {
                    n["_r"]["effects"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({ "type": shadow, "radius": 4, "visible": true }));
                    let mut mixed = Svg::new(&tree, &images).for_raster();
                    mixed.effects(&n);
                    assert!(mixed.defs.contains("result=\"hard\""));
                    assert!(mixed.defs.contains("<feMerge result=\"all\">"));
                    assert!(mixed.defs.contains("<feGaussianBlur in=\"all\""));
                    n["_r"]["effects"].as_array_mut().unwrap().pop();
                }
            }
        }
    }

    #[test]
    fn clipped_sprite_exports_keep_the_last_image_slice() {
        let tree = Tree {
            nodes: Default::default(),
            children: Default::default(),
            blobs: Vec::new(),
        };
        let mut sprite = resvg::tiny_skia::Pixmap::new(300, 100).unwrap();
        let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        for (i, pixel) in sprite.data_mut().chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&colors[(i % 300) / 100]);
        }
        for format in [
            crate::export::Format::Png,
            crate::export::Format::Jpg,
            crate::export::Format::Webp,
        ] {
            let bytes = crate::export::encode(&sprite, format, 95, None)
                .unwrap()
                .bytes;
            let images = |_: &str| Some(bytes.clone());
            for raster in [false, true] {
                for scale in [1.0, 3.0] {
                    for (index, offset) in [4.48, 70.56, 136.64].into_iter().enumerate() {
                        let frame = serde_json::json!({
                            "id": "frame", "type": "FRAME", "clipsContent": true,
                            "_r": { "size": { "x": 56, "y": 56 }, "m": [1,0,0,1,0,0] },
                            "children": [{ "id": "image", "type": "RECTANGLE", "_r": {
                                "size": { "x": 196, "y": 64.96 }, "m": [1,0,0,1,-offset,-4.48],
                                "fillPaints": [{ "type": "IMAGE", "imageScaleMode": "FILL",
                                    "image": { "hash": "sprite" } }]
                            }}]
                        });
                        let mut svg = Svg::new(&tree, &images);
                        if raster {
                            svg = svg.for_raster();
                        }
                        let pixels =
                            pixmap(&svg.document(&frame, [0.0, 0.0, 56.0, 56.0], scale)).unwrap();
                        let pixel = pixels
                            .pixel((50.0 * scale) as u32, (28.0 * scale) as u32)
                            .unwrap();
                        let expected = colors[index];
                        assert!(
                            pixel.alpha() > 250
                                && pixel.red().abs_diff(expected[0]) < 5
                                && pixel.green().abs_diff(expected[1]) < 5
                                && pixel.blue().abs_diff(expected[2]) < 5,
                            "sprite slice {index}, scale {scale}, raster {raster}, format {format:?}: {pixel:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn halving_averages_each_2x2_block() {
        let mut p = resvg::tiny_skia::Pixmap::new(2, 2).unwrap();
        for (i, px) in p.data_mut().chunks_mut(4).enumerate() {
            px.copy_from_slice(&[if i == 0 { 200 } else { 0 }, 0, 0, 255]);
        }
        assert_eq!(halve(&p).data(), &[50, 0, 0, 255]);
    }

    #[test]
    fn sections_export_with_a_margin_and_pages_as_their_children() {
        let bounds = |x: f64| serde_json::json!({ "x": x, "y": 0, "width": 10, "height": 10 });
        let section = serde_json::json!({ "type": "SECTION", "absoluteRenderBounds": bounds(0.0) });
        let page = serde_json::json!({ "type": "CANVAS", "children": [
            { "absoluteRenderBounds": bounds(0.0) }, { "absoluteRenderBounds": bounds(30.0) } ] });
        assert_eq!(
            (export_area(&section), export_area(&page)),
            (
                Some([-40.0, -40.0, 50.0, 50.0]),
                Some([0.0, 0.0, 40.0, 10.0])
            )
        );
    }

    #[test]
    fn decodes_move_cubic_and_close_commands() {
        let mut blob = vec![1u8];
        for v in [1.0f32, 2.0] {
            blob.extend(v.to_le_bytes());
        }
        blob.push(4);
        for v in [3.0f32, 4.0, 5.0, 6.0, 7.0, 8.0] {
            blob.extend(v.to_le_bytes());
        }
        blob.push(0);
        let v = |x, y| Vec2 { x, y };
        assert_eq!(
            commands(&blob),
            [
                Seg::M(v(1.0, 2.0)),
                Seg::C(v(3.0, 4.0), v(5.0, 6.0), v(7.0, 8.0)),
                Seg::Z
            ]
        );
    }

    #[test]
    fn base64_pads_like_the_standard_alphabet() {
        assert_eq!(
            (base64(b"Man"), base64(b"Ma"), base64(b"M")),
            ("TWFu".into(), "TWE=".into(), "TQ==".into())
        );
    }
}
