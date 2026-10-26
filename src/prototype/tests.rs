use super::*;

fn obj(fields: Vec<(&'static str, K<'static>)>) -> K<'static> {
    K::Object("Test", fields.into_iter().collect())
}

fn guid(id: u32) -> K<'static> {
    obj(vec![("sessionID", K::UInt(0)), ("localID", K::UInt(id))])
}

fn event(kind: &'static str, actions: Vec<K<'static>>) -> K<'static> {
    obj(vec![
        (
            "event",
            obj(vec![("interactionType", K::Enum("Event", kind))]),
        ),
        ("actions", K::Array(actions)),
    ])
}

fn action(kind: &'static str, target: u32) -> K<'static> {
    obj(vec![
        ("navigationType", K::Enum("Action", kind)),
        ("transitionNodeID", guid(target)),
    ])
}

fn layer(
    id: u32,
    parent: Option<u32>,
    kind: &'static str,
    extra: Vec<(&'static str, K<'static>)>,
) -> K<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", K::Enum("NodeType", kind)),
        ("name", K::String(format!("Node {id}"))),
        (
            "size",
            obj(vec![("x", K::Float(20.0)), ("y", K::Float(10.0))]),
        ),
    ];
    if let Some(parent) = parent {
        fields.push((
            "parentIndex",
            obj(vec![
                ("guid", guid(parent)),
                ("position", K::String(format!("{id:04}"))),
            ]),
        ));
    }
    fields.extend(extra);
    obj(fields)
}

fn message(nodes: Vec<K<'static>>) -> K<'static> {
    obj(vec![("nodeChanges", K::Array(nodes))])
}

fn patch(path: &[u32], fields: Vec<(&'static str, K<'static>)>) -> K<'static> {
    let mut all = vec![(
        "guidPath",
        obj(vec![(
            "guids",
            K::Array(path.iter().map(|id| guid(*id)).collect()),
        )]),
    )];
    all.extend(fields);
    obj(all)
}

#[test]
fn prototype_fields_keep_action_types_indices_and_dangling_destinations() {
    let message = message(vec![
        layer(
            1,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(vec![
                    obj(vec![("isDeleted", K::Bool(true))]),
                    event(
                        "ON_HOVER",
                        vec![
                            obj(vec![
                                ("navigationType", K::Enum("Navigation", "NAVIGATE")),
                                ("connectionType", K::Enum("Connection", "OPEN_OVERLAY")),
                                ("transitionNodeID", guid(2)),
                            ]),
                            action("NAVIGATE", 999),
                            obj(vec![("connectionType", K::Enum("Connection", "BACK"))]),
                        ],
                    ),
                    obj(vec![]),
                ]),
            )],
        ),
        layer(
            2,
            None,
            "FRAME",
            vec![("name", K::String("Loading".into()))],
        ),
    ]);
    let tree = Tree::new(&message);
    let evidence = stored(&tree, "0:1").unwrap();
    assert_eq!(evidence["sourceNodeId"], "0:1");
    assert_eq!(evidence["basis"], "stored-node");
    let events = evidence["interactions"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["index"], 1);
    assert_eq!(events[0]["trigger"]["value"], "ON_HOVER");
    assert_eq!(
        events[0]["trigger"]["sourceField"],
        "prototypeInteractions[1].event.interactionType"
    );
    let actions = events[0]["actions"].as_array().unwrap();
    assert_eq!(actions[0]["navigationType"]["value"], "NAVIGATE");
    assert_eq!(actions[0]["connectionType"]["value"], "OPEN_OVERLAY");
    assert_eq!(actions[0]["destination"]["state"], "resolved");
    assert_eq!(actions[0]["destination"]["nodeId"], "0:2");
    assert_eq!(actions[1]["destination"]["state"], "unresolved");
    assert_eq!(actions[1]["destination"]["nodeId"], "0:999");
    assert_eq!(
        actions[1]["destination"]["sourceField"],
        "prototypeInteractions[1].actions[1].transitionNodeID"
    );
    assert_eq!(actions[2]["connectionType"]["value"], "BACK");
    assert_eq!(actions[2]["navigationType"]["state"], "absent");
    assert_eq!(actions[2]["destination"]["state"], "absent");
    assert_eq!(events[1]["trigger"]["state"], "absent");
    assert_eq!(events[1]["actionsState"], "absent");
    assert!(events[1]["trigger"].get("value").is_none());
}

