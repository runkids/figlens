//! Original MIT-licensed smoke fixture, using the same Kiwi encoder as command tests.
//! This writes one fixed synthetic example, not general Figma documents.
use kiwi_schema::{
    Def, DefKind, Field, Schema, TYPE_BYTE, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value,
};
use std::error::Error;
use std::io::Write;

fn obj(name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
    Value::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> Value<'static> {
    obj(
        "GUID",
        vec![("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))],
    )
}

fn point(x: f32, y: f32) -> Value<'static> {
    obj(
        "Point",
        vec![("x", Value::Float(x)), ("y", Value::Float(y))],
    )
}

fn solid(rgb: [u8; 3]) -> Value<'static> {
    obj(
        "Paint",
        vec![
            ("type", Value::Enum("PaintType", "SOLID")),
            (
                "color",
                obj(
                    "Color",
                    vec![
                        ("r", Value::Float(f32::from(rgb[0]) / 255.0)),
                        ("g", Value::Float(f32::from(rgb[1]) / 255.0)),
                        ("b", Value::Float(f32::from(rgb[2]) / 255.0)),
                        ("a", Value::Float(1.0)),
                    ],
                ),
            ),
        ],
    )
}

fn node(
    id: u32,
    parent: u32,
    kind: &'static str,
    name: &str,
    bounds: [f32; 4],
    extra: Vec<(&'static str, Value<'static>)>,
) -> Value<'static> {
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
        ("size", point(w, h)),
        (
            "transform",
            obj(
                "Transform",
                vec![
                    ("m00", Value::Float(1.0)),
                    ("m01", Value::Float(0.0)),
                    ("m02", Value::Float(x)),
                    ("m10", Value::Float(0.0)),
                    ("m11", Value::Float(1.0)),
                    ("m12", Value::Float(y)),
                ],
            ),
        ),
    ];
    fields.extend(extra);
    obj("NodeChange", fields)
}

fn path(points: &[(f32, f32)], bytes: &mut Vec<u8>) {
    for (i, (x, y)) in points.iter().enumerate() {
        bytes.push(if i == 0 { 1 } else { 2 });
        bytes.extend(x.to_le_bytes());
        bytes.extend(y.to_le_bytes());
    }
    bytes.push(0);
}

// Original block lettering: only the eight letters used by this fixed fixture.
fn letter(c: char) -> [&'static str; 7] {
    match c {
        'R' => [
            "11110", "10001", "10001", "11110", "10100", "10010", "10001",
        ],
        'E' => [
            "11111", "10000", "10000", "11110", "10000", "10000", "11111",
        ],
        'A' => [
            "01110", "10001", "10001", "11111", "10001", "10001", "10001",
        ],
        'D' => [
            "11110", "10001", "10001", "10001", "10001", "10001", "11110",
        ],
        'Y' => [
            "10001", "10001", "01010", "00100", "00100", "00100", "00100",
        ],
        'L' => [
            "10000", "10000", "10000", "10000", "10000", "10000", "11111",
        ],
        'O' => [
            "01110", "10001", "10001", "10001", "10001", "10001", "01110",
        ],
        'C' => [
            "01111", "10000", "10000", "10000", "10000", "10000", "01111",
        ],
        _ => unreachable!("fixed example lettering"),
    }
}

