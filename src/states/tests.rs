use super::*;

fn obj(fields: Vec<(&'static str, Kiwi<'static>)>) -> Kiwi<'static> {
    Kiwi::Object("Test", fields.into_iter().collect())
}

fn guid(id: u32) -> Kiwi<'static> {
    obj(vec![
        ("sessionID", Kiwi::UInt(0)),
        ("localID", Kiwi::UInt(id)),
    ])
}

fn node(
    id: u32,
    parent: u32,
    kind: &'static str,
    extra: Vec<(&'static str, Kiwi<'static>)>,
) -> Kiwi<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", Kiwi::Enum("NodeType", kind)),
        ("name", Kiwi::String(format!("Node {id}"))),
        (
            "parentIndex",
            obj(vec![
                ("guid", guid(parent)),
                ("position", Kiwi::String(format!("{id:08}"))),
            ]),
        ),
        (
            "size",
            obj(vec![("x", Kiwi::Float(20.0)), ("y", Kiwi::Float(10.0))]),
        ),
    ];
    fields.extend(extra);
    obj(fields)
}

fn instance(id: u32, parent: u32, main: u32) -> Kiwi<'static> {
    node(
        id,
        parent,
        "INSTANCE",
        vec![("symbolData", obj(vec![("symbolID", guid(main))]))],
    )
}

fn spec(id: u32, value: &str) -> Kiwi<'static> {
    obj(vec![
        ("propDefId", guid(id)),
        ("value", Kiwi::String(value.into())),
    ])
}

