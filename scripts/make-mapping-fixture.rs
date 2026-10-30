//! Original synthetic mapping fixture; not a Figma export or format-accuracy oracle.
use kiwi_schema::{
    Def, DefKind, Field, Schema, TYPE_BOOL, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value,
};
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

const WORDS: [&str; 8] = [
    "checkout",
    "profile",
    "settings",
    "search",
    "billing",
    "inbox",
    "report",
    "onboarding",
];

/// Screens per top-level area: 800 x 12 effective records (8 nodes plus 4 expanded symbol
/// children) keeps one area, the first prepared scope, under 10,000 records.
const AREA: u32 = 800;

/// `--scale` screens for bounded performance runs: area > screen > section > (text label, group >
/// 4 button instances). Area k is node 0:{900000 + k}. `--revised` edits screen 0's label text
/// only, so two outputs differ by one real text edit.
fn screens(count: u32, revised: bool, library: bool) -> impl Iterator<Item = Value<'static>> {
    let area = move |k| node(900_000 + k, Some(9), "FRAME", &format!("Area {k}"), library);
    (0..count.div_ceil(AREA))
        .map(area)
        .chain((0..count).flat_map(move |s| {
            let id = 1000 + s * 8;
            let word = WORDS[s as usize % WORDS.len()];
            let mark = if revised && s == 0 { " revised" } else { "" };
            let mut label = node(id + 7, Some(id + 1), "TEXT", &format!("Label {s}"), library);
            if let Value::Object(_, fields) = &mut label {
                let text = format!("{word} total for screen {s}{mark}");
                fields.insert(
                    "textData",
                    object("TextData", vec![("characters", Value::String(text))]),
                );
            }
            let frame =
                move |id, parent, name: String| node(id, Some(parent), "FRAME", &name, library);
            let button = move |j| {
                let name = format!("Button {s}.{j} {word}");
                node(id + 3 + j, Some(id + 2), "INSTANCE", &name, library)
            };
            [
                frame(id, 900_000 + s / AREA, format!("Screen {s} {word}")),
                frame(id + 1, id, format!("Section {s} {word}")),
                frame(id + 2, id + 1, format!("Group {s}")),
                label,
            ]
            .into_iter()
            .chain((0..4).map(button))
        }))
}

/// `--properties`: the kiwi definitions of text and boolean component properties, from index
/// `at`: TextValue, ComponentPropValue, ComponentPropDef, ComponentPropAssignment, its type enum.
fn property_defs(at: i32, field: &dyn Fn(&str, i32, u32, bool) -> Field) -> Vec<Def> {
    vec![
        Def::new(
            "TextValue".into(),
            DefKind::Message,
            vec![field("characters", TYPE_STRING, 1, false)],
        ),
        Def::new(
            "ComponentPropValue".into(),
            DefKind::Message,
            vec![
                field("boolValue", TYPE_BOOL, 1, false),
                field("textValue", at, 2, false),
            ],
        ),
        Def::new(
            "ComponentPropDef".into(),
            DefKind::Message,
            vec![
                field("id", 0, 1, false),
                field("name", TYPE_STRING, 2, false),
                field("type", at + 4, 3, false),
                field("initialValue", at + 1, 4, false),
            ],
        ),
        Def::new(
            "ComponentPropAssignment".into(),
            DefKind::Message,
            vec![
                field("defID", 0, 1, false),
                field("value", at + 1, 2, false),
            ],
        ),
        Def::new(
            "ComponentPropType".into(),
            DefKind::Enum,
            ["BOOL", "TEXT"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| field(name, 0, i as u32, false))
                .collect(),
        ),
    ]
}

/// `--properties`: `Button` defines a text `Label` (default "Button") and a boolean `Show icon`
/// (default true); `First button` overrides both, `Second button` keeps the defaults.
fn add_properties(nodes: &mut [Value<'static>]) {
    let text = |s: &str| {
        object(
            "ComponentPropValue",
            vec![(
                "textValue",
                object("TextValue", vec![("characters", Value::String(s.into()))]),
            )],
        )
    };
    let flag = |b| object("ComponentPropValue", vec![("boolValue", Value::Bool(b))]);
    let def = |id, name: &str, kind, initial| {
        object(
            "ComponentPropDef",
            vec![
                ("id", guid(id)),
                ("name", Value::String(name.into())),
                ("type", Value::Enum("ComponentPropType", kind)),
                ("initialValue", initial),
            ],
        )
    };
    let assign = |id, value| {
        object(
            "ComponentPropAssignment",
            vec![("defID", guid(id)), ("value", value)],
        )
    };
    for n in nodes.iter_mut() {
        let Value::Object(_, fields) = n else {
            continue;
        };
        match fields.get("guid").and_then(|g| g.get("localID")) {
            Some(Value::UInt(10)) => {
                let defs = vec![
                    def(90, "Label", "TEXT", text("Button")),
                    def(91, "Show icon", "BOOL", flag(true)),
                ];
                fields.insert("componentPropDefs", Value::Array(defs));
            }
            Some(Value::UInt(2)) => {
                let list = vec![assign(90, text("Pay now")), assign(91, flag(false))];
                fields.insert("componentPropAssignments", Value::Array(list));
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        let found = args.iter().position(|a| a == name);
        found.map(|i| args.remove(i)).is_some()
    };
    let library = flag("--library");
    let properties = flag("--properties");
    let revised = flag("--revised");
    let scale = match args.iter().position(|a| a == "--scale") {
        Some(i) if i + 1 < args.len() => {
            args.remove(i);
            Some(args.remove(i).parse::<u32>()?)
        }
        Some(_) => return Err("--scale needs a screen count".into()),
        None => None,
    };
    let [output] = <[String; 1]>::try_from(args).map_err(
        |_| "usage: mapping-fixture [--library] [--properties] [--scale <screens> [--revised]] <new-output.fig>",
    )?;
    // `--properties` appends its definitions after the others, starting at this index.
    let props = 8 + i32::from(scale.is_some());
    let field = |name: &str, type_id, value, is_array| Field {
        name: name.into(),
        type_id,
        value,
        is_array,
    };
    let mut defs = vec![
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
            .chain(scale.map(|_| "TEXT"))
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
            .chain(scale.map(|_| field("textData", 8, 10, false)))
            .chain(
                [
                    field("componentPropDefs", props + 2, 11, true),
                    field("componentPropAssignments", props + 3, 12, true),
                ]
                .into_iter()
                .filter(|_| properties),
            )
            .collect(),
        ),
        Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("nodeChanges", 6, 1, true)],
        ),
    ];
    if scale.is_some() {
        let characters = vec![field("characters", TYPE_STRING, 1, false)];
        defs.push(Def::new("TextData".into(), DefKind::Message, characters));
    }
    if properties {
        defs.extend(property_defs(props, &field));
    }
    let schema = Schema::new(defs);
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Mapping fixture", library),
        node(9, Some(0), "CANVAS", "Original synthetic examples", library),
        node(1, Some(9), "FRAME", "Mapping review", library),
        node(2, Some(1), "INSTANCE", "First button", library),
        node(3, Some(1), "INSTANCE", "Second button", library),
        node(10, Some(9), "SYMBOL", "Button", library),
        node(11, Some(10), "RECTANGLE", "Surface", library),
    ];
    nodes.extend(screens(scale.unwrap_or(0), revised, library));
    if properties {
        add_properties(&mut nodes);
    }
    let message = object("Message", vec![("nodeChanges", Value::Array(nodes))]);
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