fn label(
    id: u32,
    text: &str,
    x: f32,
    y: f32,
    fs: f32,
    blobs: &mut Vec<Value<'static>>,
) -> Value<'static> {
    let glyphs = text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let mut bytes = Vec::new();
            for (row, cells) in letter(c).iter().enumerate() {
                for (col, cell) in cells.bytes().enumerate() {
                    if cell == b'1' {
                        let (x, y) = (col as f32 / 7.0, (6 - row) as f32 / 7.0);
                        path(
                            &[
                                (x, y),
                                (x + 1.0 / 7.0, y),
                                (x + 1.0 / 7.0, y + 1.0 / 7.0),
                                (x, y + 1.0 / 7.0),
                            ],
                            &mut bytes,
                        );
                    }
                }
            }
            let index = blobs.len() as u32;
            blobs.push(obj(
                "Blob",
                vec![(
                    "bytes",
                    Value::Array(bytes.into_iter().map(Value::Byte).collect()),
                )],
            ));
            obj(
                "Glyph",
                vec![
                    ("commandsBlob", Value::UInt(index)),
                    ("fontSize", Value::Float(fs)),
                    ("position", point(i as f32 * fs * 6.0 / 7.0, fs)),
                    ("firstCharacter", Value::UInt(i as u32)),
                ],
            )
        })
        .collect();
    node(
        id,
        2,
        "TEXT",
        text,
        [x, y, text.len() as f32 * fs * 6.0 / 7.0, fs],
        vec![
            (
                "textData",
                obj("TextData", vec![("characters", Value::String(text.into()))]),
            ),
            (
                "fontName",
                obj(
                    "FontName",
                    vec![
                        ("family", Value::String("Figlens Blocks".into())),
                        ("style", Value::String("Regular".into())),
                    ],
                ),
            ),
            ("fontSize", Value::Float(fs)),
            (
                "derivedTextData",
                obj("DerivedTextData", vec![("glyphs", Value::Array(glyphs))]),
            ),
            ("fillPaints", Value::Array(vec![solid([27, 39, 60])])),
        ],
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("usage: first-success-fixture <new-output.fig>")?;
    let field = |name: &str, type_id, value, is_array| Field {
        name: name.into(),
        type_id,
        value,
        is_array,
    };
    let def = |name: &str, kind, fields| Def::new(name.into(), kind, fields);
    let schema = Schema::new(vec![
        def(
            "GUID",
            DefKind::Struct,
            vec![
                field("sessionID", TYPE_UINT, 0, false),
                field("localID", TYPE_UINT, 0, false),
            ],
        ),
        def(
            "NodeType",
            DefKind::Enum,
            ["DOCUMENT", "CANVAS", "FRAME", "RECTANGLE", "TEXT", "VECTOR"]
                .iter()
                .enumerate()
                .map(|(i, n)| field(n, 0, i as u32, false))
                .collect(),
        ),
        def(
            "Point",
            DefKind::Struct,
            vec![
                field("x", TYPE_FLOAT, 0, false),
                field("y", TYPE_FLOAT, 0, false),
            ],
        ),
        def(
            "ParentIndex",
            DefKind::Struct,
            vec![
                field("guid", 0, 0, false),
                field("position", TYPE_STRING, 0, false),
            ],
        ),
        def(
            "Transform",
            DefKind::Struct,
            ["m00", "m01", "m02", "m10", "m11", "m12"]
                .iter()
                .map(|n| field(n, TYPE_FLOAT, 0, false))
                .collect(),
        ),
        def(
            "Color",
            DefKind::Struct,
            ["r", "g", "b", "a"]
                .iter()
                .map(|n| field(n, TYPE_FLOAT, 0, false))
                .collect(),
        ),
        def(
            "PaintType",
            DefKind::Enum,
            vec![field("SOLID", 0, 0, false), field("IMAGE", 0, 1, false)],
        ),
        def(
            "Image",
            DefKind::Struct,
            vec![field("hash", TYPE_STRING, 0, false)],
        ),
        def(
            "Paint",
            DefKind::Message,
            vec![
                field("type", 6, 1, false),
                field("color", 5, 2, false),
                field("image", 7, 3, false),
            ],
        ),
        def(
            "TextData",
            DefKind::Message,
            vec![field("characters", TYPE_STRING, 1, false)],
        ),
        def(
            "Glyph",
            DefKind::Struct,
            vec![
                field("commandsBlob", TYPE_UINT, 0, false),
                field("fontSize", TYPE_FLOAT, 0, false),
                field("position", 2, 0, false),
                field("firstCharacter", TYPE_UINT, 0, false),
            ],
        ),
        def(
            "DerivedTextData",
            DefKind::Message,
            vec![field("glyphs", 10, 1, true)],
        ),
        def(
            "FontName",
            DefKind::Struct,
            vec![
                field("family", TYPE_STRING, 0, false),
                field("style", TYPE_STRING, 0, false),
            ],
        ),
        def(
            "Geometry",
            DefKind::Struct,
            vec![field("commandsBlob", TYPE_UINT, 0, false)],
        ),
        def(
            "NodeChange",
            DefKind::Message,
            vec![
                field("guid", 0, 1, false),
                field("type", 1, 2, false),
                field("name", TYPE_STRING, 3, false),
                field("parentIndex", 3, 4, false),
                field("size", 2, 5, false),
                field("transform", 4, 6, false),
                field("fillPaints", 8, 7, true),
                field("textData", 9, 8, false),
                field("fontName", 12, 9, false),
                field("fontSize", TYPE_FLOAT, 10, false),
                field("derivedTextData", 11, 11, false),
                field("cornerRadius", TYPE_FLOAT, 12, false),
                field("fillGeometry", 13, 13, true),
            ],
        ),
        def(
            "Blob",
            DefKind::Struct,
            vec![field("bytes", TYPE_BYTE, 0, true)],
        ),
        def(
            "Message",
            DefKind::Message,
            vec![
                field("nodeChanges", 14, 1, true),
                field("blobs", 15, 2, true),
            ],
        ),
    ]);
    let mut blobs = Vec::new();
    let mut nodes = vec![
        obj(
            "NodeChange",
            vec![
                ("guid", guid(0)),
                ("type", Value::Enum("NodeType", "DOCUMENT")),
                ("name", Value::String("Example".into())),
            ],
        ),
        node(10, 0, "CANVAS", "Examples", [0.0; 4], vec![]),
        node(
            1,
            10,
            "FRAME",
            "First success",
            [0.0, 0.0, 320.0, 180.0],
            vec![("fillPaints", Value::Array(vec![solid([240, 244, 248])]))],
        ),
        node(
            2,
            1,
            "FRAME",
            "Card",
            [16.0, 16.0, 288.0, 148.0],
            vec![
                ("cornerRadius", Value::Float(12.0)),
                ("fillPaints", Value::Array(vec![solid([255; 3])])),
            ],
        ),
        node(
            3,
            2,
            "RECTANGLE",
            "Original tile",
            [16.0, 16.0, 32.0, 32.0],
            vec![(
                "fillPaints",
                Value::Array(vec![obj(
                    "Paint",
                    vec![
                        ("type", Value::Enum("PaintType", "IMAGE")),
                        (
                            "image",
                            obj(
                                "Image",
                                vec![("hash", Value::String("first-success-tile".into()))],
                            ),
                        ),
                    ],
                )]),
            )],
        ),
        label(4, "READY", 64.0, 16.0, 28.0, &mut blobs),
        label(5, "LOCAL", 64.0, 60.0, 14.0, &mut blobs),
    ];
    let triangle = blobs.len() as u32;
    let mut bytes = Vec::new();
    path(&[(0.0, 24.0), (24.0, 24.0), (12.0, 0.0)], &mut bytes);
    blobs.push(obj(
        "Blob",
        vec![(
            "bytes",
            Value::Array(bytes.into_iter().map(Value::Byte).collect()),
        )],
    ));
    nodes.push(node(
        6,
        2,
        "VECTOR",
        "Triangle",
        [232.0, 100.0, 24.0, 24.0],
        vec![
            (
                "fillGeometry",
                Value::Array(vec![obj(
                    "Geometry",
                    vec![("commandsBlob", Value::UInt(triangle))],
                )]),
            ),
            ("fillPaints", Value::Array(vec![solid([80, 100, 220])])),
        ],
    ));
    let message = obj(
        "Message",
        vec![
            ("nodeChanges", Value::Array(nodes)),
            ("blobs", Value::Array(blobs)),
        ],
    );
    let mut canvas = b"fig-kiwi".to_vec();
    canvas.extend(106u32.to_le_bytes());
    for bytes in [schema.encode(), message.encode(&schema)] {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes)?;
        let bytes = encoder.finish()?;
        canvas.extend((bytes.len() as u32).to_le_bytes());
        canvas.extend(bytes);
    }
    let mut image = resvg::tiny_skia::Pixmap::new(4, 4).unwrap();
    for (i, pixel) in image.data_mut().chunks_exact_mut(4).enumerate() {
        pixel.copy_from_slice(if i / 4 == i % 4 || i / 4 + i % 4 == 3 {
            &[255; 4]
        } else {
            &[27, 153, 139, 255]
        });
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("canvas.fig", canvas),
        ("images/first-success-tile", image.encode_png()?),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())?;
        zip.write_all(&bytes)?;
    }
    let bytes = zip.finish()?.into_inner();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&bytes)?;
    println!(
        "Wrote {} bytes: First success [0:1], 320x180, READY / LOCAL",
        bytes.len()
    );
    Ok(())
}
