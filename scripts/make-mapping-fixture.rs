//! Original synthetic mapping fixture; not a Figma export or format-accuracy oracle.
use kiwi_schema::{Def, DefKind, Field, Schema, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value};
use std::{error::Error, io::Write};

fn object(name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
    Value::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> Value<'static> {
    object(
        "GUID",
        vec![("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))],
    )
}

fn node(
    id: u32,
    parent: Option<u32>,
    kind: &'static str,
    name: &str,
    library: bool,
) -> Value<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", Value::Enum("NodeType", kind)),
        ("name", Value::String(name.into())),
        (
            "size",
            object(
                "Vector",
                vec![("x", Value::Float(100.0)), ("y", Value::Float(40.0))],
            ),
        ),
        (
            "transform",
            object(
                "Transform",
                ["m00", "m01", "m02", "m10", "m11", "m12"]
                    .into_iter()
                    .map(|key| {
                        (
                            key,
                            Value::Float(if key == "m00" || key == "m11" {
                                1.0
                            } else {
                                0.0
                            }),
                        )
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(parent) = parent {
        fields.push((
            "parentIndex",
            object(
                "ParentIndex",
                vec![
                    ("guid", guid(parent)),
                    ("position", Value::String(format!("{id:03}"))),
                ],
            ),
        ));
    }
    if kind == "INSTANCE" {
        fields.push((
            "symbolData",
            object("SymbolData", vec![("symbolID", guid(10))]),
        ));
    }
    // With `--library` the main is a cached library copy, so a usages result can name its library
    // reference; without it the output is the original mapping fixture, byte for byte.
    if library && kind == "SYMBOL" {
        // The env vars let a test build sources with other keys or namespaces; unset keeps the bytes.
        let env = |name, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        fields.push((
            "componentKey",
            Value::String(env("MAPPING_FIXTURE_COMPONENT_KEY", "synthetic-button")),
        ));
        fields.push((
            "sourceLibraryKey",
            Value::String(env("MAPPING_FIXTURE_LIBRARY_KEY", "synthetic-library")),
        ));
    }
    object("NodeChange", fields)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let library = args.first().is_some_and(|a| a == "--library");
    if library {
        args.remove(0);
    }
    let [output] = <[String; 1]>::try_from(args)
        .map_err(|_| "usage: mapping-fixture [--library] <new-output.fig>")?;
    let field = |name: &str, type_id, value, is_array| Field {
        name: name.into(),
        type_id,
        value,
        is_array,
    };
    let schema = Schema::new(vec![
        Def::new(
            "GUID".into(),
            DefKind::Struct,
            vec![
                field("sessionID", TYPE_UINT, 0, false),
                field("localID", TYPE_UINT, 0, false),
            ],
        ),
        Def::new(
            "NodeType".into(),
            DefKind::Enum,
            [
                "DOCUMENT",
                "CANVAS",
                "FRAME",
                "SYMBOL",
                "INSTANCE",
                "RECTANGLE",
            ]
            .into_iter()
            .enumerate()
            .map(|(i, name)| field(name, 0, i as u32, false))
            .collect(),
        ),
        Def::new(
            "Vector".into(),
            DefKind::Struct,
            vec![
                field("x", TYPE_FLOAT, 0, false),
                field("y", TYPE_FLOAT, 0, false),
            ],
        ),
        Def::new(
            "ParentIndex".into(),
            DefKind::Struct,
            vec![
                field("guid", 0, 0, false),
                field("position", TYPE_STRING, 0, false),
            ],
        ),
        Def::new(
            "Transform".into(),
            DefKind::Struct,
            ["m00", "m01", "m02", "m10", "m11", "m12"]
                .into_iter()
                .map(|name| field(name, TYPE_FLOAT, 0, false))
                .collect(),
        ),
        Def::new(
            "SymbolData".into(),
            DefKind::Message,
            vec![field("symbolID", 0, 1, false)],
        ),
        Def::new(
            "NodeChange".into(),
            DefKind::Message,
            vec![
                field("guid", 0, 1, false),
                field("type", 1, 2, false),
                field("name", TYPE_STRING, 3, false),
                field("parentIndex", 3, 4, false),
                field("size", 2, 5, false),
                field("transform", 4, 6, false),
                field("symbolData", 5, 7, false),
            ]
            .into_iter()
            .chain(
                [
                    field("componentKey", TYPE_STRING, 8, false),
                    field("sourceLibraryKey", TYPE_STRING, 9, false),
                ]
                .into_iter()
                .filter(|_| library),
            )
            .collect(),
        ),
        Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("nodeChanges", 6, 1, true)],
        ),
    ]);
    let message = object(
        "Message",
        vec![(
            "nodeChanges",
            Value::Array(vec![
                node(0, None, "DOCUMENT", "Mapping fixture", library),
                node(9, Some(0), "CANVAS", "Original synthetic examples", library),
                node(1, Some(9), "FRAME", "Mapping review", library),
                node(2, Some(1), "INSTANCE", "First button", library),
                node(3, Some(1), "INSTANCE", "Second button", library),
                node(10, Some(9), "SYMBOL", "Button", library),
                node(11, Some(10), "RECTANGLE", "Surface", library),
            ]),
        )],
    );
    let mut canvas = b"fig-kiwi".to_vec();
    canvas.extend(1u32.to_le_bytes());
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
    zip.start_file("canvas.fig", zip::write::SimpleFileOptions::default())?;
    zip.write_all(&canvas)?;
    zip.finish()?;
    Ok(())
}
