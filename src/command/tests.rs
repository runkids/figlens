use crate::{call_tool, run};
use std::io::Write as _;

use crate::test_support::TestDir;

struct Fixture(TestDir);

impl Fixture {
    fn new() -> Self {
        Self::named("Frame")
    }

    fn named(name: &str) -> Self {
        use kiwi_schema::{Def, DefKind, Field, Schema, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value};
        use std::collections::HashMap;
        let dir = TestDir::new();
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
                vec![field("DOCUMENT", 0, 0, false), field("FRAME", 0, 1, false)],
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
                "NodeChange".into(),
                DefKind::Message,
                vec![
                    field("guid", 0, 1, false),
                    field("type", 1, 2, false),
                    field("name", TYPE_STRING, 3, false),
                    field("parentIndex", 3, 4, false),
                    field("size", 2, 5, false),
                ],
            ),
            Def::new(
                "Message".into(),
                DefKind::Message,
                vec![field("nodeChanges", 4, 1, true)],
            ),
        ]);
        let guid = |id| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))]),
            )
        };
        let message = Value::Object(
            "Message",
            HashMap::from([(
                "nodeChanges",
                Value::Array(vec![
                    Value::Object(
                        "NodeChange",
                        HashMap::from([
                            ("guid", guid(0)),
                            ("type", Value::Enum("NodeType", "DOCUMENT")),
                            ("name", Value::String("Document".into())),
                        ]),
                    ),
                    Value::Object(
                        "NodeChange",
                        HashMap::from([
                            ("guid", guid(1)),
                            ("type", Value::Enum("NodeType", "FRAME")),
                            ("name", Value::String(name.into())),
                            (
                                "parentIndex",
                                Value::Object(
                                    "ParentIndex",
                                    HashMap::from([
                                        ("guid", guid(0)),
                                        ("position", Value::String("a".into())),
                                    ]),
                                ),
                            ),
                            (
                                "size",
                                Value::Object(
                                    "Vector",
                                    HashMap::from([
                                        ("x", Value::Float(16.0)),
                                        ("y", Value::Float(8.0)),
                                    ]),
                                ),
                            ),
                        ]),
                    ),
                ]),
            )]),
        );
        let mut canvas = b"fig-kiwi".to_vec();
        canvas.extend(1u32.to_le_bytes());
        for bytes in [schema.encode(), message.encode(&schema)] {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&bytes).unwrap();
            let chunk = encoder.finish().unwrap();
            canvas.extend((chunk.len() as u32).to_le_bytes());
            canvas.extend(chunk);
        }
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join("input.fig")).unwrap());
        zip.start_file("canvas.fig", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&canvas).unwrap();
        zip.finish().unwrap();
        Self(dir)
    }

    fn file(&self) -> String {
        self.0.join("input.fig").to_string_lossy().into_owned()
    }
}

fn assert_bundle(path: &str) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest["root"]["name"], "Renamed");
    assert_eq!(manifest["source"]["fileKey"], "Key");
    assert_eq!(manifest["source"]["nodeId"], "0:1");
    for entry in [
        "design/nodes.json",
        "design/variables.json",
        "references/screen.png",
        "assets.json",
        "missing-assets.json",
        "context.md",
    ] {
        assert!(zip.by_name(entry).is_ok(), "missing {entry}");
    }
    let nodes: serde_json::Value =
        serde_json::from_reader(zip.by_name("design/nodes.json").unwrap()).unwrap();
    assert_eq!(nodes["document"]["id"], "0:1");
    assert!(!nodes.to_string().contains("\"_r\""));
}

