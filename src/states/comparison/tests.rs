use super::*;
use crate::states::tests::{instance, message, node, obj};

fn size(w: f32) -> K<'static> {
    obj(vec![("x", K::Float(w)), ("y", K::Float(20.0))])
}

fn text(value: &str) -> K<'static> {
    obj(vec![("characters", K::String(value.into()))])
}

fn paint(hash: &str) -> K<'static> {
    K::Array(vec![obj(vec![
        ("type", K::Enum("Paint", "IMAGE")),
        ("image", obj(vec![("hash", K::String(hash.into()))])),
    ])])
}

#[test]
fn variant_comparison_keeps_root_changes_and_unpaired_descendant_multisets() {
    let message = message(vec![
        node(
            10,
            40,
            "SYMBOL",
            vec![
                ("size", size(40.0)),
                ("fillPaints", paint("red")),
                (
                    "transform",
                    obj(vec![("m02", K::Float(100.0)), ("m12", K::Float(300.0))]),
                ),
            ],
        ),
        node(
            11,
            40,
            "SYMBOL",
            vec![
                ("size", size(80.0)),
                ("visible", K::Bool(false)),
                ("fillPaints", paint("blue")),
            ],
        ),
        node(
            70,
            10,
            "TEXT",
            vec![
                ("name", K::String("Label".into())),
                ("textData", text("Submit")),
            ],
        ),
        node(
            71,
            11,
            "TEXT",
            vec![
                ("name", K::String("Label".into())),
                ("textData", text("Loading")),
                ("visible", K::Bool(false)),
            ],
        ),
        node(72, 10, "RECTANGLE", vec![("fillPaints", paint("red"))]),
        node(
            73,
            11,
            "RECTANGLE",
            vec![("fillPaints", paint("blue")), ("size", size(30.0))],
        ),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("left.fig")).unwrap();
    assert_eq!(result["componentSetId"], "0:40");
    assert_eq!(result["state"], "differences-recorded");
    let changes = result["rootChanges"].as_array().unwrap();
    let visibility = changes.iter().find(|c| c["field"] == "visible").unwrap();
    assert_eq!(visibility["before"]["state"], "absent");
    assert_eq!(visibility["after"]["value"], false);
    assert_eq!(visibility["after"]["sourceNodeId"], "0:11");
    assert!(changes.iter().any(|c| c["field"] == "size"));
    assert!(changes.iter().any(|c| c["field"] == "fillPaints"));
    assert!(!result.to_string().contains("m02"));
    // The only TEXT named Label on each side pairs; its changes are reported per pair.
    let pairs = result["pairedNodes"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (&pairs[0]["leftNodeId"], &pairs[0]["rightNodeId"]),
        (&json!("0:70"), &json!("0:71"))
    );
    let pair = pairs[0]["changes"].as_array().unwrap();
    let texts = pair
        .iter()
        .find(|c| c["field"] == "textData.characters")
        .unwrap();
    assert_eq!(
        (&texts["before"]["value"], &texts["after"]["value"]),
        (&json!("Submit"), &json!("Loading"))
    );
    assert_eq!(
        pair.iter().find(|c| c["field"] == "visible").unwrap()["before"]["state"],
        "absent"
    );
    let collections = result["collectionChanges"].as_array().unwrap();
    assert!(
        !collections
            .iter()
            .any(|c| c["field"] == "textData.characters")
    );
    assert!(collections.iter().any(|c| c["field"] == "size"));
    let fills = collections
        .iter()
        .find(|c| c["field"] == "fillPaints")
        .unwrap();
    assert_eq!(fills["pairing"], "not-inferred");
    assert_eq!(result["unpairedNodes"].as_array().unwrap().len(), 2);
    assert!(
        result["unpairedNodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["state"] == "not-paired")
    );
    assert_eq!(result["truncated"], false);
    let other = compare_variants(&tree, "0:10", "0:11", Path::new("right.fig")).unwrap();
    assert_ne!(result["source"], other["source"]);
}

#[test]
fn left_and_top_padding_use_stored_fields_on_roots_and_descendants() {
    for field in ["stackHorizontalPadding", "stackVerticalPadding"] {
        for descendants in [false, true] {
            let mut nodes = vec![
                node(
                    10,
                    40,
                    "SYMBOL",
                    if descendants {
                        vec![]
                    } else {
                        vec![(field, K::Float(8.0))]
                    },
                ),
                node(
                    11,
                    40,
                    "SYMBOL",
                    if descendants {
                        vec![]
                    } else {
                        vec![(field, K::Float(16.0))]
                    },
                ),
            ];
            if descendants {
                nodes.push(node(70, 10, "FRAME", vec![(field, K::Float(8.0))]));
                nodes.push(node(71, 11, "FRAME", vec![(field, K::Float(16.0))]));
            }
            let message = message(nodes);
            let tree = Tree::new(&message);
            let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
            assert_eq!(result["state"], "differences-recorded");
            let changes = result[if descendants {
                "collectionChanges"
            } else {
                "rootChanges"
            }]
            .as_array()
            .unwrap();
            let change = changes.iter().find(|c| c["field"] == field).unwrap();
            if descendants {
                assert_eq!(change["before"][0]["value"], 8.0);
                assert_eq!(change["after"][0]["value"], 16.0);
                assert_eq!(change["before"][0]["sources"][0]["sourceField"], field);
                assert_eq!(change["after"][0]["sources"][0]["sourceField"], field);
            } else {
                assert_eq!(change["before"]["value"], 8.0);
                assert_eq!(change["after"]["value"], 16.0);
                assert_eq!(change["before"]["sourceField"], field);
                assert_eq!(change["after"]["sourceField"], field);
            }
            assert_eq!(result["truncated"], false);
        }
    }
}

#[test]
fn variant_comparison_refuses_instances_unrelated_frames_and_different_sets() {
    let message = message(vec![
        node(80, 0, "FRAME", vec![("isStateGroup", K::Bool(true))]),
        node(81, 80, "SYMBOL", vec![]),
        node(82, 0, "SYMBOL", vec![]),
    ]);
    let tree = Tree::new(&message);
    let error = compare_variants(
        &tree,
        &"x".repeat(MAX_ID_BYTES + 1),
        "0:11",
        Path::new("test.fig"),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<FigError>().unwrap().kind,
        ErrorKind::InvalidArguments
    );
    for (a, b) in [
        ("0:2", "0:11"),
        ("0:10", "0:60"),
        ("0:10", "0:81"),
        ("0:10", "0:82"),
        ("0:10", "0:10"),
        ("0:10", "9:9"),
    ] {
        assert!(
            compare_variants(&tree, a, b, Path::new("test.fig")).is_err(),
            "{a} {b}"
        );
    }
}

#[test]
fn incomplete_and_unpaired_comparisons_never_claim_no_differences() {
    let message = message(vec![
        node(10, 40, "SYMBOL", vec![("size", size(20.0))]),
        node(11, 40, "SYMBOL", vec![("size", size(20.0))]),
        node(70, 10, "TEXT", vec![("textData", text("same"))]),
        node(71, 11, "TEXT", vec![("textData", text("same"))]),
        instance(72, 10, 999),
        instance(73, 11, 999),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert_eq!(result["state"], "unknown");
    assert_eq!(result["coverage"]["nestedInstancesNotExpanded"], 2);
    assert!(result["rootChanges"].as_array().unwrap().is_empty());
    assert!(result["collectionChanges"].as_array().unwrap().is_empty());
    let message = super::tests::message(vec![
        node(10, 40, "SYMBOL", vec![("size", size(20.0))]),
        node(11, 40, "SYMBOL", vec![("size", size(20.0))]),
    ]);
    let tree = Tree::new(&message);
    assert_eq!(
        compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap()["state"],
        "no-differences-in-compared-fields"
    );
}

#[test]
fn comparison_limits_and_unsupported_fields_are_explicit() {
    let mut extra = vec![
        node(
            10,
            40,
            "SYMBOL",
            vec![(
                "fillPaints",
                K::Array(vec![obj(vec![("opacity", K::Float(f32::NAN))])]),
            )],
        ),
        node(11, 40, "SYMBOL", vec![("size", K::String("bad".into()))]),
        node(
            70,
            10,
            "TEXT",
            vec![("textData", text(&"x".repeat(MAX_VALUE_BYTES + 1)))],
        ),
    ];
    extra.extend((100..400).map(|id| node(id, 11, "RECTANGLE", vec![])));
    let message = message(extra);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert_eq!(result["truncated"], true);
    assert!(!result["unknownFields"].as_array().unwrap().is_empty());
    assert!(result["coverage"]["scannedRight"].as_u64().unwrap() <= MAX_NODES as u64);
    assert!(result["coverage"]["workItems"].as_u64().unwrap() <= MAX_WORK as u64);
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_BYTES);
    assert_eq!(result["state"], "unknown");
    assert!(
        result["collectionChanges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["state"] == "unknown")
    );
}

#[test]
fn malformed_containers_and_limited_values_are_not_absent_or_equal() {
    let mut deep = K::Bool(true);
    for _ in 0..20 {
        deep = obj(vec![("nested", deep)]);
    }
    let message = message(vec![
        node(10, 40, "SYMBOL", vec![]),
        node(11, 40, "SYMBOL", vec![]),
        node(
            70,
            10,
            "TEXT",
            vec![("textData", K::String("bad container".into()))],
        ),
        node(71, 11, "TEXT", vec![("textData", text("Loading"))]),
        node(72, 10, "RECTANGLE", vec![("effects", K::Array(vec![deep]))]),
        node(
            73,
            11,
            "RECTANGLE",
            vec![("fillPaints", K::Array(vec![K::Bool(false)]))],
        ),
        node(74, 11, "TEXT", vec![("type", K::String("TEXT".into()))]),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    let unknown = result["unknownFields"].as_array().unwrap();
    assert!(unknown.iter().any(|c| c["sourceNodeId"] == "0:70"
        && c["sourceField"] == "textData.characters"
        && c["reason"] == "malformed_field_container"));
    assert!(
        unknown
            .iter()
            .any(|c| c["sourceNodeId"] == "0:72" && c["sourceField"] == "effects")
    );
    assert!(
        unknown
            .iter()
            .any(|c| c["sourceNodeId"] == "0:73" && c["reason"] == "malformed_list_entry")
    );
    let texts = result["collectionChanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "textData.characters")
        .unwrap();
    assert_eq!(texts["state"], "unknown");
    assert!(
        unknown
            .iter()
            .any(|c| c["sourceNodeId"] == "0:74" && c["sourceField"] == "type")
    );
    assert_eq!(result["state"], "unknown");
}

#[test]
fn descendant_value_multiplicity_is_compared_without_pairing_names() {
    let message = message(vec![
        node(10, 40, "SYMBOL", vec![]),
        node(11, 40, "SYMBOL", vec![]),
        node(70, 10, "TEXT", vec![("textData", text("Label"))]),
        node(71, 10, "TEXT", vec![("textData", text("Label"))]),
        node(72, 11, "TEXT", vec![("textData", text("Label"))]),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    let text = result["collectionChanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "textData.characters")
        .unwrap();
    assert_eq!(text["state"], "differences-recorded");
    assert_eq!(text["before"][0]["sources"].as_array().unwrap().len(), 2);
    assert_eq!(text["after"][0]["sources"].as_array().unwrap().len(), 1);
    assert_eq!(result["unpairedNodes"].as_array().unwrap().len(), 3);
}

#[test]
fn image_hash_bytes_are_preserved_and_canvas_translation_is_ignored() {
    let paints = |hash| K::Array(vec![obj(vec![("image", obj(vec![("hash", hash)]))])]);
    let message = message(vec![
        node(
            10,
            40,
            "SYMBOL",
            vec![
                ("fillPaints", paints(K::Bytes(vec![1, 2]))),
                (
                    "transform",
                    obj(vec![("m00", K::Float(1.0)), ("m02", K::Float(100.0))]),
                ),
            ],
        ),
        node(
            11,
            40,
            "SYMBOL",
            vec![
                ("fillPaints", paints(K::Array(vec![K::Byte(1), K::Byte(2)]))),
                (
                    "transform",
                    obj(vec![("m00", K::Float(1.0000001)), ("m02", K::Float(300.0))]),
                ),
            ],
        ),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert_eq!(result["state"], "no-differences-in-compared-fields");
    assert_eq!(result["truncated"], false);
    assert!(result["unknownFields"].as_array().unwrap().is_empty());
    let mut budget = Budget {
        work: 0,
        bytes: MAX_BYTES,
        reasons: Default::default(),
    };
    let before = field(tree.nodes["0:10"], "fillPaints", "0:10", &mut budget);
    assert_eq!(before["value"][0]["image"]["hash"], "0102");
}

#[test]
fn ambiguous_names_stay_unpaired_and_bound_variables_are_named() {
    let named = |name: &str| ("name", K::String(name.into()));
    let bound = |variable: u32| {
        K::Array(vec![obj(vec![
            ("type", K::Enum("Paint", "SOLID")),
            (
                "colorVar",
                obj(vec![
                    ("dataType", K::Enum("VariableDataType", "ALIAS")),
                    (
                        "value",
                        obj(vec![(
                            "alias",
                            obj(vec![(
                                "guid",
                                obj(vec![
                                    ("sessionID", K::UInt(0)),
                                    ("localID", K::UInt(variable)),
                                ]),
                            )]),
                        )]),
                    ),
                ]),
            ),
        ])])
    };
    let message = message(vec![
        node(10, 40, "SYMBOL", vec![("fillPaints", bound(90))]),
        node(11, 40, "SYMBOL", vec![("fillPaints", bound(91))]),
        node(70, 10, "TEXT", vec![named("Label")]),
        node(71, 10, "TEXT", vec![named("Label")]),
        node(72, 11, "TEXT", vec![named("Label")]),
        node(90, 1, "VARIABLE", vec![named("color/bg/brand-soft")]),
        node(91, 1, "VARIABLE", vec![named("color/bg/brand")]),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert_eq!(result["pairedNodes"], json!([]));
    assert_eq!(result["unpairedNodes"].as_array().unwrap().len(), 3);
    let fill = result["rootChanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "fillPaints")
        .unwrap();
    let var = |side: &str| fill[side]["value"][0]["colorVar"].clone();
    assert_eq!(
        (&var("before")["variableId"], &var("before")["variableName"]),
        (&json!("VariableID:0:90"), &json!("color/bg/brand-soft"))
    );
    assert_eq!(var("after")["variableName"], "color/bg/brand");
    assert_eq!(var("after")["variableNameTruncated"], false);
}

#[test]
fn long_bound_variable_names_are_cut_and_charged() {
    let long = "v".repeat(MAX_VALUE_BYTES);
    let alias = |variable: u32| {
        K::Array(vec![obj(vec![
            ("type", K::Enum("Paint", "SOLID")),
            (
                "colorVar",
                obj(vec![
                    ("dataType", K::Enum("VariableDataType", "ALIAS")),
                    (
                        "value",
                        obj(vec![(
                            "alias",
                            obj(vec![(
                                "guid",
                                obj(vec![
                                    ("sessionID", K::UInt(0)),
                                    ("localID", K::UInt(variable)),
                                ]),
                            )]),
                        )]),
                    ),
                ]),
            ),
        ])])
    };
    let mut nodes = vec![
        node(10, 40, "SYMBOL", vec![]),
        node(11, 40, "SYMBOL", vec![]),
        node(90, 1, "VARIABLE", vec![("name", K::String(long.clone()))]),
    ];
    nodes.extend((100..300).map(|id| {
        node(
            id,
            10 + id % 2,
            "RECTANGLE",
            vec![("fillPaints", alias(90))],
        )
    }));
    let message = message(nodes);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert!(!result.to_string().contains(&long));
    assert!(
        result
            .to_string()
            .contains("\"variableNameTruncated\":true")
    );
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_BYTES);
    assert_eq!(result["truncated"], true);
    // Identical bindings never become a difference because only one side was named.
    assert_ne!(result["state"], "differences-recorded");
    assert!(
        result["unknownFields"]
            .to_string()
            .contains("variable_name_limit")
    );
}

#[test]
fn library_aliases_are_named_from_cached_copies() {
    let s = |v: &str| K::String(v.into());
    let fill_of = |key: &str, version: &str| {
        let alias = obj(vec![(
            "assetRef",
            obj(vec![("key", s(key)), ("version", s(version))]),
        )]);
        K::Array(vec![obj(vec![
            ("type", K::Enum("Paint", "SOLID")),
            (
                "colorVar",
                obj(vec![
                    ("dataType", K::Enum("VariableDataType", "ALIAS")),
                    ("value", obj(vec![("alias", alias)])),
                ]),
            ),
        ])])
    };
    let fill = fill_of("vk", "3:4");
    let message = message(vec![
        node(10, 40, "SYMBOL", vec![]),
        node(11, 40, "SYMBOL", vec![("fillPaints", fill)]),
        node(
            90,
            1,
            "VARIABLE",
            vec![
                ("name", s("color/bg/brand")),
                ("key", s("vk")),
                ("version", s("3:4")),
            ],
        ),
    ]);
    let tree = Tree::new(&message);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    let fill = result["rootChanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "fillPaints")
        .unwrap();
    let var = &fill["after"]["value"][0]["colorVar"];
    assert_eq!(
        (&var["variableId"], &var["variableName"]),
        (&json!("VariableID:vk/3:4"), &json!("color/bg/brand"))
    );
    // Another cached version of the key still names the alias, as `variables` resolves it.
    let other = super::tests::message(vec![
        node(10, 40, "SYMBOL", vec![]),
        node(11, 40, "SYMBOL", vec![("fillPaints", fill_of("vk", "3:4"))]),
        node(
            90,
            1,
            "VARIABLE",
            vec![
                ("name", s("color/bg/old")),
                ("key", s("vk")),
                ("version", s("1:1")),
            ],
        ),
    ]);
    let tree = Tree::new(&other);
    let result = compare_variants(&tree, "0:10", "0:11", Path::new("test.fig")).unwrap();
    assert!(
        result
            .to_string()
            .contains("\"variableName\":\"color/bg/old\"")
    );
}

fn named(
    id: u32,
    parent: u32,
    kind: &'static str,
    name: &str,
    extra: Vec<(&'static str, K<'static>)>,
) -> K<'static> {
    let mut fields = vec![("name", K::String(name.into()))];
    fields.extend(extra);
    node(id, parent, kind, fields)
}

fn at(x: f32, y: f32) -> K<'static> {
    obj(vec![("m02", K::Float(x)), ("m12", K::Float(y))])
}

#[test]
fn plain_frames_report_changed_fields_root_delta_and_added_children() {
    let message = message(vec![
        named(
            20,
            1,
            "FRAME",
            "Empty",
            vec![("size", size(100.0)), ("transform", at(10.0, 20.0))],
        ),
        named(
            21,
            1,
            "FRAME",
            "Filled",
            vec![("size", size(150.0)), ("transform", at(10.0, 50.0))],
        ),
        named(
            70,
            20,
            "TEXT",
            "Title",
            vec![("textData", text("No items"))],
        ),
        named(71, 21, "TEXT", "Title", vec![("textData", text("3 items"))]),
        named(
            72,
            20,
            "RECTANGLE",
            "Bg",
            vec![("fillPaints", paint("red")), ("size", size(10.0))],
        ),
        named(
            73,
            21,
            "RECTANGLE",
            "Bg",
            vec![("fillPaints", paint("blue")), ("size", size(30.0))],
        ),
        named(74, 20, "RECTANGLE", "Icon", vec![]),
        named(
            75,
            21,
            "RECTANGLE",
            "Icon",
            vec![("visible", K::Bool(false))],
        ),
        named(76, 21, "RECTANGLE", "Badge", vec![]),
    ]);
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    assert!(result.get("componentSetId").is_none());
    assert_eq!(result["state"], "differences-recorded");
    let delta = &result["rootDelta"];
    assert_eq!(
        (&delta["width"], &delta["height"], &delta["x"], &delta["y"]),
        (&json!(50.0), &json!(0.0), &json!(0.0), &json!(30.0))
    );
    let fields = |name: &str| -> Vec<String> {
        let pair = result["pairedNodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap();
        pair["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["field"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(fields("Title"), ["textData.characters"]);
    assert_eq!(fields("Bg"), ["size", "fillPaints"]);
    assert_eq!(fields("Icon"), ["visible"]);
    let unpaired = result["unpairedNodes"].as_array().unwrap();
    assert_eq!(unpaired.len(), 1);
    assert_eq!(unpaired[0]["name"], "Badge");
    assert_eq!(unpaired[0]["reason"], "no-counterpart");
}

#[test]
fn duplicate_names_are_unpaired_as_ambiguous_never_guessed() {
    let message = message(vec![
        named(20, 1, "FRAME", "A", vec![]),
        named(21, 1, "FRAME", "B", vec![]),
        named(70, 20, "TEXT", "Row", vec![("textData", text("one"))]),
        named(71, 20, "TEXT", "Row", vec![("textData", text("two"))]),
        named(72, 21, "TEXT", "Row", vec![("textData", text("one"))]),
        named(73, 21, "TEXT", "Only", vec![]),
    ]);
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    assert!(result["pairedNodes"].as_array().unwrap().is_empty());
    let reasons: Vec<_> = result["unpairedNodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["name"].as_str().unwrap(), n["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(
        reasons
            .iter()
            .filter(|r| **r == ("Row", "ambiguous-name"))
            .count(),
        3
    );
    assert!(reasons.contains(&("Only", "no-counterpart")));
    assert_ne!(result["state"], "no-differences-in-compared-fields");
}

#[test]
fn frame_comparison_is_bounded_and_rejects_bad_selections() {
    let mut extra = vec![
        named(20, 1, "FRAME", "A", vec![]),
        named(21, 1, "FRAME", "B", vec![]),
    ];
    extra.extend((0..250).map(|i| named(100 + i, 20, "RECTANGLE", &format!("R{i}"), vec![])));
    let message = message(extra);
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    assert_eq!(result["truncated"], true);
    assert!(result["coverage"]["scannedLeft"].as_u64().unwrap() <= MAX_NODES as u64);
    for (a, b) in [("0:20", "0:20"), ("0:20", "9:9"), ("9:9", "0:20")] {
        assert!(
            compare_nodes(&tree, a, b, Path::new("test.fig")).is_err(),
            "{a} {b}"
        );
    }
    // Variants of one set still compare through the general entry point.
    assert!(compare_nodes(&tree, "0:10", "0:11", Path::new("test.fig")).is_ok());
}

fn paired_by(result: &Value) -> Vec<(String, String)> {
    result["pairedNodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["rightNodeId"].as_str().unwrap().to_owned(),
                p["pairedBy"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn instance_of(id: u32, parent: u32, name: &str, main: u32) -> K<'static> {
    let symbol = obj(vec![("symbolID", crate::states::tests::guid(main))]);
    named(id, parent, "INSTANCE", name, vec![("symbolData", symbol)])
}

#[test]
fn instances_of_one_main_component_pair_when_unique_even_if_renamed() {
    let message = message(vec![
        named(20, 1, "FRAME", "A", vec![]),
        named(21, 1, "FRAME", "B", vec![]),
        named(70, 20, "TEXT", "Title", vec![("textData", text("x"))]),
        named(71, 21, "TEXT", "Title", vec![("textData", text("x"))]),
        instance_of(72, 20, "Btn", 900),
        instance_of(73, 21, "Button", 900),
    ]);
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    let mut by = paired_by(&result);
    by.sort();
    assert_eq!(
        by,
        [
            ("0:71".to_owned(), "name".to_owned()),
            ("0:73".to_owned(), "component".to_owned())
        ]
    );
    assert!(result["unpairedNodes"].as_array().unwrap().is_empty());
}

#[test]
fn repeated_siblings_pair_by_order_only_with_equal_counts() {
    let rows = |third: bool| {
        let mut v = vec![
            named(20, 1, "FRAME", "A", vec![]),
            named(21, 1, "FRAME", "B", vec![]),
            named(70, 20, "TEXT", "Row", vec![("textData", text("one"))]),
            named(71, 20, "TEXT", "Row", vec![("textData", text("two"))]),
            named(72, 21, "TEXT", "Row", vec![("textData", text("one"))]),
            named(73, 21, "TEXT", "Row", vec![("textData", text("three"))]),
        ];
        if third {
            v.push(named(74, 21, "TEXT", "Row", vec![]));
        }
        v
    };
    let message = message(rows(false));
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    assert_eq!(
        paired_by(&result),
        [
            ("0:72".to_owned(), "order".to_owned()),
            ("0:73".to_owned(), "order".to_owned())
        ]
    );
    let changed: Vec<_> = result["pairedNodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| !p["changes"].as_array().unwrap().is_empty())
        .map(|p| p["rightNodeId"].as_str().unwrap())
        .collect();
    assert_eq!(changed, ["0:73"]);
    let message = super::tests::message(rows(true));
    let tree = Tree::new(&message);
    let result = compare_nodes(&tree, "0:20", "0:21", Path::new("test.fig")).unwrap();
    assert!(result["pairedNodes"].as_array().unwrap().is_empty());
    assert!(
        result["unpairedNodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["reason"] == "ambiguous-name")
    );
}

#[test]
fn a_node_is_never_compared_with_its_ancestor_or_descendant() {
    let message = message(vec![
        named(20, 1, "FRAME", "A", vec![]),
        named(70, 20, "FRAME", "Inner", vec![]),
        named(71, 70, "TEXT", "Deep", vec![]),
    ]);
    let tree = Tree::new(&message);
    for (a, b) in [
        ("0:20", "0:70"),
        ("0:70", "0:20"),
        ("0:20", "0:71"),
        ("0:71", "0:20"),
    ] {
        let error = compare_nodes(&tree, a, b, Path::new("test.fig"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("ancestor"), "{a} {b}: {error}");
    }
}
