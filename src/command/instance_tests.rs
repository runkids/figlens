//! Normal, generated local-copy fixture: two copies of a component with a nested instance.
use super::*;
use crate::{call_tool, run, serve, test_support::TestDir};
use kiwi_schema::{Def, DefKind, Field, Schema, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value as K};
use std::io::{Read, Write};

fn obj(name: &'static str, fields: Vec<(&'static str, K<'static>)>) -> K<'static> {
    K::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> K<'static> {
    obj(
        "GUID",
        vec![("sessionID", K::UInt(0)), ("localID", K::UInt(id))],
    )
}

fn size(w: f32, h: f32) -> K<'static> {
    obj("Vector", vec![("x", K::Float(w)), ("y", K::Float(h))])
}

fn transform(x: f32, y: f32) -> K<'static> {
    obj(
        "Transform",
        vec![
            ("m00", K::Float(1.0)),
            ("m01", K::Float(0.0)),
            ("m02", K::Float(x)),
            ("m10", K::Float(0.0)),
            ("m11", K::Float(1.0)),
            ("m12", K::Float(y)),
        ],
    )
}

fn paint(hash: &str) -> K<'static> {
    K::Array(vec![obj(
        "Paint",
        vec![
            ("type", K::Enum("PaintType", "IMAGE")),
            (
                "image",
                obj("Image", vec![("hash", K::String(hash.into()))]),
            ),
        ],
    )])
}

fn text(s: &str) -> K<'static> {
    obj("TextData", vec![("characters", K::String(s.into()))])
}

struct Fixture {
    dir: TestDir,
    file: String,
}