#[test]
fn cli_and_mcp_success_results_match() {
    let fixture = Fixture::new();
    for (name, cli, arguments, expected) in [
        (
            "tree",
            vec!["--depth", "1"],
            serde_json::json!({"depth": 1}),
            Some("0:0 DOCUMENT \"Document\"\n  0:1 FRAME \"Frame\"\n"),
        ),
        (
            "tree",
            vec!["0-1", "--json", "--depth", "0"],
            serde_json::json!({"node": "0-1", "json": true, "depth": 0}),
            Some("{\"id\":\"0:1\",\"name\":\"Frame\",\"type\":\"FRAME\"}\n"),
        ),
        (
            "nodes",
            vec!["0-1"],
            serde_json::json!({"node": "0-1"}),
            None,
        ),
        (
            "find",
            vec!["frame", "--type", "frame", "--node", "0-0", "--limit", "1"],
            serde_json::json!({"query": "frame", "type": "frame", "node": "0-0", "limit": 1}),
            None,
        ),
        (
            "inspect",
            vec!["0-1", "--depth", "0"],
            serde_json::json!({"node": "0-1", "depth": 0}),
            None,
        ),
        ("variables", vec![], serde_json::json!({}), None),
        (
            "variables",
            vec!["0-1"],
            serde_json::json!({"node": "0-1"}),
            None,
        ),
        (
            "export",
            vec!["0-1", "image.png", "--scale", "2", "--quality", "90"],
            serde_json::json!({"node": "0-1", "out": "image.png", "scale": 2, "quality": 90}),
            None,
        ),
        (
            "bundle",
            vec![
                "https://www.figma.com/design/Key/Test?node-id=0-1",
                "bundle.zip",
                "--name",
                "Renamed",
            ],
            serde_json::json!({"node": "https://www.figma.com/design/Key/Test?node-id=0-1", "out": "bundle.zip", "name": "Renamed"}),
            None,
        ),
    ] {
        let mut cli: Vec<String> = cli.into_iter().map(str::to_owned).collect();
        let mut arguments = arguments;
        arguments["file"] = fixture.file().into();
        if let Some(out) = arguments["out"].as_str() {
            let out = fixture.0.join(out).to_string_lossy().into_owned();
            cli[1] = out.clone();
            arguments["out"] = out.into();
        }
        let mut argv = vec![name.into(), fixture.file()];
        argv.extend(cli);
        let mut text = String::new();
        run(argv, &mut text).unwrap();
        if name == "bundle" {
            let path = arguments["out"].as_str().unwrap();
            assert_bundle(path);
            std::fs::remove_file(path).unwrap();
        }
        let reply = call_tool(
            &serde_json::json!({"name": name, "arguments": arguments}),
            None,
        );
        assert_eq!(reply["isError"], false, "{name}: {reply}");
        assert_eq!(reply["content"][0]["text"], text, "{name}");
        if let Some(expected) = expected {
            assert_eq!(text, expected);
        } else {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            match name {
                "nodes" => assert_eq!(value["document"]["id"], "0:1"),
                "variables" => assert!(value.is_object()),
                "find" => {
                    assert_eq!(value["results"][0]["id"], "0:1");
                    assert_eq!(
                        value["results"][0]["size"],
                        serde_json::json!({"width": 16.0, "height": 8.0})
                    );
                    assert_eq!(value["truncated"], false);
                    assert_eq!(reply["structuredContent"], value);
                }
                "inspect" => {
                    assert_eq!(value["node"]["id"], "0:1");
                    assert_eq!(value["node"]["childCount"], 0);
                    assert_eq!(reply["structuredContent"], value);
                }
                "export" => {
                    assert_eq!(
                        (value["width"].as_u64(), value["height"].as_u64()),
                        (Some(32), Some(16))
                    );
                    assert_eq!(value["quality"], 90);
                    assert_eq!(reply["structuredContent"], value);
                    assert_eq!(reply["content"][1]["type"], "image");
                    assert_eq!(reply["content"][1]["mimeType"], "image/png");
                    assert_eq!(
                        reply["content"][1]["data"],
                        figlens::render::base64(
                            &std::fs::read(arguments["out"].as_str().unwrap()).unwrap()
                        )
                    );
                    assert!(
                        std::fs::read(arguments["out"].as_str().unwrap())
                            .unwrap()
                            .starts_with(b"\x89PNG")
                    );
                }
                "bundle" => {
                    assert_eq!(value["nodeId"], "0:1");
                    assert!(value["warnings"].is_array());
                    assert_eq!(reply["structuredContent"], value);
                    assert_bundle(arguments["out"].as_str().unwrap());
                }
                _ => unreachable!(),
            }
        }
        if !matches!(name, "find" | "inspect" | "bundle" | "export") {
            assert!(reply.get("structuredContent").is_none());
        }
    }
}

#[test]
fn mcp_images_keep_the_exported_format_and_bytes() {
    let fixture = Fixture::new();
    for (extension, mime) in [
        ("jpg", "image/jpeg"),
        ("webp", "image/webp"),
        ("avif", "image/avif"),
    ] {
        let out = fixture.0.join(&format!("image.{extension}"));
        let reply = call_tool(
            &serde_json::json!({"name": "export", "arguments": {
                "file": fixture.file(), "node": "0:1", "out": out,
            }}),
            None,
        );
        assert_eq!(reply["isError"], false, "{reply}");
        let bytes = std::fs::read(out).unwrap();
        assert_eq!(reply["structuredContent"]["bytes"], bytes.len());
        assert_eq!(reply["content"][1]["mimeType"], mime);
        assert_eq!(reply["content"][1]["data"], figlens::render::base64(&bytes));
    }
}

