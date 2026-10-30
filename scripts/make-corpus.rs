//! Original synthetic regression corpus for CI; not a Figma export or an accuracy oracle.
//! Each top-level frame isolates shapes the public first-success fixture does not cover.
use kiwi_schema::{
    Def, DefKind, Field, Schema, TYPE_BOOL, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value,
};
use std::{error::Error, io::Write};

type V = Value<'static>;

fn obj(name: &'static str, fields: Vec<(&'static str, V)>) -> V {
    Value::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> V {
    obj(
        "GUID",
        vec![("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))],
    )
}

fn vector(x: f32, y: f32) -> V {
    obj(
        "Vector",
        vec![("x", Value::Float(x)), ("y", Value::Float(y))],
    )
}

fn color(rgb: [u8; 3], a: f32) -> V {
    obj(
        "Color",
        vec![
            ("r", Value::Float(f32::from(rgb[0]) / 255.0)),
            ("g", Value::Float(f32::from(rgb[1]) / 255.0)),
            ("b", Value::Float(f32::from(rgb[2]) / 255.0)),
            ("a", Value::Float(a)),
        ],
    )
}

fn transform(degrees: f32, x: f32, y: f32) -> V {
    let (sin, cos) = degrees.to_radians().sin_cos();
    obj(
        "Transform",
        vec![
            ("m00", Value::Float(cos)),
            ("m01", Value::Float(-sin)),
            ("m02", Value::Float(x)),
            ("m10", Value::Float(sin)),
            ("m11", Value::Float(cos)),
            ("m12", Value::Float(y)),
        ],
    )
}

fn paint(kind: &'static str, extra: Vec<(&'static str, V)>) -> V {
    let mut fields = vec![
        ("type", Value::Enum("PaintType", kind)),
        ("visible", Value::Bool(true)),
        ("opacity", Value::Float(1.0)),
        ("blendMode", Value::Enum("BlendMode", "NORMAL")),
    ];
    fields.extend(extra);
    obj("Paint", fields)
}

fn solid(rgb: [u8; 3]) -> V {
    Value::Array(vec![paint("SOLID", vec![("color", color(rgb, 1.0))])])
}

fn node(
    id: u32,
    parent: u32,
    kind: &'static str,
    name: &str,
    bounds: [f32; 4],
    extra: Vec<(&'static str, V)>,
) -> V {
    let [x, y, w, h] = bounds;
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", Value::Enum("NodeType", kind)),
        ("name", Value::String(name.into())),
        (
            "parentIndex",
            obj(
                "ParentIndex",
                vec![
                    ("guid", guid(parent)),
                    ("position", Value::String(format!("{id:03}"))),
                ],
            ),
        ),
        ("size", vector(w, h)),
        ("transform", transform(0.0, x, y)),
    ];
    fields.extend(extra);
    obj("NodeChange", fields)
}

fn nodes() -> Vec<V> {
    let e = |name: &'static str, value: &'static str| Value::Enum(name, value);
    vec![
        obj(
            "NodeChange",
            vec![
                ("guid", guid(0)),
                ("type", e("NodeType", "DOCUMENT")),
                ("name", Value::String("Corpus".into())),
            ],
        ),
        node(9, 0, "CANVAS", "Regression corpus", [0.0; 4], vec![]),
        // Auto layout: layoutMode, spacing, padding, child sizing and absolute positioning.
        node(
            1,
            9,
            "FRAME",
            "Auto layout",
            [0.0, 0.0, 240.0, 80.0],
            vec![
                ("fillPaints", solid([240, 244, 248])),
                ("stackMode", e("StackMode", "HORIZONTAL")),
                ("stackSpacing", Value::Float(8.0)),
                ("stackHorizontalPadding", Value::Float(12.0)),
                ("stackVerticalPadding", Value::Float(12.0)),
                ("stackPaddingRight", Value::Float(12.0)),
                ("stackPaddingBottom", Value::Float(12.0)),
                ("stackPrimarySizing", e("StackSize", "FIXED")),
                ("stackCounterSizing", e("StackSize", "FIXED")),
                ("stackCounterAlignItems", e("StackAlign", "CENTER")),
            ],
        ),
        node(
            11,
            1,
            "RECTANGLE",
            "Fixed",
            [12.0, 20.0, 40.0, 40.0],
            vec![("fillPaints", solid([27, 153, 139]))],
        ),
        node(
            12,
            1,
            "RECTANGLE",
            "Grow",
            [60.0, 12.0, 168.0, 56.0],
            vec![
                ("fillPaints", solid([80, 100, 220])),
                ("stackChildPrimaryGrow", Value::Float(1.0)),
                ("stackChildAlignSelf", e("StackAlign", "STRETCH")),
            ],
        ),
        node(
            13,
            1,
            "RECTANGLE",
            "Absolute badge",
            [224.0, 4.0, 12.0, 12.0],
            vec![
                ("fillPaints", solid([220, 60, 60])),
                ("stackPositioning", e("StackPositioning", "ABSOLUTE")),
            ],
        ),
        // Paints and strokes: linear gradient, inside stroke, mixed corner radii, translucent fill.
        node(
            2,
            9,
            "FRAME",
            "Paints and strokes",
            [0.0, 100.0, 240.0, 120.0],
            vec![("fillPaints", solid([255, 255, 255]))],
        ),
        node(
            21,
            2,
            "RECTANGLE",
            "Gradient card",
            [16.0, 16.0, 120.0, 88.0],
            vec![
                (
                    "fillPaints",
                    Value::Array(vec![paint(
                        "GRADIENT_LINEAR",
                        vec![
                            (
                                "stops",
                                Value::Array(vec![
                                    obj(
                                        "ColorStop",
                                        vec![
                                            ("color", color([255, 170, 0], 1.0)),
                                            ("position", Value::Float(0.0)),
                                        ],
                                    ),
                                    obj(
                                        "ColorStop",
                                        vec![
                                            ("color", color([200, 0, 120], 1.0)),
                                            ("position", Value::Float(1.0)),
                                        ],
                                    ),
                                ]),
                            ),
                            ("transform", transform(0.0, 0.0, 0.0)),
                        ],
                    )]),
                ),
                ("strokePaints", solid([27, 39, 60])),
                ("strokeWeight", Value::Float(4.0)),
                ("strokeAlign", e("StrokeAlign", "INSIDE")),
                ("rectangleCornerRadiiIndependent", Value::Bool(true)),
                ("rectangleTopLeftCornerRadius", Value::Float(0.0)),
                ("rectangleTopRightCornerRadius", Value::Float(24.0)),
                ("rectangleBottomRightCornerRadius", Value::Float(0.0)),
                ("rectangleBottomLeftCornerRadius", Value::Float(24.0)),
            ],
        ),
        node(
            22,
            2,
            "RECTANGLE",
            "Translucent",
            [152.0, 16.0, 72.0, 88.0],
            vec![
                (
                    "fillPaints",
                    Value::Array(vec![paint(
                        "SOLID",
                        vec![
                            ("color", color([27, 153, 139], 1.0)),
                            ("opacity", Value::Float(0.5)),
                        ],
                    )]),
                ),
                ("cornerRadius", Value::Float(8.0)),
            ],
        ),
        // Effects and clipping: drop shadow, rotation, layer opacity and a clipped overflow.
        node(
            3,
            9,
            "FRAME",
            "Effects and clipping",
            [0.0, 240.0, 240.0, 120.0],
            vec![
                ("fillPaints", solid([240, 244, 248])),
                ("frameMaskDisabled", Value::Bool(false)),
            ],
        ),
        node(
            31,
            3,
            "RECTANGLE",
            "Shadowed",
            [16.0, 16.0, 64.0, 64.0],
            vec![
                ("fillPaints", solid([255, 255, 255])),
                ("cornerRadius", Value::Float(8.0)),
                (
                    "effects",
                    Value::Array(vec![obj(
                        "Effect",
                        vec![
                            ("type", e("EffectType", "DROP_SHADOW")),
                            ("color", color([0, 0, 0], 0.35)),
                            ("offset", vector(0.0, 6.0)),
                            ("radius", Value::Float(8.0)),
                            ("spread", Value::Float(0.0)),
                            ("visible", Value::Bool(true)),
                            ("blendMode", e("BlendMode", "NORMAL")),
                        ],
                    )]),
                ),
            ],
        ),
        // Later fields replace the placeholder size and transform: 48x48, rotated 30 degrees.
        node(
            32,
            3,
            "RECTANGLE",
            "Rotated",
            [0.0; 4],
            vec![
                ("fillPaints", solid([80, 100, 220])),
                ("opacity", Value::Float(0.6)),
                ("size", vector(48.0, 48.0)),
                ("transform", transform(30.0, 128.0, 24.0)),
            ],
        ),
        node(
            33,
            3,
            "RECTANGLE",
            "Overflow",
            [200.0, 72.0, 80.0, 80.0],
            vec![("fillPaints", solid([220, 60, 60]))],
        ),
        // Components: two instances expand the same component's children.
        node(
            4,
            9,
            "FRAME",
            "Components",
            [0.0, 380.0, 240.0, 80.0],
            vec![("fillPaints", solid([255, 255, 255]))],
        ),
        node(
            41,
            4,
            "INSTANCE",
            "First chip",
            [16.0, 20.0, 96.0, 40.0],
            vec![
                ("fillPaints", solid([27, 39, 60])),
                ("cornerRadius", Value::Float(20.0)),
                (
                    "symbolData",
                    obj("SymbolData", vec![("symbolID", guid(50))]),
                ),
            ],
        ),
        node(
            42,
            4,
            "INSTANCE",
            "Second chip",
            [128.0, 20.0, 96.0, 40.0],
            vec![
                ("fillPaints", solid([27, 39, 60])),
                ("cornerRadius", Value::Float(20.0)),
                (
                    "symbolData",
                    obj("SymbolData", vec![("symbolID", guid(50))]),
                ),
            ],
        ),
        node(
            50,
            9,
            "SYMBOL",
            "Chip",
            [300.0, 380.0, 96.0, 40.0],
            vec![
                ("fillPaints", solid([27, 39, 60])),
                ("cornerRadius", Value::Float(20.0)),
            ],
        ),
        node(
            51,
            50,
            "RECTANGLE",
            "Dot",
            [12.0, 12.0, 16.0, 16.0],
            vec![
                ("fillPaints", solid([27, 153, 139])),
                ("cornerRadius", Value::Float(8.0)),
            ],
        ),
    ]
}

/// A later revision for `figlens diff` tests: a renamed frame, a changed fill, a removed and
/// an added layer. The default corpus stays byte-identical.
fn revise(nodes: &mut Vec<V>) {
    let id = |n: &V| match n.get("guid").and_then(|g| g.get("localID")) {
        Some(Value::UInt(id)) => *id,
        _ => u32::MAX,
    };
    nodes.retain(|n| id(n) != 13);
    for n in nodes.iter_mut() {
        let (key, value) = match id(n) {
            1 => ("name", Value::String("Auto layout v2".into())),
            11 => ("fillPaints", solid([200, 0, 0])),
            _ => continue,
        };
        if let Value::Object(_, fields) = n {
            fields.insert(key, value);
        }
    }
    nodes.push(node(
        14,
        1,
        "RECTANGLE",
        "New badge",
        [200.0, 4.0, 12.0, 12.0],
        vec![],
    ));
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("usage: corpus-fixture <new-output.fig> [--revised]")?;
    let revised = std::env::args().nth(2).as_deref() == Some("--revised");
    write(&output, revised)
}

/// Writes the corpus, or its `revise`d revision, to a new file at `output`.
pub fn write(output: &std::ffi::OsStr, revised: bool) -> Result<(), Box<dyn Error>> {
    write_with(output, if revised { revise } else { |_| {} })
}

/// Writes the corpus with only the Chip component's fill changed, for impact tests: both chip
/// instances, inside the Components frame, keep fills of their own.
#[allow(dead_code, reason = "only tests write it")]
pub fn write_restyled(output: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    write_with(output, |nodes| {
        for n in nodes.iter_mut() {
            let chip = matches!(
                n.get("guid").and_then(|g| g.get("localID")),
                Some(Value::UInt(50))
            );
            if let (true, Value::Object(_, fields)) = (chip, n) {
                fields.insert("fillPaints", solid([200, 0, 0]));
            }
        }
    })
}

/// Writes the corpus with only the Chip component's `Dot` child restyled, which neither chip
/// instance overrides: both reach the change through the component.
#[allow(dead_code, reason = "only tests write it")]
pub fn write_dot_restyled(output: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    write_with(output, restyle_dot)
}

/// `write_dot_restyled` with the second chip instance gone, so its link to the component
/// exists only in the old file.
#[allow(dead_code, reason = "only tests write it")]
pub fn write_dot_restyled_chip_removed(output: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    write_with(output, |nodes| {
        restyle_dot(nodes);
        nodes.retain(|n| local_id(n) != 42);
    })
}

/// `write_dot_restyled` with the second chip instance moved into the Effects frame, so it
/// is held by another top-level frame on each side.
#[allow(dead_code, reason = "only tests write it")]
pub fn write_dot_restyled_chip_moved(output: &std::ffi::OsStr) -> Result<(), Box<dyn Error>> {
    write_with(output, |nodes| {
        restyle_dot(nodes);
        for n in nodes.iter_mut().filter(|n| local_id(n) == 42) {
            if let Value::Object(_, fields) = n {
                fields.insert(
                    "parentIndex",
                    obj(
                        "ParentIndex",
                        vec![("guid", guid(3)), ("position", Value::String("042".into()))],
                    ),
                );
            }
        }
    })
}

fn local_id(n: &V) -> u32 {
    match n.get("guid").and_then(|g| g.get("localID")) {
        Some(Value::UInt(id)) => *id,
        _ => u32::MAX,
    }
}

fn restyle_dot(nodes: &mut Vec<V>) {
    for n in nodes.iter_mut().filter(|n| local_id(n) == 51) {
        if let Value::Object(_, fields) = n {
            fields.insert("fillPaints", solid([200, 0, 0]));
        }
    }
}

fn write_with(output: &std::ffi::OsStr, edit: fn(&mut Vec<V>)) -> Result<(), Box<dyn Error>> {
    // Definition order gives each user type its index.
    const DEFS: [&str; 20] = [
        "GUID",
        "NodeType",
        "Vector",
        "ParentIndex",
        "Transform",
        "Color",
        "PaintType",
        "BlendMode",
        "ColorStop",
        "Paint",
        "EffectType",
        "Effect",
        "StrokeAlign",
        "StackMode",
        "StackSize",
        "StackAlign",
        "StackPositioning",
        "SymbolData",
        "NodeChange",
        "Message",
    ];
    let ty = |name: &str| DEFS.iter().position(|d| *d == name).unwrap() as i32;
    let def = |name: &str, kind, fields: Vec<(&str, i32, bool)>| {
        let message = kind == DefKind::Message;
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, type_id, is_array))| Field {
                name: name.into(),
                type_id,
                value: if message { i as u32 + 1 } else { 0 },
                is_array,
            })
            .collect();
        Def::new(name.into(), kind, fields)
    };
    let enumeration = |name: &str, values: &[&str]| {
        let fields = values
            .iter()
            .enumerate()
            .map(|(i, value)| Field {
                name: (*value).into(),
                type_id: 0,
                value: i as u32,
                is_array: false,
            })
            .collect();
        Def::new(name.into(), DefKind::Enum, fields)
    };
    let floats = |names: &[&'static str]| names.iter().map(|n| (*n, TYPE_FLOAT, false)).collect();
    let defs = vec![
        def(
            "GUID",
            DefKind::Struct,
            vec![
                ("sessionID", TYPE_UINT, false),
                ("localID", TYPE_UINT, false),
            ],
        ),
        enumeration(
            "NodeType",
            &[
                "DOCUMENT",
                "CANVAS",
                "FRAME",
                "RECTANGLE",
                "SYMBOL",
                "INSTANCE",
            ],
        ),
        def("Vector", DefKind::Struct, floats(&["x", "y"])),
        def(
            "ParentIndex",
            DefKind::Struct,
            vec![
                ("guid", ty("GUID"), false),
                ("position", TYPE_STRING, false),
            ],
        ),
        def(
            "Transform",
            DefKind::Struct,
            floats(&["m00", "m01", "m02", "m10", "m11", "m12"]),
        ),
        def("Color", DefKind::Struct, floats(&["r", "g", "b", "a"])),
        enumeration("PaintType", &["SOLID", "GRADIENT_LINEAR"]),
        enumeration("BlendMode", &["NORMAL"]),
        def(
            "ColorStop",
            DefKind::Struct,
            vec![
                ("color", ty("Color"), false),
                ("position", TYPE_FLOAT, false),
            ],
        ),
        def(
            "Paint",
            DefKind::Message,
            vec![
                ("type", ty("PaintType"), false),
                ("color", ty("Color"), false),
                ("opacity", TYPE_FLOAT, false),
                ("visible", TYPE_BOOL, false),
                ("blendMode", ty("BlendMode"), false),
                ("stops", ty("ColorStop"), true),
                ("transform", ty("Transform"), false),
            ],
        ),
        enumeration("EffectType", &["DROP_SHADOW"]),
        def(
            "Effect",
            DefKind::Message,
            vec![
                ("type", ty("EffectType"), false),
                ("color", ty("Color"), false),
                ("offset", ty("Vector"), false),
                ("radius", TYPE_FLOAT, false),
                ("visible", TYPE_BOOL, false),
                ("blendMode", ty("BlendMode"), false),
                ("spread", TYPE_FLOAT, false),
            ],
        ),
        enumeration("StrokeAlign", &["CENTER", "INSIDE", "OUTSIDE"]),
        enumeration("StackMode", &["NONE", "HORIZONTAL", "VERTICAL"]),
        enumeration("StackSize", &["FIXED", "RESIZE_TO_FIT"]),
        enumeration("StackAlign", &["MIN", "CENTER", "MAX", "STRETCH"]),
        enumeration("StackPositioning", &["AUTO", "ABSOLUTE"]),
        def(
            "SymbolData",
            DefKind::Message,
            vec![("symbolID", ty("GUID"), false)],
        ),
        def(
            "NodeChange",
            DefKind::Message,
            vec![
                ("guid", ty("GUID"), false),
                ("type", ty("NodeType"), false),
                ("name", TYPE_STRING, false),
                ("parentIndex", ty("ParentIndex"), false),
                ("size", ty("Vector"), false),
                ("transform", ty("Transform"), false),
                ("fillPaints", ty("Paint"), true),
                ("strokePaints", ty("Paint"), true),
                ("strokeWeight", TYPE_FLOAT, false),
                ("strokeAlign", ty("StrokeAlign"), false),
                ("cornerRadius", TYPE_FLOAT, false),
                ("rectangleCornerRadiiIndependent", TYPE_BOOL, false),
                ("rectangleTopLeftCornerRadius", TYPE_FLOAT, false),
                ("rectangleTopRightCornerRadius", TYPE_FLOAT, false),
                ("rectangleBottomRightCornerRadius", TYPE_FLOAT, false),
                ("rectangleBottomLeftCornerRadius", TYPE_FLOAT, false),
                ("opacity", TYPE_FLOAT, false),
                ("effects", ty("Effect"), true),
                ("frameMaskDisabled", TYPE_BOOL, false),
                ("stackMode", ty("StackMode"), false),
                ("stackSpacing", TYPE_FLOAT, false),
                ("stackHorizontalPadding", TYPE_FLOAT, false),
                ("stackVerticalPadding", TYPE_FLOAT, false),
                ("stackPaddingRight", TYPE_FLOAT, false),
                ("stackPaddingBottom", TYPE_FLOAT, false),
                ("stackPrimarySizing", ty("StackSize"), false),
                ("stackCounterSizing", ty("StackSize"), false),
                ("stackCounterAlignItems", ty("StackAlign"), false),
                ("stackChildPrimaryGrow", TYPE_FLOAT, false),
                ("stackChildAlignSelf", ty("StackAlign"), false),
                ("stackPositioning", ty("StackPositioning"), false),
                ("symbolData", ty("SymbolData"), false),
            ],
        ),
        def(
            "Message",
            DefKind::Message,
            vec![("nodeChanges", ty("NodeChange"), true)],
        ),
    ];
    assert!(
        defs.iter().map(|d| d.name.as_str()).eq(DEFS),
        "DEFS must list the definitions in order"
    );
    let schema = Schema::new(defs);
    let mut nodes = nodes();
    edit(&mut nodes);
    let message = obj("Message", vec![("nodeChanges", Value::Array(nodes))]);
    let mut canvas = b"fig-kiwi".to_vec();
    canvas.extend(106u32.to_le_bytes());
    for bytes in [schema.encode(), message.encode(&schema)] {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes)?;
        let bytes = encoder.finish()?;
        canvas.extend(u32::try_from(bytes.len())?.to_le_bytes());
        canvas.extend(bytes);
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut zip = zip::ZipWriter::new(file);
    // A fixed timestamp keeps the file's SHA-256 reproducible.
    let options =
        zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
    zip.start_file("canvas.fig", options)?;
    zip.write_all(&canvas)?;
    zip.finish()?;
    Ok(())
}