#[test]
fn component_definition_events_do_not_prove_instance_inheritance() {
    let message = message(vec![
        layer(
            1,
            None,
            "INSTANCE",
            vec![("symbolData", obj(vec![("symbolID", guid(10))]))],
        ),
        layer(
            2,
            None,
            "INSTANCE",
            vec![("symbolData", obj(vec![("symbolID", guid(999))]))],
        ),
        layer(
            10,
            None,
            "SYMBOL",
            vec![(
                "prototypeInteractions",
                K::Array(vec![event("ON_CLICK", vec![])]),
            )],
        ),
        layer(
            3,
            None,
            "FRAME",
            vec![("name", K::String("Loading / Error".into()))],
        ),
    ]);
    let tree = Tree::new(&message);
    let evidence = node(&tree, "0:1", None, "INSTANCE").unwrap();
    assert_eq!(evidence["sourceNodeId"], "0:10");
    assert_eq!(evidence["basis"], "component-definition");
    assert_eq!(evidence["inheritanceState"], "unsupported");
    assert_eq!(
        node(&tree, "0:2", None, "INSTANCE").unwrap()["reason"],
        "component_source_unavailable"
    );
    assert!(stored(&tree, "0:3").is_none());
}

#[test]
fn nested_overrides_keep_the_winning_source_even_with_library_keys_and_later_geometry() {
    let local = vec![patch(
        &[121],
        vec![(
            "prototypeInteractions",
            K::Array(vec![event("ON_HOVER", vec![action("NAVIGATE", 1)])]),
        )],
    )];
    let outer = vec![
        patch(
            &[11, 121],
            vec![(
                "prototypeInteractions",
                K::Array(vec![event("ON_PRESS", vec![action("NAVIGATE", 999)])]),
            )],
        ),
        patch(&[11, 121], vec![("opacity", K::Float(0.5))]),
    ];
    let message = message(vec![
        layer(1, None, "FRAME", vec![]),
        layer(
            2,
            Some(1),
            "INSTANCE",
            vec![(
                "symbolData",
                obj(vec![
                    ("symbolID", guid(10)),
                    ("symbolOverrides", K::Array(outer)),
                ]),
            )],
        ),
        layer(
            3,
            Some(1),
            "INSTANCE",
            vec![("symbolData", obj(vec![("symbolID", guid(10))]))],
        ),
        layer(10, None, "SYMBOL", vec![]),
        layer(
            11,
            Some(10),
            "INSTANCE",
            vec![(
                "symbolData",
                obj(vec![
                    ("symbolID", guid(20)),
                    ("symbolOverrides", K::Array(local)),
                ]),
            )],
        ),
        layer(20, None, "SYMBOL", vec![]),
        layer(
            21,
            Some(20),
            "RECTANGLE",
            vec![
                ("overrideKey", guid(121)),
                (
                    "prototypeInteractions",
                    K::Array(vec![event("ON_CLICK", vec![])]),
                ),
            ],
        ),
    ]);
    let tree = Tree::new(&message);
    for (id, source, trigger) in [
        ("I0:2;0:11;0:121", "0:2", "ON_PRESS"),
        ("I0:3;0:11;0:121", "0:11", "ON_HOVER"),
    ] {
        let doc = crate::rest::resolve_nodes(&tree, id).unwrap();
        let evidence = &doc["document"]["_figlens"]["prototype"];
        assert_eq!(evidence["sourceNodeId"], source);
        assert_eq!(
            evidence["sourceField"],
            "symbolData.symbolOverrides[0].prototypeInteractions"
        );
        assert_eq!(evidence["basis"], "instance-override");
        assert_eq!(evidence["interactions"][0]["trigger"]["value"], trigger);
        assert_eq!(doc["document"]["id"], id);
        let states = crate::states::inspect(
            &tree,
            &doc["document"],
            std::path::Path::new("synthetic.fig"),
        );
        assert_eq!(states["triggerEvidence"]["nodes"][0]["nodeId"], id);
        assert_eq!(
            states["triggerEvidence"]["nodes"][0]["sourceNodeId"],
            source
        );
    }
}