impl Fixture {
    fn new() -> Self {
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
                [
                    "DOCUMENT",
                    "FRAME",
                    "SYMBOL",
                    "INSTANCE",
                    "RECTANGLE",
                    "TEXT",
                    "CANVAS",
                ]
                .into_iter()
                .enumerate()
                .map(|(i, n)| field(n, 0, i as u32, false))
                .collect(),
            ),
            def(
                "Vector",
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
                    .into_iter()
                    .map(|n| field(n, TYPE_FLOAT, 0, false))
                    .collect(),
            ),
            def("Path", DefKind::Message, vec![field("guids", 0, 1, true)]),
            def(
                "Image",
                DefKind::Struct,
                vec![field("hash", TYPE_STRING, 0, false)],
            ),
            def(
                "PaintType",
                DefKind::Enum,
                vec![field("IMAGE", 0, 0, false)],
            ),
            def(
                "Paint",
                DefKind::Message,
                vec![field("type", 7, 1, false), field("image", 6, 2, false)],
            ),
            def(
                "TextData",
                DefKind::Message,
                vec![field("characters", TYPE_STRING, 1, false)],
            ),
            def(
                "Override",
                DefKind::Message,
                vec![
                    field("guidPath", 5, 1, false),
                    field("name", TYPE_STRING, 2, false),
                    field("size", 2, 3, false),
                    field("fillPaints", 8, 4, true),
                    field("textData", 9, 5, false),
                ],
            ),
            def(
                "SymbolData",
                DefKind::Message,
                vec![
                    field("symbolID", 0, 1, false),
                    field("symbolOverrides", 10, 2, true),
                ],
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
                    field("symbolData", 11, 7, false),
                    field("fillPaints", 8, 8, true),
                    field("textData", 9, 9, false),
                    field("overrideKey", 0, 10, false),
                ],
            ),
            def(
                "Message",
                DefKind::Message,
                vec![field("nodeChanges", 12, 1, true)],
            ),
        ]);
        let node = |id, parent, kind, name: &str, extra: Vec<_>| {
            let mut fields = vec![
                ("guid", guid(id)),
                ("type", K::Enum("NodeType", kind)),
                ("name", K::String(name.into())),
            ];
            if let Some(p) = parent {
                fields.push((
                    "parentIndex",
                    obj(
                        "ParentIndex",
                        vec![
                            ("guid", guid(p)),
                            ("position", K::String(format!("{id:03}"))),
                        ],
                    ),
                ));
            }
            fields.extend(extra);
            obj("NodeChange", fields)
        };
        let patch = |ids: &[u32], values: Vec<_>| {
            let mut fields = vec![(
                "guidPath",
                obj(
                    "Path",
                    vec![("guids", K::Array(ids.iter().copied().map(guid).collect()))],
                ),
            )];
            fields.extend(values);
            obj("Override", fields)
        };
        let instance = |id, x, y, label: &str, hash: &str, w, h| {
            node(
                id,
                Some(1),
                "INSTANCE",
                label,
                vec![
                    ("size", size(40.0, 30.0)),
                    ("transform", transform(x, y)),
                    (
                        "symbolData",
                        obj(
                            "SymbolData",
                            vec![
                                ("symbolID", guid(10)),
                                (
                                    "symbolOverrides",
                                    K::Array(vec![
                                        patch(
                                            &[11],
                                            vec![
                                                ("name", K::String(label.into())),
                                                ("size", size(w, h)),
                                            ],
                                        ),
                                        patch(
                                            &[112],
                                            vec![("size", size(w, h)), ("fillPaints", paint(hash))],
                                        ),
                                        patch(&[13], vec![("textData", text(label))]),
                                        patch(&[14, 21], vec![("fillPaints", paint(hash))]),
                                    ]),
                                ),
                            ],
                        ),
                    ),
                ],
            )
        };
        let message = obj(
            "Message",
            vec![(
                "nodeChanges",
                K::Array(vec![
                    node(0, None, "DOCUMENT", "Document", vec![]),
                    node(30, Some(0), "CANVAS", "Empty page", vec![]),
                    node(
                        1,
                        Some(0),
                        "FRAME",
                        "Screen",
                        vec![("size", size(100.0, 100.0))],
                    ),
                    instance(2, 10.0, 20.0, "First", "blue", 8.0, 6.0),
                    instance(3, 50.0, 60.0, "Second", "green", 6.0, 4.0),
                    node(
                        10,
                        Some(0),
                        "SYMBOL",
                        "Card",
                        vec![("size", size(40.0, 30.0))],
                    ),
                    node(
                        11,
                        Some(10),
                        "FRAME",
                        "Default card",
                        vec![("size", size(8.0, 6.0)), ("transform", transform(2.0, 3.0))],
                    ),
                    // A copied library layer uses overrideKey in expanded IDs, not its local guid.
                    node(
                        12,
                        Some(11),
                        "RECTANGLE",
                        "Artwork",
                        vec![
                            ("overrideKey", guid(112)),
                            ("size", size(8.0, 6.0)),
                            ("fillPaints", paint("red")),
                        ],
                    ),
                    node(
                        13,
                        Some(11),
                        "TEXT",
                        "Label",
                        vec![("textData", text("Default"))],
                    ),
                    node(
                        14,
                        Some(10),
                        "INSTANCE",
                        "Nested",
                        vec![
                            ("size", size(4.0, 3.0)),
                            ("transform", transform(20.0, 2.0)),
                            (
                                "symbolData",
                                obj("SymbolData", vec![("symbolID", guid(20))]),
                            ),
                        ],
                    ),
                    node(
                        20,
                        Some(0),
                        "SYMBOL",
                        "Badge",
                        vec![("size", size(4.0, 3.0))],
                    ),
                    node(
                        21,
                        Some(20),
                        "RECTANGLE",
                        "Badge artwork",
                        vec![("size", size(4.0, 3.0)), ("fillPaints", paint("red"))],
                    ),
                ]),
            )],
        );
        let dir = TestDir::new();
        let file = dir.join("instances.fig");
        let mut canvas = b"fig-kiwi".to_vec();
        canvas.extend(1u32.to_le_bytes());
        for bytes in [schema.encode(), message.encode(&schema)] {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&bytes).unwrap();
            let bytes = encoder.finish().unwrap();
            canvas.extend((bytes.len() as u32).to_le_bytes());
            canvas.extend(bytes);
        }
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        zip.start_file("canvas.fig", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&canvas).unwrap();
        for (hash, color) in [
            ("red", [255, 0, 0, 255]),
            ("blue", [0, 0, 255, 255]),
            ("green", [0, 255, 0, 255]),
        ] {
            let mut pixel = resvg::tiny_skia::Pixmap::new(1, 1).unwrap();
            pixel.data_mut().copy_from_slice(&color);
            zip.start_file(
                format!("images/{hash}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&pixel.encode_png().unwrap()).unwrap();
        }
        zip.finish().unwrap();
        Self {
            file: file.to_string_lossy().into_owned(),
            dir,
        }
    }

    fn cli(&self, name: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
        let mut argv = vec![name.into(), self.file.clone()];
        argv.extend(args.iter().map(|s| (*s).into()));
        let mut out = String::new();
        run(argv, &mut out)?;
        Ok(out)
    }

    fn tool(&self, name: &str, mut arguments: Value) -> Value {
        arguments["file"] = self.file.clone().into();
        call_tool(&json!({"name": name, "arguments": arguments}), None)
    }
}