fn message(mut extra: Vec<Kiwi<'static>>) -> Kiwi<'static> {
    let mut nodes = vec![
        node(1, 0, "FRAME", vec![]),
        instance(2, 1, 10),
        instance(3, 1, 10),
        node(
            10,
            40,
            "SYMBOL",
            vec![(
                "variantPropSpecs",
                Kiwi::Array(vec![spec(50, "Default"), spec(51, "Small")]),
            )],
        ),
        node(
            11,
            40,
            "SYMBOL",
            vec![
                ("name", Kiwi::String("Loading".into())),
                ("variantPropSpecs", Kiwi::Array(vec![spec(50, "Loading")])),
            ],
        ),
        node(
            12,
            40,
            "SYMBOL",
            vec![("name", Kiwi::String("Error".into()))],
        ),
        node(
            40,
            0,
            "FRAME",
            vec![
                ("isStateGroup", Kiwi::Bool(true)),
                (
                    "componentPropDefs",
                    Kiwi::Array(vec![
                        obj(vec![
                            ("id", guid(50)),
                            ("name", Kiwi::String("State".into())),
                            ("type", Kiwi::Enum("PropertyType", "VARIANT")),
                        ]),
                        obj(vec![
                            ("id", guid(51)),
                            ("name", Kiwi::String("Size".into())),
                            ("type", Kiwi::Enum("PropertyType", "VARIANT")),
                        ]),
                    ]),
                ),
            ],
        ),
        node(
            60,
            0,
            "FRAME",
            vec![("name", Kiwi::String("Loading".into()))],
        ),
    ];
    nodes.append(&mut extra);
    obj(vec![("nodeChanges", Kiwi::Array(nodes))])
}

fn inspect_root(tree: &Tree, id: &str) -> Value {
    let doc = crate::rest::resolve_nodes(tree, id).unwrap();
    inspect(tree, &doc["document"], Path::new("synthetic.fig"))
}

#[test]
fn malformed_property_reference_and_definition_guids_remain_unsupported() {
    for (invalid, collision) in [
        (
            obj(vec![
                ("sessionID", Kiwi::String("bad".into())),
                ("localID", Kiwi::UInt(50)),
            ]),
            50,
        ),
        (
            obj(vec![
                ("sessionID", Kiwi::UInt(0)),
                ("localID", Kiwi::String("bad".into())),
            ]),
            0,
        ),
    ] {
        for invalid_definition in [false, true] {
            let reference = if invalid_definition {
                guid(collision)
            } else {
                invalid.clone()
            };
            let definition = if invalid_definition {
                invalid.clone()
            } else {
                guid(50)
            };
            let message = message(vec![
                node(
                    10,
                    40,
                    "SYMBOL",
                    vec![(
                        "variantPropSpecs",
                        Kiwi::Array(vec![obj(vec![
                            ("propDefId", reference),
                            ("value", Kiwi::String("Default".into())),
                        ])]),
                    )],
                ),
                node(
                    11,
                    40,
                    "SYMBOL",
                    vec![("variantPropSpecs", Kiwi::Array(vec![spec(51, "Loading")]))],
                ),
                node(
                    40,
                    0,
                    "FRAME",
                    vec![
                        ("isStateGroup", Kiwi::Bool(true)),
                        (
                            "componentPropDefs",
                            Kiwi::Array(vec![
                                obj(vec![
                                    ("id", definition),
                                    ("name", Kiwi::String("State".into())),
                                    ("type", Kiwi::Enum("PropertyType", "VARIANT")),
                                ]),
                                obj(vec![
                                    ("id", guid(51)),
                                    ("name", Kiwi::String("Valid".into())),
                                    ("type", Kiwi::Enum("PropertyType", "VARIANT")),
                                ]),
                            ]),
                        ),
                    ],
                ),
            ]);
            let tree = Tree::new(&message);
            let result = inspect(
                &tree,
                &json!({"id":"0:40", "type":"COMPONENT_SET"}),
                Path::new("synthetic.fig"),
            );
            let variants = result["sets"][0]["variants"].as_array().unwrap();
            let first = &variants[0]["properties"]["entries"][0];
            assert_eq!(first["state"], "unsupported");
            if invalid_definition {
                assert_eq!(first["definitionId"], format!("0:{collision}"));
            } else {
                assert!(first["definitionId"].is_null());
            }
            assert!(first.get("value").is_none());
            assert_eq!(variants[1]["properties"]["entries"][0]["state"], "recorded");
        }
    }
}

#[test]
fn membership_properties_and_source_ids_are_explicit_and_deduplicated() {
    let message = message(vec![]);
    let tree = Tree::new(&message);
    let result = inspect_root(&tree, "0:1");
    assert_eq!(result["source"]["file"], "synthetic.fig");
    assert_eq!(result["sets"].as_array().unwrap().len(), 1);
    assert_eq!(result["instances"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["instances"][0]["currentProperties"][0]["value"],
        "Small"
    );
    let variants = result["sets"][0]["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 3);
    assert_eq!(
        variants[0]["properties"]["entries"][0]["definitionId"],
        "0:50"
    );
    assert_eq!(
        variants[0]["properties"]["entries"][0]["sourceNodeId"],
        "0:10"
    );
    assert_eq!(
        variants[0]["properties"]["entries"][0]["definitionNodeId"],
        "0:40"
    );
    assert_eq!(variants[2]["properties"]["state"], "absent");
    assert_eq!(result["triggerEvidence"]["state"], "absent");
    assert_eq!(result["truncated"], false);
    for variant in variants {
        let id = variant["nodeId"].as_str().unwrap();
        assert_eq!(
            crate::rest::resolve_nodes(&tree, id).unwrap()["document"]["id"],
            id
        );
        assert_ne!(id, "0:60");
    }
    for id in ["0:10", "0:40"] {
        assert_eq!(inspect_root(&tree, id)["sets"][0]["nodeId"], "0:40");
    }
    let doc = crate::rest::resolve_nodes(&tree, "0:1").unwrap();
    let other = inspect(&tree, &doc["document"], Path::new("other.fig"));
    assert_ne!(result["source"], other["source"]);
}

#[test]
fn missing_main_standalone_components_and_hidden_instances_keep_their_limits() {
    let message = message(vec![
        instance(4, 1, 999),
        instance(5, 1, 70),
        node(70, 0, "SYMBOL", vec![]),
        node(
            6,
            1,
            "INSTANCE",
            vec![
                ("visible", Kiwi::Bool(false)),
                ("symbolData", obj(vec![("symbolID", guid(10))])),
            ],
        ),
    ]);
    let tree = Tree::new(&message);
    let result = inspect_root(&tree, "0:1");
    let instances = result["instances"].as_array().unwrap();
    assert_eq!(instances[2]["state"], "missing_component");
    assert_eq!(instances[2]["componentId"], "0:999");
    assert_eq!(instances[3]["state"], "no_component_set");
    assert_eq!(instances[4]["visible"], false);
    assert_eq!(result["coverage"]["hiddenSubtreesNotExpanded"], true);
    assert_eq!(inspect_root(&tree, "0:60")["sets"], json!([]));
}

#[test]
fn nested_property_swaps_use_the_effective_source_without_replacing_instance_data() {
    let assignment = obj(vec![
        ("defID", guid(80)),
        (
            "varValue",
            obj(vec![(
                "value",
                obj(vec![("symbolIdValue", obj(vec![("guid", guid(11))]))]),
            )]),
        ),
    ]);
    let message = message(vec![
        node(
            71,
            10,
            "INSTANCE",
            vec![
                ("symbolData", obj(vec![("symbolID", guid(70))])),
                (
                    "componentPropRefs",
                    Kiwi::Array(vec![obj(vec![
                        ("defID", guid(80)),
                        (
                            "componentPropNodeField",
                            Kiwi::Enum("Field", "OVERRIDDEN_SYMBOL_ID"),
                        ),
                    ])]),
                ),
            ],
        ),
        node(70, 0, "SYMBOL", vec![]),
        node(
            2,
            1,
            "INSTANCE",
            vec![
                ("componentPropAssignments", Kiwi::Array(vec![assignment])),
                ("symbolData", obj(vec![("symbolID", guid(10))])),
            ],
        ),
    ]);
    let tree = Tree::new(&message);
    let doc = crate::rest::resolve_nodes(&tree, "0:2").unwrap();
    let before = doc.clone();
    let result = inspect(&tree, &doc["document"], Path::new("synthetic.fig"));
    let nested = &result["instances"][1];
    assert_eq!(nested["nodeId"], "I0:2;0:71");
    assert_eq!(nested["componentId"], "0:11");
    assert_eq!(nested["componentSetId"], "0:40");
    assert_eq!(nested["currentPropertiesState"], "unsupported");
    assert_eq!(doc, before);
    assert_eq!(
        inspect_root(&tree, "I0:2;0:71")["instances"][0]["componentId"],
        "0:11"
    );
}

#[test]
fn large_sets_keep_the_current_component_and_report_partial_membership() {
    let extra = (100..180)
        .map(|id| node(id, 40, "SYMBOL", vec![]))
        .collect();
    let message = message(extra);
    let tree = Tree::new(&message);
    let result = inspect_root(&tree, "0:179");
    let set = &result["sets"][0];
    assert_eq!(set["variants"][0]["nodeId"], "0:179");
    assert_eq!(set["variants"].as_array().unwrap().len(), MAX_SET_VARIANTS);
    assert_eq!(set["membershipComplete"], false);
    assert_eq!(result["truncated"], true);
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("variant_limit"))
    );
}

#[test]
fn work_instances_properties_and_output_bytes_are_bounded() {
    let message = message(vec![]);
    let tree = Tree::new(&message);
    let doc = json!({"id":"scope", "children":(0..MAX_WORK + 1).map(|i| json!({"id":format!("instance-{i}"), "type":"INSTANCE", "componentId":"0:10"})).collect::<Vec<_>>()});
    let result = inspect(&tree, &doc, Path::new("synthetic.fig"));
    assert!(result["coverage"]["scannedNodes"].as_u64().unwrap() <= MAX_WORK as u64);
    assert_eq!(result["instances"].as_array().unwrap().len(), MAX_INSTANCES);
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("work_limit"))
    );
    let properties: serde_json::Map<String, Value> = (0..MAX_PROPERTIES + 1)
        .map(|i| {
            (
                format!("{i:03}{}", "\n".repeat(240)),
                json!({"type":"TEXT", "value":"\n".repeat(256)}),
            )
        })
        .collect();
    let doc = json!({"id":"scope", "children":(0..MAX_INSTANCES).map(|i| json!({"id":format!("instance-{i}"), "type":"INSTANCE", "componentId":"0:10", "componentProperties":properties})).collect::<Vec<_>>()});
    let result = inspect(&tree, &doc, Path::new(&"\n".repeat(MAX_ID_BYTES)));
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_BYTES);
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("byte_limit"))
    );
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("property_limit"))
    );
    assert!(
        result["instances"][0]["currentProperties"]
            .as_array()
            .unwrap()
            .len()
            <= MAX_PROPERTIES
    );
}