#[test]
fn swaps_keep_expanded_sources_and_mark_unverified_instance_local_precedence() {
    let message = message(vec![
        layer(
            1,
            None,
            "INSTANCE",
            vec![(
                "symbolData",
                obj(vec![
                    ("symbolID", guid(10)),
                    (
                        "symbolOverrides",
                        K::Array(vec![
                            patch(&[11], vec![("overriddenSymbolID", guid(30))]),
                            patch(&[12], vec![("overriddenSymbolID", guid(30))]),
                        ]),
                    ),
                ]),
            )],
        ),
        layer(10, None, "SYMBOL", vec![]),
        layer(
            11,
            Some(10),
            "INSTANCE",
            vec![("symbolData", obj(vec![("symbolID", guid(20))]))],
        ),
        layer(
            12,
            Some(10),
            "INSTANCE",
            vec![
                ("symbolData", obj(vec![("symbolID", guid(20))])),
                (
                    "prototypeInteractions",
                    K::Array(vec![event("ON_CLICK", vec![])]),
                ),
            ],
        ),
        layer(20, None, "SYMBOL", vec![]),
        layer(
            30,
            None,
            "SYMBOL",
            vec![(
                "prototypeInteractions",
                K::Array(vec![event("ON_HOVER", vec![])]),
            )],
        ),
    ]);
    let tree = Tree::new(&message);
    let doc = crate::rest::resolve_nodes(&tree, "I0:1;0:11").unwrap();
    let evidence = &doc["document"]["_figlens"]["prototype"];
    assert_eq!(evidence["sourceNodeId"], "0:30");
    assert_eq!(evidence["basis"], "component-definition");
    assert_eq!(evidence["inheritanceState"], "unsupported");
    assert_eq!(evidence["reason"], "instance_inheritance_not_verified");
    assert_eq!(evidence["interactions"][0]["trigger"]["value"], "ON_HOVER");
    let doc = crate::rest::resolve_nodes(&tree, "I0:1;0:12").unwrap();
    let evidence = &doc["document"]["_figlens"]["prototype"];
    assert_eq!(evidence["sourceNodeId"], "0:12");
    assert_eq!(evidence["basis"], "instance-local");
    assert_eq!(evidence["swappedDefinitionId"], "0:30");
    assert_eq!(evidence["precedenceState"], "unsupported");
    assert_eq!(evidence["interactions"][0]["trigger"]["value"], "ON_CLICK");
}

#[test]
fn malformed_lists_events_deletion_markers_and_unset_targets_remain_explicit() {
    let message = message(vec![
        layer(
            1,
            None,
            "FRAME",
            vec![("prototypeInteractions", K::String("ON_CLICK".into()))],
        ),
        layer(
            2,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(vec![obj(vec![("event", K::Bool(false))])]),
            )],
        ),
        layer(
            3,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(vec![obj(vec![
                    ("isDeleted", K::String("false".into())),
                    (
                        "event",
                        obj(vec![("interactionType", K::Enum("Event", "ON_CLICK"))]),
                    ),
                ])]),
            )],
        ),
        layer(
            4,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(vec![obj(vec![(
                    "event",
                    obj(vec![("interactionType", K::String("x".repeat(129)))]),
                )])]),
            )],
        ),
        layer(
            5,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(vec![event(
                    "ON_HOVER",
                    vec![obj(vec![(
                        "transitionNodeID",
                        obj(vec![
                            ("sessionID", K::UInt(u32::MAX)),
                            ("localID", K::UInt(u32::MAX)),
                        ]),
                    )])],
                )]),
            )],
        ),
    ]);
    let tree = Tree::new(&message);
    assert_eq!(stored(&tree, "0:1").unwrap()["state"], "unsupported");
    for id in ["0:2", "0:3", "0:4"] {
        let evidence = stored(&tree, id).unwrap();
        let trigger = &evidence["interactions"][0]["trigger"];
        assert_eq!(trigger["state"], "unsupported");
        assert!(trigger.get("value").is_none());
    }
    let evidence = stored(&tree, "0:5").unwrap();
    let target = &evidence["interactions"][0]["actions"][0]["destination"];
    assert_eq!(target["state"], "absent");
    assert_eq!(target["reason"], "unset_node_reference");
    assert!(target.get("nodeId").is_none());
}