fn zip_bytes(zip: &mut zip::ZipArchive<std::fs::File>, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    zip.by_name(path).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

fn descendant<'a>(node: &'a Value, id: &str) -> &'a Value {
    if node["id"] == id {
        return node;
    }
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|n| {
            let found = descendant(n, id);
            (!found.is_null()).then_some(found)
        })
        .unwrap_or(&Value::Null)
}

#[test]
fn emitted_instance_ids_select_the_same_structure_assets_and_pixels() {
    let f = Fixture::new();
    let found = f.tool("find", json!({"query": "Screen"}));
    assert_eq!(found["isError"], false, "{found}");
    let screen = found["structuredContent"]["results"][0]["id"]
        .as_str()
        .unwrap();
    let discovery = f.tool("inspect", json!({"node": screen, "depth": 4}));
    assert_eq!(discovery["isError"], false, "{discovery}");
    let expanded = &discovery["structuredContent"]["node"];
    for (id, name, hash, w, h, x, y, rgba) in [
        ("I0:2;0:11", "First", "blue", 8, 6, 12, 23, [0, 0, 255, 255]),
        (
            "I0:3;0:11",
            "Second",
            "green",
            6,
            4,
            52,
            63,
            [0, 255, 0, 255],
        ),
        (
            "I0:2;0:112",
            "Artwork",
            "blue",
            8,
            6,
            12,
            23,
            [0, 0, 255, 255],
        ),
        (
            "I0:3;0:112",
            "Artwork",
            "green",
            6,
            4,
            52,
            63,
            [0, 255, 0, 255],
        ),
        (
            "I0:2;0:14;0:21",
            "Badge artwork",
            "blue",
            4,
            3,
            30,
            22,
            [0, 0, 255, 255],
        ),
        (
            "I0:3;0:14;0:21",
            "Badge artwork",
            "green",
            4,
            3,
            70,
            62,
            [0, 255, 0, 255],
        ),
    ] {
        // Feed back the actual tool-emitted ID, never a raw component child as a fallback.
        let emitted = descendant(expanded, id)["id"].as_str().unwrap();
        let png = f.dir.join("export.png");
        let bundle = f.dir.join("bundle.zip");
        f.cli("export", &[emitted, png.to_str().unwrap()]).unwrap();
        let expected_pixels =
            resvg::tiny_skia::Pixmap::decode_png(&std::fs::read(&png).unwrap()).unwrap();
        assert_eq!((expected_pixels.width(), expected_pixels.height()), (w, h));
        assert!(
            expected_pixels.data().chunks_exact(4).all(|p| p == rgba),
            "{id}"
        );
        for mcp in [false, true] {
            if mcp {
                let reply = f.tool("bundle", json!({"node": emitted, "out": bundle}));
                assert_eq!(reply["isError"], false, "{reply}");
                assert_eq!(reply["structuredContent"]["nodeId"], emitted);
            } else {
                f.cli("bundle", &[emitted, bundle.to_str().unwrap()])
                    .unwrap();
            }
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&bundle).unwrap()).unwrap();
            let nodes: Value =
                serde_json::from_slice(&zip_bytes(&mut zip, "design/nodes.json")).unwrap();
            let n = &nodes["document"];
            assert_eq!(n["id"], emitted);
            assert_eq!(n["name"], name);
            assert_eq!(
                n["absoluteBoundingBox"],
                json!({"x": f64::from(x), "y": f64::from(y), "width": f64::from(w), "height": f64::from(h)})
            );
            assert!(!nodes.to_string().contains("\"_r\""));
            if id.ends_with(";0:11") {
                assert_eq!(n["children"][1]["characters"], name);
            }
            let reference =
                resvg::tiny_skia::Pixmap::decode_png(&zip_bytes(&mut zip, "references/screen.png"))
                    .unwrap();
            assert_eq!(reference, expected_pixels);
            let assets: Value =
                serde_json::from_slice(&zip_bytes(&mut zip, "assets.json")).unwrap();
            assert_eq!(assets.as_array().unwrap().len(), 1);
            assert_eq!(assets[0]["key"], hash);
            let asset = resvg::tiny_skia::Pixmap::decode_png(&zip_bytes(
                &mut zip,
                assets[0]["path"].as_str().unwrap(),
            ))
            .unwrap();
            assert_eq!(asset.data(), rgba);
            let manifest: Value =
                serde_json::from_slice(&zip_bytes(&mut zip, "manifest.json")).unwrap();
            assert_eq!(manifest["source"]["nodeId"], emitted);
            let args = json!({"node": emitted, "depth": 2, "images": true});
            let inspected = f.tool("inspect", args);
            assert_eq!(inspected["isError"], false, "{inspected}");
            let summary = &inspected["structuredContent"];
            assert_eq!(summary["node"]["id"], emitted);
            assert_eq!(
                summary["node"]["absoluteBoundingBox"],
                n["absoluteBoundingBox"]
            );
            assert_eq!(summary["images"]["items"][0]["imageRef"], hash);
            let cli: Value = serde_json::from_str(
                &f.cli("inspect", &[emitted, "--depth", "2", "--images"])
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(cli, *summary);
            let cli: Value = serde_json::from_str(&f.cli("nodes", &[emitted]).unwrap()).unwrap();
            assert_eq!(cli, nodes);
            let reply = f.tool("nodes", json!({"node": emitted}));
            assert_eq!(reply["isError"], false, "{reply}");
            assert_eq!(
                serde_json::from_str::<Value>(reply["content"][0]["text"].as_str().unwrap())
                    .unwrap(),
                nodes
            );
        }
        let reply = f.tool("export", json!({"node": emitted, "out": png}));
        assert_eq!(reply["isError"], false, "{reply}");
        assert_eq!(
            resvg::tiny_skia::Pixmap::decode_png(&std::fs::read(png).unwrap()).unwrap(),
            expected_pixels
        );
        let svg = f.dir.join("export.svg");
        f.cli("export", &[emitted, svg.to_str().unwrap()]).unwrap();
        assert_eq!(
            figlens::render::pixmap(&std::fs::read_to_string(&svg).unwrap()).unwrap(),
            expected_pixels
        );
        let reply = f.tool("export", json!({"node": emitted, "out": svg}));
        assert_eq!(reply["isError"], false, "{reply}");
        assert_eq!(
            figlens::render::pixmap(&std::fs::read_to_string(svg).unwrap()).unwrap(),
            expected_pixels
        );
    }
}