#[test]
fn oversized_names_values_and_ids_do_not_become_invented_facts() {
    let message = message(vec![node(
        11,
        40,
        "SYMBOL",
        vec![
            ("name", Kiwi::String("x".repeat(300))),
            (
                "variantPropSpecs",
                Kiwi::Array(vec![spec(50, &"x".repeat(300)), spec(999, "Error")]),
            ),
        ],
    )]);
    let tree = Tree::new(&message);
    let result = inspect_root(&tree, "0:11");
    let variant = &result["sets"][0]["variants"][0];
    assert_eq!(variant["nameTruncated"], true);
    for entry in variant["properties"]["entries"].as_array().unwrap() {
        assert_eq!(entry["state"], "unsupported");
        assert!(entry.get("value").is_none());
    }
    let doc = json!({"id":"x".repeat(MAX_ID_BYTES + 1),"type":"INSTANCE","componentId":"0:10"});
    let result = inspect(&tree, &doc, Path::new("synthetic.fig"));
    assert_eq!(result["instances"], json!([]));
    assert_eq!(result["scopeNodeId"], Value::Null);
    assert_eq!(result["truncated"], true);
}

#[test]
fn set_and_global_variant_limits_are_independent() {
    let mut extra = Vec::new();
    for group in 0..MAX_SETS + 1 {
        let set = 1000 + group as u32 * 100;
        extra.push(node(
            set,
            1,
            "FRAME",
            vec![("isStateGroup", Kiwi::Bool(true))],
        ));
        extra.push(node(set + 1, set, "SYMBOL", vec![]));
    }
    let message_sets = message(extra);
    let tree = Tree::new(&message_sets);
    let result = inspect_root(&tree, "0:1");
    assert_eq!(result["sets"].as_array().unwrap().len(), MAX_SETS);
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("set_limit"))
    );

    let mut extra = Vec::new();
    for group in 0..5 {
        let set = 1000 + group * 100;
        extra.push(node(
            set,
            1,
            "FRAME",
            vec![("isStateGroup", Kiwi::Bool(true))],
        ));
        for variant in 1..=64 {
            extra.push(node(set + variant, set, "SYMBOL", vec![]));
        }
    }
    let message_variants = message(extra);
    let tree = Tree::new(&message_variants);
    let result = inspect_root(&tree, "0:1");
    let count: usize = result["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["variants"].as_array().unwrap().len())
        .sum();
    assert_eq!(count, MAX_VARIANTS);
    assert!(
        result["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("variant_limit"))
    );
}

#[test]
fn effective_boolean_false_empty_text_and_malformed_properties_remain_distinct() {
    let message = message(vec![]);
    let tree = Tree::new(&message);
    let document = json!({"id":"0:2","type":"INSTANCE","componentId":"0:10","componentProperties":{
        "Flag#0:90":{"type":"BOOLEAN","value":false},
        "Label#0:91":{"type":"TEXT","value":""},
        "Unknown":{"type":"SLOT","value":"Default"},
        "WrongBoolean":{"type":"BOOLEAN","value":"false"}
    }});
    let result = inspect(&tree, &document, Path::new("synthetic.fig"));
    let values = result["instances"][0]["currentProperties"]
        .as_array()
        .unwrap();
    assert_eq!(values[0]["value"], false);
    assert_eq!(values[1]["value"], "");
    assert_eq!(values[2]["state"], "unsupported");
    assert!(values[2].get("value").is_none());
    assert_eq!(values[3]["state"], "unsupported");
    assert!(values[3].get("value").is_none());
}

#[test]
fn expanded_component_sets_use_source_ids_instead_of_instance_paths() {
    let message = message(vec![
        node(80, 10, "FRAME", vec![("isStateGroup", Kiwi::Bool(true))]),
        node(81, 80, "SYMBOL", vec![]),
    ]);
    let tree = Tree::new(&message);
    let result = inspect_root(&tree, "I0:2;0:80");
    assert_eq!(result["scopeNodeId"], "I0:2;0:80");
    assert_eq!(result["sets"][0]["nodeId"], "0:80");
    assert_eq!(result["sets"][0]["variants"][0]["nodeId"], "0:81");
}
