//! Original synthetic token fixture; not a Figma export or format-accuracy oracle.
use kiwi_schema::{Def, DefKind, Field, Schema, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value};
use std::{error::Error, io::Write, path::Path};

fn object(name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
    Value::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> Value<'static> {
    object(
        "GUID",
        vec![("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))],
    )
}

fn reference(id: u32) -> Value<'static> {
    object("VariableReference", vec![("guid", guid(id))])
}

fn node(
    id: u32,
    parent: Option<u32>,
    kind: &'static str,
    name: &'static str,
    mut extra: Vec<(&'static str, Value<'static>)>,
) -> Value<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", Value::Enum("NodeType", kind)),
        ("name", Value::String(name.into())),
        ("sortPosition", Value::String(format!("{id:03}"))),
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
    fields.append(&mut extra);
    object("NodeChange", fields)
}

fn color(r: f32, g: f32, b: f32, a: f32) -> Value<'static> {
    data(
        "COLOR",
        "colorValue",
        object(
            "Color",
            vec![
                ("r", Value::Float(r)),
                ("g", Value::Float(g)),
                ("b", Value::Float(b)),
                ("a", Value::Float(a)),
            ],
        ),
    )
}

fn data(kind: &'static str, key: &'static str, value: Value<'static>) -> Value<'static> {
    object(
        "VariableData",
        vec![
            ("dataType", Value::Enum("DataType", kind)),
            ("value", object("VariableValue", vec![(key, value)])),
        ],
    )
}

fn variable(
    id: u32,
    name: &'static str,
    kind: &'static str,
    light: Value<'static>,
    dark: Value<'static>,
) -> Value<'static> {
    let entries = [light, dark]
        .into_iter()
        .zip([11, 12])
        .map(|(value, mode)| {
            object(
                "ModeValue",
                vec![("modeID", guid(mode)), ("variableData", value)],
            )
        })
        .collect();
    node(
        id,
        Some(1),
        "VARIABLE",
        name,
        vec![
            ("variableSetID", reference(10)),
            ("variableResolvedType", Value::Enum("VariableType", kind)),
            (
                "variableDataValues",
                object(
                    "VariableDataValues",
                    vec![("entries", Value::Array(entries))],
                ),
            ),
        ],
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let [output] = <[String; 1]>::try_from(std::env::args().skip(1).collect::<Vec<_>>())
        .map_err(|_| "usage: token-fixture <samples/new-output.fig>")?;
    write(Path::new(&output))
}

pub fn write(output: &Path) -> Result<(), Box<dyn Error>> {
    let samples = Path::new("samples").canonicalize()?;
    if !output
        .parent()
        .ok_or("output must be inside samples/")?
        .canonicalize()?
        .starts_with(samples)
    {
        return Err("generated designs must stay inside samples/".into());
    }
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
            ["DOCUMENT", "CANVAS", "VARIABLE_SET", "VARIABLE"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| field(name, 0, i as u32, false))
                .collect(),
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
            "VariableType".into(),
            DefKind::Enum,
            vec![field("COLOR", 0, 0, false), field("FLOAT", 0, 1, false)],
        ),
        Def::new(
            "DataType".into(),
            DefKind::Enum,
            vec![
                field("COLOR", 0, 0, false),
                field("FLOAT", 0, 1, false),
                field("ALIAS", 0, 2, false),
            ],
        ),
        Def::new(
            "Color".into(),
            DefKind::Struct,
            ["r", "g", "b", "a"]
                .into_iter()
                .map(|name| field(name, TYPE_FLOAT, 0, false))
                .collect(),
        ),
        Def::new(
            "VariableReference".into(),
            DefKind::Message,
            vec![field("guid", 0, 1, false)],
        ),
        Def::new(
            "VariableValue".into(),
            DefKind::Message,
            vec![
                field("colorValue", 5, 1, false),
                field("floatValue", TYPE_FLOAT, 2, false),
                field("alias", 6, 3, false),
            ],
        ),
        Def::new(
            "VariableData".into(),
            DefKind::Message,
            vec![field("dataType", 4, 1, false), field("value", 7, 2, false)],
        ),
        Def::new(
            "ModeValue".into(),
            DefKind::Struct,
            vec![
                field("modeID", 0, 0, false),
                field("variableData", 8, 0, false),
            ],
        ),
        Def::new(
            "VariableDataValues".into(),
            DefKind::Message,
            vec![field("entries", 9, 1, true)],
        ),
        Def::new(
            "VariableMode".into(),
            DefKind::Struct,
            vec![
                field("id", 0, 0, false),
                field("name", TYPE_STRING, 0, false),
                field("sortPosition", TYPE_STRING, 0, false),
            ],
        ),
        Def::new(
            "NodeChange".into(),
            DefKind::Message,
            vec![
                field("guid", 0, 1, false),
                field("type", 1, 2, false),
                field("name", TYPE_STRING, 3, false),
                field("parentIndex", 2, 4, false),
                field("sortPosition", TYPE_STRING, 5, false),
                field("variableSetModes", 11, 6, true),
                field("variableSetID", 6, 7, false),
                field("variableResolvedType", 3, 8, false),
                field("variableDataValues", 10, 9, false),
            ],
        ),
        Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("nodeChanges", 12, 1, true)],
        ),
    ]);
    let modes = [(11, "Light"), (12, "Dark")]
        .into_iter()
        .map(|(id, name)| {
            object(
                "VariableMode",
                vec![
                    ("id", guid(id)),
                    ("name", Value::String(name.into())),
                    ("sortPosition", Value::String(format!("{id:03}"))),
                ],
            )
        })
        .collect();
    let alias = |target| data("ALIAS", "alias", reference(target));
    let message = object(
        "Message",
        vec![(
            "nodeChanges",
            Value::Array(vec![
                node(0, None, "DOCUMENT", "Token fixture", vec![]),
                node(1, Some(0), "CANVAS", "Original synthetic tokens", vec![]),
                node(
                    10,
                    Some(1),
                    "VARIABLE_SET",
                    "Theme",
                    vec![("variableSetModes", Value::Array(modes))],
                ),
                variable(
                    20,
                    "palette/brand",
                    "COLOR",
                    color(0.0, 0.25, 1.0, 0.5),
                    color(1.0, 0.25, 0.0, 1.0),
                ),
                variable(21, "semantic/link", "COLOR", alias(20), alias(20)),
                variable(22, "semantic/action", "COLOR", alias(21), alias(21)),
                variable(
                    23,
                    "ratio",
                    "FLOAT",
                    data("FLOAT", "floatValue", Value::Float(1.5)),
                    data("FLOAT", "floatValue", Value::Float(2.0)),
                ),
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