#[test]
fn stored_nodes_and_instances_keep_their_identity() {
    let f = Fixture::new();
    for (id, kind, name, w, h) in [
        ("0:1", "FRAME", "Screen", 100, 100),
        ("0:2", "INSTANCE", "First", 40, 30),
        ("0:3", "INSTANCE", "Second", 40, 30),
        ("0:12", "RECTANGLE", "Artwork", 8, 6),
        ("I0:2;0:14", "INSTANCE", "Nested", 4, 3),
    ] {
        let doc: Value = serde_json::from_str(&f.cli("nodes", &[id]).unwrap()).unwrap();
        assert_eq!(doc["document"]["id"], id);
        assert_eq!(doc["document"]["type"], kind);
        assert_eq!(doc["document"]["name"], name);
        let inspected = f.tool("inspect", json!({"node": id, "depth": 4}));
        assert_eq!(inspected["isError"], false, "{inspected}");
        assert_eq!(inspected["structuredContent"]["node"]["id"], id);
        let png = f.dir.join("stored.png");
        let bundle = f.dir.join("stored.zip");
        f.cli("export", &[id, png.to_str().unwrap()]).unwrap();
        f.cli("bundle", &[id, bundle.to_str().unwrap()]).unwrap();
        let expected = resvg::tiny_skia::Pixmap::decode_png(&std::fs::read(png).unwrap()).unwrap();
        assert_eq!((expected.width(), expected.height()), (w, h));
        let mut zip = zip::ZipArchive::new(std::fs::File::open(bundle).unwrap()).unwrap();
        let bundled: Value =
            serde_json::from_slice(&zip_bytes(&mut zip, "design/nodes.json")).unwrap();
        assert_eq!(bundled, doc);
        assert_eq!(
            resvg::tiny_skia::Pixmap::decode_png(&zip_bytes(&mut zip, "references/screen.png"))
                .unwrap(),
            expected
        );
        if id == "0:12" {
            assert_eq!(doc["document"]["fills"][0]["imageRef"], "red");
            assert!(
                expected
                    .data()
                    .chunks_exact(4)
                    .all(|p| p == [255, 0, 0, 255])
            );
        }
    }
    // Existing dash and URL inputs keep the entire expanded path and source manifest ID.
    for input in [
        "I0-2;0-14;0-21",
        "https://www.figma.com/design/Key/Test?node-id=I0-2;0-14;0-21",
    ] {
        let n: Value = serde_json::from_str(&f.cli("nodes", &[input]).unwrap()).unwrap();
        assert_eq!(n["document"]["id"], "I0:2;0:14;0:21");
        let reply = f.tool("variables", json!({"node": input}));
        assert_eq!(reply["isError"], false, "{reply}");
    }
}