#[test]
fn mcp_exports_fit_max_size_unless_disabled() {
    let fixture = Fixture::new();
    // The fixture frame is 16x8; scale 200 would be 3200 wide.
    for (max_size, width) in [(None, 1600), (Some(0), 3200), (Some(400), 400)] {
        let out = fixture.0.join("sized.png");
        let mut arguments = serde_json::json!({
            "file": fixture.file(), "node": "0:1", "out": out, "scale": 200,
        });
        if let Some(max_size) = max_size {
            arguments["maxSize"] = max_size.into();
        }
        let reply = call_tool(
            &serde_json::json!({"name": "export", "arguments": arguments}),
            None,
        );
        assert_eq!(reply["isError"], false, "{reply}");
        assert_eq!(
            reply["structuredContent"]["width"], width,
            "maxSize {max_size:?}"
        );
    }
    let mut text = String::new();
    let out = fixture.0.join("cli.png");
    run(
        vec![
            "export".into(),
            fixture.file(),
            "0:1".into(),
            out.to_string_lossy().into_owned(),
            "--scale".into(),
            "200".into(),
        ],
        &mut text,
    )
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(metadata["width"], 3200);
}

#[test]
fn svg_exports_directly_through_cli_and_mcp() {
    let fixture = Fixture::new();
    let out = fixture.0.join("frame.svg");
    let mut text = String::new();
    run(
        vec![
            "export".into(),
            fixture.file(),
            "0:1".into(),
            out.display().to_string(),
            "--scale".into(),
            "2".into(),
        ],
        &mut text,
    )
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(metadata["width"], 16);
    assert_eq!(metadata["height"], 8);
    let svg = std::fs::read_to_string(&out).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.contains("viewBox=\"0 0 16 8\""));
    let parsed = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
    assert_eq!(parsed.size().width(), 16.0);
    assert_eq!(parsed.size().height(), 8.0);
    let reply = call_tool(
        &serde_json::json!({"name": "export", "arguments": {
            "file": fixture.file(), "node": "0:1", "out": out,
        }}),
        None,
    );
    assert_eq!(reply["isError"], false, "{reply}");
    assert_eq!(reply["structuredContent"]["bytes"], svg.len());
    assert_eq!(reply["structuredContent"], metadata);
    assert_eq!(reply["content"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), svg);
    let error = run(
        vec![
            "export".into(),
            fixture.file(),
            "0:1".into(),
            out.display().to_string(),
            "--max-kb".into(),
            "0".into(),
        ],
        &mut String::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("SVG comes to"), "{error}");
    assert_eq!(std::fs::read_to_string(out).unwrap(), svg);
}