#[test]
fn prototype_evidence_shares_the_state_inventory_byte_budget() {
    let mut nodes = vec![
        layer(1, None, "FRAME", vec![]),
        layer(
            100,
            None,
            "SYMBOL",
            vec![(
                "prototypeInteractions",
                K::Array(
                    (0..32)
                        .map(|_| event("ON_CLICK", vec![action("NAVIGATE", 999)]))
                        .collect(),
                ),
            )],
        ),
    ];
    nodes.extend((2..40).map(|id| {
        layer(
            id,
            Some(1),
            "INSTANCE",
            vec![("symbolData", obj(vec![("symbolID", guid(100))]))],
        )
    }));
    let message = message(nodes);
    let tree = Tree::new(&message);
    let doc = crate::rest::resolve_nodes(&tree, "0:1").unwrap();
    let states = crate::states::inspect(
        &tree,
        &doc["document"],
        std::path::Path::new("synthetic.fig"),
    );
    assert_eq!(states["truncated"], true);
    assert!(
        states["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("byte_limit"))
    );
    assert!(serde_json::to_vec(&states).unwrap().len() <= 128 * 1024);
}

#[test]
fn explicitly_empty_overrides_do_not_fall_back_to_component_events() {
    let message = message(vec![
        layer(
            1,
            None,
            "INSTANCE",
            vec![(
                "symbolData",
                obj(vec![
                    ("symbolID", guid(10)),
                    (
                        "symbolOverrides",
                        K::Array(vec![patch(
                            &[11],
                            vec![("prototypeInteractions", K::Array(vec![]))],
                        )]),
                    ),
                ]),
            )],
        ),
        layer(10, None, "SYMBOL", vec![]),
        layer(
            11,
            Some(10),
            "RECTANGLE",
            vec![(
                "prototypeInteractions",
                K::Array(vec![event("ON_CLICK", vec![])]),
            )],
        ),
    ]);
    let tree = Tree::new(&message);
    let doc = crate::rest::resolve_nodes(&tree, "I0:1;0:11").unwrap();
    let evidence = &doc["document"]["_figlens"]["prototype"];
    assert_eq!(evidence["basis"], "instance-override");
    assert_eq!(evidence["state"], "recorded");
    assert_eq!(evidence["interactions"], json!([]));
}

#[test]
fn malformed_fields_and_large_lists_are_bounded_without_invented_defaults() {
    let malformed = event(
        "ON_CLICK",
        vec![
            obj(vec![(
                "transitionNodeID",
                obj(vec![
                    ("sessionID", K::String("0".into())),
                    ("localID", K::UInt(1)),
                ]),
            )]),
            K::Bool(false),
        ],
    );
    let message = message(vec![layer(
        1,
        None,
        "FRAME",
        vec![("prototypeInteractions", K::Array(vec![malformed]))],
    )]);
    let tree = Tree::new(&message);
    let evidence = stored(&tree, "0:1").unwrap();
    assert_eq!(
        evidence["interactions"][0]["actions"][0]["destination"]["state"],
        "unsupported"
    );
    assert!(
        evidence["interactions"][0]["actions"][0]["destination"]
            .get("nodeId")
            .is_none()
    );
    assert_eq!(
        evidence["interactions"][0]["actions"][1]["state"],
        "unsupported"
    );
    let message = super::tests::message(vec![layer(
        1,
        None,
        "FRAME",
        vec![(
            "prototypeInteractions",
            K::Array(
                (0..40)
                    .map(|_| {
                        event(
                            "ON_HOVER",
                            (0..20).map(|_| action("NAVIGATE", 999)).collect(),
                        )
                    })
                    .collect(),
            ),
        )],
    )]);
    let tree = Tree::new(&message);
    let evidence = stored(&tree, "0:1").unwrap();
    assert_eq!(evidence["truncated"], true);
    assert!(evidence["interactions"].as_array().unwrap().len() <= MAX_INTERACTIONS);
    assert!(serde_json::to_vec(&evidence).unwrap().len() <= MAX_BYTES);
}

#[test]
fn prototype_action_limits_distinguish_omitted_actions_from_empty_lists() {
    for action_count in [16, 20] {
        let message = message(vec![layer(
            1,
            None,
            "FRAME",
            vec![(
                "prototypeInteractions",
                K::Array(
                    (0..32)
                        .map(|_| {
                            event(
                                "ON_HOVER",
                                (0..action_count).map(|_| action("NAVIGATE", 999)).collect(),
                            )
                        })
                        .collect(),
                ),
            )],
        )]);
        let tree = Tree::new(&message);
        let evidence = stored(&tree, "0:1").unwrap();
        let interactions = evidence["interactions"].as_array().unwrap();
        assert!(
            interactions
                .iter()
                .any(|i| i["actions"].as_array().unwrap().is_empty())
        );
        for interaction in interactions {
            assert_eq!(interaction["actionsState"], "recorded");
            assert_eq!(
                interaction["actionsTruncated"],
                interaction["actions"].as_array().unwrap().len() < action_count
            );
        }
        assert_eq!(evidence["truncated"], true);
        assert!(serde_json::to_vec(&evidence).unwrap().len() <= MAX_BYTES);
    }
    let message = message(vec![layer(
        1,
        None,
        "FRAME",
        vec![(
            "prototypeInteractions",
            K::Array(vec![event("ON_HOVER", vec![])]),
        )],
    )]);
    let tree = Tree::new(&message);
    let evidence = stored(&tree, "0:1").unwrap();
    assert_eq!(evidence["interactions"][0]["actionsTruncated"], false);
    assert_eq!(evidence["truncated"], false);
}