#[test]
fn batch_uses_the_same_bundle_target_resolver() {
    let f = Fixture::new();
    let list = f.dir.join("frames.txt");
    let out = f.dir.join("batch");
    let ids = ["0:12", "I0:2;0:11", "I0:3;0:14;0:21"];
    std::fs::write(&list, ids.join("\n")).unwrap();
    f.cli(
        "bundle",
        &[
            "--frames",
            list.to_str().unwrap(),
            "--out-dir",
            out.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    )
    .unwrap();
    for id in ids {
        let path = out.join(format!("{}.zip", id.replace(':', "-")));
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let nodes: Value =
            serde_json::from_slice(&zip_bytes(&mut zip, "design/nodes.json")).unwrap();
        assert_eq!(nodes["document"]["id"], id);
        assert!(
            resvg::tiny_skia::Pixmap::decode_png(&zip_bytes(&mut zip, "references/screen.png"))
                .is_ok()
        );
    }
}

#[test]
fn missing_and_unsupported_ids_fail_without_fallback_or_output() {
    let f = Fixture::new();
    let png = f.dir.join("invalid.png");
    let bundle = f.dir.join("invalid.zip");
    for id in [
        "",
        "invalid",
        "9:9",
        "I9:9;0:112",
        "I0:2",
        "I0:2;",
        "I0:2;;0:112",
        "I0:2;0:12",       // Stored library guid is not its expanded overrideKey.
        "I0:2;0:21",       // Missing nested instance boundary.
        "I0:2;0:11;0:112", // Plain frames do not form instance boundaries.
        "I0:2;0:14;0:999",
        "I0:1;0:2", // No selecting a stored child instead.
    ] {
        let mut errors = Vec::new();
        for (name, args, tool_args) in [
            ("nodes", vec![id], json!({"node": id})),
            ("inspect", vec![id], json!({"node": id})),
            (
                "inspect",
                vec![id, "--images"],
                json!({"node": id, "images": true}),
            ),
            (
                "export",
                vec![id, png.to_str().unwrap()],
                json!({"node": id, "out": png}),
            ),
            (
                "bundle",
                vec![id, bundle.to_str().unwrap()],
                json!({"node": id, "out": bundle}),
            ),
        ] {
            let error = f.cli(name, &args).unwrap_err().to_string();
            assert!(error.contains("not found"), "{name} {id}: {error}");
            let reply = f.tool(name, tool_args);
            assert_eq!(reply["isError"], true, "{reply}");
            assert_eq!(reply["content"][0]["text"], error);
            assert_eq!(reply["_meta"]["figlens"]["code"], "node_not_found");
            errors.push(error);
        }
        assert!(
            errors.iter().all(|error| error == &errors[0]),
            "{id}: {errors:?}"
        );
        assert!(!png.exists());
        assert!(!bundle.exists());
    }
    for (name, args, arguments) in [
        ("tree", vec!["I0:2;0:11"], json!({"node": "I0:2;0:11"})),
        (
            "find",
            vec!["Artwork", "--node", "I0:2;0:11"],
            json!({"query": "Artwork", "node": "I0:2;0:11"}),
        ),
    ] {
        let error = f.cli(name, &args).unwrap_err().to_string();
        assert!(error.contains("supports stored nodes only"), "{error}");
        assert!(!error.contains("not found"));
        let reply = f.tool(name, arguments);
        assert_eq!(reply["isError"], true);
        assert_eq!(reply["_meta"]["figlens"]["code"], "unsupported_node");
        assert_eq!(reply["content"][0]["text"], error);
    }
    assert!(
        f.cli("node", &["I0:2;0:11"])
            .unwrap_err()
            .to_string()
            .contains("supports stored nodes only")
    );
    // An empty page exists and is inspectable, but has no child union to render.
    assert_eq!(f.tool("inspect", json!({"node": "0:30"}))["isError"], false);
    for (name, out) in [("export", &png), ("bundle", &bundle)] {
        let error = f
            .cli(name, &["0:30", out.to_str().unwrap()])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("unsupported for {name}: no drawable bounds")),
            "{error}"
        );
        let reply = f.tool(name, json!({"node": "0:30", "out": out}));
        assert_eq!(reply["isError"], true);
        assert_eq!(reply["content"][0]["text"], error);
        assert!(!out.exists());
    }
    let request = |id, method, params| {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    };
    let input = [
        request(1, "tools/call", json!({"name": "bundle", "arguments": {"file": f.file, "node": "I0:2;0:21", "out": bundle}})),
        request(2, "ping", json!({})),
        request(3, "tools/call", json!({"name": "inspect", "arguments": {"file": f.file, "node": "I0:2;0:14;0:21"}})),
    ].join("\n") + "\n";
    let mut output = Vec::new();
    serve(input.as_bytes(), &mut output, &[]).unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["result"]["isError"], true);
    assert_eq!(replies[1]["result"], json!({}));
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["node"]["id"],
        "I0:2;0:14;0:21"
    );
}