#[test]
fn discovery_rejects_invalid_bounds_in_cli_and_mcp() {
    let fixture = Fixture::new();
    for (name, cli, arguments, error) in [
        (
            "find",
            vec!["  "],
            serde_json::json!({"query": "  "}),
            "non-empty query",
        ),
        (
            "find",
            vec!["frame", "--limit", "0"],
            serde_json::json!({"query": "frame", "limit": 0}),
            "--limit takes 1-200",
        ),
        (
            "inspect",
            vec!["0:1", "--depth", "11"],
            serde_json::json!({"node": "0:1", "depth": 11}),
            "--depth takes 0-10",
        ),
        (
            "find",
            vec!["frame", "--node", "9:9"],
            serde_json::json!({"query": "frame", "node": "9:9"}),
            "node 9:9 not found",
        ),
    ] {
        let mut argv = vec![name.to_owned(), fixture.file()];
        argv.extend(cli.into_iter().map(str::to_owned));
        assert!(
            run(argv, &mut String::new())
                .unwrap_err()
                .to_string()
                .contains(error)
        );
        let mut arguments = arguments;
        arguments["file"] = fixture.file().into();
        let reply = call_tool(
            &serde_json::json!({"name": name, "arguments": arguments}),
            None,
        );
        assert_eq!(reply["isError"], true);
        assert!(
            reply["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(error)
        );
        assert_eq!(reply["content"].as_array().unwrap().len(), 1);
        assert!(reply.get("structuredContent").is_none());
    }
}

#[test]
fn variables_commands_distinguish_local_variables_from_frame_bindings() {
    use super::Command;
    use figlens::Tree;
    use kiwi_schema::Value;
    use std::collections::HashMap;
    use std::path::Path;

    let obj = |fields: Vec<(&'static str, Value<'static>)>| {
        Value::Object("Test", fields.into_iter().collect())
    };
    let guid = |id| {
        obj(vec![
            ("sessionID", Value::UInt(0)),
            ("localID", Value::UInt(id)),
        ])
    };
    let reference = |id| obj(vec![("guid", guid(id))]);
    let frame = obj(vec![
        ("guid", guid(1)),
        ("type", Value::Enum("NodeType", "FRAME")),
        ("parentIndex", reference(2)),
        (
            "parameterConsumptionMap",
            obj(vec![(
                "entries",
                Value::Array(vec![obj(vec![
                    (
                        "variableField",
                        Value::Enum("VariableField", "STACK_SPACING"),
                    ),
                    (
                        "variableData",
                        obj(vec![
                            ("dataType", Value::Enum("VariableDataType", "ALIAS")),
                            ("value", obj(vec![("alias", reference(4))])),
                        ]),
                    ),
                ])]),
            )]),
        ),
    ]);
    let canvas = obj(vec![("type", Value::Enum("NodeType", "CANVAS"))]);
    let set = obj(vec![
        ("guid", guid(3)),
        ("type", Value::Enum("NodeType", "VARIABLE_SET")),
        ("name", Value::String("Spacing".into())),
        (
            "variableSetModes",
            Value::Array(vec![obj(vec![
                ("id", guid(6)),
                ("name", Value::String("Default".into())),
            ])]),
        ),
    ]);
    let variable = |id, name: &str, position: &str, number| {
        obj(vec![
            ("guid", guid(id)),
            ("type", Value::Enum("NodeType", "VARIABLE")),
            ("name", Value::String(name.into())),
            ("sortPosition", Value::String(position.into())),
            (
                "variableResolvedType",
                Value::Enum("VariableResolvedType", "FLOAT"),
            ),
            ("variableSetID", reference(3)),
            (
                "variableDataValues",
                obj(vec![(
                    "entries",
                    Value::Array(vec![obj(vec![
                        ("modeID", guid(6)),
                        (
                            "variableData",
                            obj(vec![
                                ("dataType", Value::Enum("VariableDataType", "FLOAT")),
                                ("value", obj(vec![("floatValue", Value::Float(number))])),
                            ]),
                        ),
                    ])]),
                )]),
            ),
        ])
    };
    let used = variable(4, "Used", "a", 8.0);
    let unused = variable(5, "Unused", "b", 16.0);
    let tree = Tree {
        nodes: HashMap::from([
            ("0:1".into(), &frame),
            ("0:2".into(), &canvas),
            ("0:3".into(), &set),
            ("0:4".into(), &used),
            ("0:5".into(), &unused),
        ]),
        children: HashMap::from([
            ("0:0".into(), vec!["0:2".into()]),
            (
                "0:2".into(),
                vec!["0:1".into(), "0:3".into(), "0:4".into(), "0:5".into()],
            ),
        ]),
        blobs: Vec::new(),
    };
    for (cli, arguments, expected) in [
        (
            vec![],
            serde_json::json!({}),
            vec!["VariableID:0:4", "VariableID:0:5"],
        ),
        (
            vec!["0-1".into()],
            serde_json::json!({"node": "0-1"}),
            vec!["VariableID:0:4"],
        ),
    ] {
        for command in [
            Command::from_cli("variables", &cli, &tree).unwrap(),
            Command::from_tool("variables", &arguments, &tree).unwrap(),
        ] {
            let output = command.execute(&tree, Path::new("unused.fig")).unwrap();
            let value = output.json().unwrap();
            let ids: Vec<_> = value["variables"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, expected);
            assert_eq!(value["variables"][0]["valuesByMode"]["0:6"], 8.0);
            assert_eq!(value["collections"][0]["id"], "VariableCollectionId:0:3");
            assert_eq!(value["missingVariables"], serde_json::json!([]));
        }
    }
}

#[test]
fn cli_and_mcp_preserve_validation_order_and_errors() {
    let fixture = Fixture::new();
    for (name, cli, mut arguments, expected) in [
        (
            "nodes",
            vec!["9-9"],
            serde_json::json!({"node": "9-9"}),
            "node 9:9 not found; list ids with `figlens tree <file.fig> --depth 2`",
        ),
        (
            "variables",
            vec!["9-9"],
            serde_json::json!({"node": "9-9"}),
            "node 9:9 not found; list ids with `figlens tree <file.fig> --depth 2`",
        ),
        (
            "bundle",
            vec!["9-9", "output.zip", "--name", "ignored"],
            serde_json::json!({"node": "9-9", "out": "output.zip", "name": "ignored"}),
            "node 9:9 not found; list ids with `figlens tree <file.fig> --depth 2`",
        ),
        (
            "export",
            vec!["9-9;1:1", "output.txt", "--quality", "0"],
            serde_json::json!({"node": "9-9;1:1", "out": "output.txt", "quality": 0}),
            "node 9:9;1:1 not found; list ids with `figlens tree <file.fig> --depth 2`",
        ),
        (
            "export",
            vec!["0-1", "output.txt", "--quality", "0"],
            serde_json::json!({"node": "0-1", "out": "output.txt", "quality": 0}),
            "the output must end in .png, .jpg, .webp, .avif or .svg",
        ),
        (
            "export",
            vec!["0-1", "output.png", "--quality", "0"],
            serde_json::json!({"node": "0-1", "out": "output.png", "quality": 0}),
            "--quality takes 1-100 and --scale a positive number",
        ),
        (
            "tree",
            vec!["--depth", "invalid"],
            serde_json::json!({"depth": "invalid"}),
            "invalid digit found in string",
        ),
    ] {
        arguments["file"] = fixture.file().into();
        let mut argv = vec![name.into(), fixture.file()];
        argv.extend(cli.into_iter().map(str::to_owned));
        let mut text = String::new();
        let error = run(argv, &mut text).unwrap_err().to_string();
        assert_eq!(error, expected);
        assert!(text.is_empty());
        let reply = call_tool(
            &serde_json::json!({"name": name, "arguments": arguments}),
            None,
        );
        assert_eq!(reply["isError"], true);
        assert_eq!(reply["content"][0]["text"], expected);
        assert!(reply.get("structuredContent").is_none());
    }
}

#[test]
fn preview_snapshot_embeds_design_safely_and_never_overwrites_source() {
    let fixture = Fixture::named("</ScRiPt><script>globalThis.pwned=1</script>&");
    let output = fixture.0.join("preview.html");
    let mut result = String::new();
    run(
        vec![
            "preview".into(),
            fixture.file(),
            "0:1".into(),
            "--out".into(),
            output.to_string_lossy().into_owned(),
        ],
        &mut result,
    )
    .unwrap();
    let page = std::fs::read_to_string(&output).unwrap();
    assert!(!page.contains("</ScRiPt>"));
    let boot = page
        .split("<script id=\"boot\" type=\"application/json\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(boot).unwrap();
    assert_eq!(
        data["snapshot"]["nodes"]["document"]["name"],
        "</ScRiPt><script>globalThis.pwned=1</script>&"
    );
    assert!(
        data["snapshot"]["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert!(
        data["snapshot"]["bundle"]
            .as_str()
            .unwrap()
            .starts_with("UEs")
    );
    let source = fixture.0.join("source.html");
    std::fs::copy(fixture.file(), &source).unwrap();
    let before = std::fs::read(&source).unwrap();
    assert!(figlens::preview::snapshot(&source, "0:1", &source).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), before);
    assert!(
        figlens::preview::snapshot(
            std::path::Path::new(&fixture.file()),
            "0:1",
            &fixture.0.join("input.fig")
        )
        .is_err()
    );
}

#[test]
fn mcp_preserves_permissive_argument_coercions() {
    let fixture = Fixture::new();
    for (cli, mut arguments) in [
        (vec!["--json"], serde_json::json!({"node": "--json"})),
        (vec!["--depth", "0"], serde_json::json!({"depth": "0"})),
        (
            vec![],
            serde_json::json!({"node": false, "depth": null, "json": false}),
        ),
    ] {
        arguments["file"] = fixture.file().into();
        let mut argv = vec!["tree".into(), fixture.file()];
        argv.extend(cli.into_iter().map(str::to_owned));
        let mut text = String::new();
        run(argv, &mut text).unwrap();
        let reply = call_tool(
            &serde_json::json!({"name": "tree", "arguments": arguments}),
            None,
        );
        assert_eq!(reply["isError"], false);
        assert_eq!(reply["content"][0]["text"], text);
        assert!(reply.get("structuredContent").is_none());
    }
    let reply = call_tool(
        &serde_json::json!({
            "name": "export", "arguments": { "file": fixture.file(), "node": "0:1",
                "out": fixture.0.join("coerced.png"), "scale": "2x" }
        }),
        None,
    );
    assert_eq!(reply["isError"], false, "{reply}");
    assert_eq!(reply["structuredContent"]["width"], 32);
    assert_eq!(reply["structuredContent"]["height"], 16);
    assert_eq!(reply["content"][1]["mimeType"], "image/png");
    assert_eq!(
        reply["content"][1]["data"],
        figlens::render::base64(&std::fs::read(fixture.0.join("coerced.png")).unwrap())
    );
}
