use crate::{call_tool, run};
use std::io::Write as _;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        use kiwi_schema::{Def, DefKind, Field, Schema, TYPE_FLOAT, TYPE_STRING, TYPE_UINT, Value};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::path::PathBuf::from(format!(
            "samples/command-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
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
                            ("name", Value::String("Frame".into())),
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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
        let reply = call_tool(&serde_json::json!({"name": name, "arguments": arguments}));
        assert_eq!(reply["isError"], false, "{name}: {reply}");
        assert_eq!(reply["content"][0]["text"], text, "{name}");
        if let Some(expected) = expected {
            assert_eq!(text, expected);
        } else {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            match name {
                "nodes" => assert_eq!(value["document"]["id"], "0:1"),
                "variables" => assert!(value.is_object()),
                "export" => {
                    assert_eq!(
                        (value["width"].as_u64(), value["height"].as_u64()),
                        (Some(32), Some(16))
                    );
                    assert_eq!(value["quality"], 90);
                    assert_eq!(reply["structuredContent"], value);
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
                    assert!(std::fs::File::open(arguments["out"].as_str().unwrap()).is_ok());
                }
                _ => unreachable!(),
            }
        }
        if !matches!(name, "bundle" | "export") {
            assert!(reply.get("structuredContent").is_none());
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
            "the output must end in .png, .jpg, .webp or .avif",
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
        let reply = call_tool(&serde_json::json!({"name": name, "arguments": arguments}));
        assert_eq!(reply["isError"], true);
        assert_eq!(reply["content"][0]["text"], expected);
        assert!(reply.get("structuredContent").is_none());
    }
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
        let reply = call_tool(&serde_json::json!({"name": "tree", "arguments": arguments}));
        assert_eq!(reply["isError"], false);
        assert_eq!(reply["content"][0]["text"], text);
        assert!(reply.get("structuredContent").is_none());
    }
    let reply = call_tool(&serde_json::json!({
        "name": "export", "arguments": { "file": fixture.file(), "node": "0:1",
            "out": fixture.0.join("coerced.png"), "scale": "2x" }
    }));
    assert_eq!(reply["isError"], false, "{reply}");
    assert_eq!(reply["structuredContent"]["width"], 32);
    assert_eq!(reply["structuredContent"]["height"], 16);
}
