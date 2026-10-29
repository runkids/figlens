use super::super::tests::{bounds, guid, instance, node, obj, path, stored};
use super::*;
use crate::search::store::Stored;
use crate::search::{WORK, build};
use kiwi_schema::Value as K;

fn alias(id: u32) -> K<'static> {
    obj(vec![
        ("dataType", K::Enum("VariableDataType", "ALIAS")),
        ("value", obj(vec![("alias", obj(vec![("guid", guid(id))]))])),
    ])
}

fn solid(var: u32) -> K<'static> {
    K::Array(vec![obj(vec![
        ("type", K::Enum("PaintType", "SOLID")),
        ("colorVar", alias(var)),
    ])])
}

fn click_to(target: u32) -> (&'static str, K<'static>) {
    (
        "prototypeInteractions",
        K::Array(vec![obj(vec![
            (
                "event",
                obj(vec![(
                    "interactionType",
                    K::Enum("InteractionType", "ON_CLICK"),
                )]),
            ),
            (
                "actions",
                K::Array(vec![obj(vec![
                    ("transitionNodeID", guid(target)),
                    ("navigationType", K::Enum("NavigationType", "NAVIGATE")),
                ])]),
            ),
        ])]),
    )
}

/// Two same-name components, a card nesting one, a swap, hidden copies, a missing main, a
/// detached frame, a variant set, variable bindings and prototype links.
fn design() -> K<'static> {
    let set = ("isStateGroup", K::Bool(true));
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Components", vec![]),
            node(2, Some(0), "CANVAS", "Screens", vec![]),
            node(20, Some(1), "SYMBOL", "Button", vec![]),
            node(21, Some(1), "SYMBOL", "Button", vec![]),
            node(22, Some(1), "SYMBOL", "Icon", vec![]),
            node(30, Some(1), "SYMBOL", "Card", vec![]),
            instance(31, 30, "Action", 20, vec![]),
            node(50, Some(1), "FRAME", "State", vec![set]),
            node(51, Some(50), "SYMBOL", "State=On", vec![]),
            node(52, Some(50), "SYMBOL", "State=Off", vec![]),
            node(60, Some(1), "VARIABLE", "brand", vec![]),
            node(10, Some(2), "FRAME", "Screen", vec![click_to(11)]),
            node(11, Some(2), "FRAME", "Next", vec![click_to(98)]),
            instance(3, 10, "Plain card", 30, vec![]),
            instance(
                4,
                10,
                "Swapped card",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![obj(vec![
                        path(&[31]),
                        ("overriddenSymbolID", guid(22)),
                    ])]),
                )],
            ),
            instance(
                5,
                10,
                "Quiet card",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![obj(vec![path(&[31]), ("visible", K::Bool(false))])]),
                )],
            ),
            instance(6, 10, "Other button", 21, vec![]),
            instance(7, 10, "Lost", 99, vec![]),
            node(8, Some(10), "FRAME", "Button", vec![]),
            instance(9, 10, "On", 51, vec![]),
            instance(12, 10, "Off", 52, vec![]),
            node(
                13,
                Some(10),
                "RECTANGLE",
                "Swatch",
                vec![("fillPaints", solid(60)), ("strokePaints", solid(60))],
            ),
            node(
                14,
                Some(10),
                "RECTANGLE",
                "Broken swatch",
                vec![("fillPaints", solid(97))],
            ),
            // A component whose own link its instance inherits unverified, and styled layers.
            node(23, Some(1), "SYMBOL", "Link", vec![click_to(10)]),
            instance(15, 10, "Linked", 23, vec![]),
            node(61, Some(1), "RECTANGLE", "Fill style", vec![]),
            node(
                16,
                Some(10),
                "RECTANGLE",
                "Styled",
                vec![
                    ("styleIdForFill", obj(vec![("guid", guid(61))])),
                    (
                        "styleIdForStrokeFill",
                        obj(vec![(
                            "assetRef",
                            obj(vec![("key", K::String("lib1".into()))]),
                        )]),
                    ),
                    ("styleIdForEffect", obj(vec![("guid", guid(96))])),
                    (
                        "fillPaints",
                        K::Array(vec![obj(vec![
                            ("type", K::Enum("PaintType", "IMAGE")),
                            ("image", obj(vec![("hash", K::Bytes(vec![0xab, 0xcd]))])),
                        ])]),
                    ),
                ],
            ),
        ]),
    )])
}

fn source() -> Json {
    json!({"sourceId": "local:test", "snapshotId": "sha256:a"})
}

fn req(target: &str) -> Request {
    Request {
        target: target.into(),
        resource: target.starts_with("VariableID:"),
        scope: "0:0".into(),
        relations: Vec::new(),
        outgoing: false,
        snapshot: None,
        include_hidden: false,
        limit: DEFAULT_LIMIT,
        cursor: None,
    }
}

fn run(index: &Index, r: &Request) -> Result<Json> {
    let q = r.validate()?;
    query(
        &mut Whole(index),
        &q,
        r,
        Served {
            source: &source(),
            gen_id: 0,
        },
    )
}

fn ids(out: &Json) -> Vec<String> {
    out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"]["nodeId"].as_str().unwrap().to_owned())
        .collect()
}

fn index() -> Index {
    let message = design();
    let tree = Tree::new(&message);
    build(&tree, "0:0", bounds(WORK))
}

#[test]
fn same_name_components_keep_their_own_instances() {
    let index = index();
    let out = run(&index, &req("0:21")).unwrap();
    assert_eq!(ids(&out), ["0:6"]);
    // The detached frame named Button is no usage of either.
    assert!(!ids(&run(&index, &req("0:20")).unwrap()).contains(&"0:8".to_owned()));
}

#[test]
fn nested_effective_instances_and_swaps_are_usages_of_their_actual_main() {
    let index = index();
    let button = run(&index, &req("0:20")).unwrap();
    // The definition's own instance and the unswapped card's nested copy; the swapped one
    // uses the icon instead, and the hidden copy needs includeHidden.
    assert_eq!(ids(&button), ["0:31", "I0:3;0:31"]);
    let nested = &button["results"][1];
    assert_eq!(
        (&nested["basis"], &nested["evidence"], &nested["origin"]),
        (
            &json!("resolved"),
            &json!("definition"),
            &json!("instance-effective")
        )
    );
    let icon = run(&index, &req("0:22")).unwrap();
    assert_eq!(ids(&icon), ["I0:4;0:31"]);
    assert_eq!(icon["results"][0]["evidence"], "swap");
    assert_eq!(icon["definition"]["state"], "indexed");
}

#[test]
fn hidden_overridden_copies_are_returned_only_on_request() {
    let index = index();
    let codes = |out: &Json| -> Vec<Json> {
        let reasons = out["coverage"]["reasons"].as_array().unwrap();
        reasons.iter().map(|r| r["code"].clone()).collect()
    };
    let visible = run(&index, &req("0:20")).unwrap();
    assert_eq!(ids(&visible), ["0:31", "I0:3;0:31"]);
    assert!(codes(&visible).contains(&json!("hidden_locations_excluded")));
    let mut r = req("0:20");
    r.include_hidden = true;
    assert_eq!(
        ids(&run(&index, &r).unwrap()),
        ["0:31", "I0:3;0:31", "I0:5;0:31"]
    );
    assert!(!codes(&run(&index, &r).unwrap()).contains(&json!("hidden_locations_excluded")));
}

#[test]
fn a_missing_main_is_reported_missing_with_its_instances() {
    let index = index();
    let out = run(&index, &req("0:99")).unwrap();
    assert_eq!(out["definition"]["state"], "missing");
    assert_eq!(ids(&out), ["0:7"]);
    assert_eq!(out["results"][0]["state"], "missing");
}

#[test]
fn a_component_set_lists_members_and_their_instances_apart() {
    let index = index();
    let out = run(&index, &req("0:50")).unwrap();
    assert_eq!(out["members"]["nodeIds"], json!(["0:51", "0:52"]));
    let found: Vec<(String, Json, Json)> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["ref"]["nodeId"].as_str().unwrap().to_owned(),
                r["relation"].clone(),
                r["via"].clone(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("0:51".to_owned(), json!("variant_of"), Json::Null),
            ("0:52".to_owned(), json!("variant_of"), Json::Null),
            ("0:9".to_owned(), json!("instance_of"), json!("0:51")),
            ("0:12".to_owned(), json!("instance_of"), json!("0:52")),
        ]
    );
}

#[test]
fn variable_and_prototype_links_keep_unresolved_targets() {
    let index = index();
    let brand = run(&index, &req("VariableID:0:60")).unwrap();
    assert_eq!(brand["definition"]["name"], "brand");
    let fields: Vec<&Json> = brand["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["evidence"])
        .collect();
    assert_eq!(fields, [&json!("fills"), &json!("strokes")]);
    let lost = run(&index, &req("VariableID:0:97")).unwrap();
    assert_eq!(
        (&lost["definition"]["state"], ids(&lost)),
        (&json!("missing"), vec!["0:14".to_owned()])
    );
    let next = run(&index, &req("0:11")).unwrap();
    assert_eq!(ids(&next), ["0:10"]);
    assert_eq!(next["results"][0]["relation"], "prototype_target");
    let gone = run(&index, &req("0:98")).unwrap();
    assert_eq!(
        (&gone["definition"]["state"], ids(&gone)),
        (&json!("missing"), vec!["0:11".to_owned()])
    );
}

#[test]
fn pages_follow_every_edge_of_one_location_in_order() {
    let index = index();
    let mut r = req("VariableID:0:60");
    r.limit = 1;
    let first = run(&index, &r).unwrap();
    assert_eq!(first["results"][0]["evidence"], "fills");
    r.cursor = first["nextCursor"].as_str().map(str::to_owned);
    let second = run(&index, &r).unwrap();
    assert_eq!(second["results"][0]["evidence"], "strokes");
    assert_eq!(ids(&second), ["0:13"]);
    assert_eq!(second["nextCursor"], Json::Null);
    // The same cursor over another snapshot is stale, not silently reused.
    let q = r.validate().unwrap();
    let other = json!({"sourceId": "local:test", "snapshotId": "sha256:b"});
    let e = query(
        &mut Whole(&index),
        &q,
        &r,
        Served {
            source: &other,
            gen_id: 0,
        },
    )
    .unwrap_err();
    assert_eq!(
        e.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::CursorStale
    );
}

#[test]
fn a_ref_from_another_snapshot_is_stale_and_an_unknown_id_not_found() {
    let index = index();
    let mut r = req("0:20");
    r.snapshot = Some("sha256:old".into());
    let e = run(&index, &r).unwrap_err();
    assert_eq!(
        e.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::SourceStale
    );
    let e = run(&index, &req("0:4242")).unwrap_err();
    assert_eq!(
        e.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::NodeNotFound
    );
}

#[test]
fn outgoing_lists_what_a_node_points_at() {
    let index = index();
    let mut r = req("I0:4;0:31");
    r.outgoing = true;
    let out = run(&index, &r).unwrap();
    let edges: Vec<(&Json, &Json)> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (&e["relation"], &e["target"]["ref"]["nodeId"]))
        .collect();
    assert_eq!(
        edges,
        [
            (&json!("contains"), &json!("0:4")),
            (&json!("instance_of"), &json!("0:22"))
        ]
    );
    assert_eq!(out["results"][1]["definition"]["name"], "Icon");
}

#[test]
fn a_scope_returns_only_its_locations_but_resolves_definitions_outside_it() {
    let message = design();
    let tree = Tree::new(&message);
    let screens = build(&tree, "0:2", bounds(WORK));
    let mut r = req("0:20");
    r.scope = "0:2".into();
    let out = run(&screens, &r).unwrap();
    assert_eq!(ids(&out), ["I0:3;0:31"]);
    assert_eq!(out["definition"]["state"], "outside_scope");
    assert_eq!(out["definition"]["page"], "0:1");
}

#[test]
fn a_build_stopped_by_its_budget_reports_partial_coverage() {
    let message = design();
    let tree = Tree::new(&message);
    // The smallest budget that indexes the scope root but stops before the rest.
    let small = (1..1000)
        .map(|w| build(&tree, "0:0", bounds(w)))
        .find(|i| i.total > 0)
        .unwrap();
    assert!(small.stop.is_some());
    let out = run(&small, &req("0:0")).unwrap();
    assert_eq!(out["coverage"]["complete"], false);
    assert_eq!(out["coverage"]["reasons"][0]["code"], small.stop.unwrap());
    let full = run(&index(), &req("0:20")).unwrap();
    assert_eq!(full["coverage"]["layers"]["relations"]["state"], "partial");
    assert!(full.get("total").is_none());
}

#[test]
fn a_stored_generation_answers_like_the_whole_index() {
    let index = index();
    let conn = stored(&index);
    for target in ["0:20", "0:50", "VariableID:0:60", "0:99", "0:11"] {
        for outgoing in [false, true] {
            let mut r = req(target);
            r.outgoing = outgoing;
            r.limit = 2;
            loop {
                let q = r.validate().unwrap();
                let whole = query(
                    &mut Whole(&index),
                    &q,
                    &r,
                    Served {
                        source: &source(),
                        gen_id: 1,
                    },
                );
                let graph = &mut Stored {
                    conn: &conn,
                    gen_id: 1,
                    index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
                    spent: 0,
                };
                let disk = query(
                    graph,
                    &q,
                    &r,
                    Served {
                        source: &source(),
                        gen_id: 1,
                    },
                );
                match (whole, disk) {
                    (Ok(a), Ok(b)) => {
                        assert_eq!(a, b, "{target} out={outgoing}");
                        let Some(next) = a["nextCursor"].as_str() else {
                            break;
                        };
                        r.cursor = Some(next.to_owned());
                    }
                    (Err(a), Err(b)) => {
                        assert_eq!(a.to_string(), b.to_string());
                        break;
                    }
                    (a, b) => panic!("{target}: {a:?} vs {b:?}"),
                }
            }
        }
    }
}

#[test]
fn an_index_built_from_a_held_tree_answers_with_the_callers_source_identity() {
    let message = design();
    let tree = Tree::new(&message);
    let index = super::super::index_tree(&tree, "0:0", super::super::deadline()).unwrap();
    let source = json!({"sourceId": "local:viewer", "snapshotId": "sha256:held"});
    let out = super::super::usages_index(&index, &req("0:21"), source).unwrap();
    assert_eq!(ids(&out), ["0:6"]);
    assert_eq!(out["results"][0]["ref"]["snapshotId"], "sha256:held");
    assert!(super::super::index_tree(&tree, "9:9", super::super::deadline()).is_err());
}

#[test]
fn styles_and_images_are_exact_recorded_targets() {
    let index = index();
    let local = run(&index, &req("0:61")).unwrap();
    assert_eq!(ids(&local), ["0:16"]);
    assert_eq!(local["results"][0]["evidence"], "styleIdForFill");
    let mut lib = req("StyleKey:lib1");
    lib.resource = true;
    let lib = run(&index, &lib).unwrap();
    assert_eq!(
        (ids(&lib), &lib["results"][0]["state"]),
        (vec!["0:16".to_owned()], &json!("external"))
    );
    let mut image = req("image:abcd");
    image.resource = true;
    let image = run(&index, &image).unwrap();
    assert_eq!(image["results"][0]["evidence"], "fillPaints[0]");
    let mut out = req("0:16");
    out.outgoing = true;
    let states: Vec<(Json, Json)> = run(&index, &out).unwrap()["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["relation"] == "uses_style")
        .map(|r| (r["evidence"].clone(), r["state"].clone()))
        .collect();
    assert!(states.contains(&(json!("styleIdForEffect"), json!("missing"))));
}

#[test]
fn inherited_prototype_links_make_that_relation_partial_not_absent() {
    let index = index();
    let mut r = req("0:10");
    r.relations = vec!["prototype_target".into()];
    let out = run(&index, &r).unwrap();
    // The component's own link is a usage; its instance's inherited copy is not claimed.
    assert_eq!(ids(&out), ["0:23"]);
    assert_eq!(out["coverage"]["complete"], false);
    let codes: Vec<&Json> = out["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["code"])
        .collect();
    assert!(codes.contains(&&json!("relation_unverified")));
    assert_eq!(
        out["coverage"]["layers"]["relations"]["kinds"]["prototype_target"],
        "partial"
    );
    let mut other = req("0:21");
    other.relations = vec!["instance_of".into()];
    assert_eq!(run(&index, &other).unwrap()["coverage"]["complete"], true);
}

#[test]
fn a_set_outside_the_scope_lists_only_the_members_it_uses_as_incomplete() {
    let index = {
        let message = design();
        build(&Tree::new(&message), "0:2", bounds(WORK))
    };
    let mut r = req("0:50");
    r.scope = "0:2".into();
    let out = run(&index, &r).unwrap();
    assert_eq!(out["members"]["complete"], false);
    let codes: Vec<&Json> = out["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["code"])
        .collect();
    assert!(codes.contains(&&json!("members_outside_scope")));
    assert_eq!(index_of_whole()["members"]["complete"], true);
}

fn index_of_whole() -> Json {
    run(&index(), &req("0:50")).unwrap()
}

#[test]
fn an_unreadable_field_or_overlong_target_is_counted_and_explicit_not_the_end_of_the_index() {
    let huge = K::Array(
        (0..70_000)
            .map(|_| obj(vec![("colorVar", alias(60))]))
            .collect(),
    );
    let long = "k".repeat(super::super::store::ID_BYTES + 1);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(60, Some(1), "VARIABLE", "brand", vec![]),
            node(2, Some(1), "RECTANGLE", "Huge", vec![("fillPaints", huge)]),
            node(
                3,
                Some(1),
                "RECTANGLE",
                "Later",
                vec![
                    ("fillPaints", solid(60)),
                    (
                        "styleIdForFill",
                        obj(vec![(
                            "assetRef",
                            obj(vec![("key", K::String(long.as_str().into()))]),
                        )]),
                    ),
                ],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let out = run(&index, &req("VariableID:0:60")).unwrap();
    assert_eq!(ids(&out), ["0:3"]);
    assert_eq!(out["coverage"]["complete"], false);
    assert_eq!(
        out["coverage"]["layers"]["relations"]["kinds"]["binds_variable"],
        "partial"
    );
    let mut r = req("0:3");
    r.outgoing = true;
    let style = run(&index, &r).unwrap()["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["relation"] == "uses_style")
        .cloned()
        .unwrap();
    assert_eq!(
        (&style["target"], &style["state"]),
        (&Json::Null, &json!("unsupported"))
    );
}

#[test]
fn a_widely_used_component_pages_stably_from_the_index_without_sorting_earlier_matches() {
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
        node(20, Some(1), "SYMBOL", "Button", vec![]),
    ];
    let mut expected = Vec::new();
    for i in 1000..4000 {
        let hidden = i % 3 == 0;
        let extra = if hidden {
            vec![("visible", K::Bool(false))]
        } else {
            vec![]
        };
        nodes.push(instance(i, 1, "Use", 20, extra));
        if !hidden {
            expected.push(format!("0:{i}"));
        }
    }
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let conn = stored(&index);
    let src = source();
    let served = || Served {
        source: &src,
        gen_id: 1,
    };
    let mut r = req("0:20");
    r.relations = vec!["instance_of".into()];
    r.limit = 50;
    let mut seen = Vec::new();
    loop {
        let q = r.validate().unwrap();
        let whole = query(&mut Whole(&index), &q, &r, served()).unwrap();
        let graph = &mut Stored {
            conn: &conn,
            gen_id: 1,
            index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
            spent: 0,
        };
        assert_eq!(whole, query(graph, &q, &r, served()).unwrap());
        seen.extend(ids(&whole));
        match whole["nextCursor"].as_str() {
            Some(next) => r.cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen, expected);
    // Each page reads one index range from its cursor: no temporary sort of every match.
    for (sql, index) in [
        (crate::search::store::INBOUND, "edges_target"),
        (crate::search::store::CHILDREN, "records_parent"),
    ] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let count = stmt.parameter_count();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter((0..count).map(|_| 1)), |row| {
                row.get(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("; ");
        assert!(
            plan.contains(index) && !plan.contains("TEMP B-TREE"),
            "{plan}"
        );
    }
}

#[test]
fn a_page_past_its_scan_cap_is_a_work_limit_not_an_empty_complete_answer() {
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
        node(20, Some(1), "SYMBOL", "Button", vec![]),
    ];
    // Every use but the last is hidden, so the visible one lies past the cap.
    let last = 1000 + SCAN as u32 + 1;
    for i in 1000..=last {
        let extra = if i < last {
            vec![("visible", K::Bool(false))]
        } else {
            vec![]
        };
        nodes.push(instance(i, 1, "Use", 20, extra));
    }
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    assert_eq!(index.stop, None);
    let conn = stored(&index);
    let src = source();
    let served = || Served {
        source: &src,
        gen_id: 1,
    };
    let mut r = req("0:20");
    r.relations = vec!["instance_of".into()];
    let q = r.validate().unwrap();
    let mut disk = Stored {
        conn: &conn,
        gen_id: 1,
        index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
        spent: 0,
    };
    for out in [
        query(&mut Whole(&index), &q, &r, served()),
        query(&mut disk, &q, &r, served()),
    ] {
        let kind = out
            .unwrap_err()
            .downcast_ref::<FigError>()
            .map(FigError::kind);
        assert_eq!(kind, Some(ErrorKind::QueryWorkLimit));
    }
    r.include_hidden = true;
    r.limit = 10;
    let q = r.validate().unwrap();
    assert_eq!(
        ids(&query(&mut Whole(&index), &q, &r, served()).unwrap()).len(),
        10
    );
}

#[test]
fn styled_text_and_vector_region_images_and_capped_prototypes_are_kept_or_partial() {
    let image = |b: u8| {
        obj(vec![(
            "styleOverrideTable",
            K::Array(vec![obj(vec![
                ("styleID", K::UInt(1)),
                (
                    "fillPaints",
                    K::Array(vec![obj(vec![
                        ("type", K::Enum("PaintType", "IMAGE")),
                        ("image", obj(vec![("hash", K::Bytes(vec![b]))])),
                    ])]),
                ),
            ])]),
        )])
    };
    // 17 actions in one interaction: the reader keeps 16 and reports truncation.
    let actions = K::Array(
        (0..17)
            .map(|_| {
                obj(vec![
                    ("transitionNodeID", guid(2)),
                    ("navigationType", K::Enum("NavigationType", "NAVIGATE")),
                ])
            })
            .collect(),
    );
    let click = obj(vec![
        (
            "event",
            obj(vec![(
                "interactionType",
                K::Enum("InteractionType", "ON_CLICK"),
            )]),
        ),
        ("actions", actions),
    ]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Target", vec![]),
            node(3, Some(1), "TEXT", "Label", vec![("textData", image(0x01))]),
            node(
                4,
                Some(1),
                "VECTOR",
                "Shape",
                vec![("vectorData", image(0x02))],
            ),
            node(
                5,
                Some(1),
                "FRAME",
                "Menu",
                vec![("prototypeInteractions", K::Array(vec![click]))],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    for (hash, id, evidence) in [
        (
            "image:01",
            "0:3",
            "textData.styleOverrideTable[0].fillPaints[0]",
        ),
        (
            "image:02",
            "0:4",
            "vectorData.styleOverrideTable[0].fillPaints[0]",
        ),
    ] {
        let mut r = req(hash);
        r.resource = true;
        let out = run(&index, &r).unwrap();
        assert_eq!(
            (ids(&out), &out["results"][0]["evidence"]),
            (vec![id.to_owned()], &json!(evidence))
        );
    }
    let mut r = req("0:5");
    r.outgoing = true;
    r.relations = vec!["prototype_target".into()];
    let out = run(&index, &r).unwrap();
    assert_eq!(
        (
            &out["coverage"]["complete"],
            &out["coverage"]["layers"]["relations"]["kinds"]["prototype_target"]
        ),
        (&json!(false), &json!("partial"))
    );
}

#[test]
fn definitions_stop_on_spent_bounds_before_scanning_records() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Plain", vec![]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let mut spent = bounds(WORK);
    spent.deadline = std::time::Instant::now();
    // No record has a resolved target, so only the charge per record can stop it.
    assert_eq!(
        (
            definitions(&tree, &index.records, &mut spent).is_none(),
            spent.stop
        ),
        (true, Some(crate::expand::Stop::Deadline))
    );
}

#[test]
fn a_deep_nested_instance_id_from_search_is_a_valid_usages_target() {
    // 120 nested components, each holding an instance of the next: the innermost expanded id
    // is far past 1 KiB, yet within what an index stores.
    let base = 1_000_000;
    let depth = 120;
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
        instance(5, 1, "Top", base, vec![]),
    ];
    for k in 0..=depth {
        let symbol = base + 2 * k;
        nodes.push(node(symbol, Some(1), "SYMBOL", "Level", vec![]));
        if k < depth {
            let name = if k + 1 == depth { "Deepest" } else { "Step" };
            nodes.push(instance(symbol + 1, symbol, name, symbol + 2, vec![]));
        }
    }
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let tree = Tree::new(&message);
    // A supported stored scope: the page.
    let index = build(&tree, "0:1", bounds(WORK));
    let found = crate::search::search_index(
        &index,
        &crate::search::Request {
            query: "Deepest".into(),
            scope: "0:1".into(),
            types: Vec::new(),
            include_hidden: false,
            include_internal: false,
            limit: 50,
            cursor: None,
        },
        source(),
    )
    .unwrap();
    let id = found["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["ref"]["nodeId"].as_str())
        .max_by_key(|id| id.len())
        .unwrap()
        .to_owned();
    assert!(
        id.len() > 1024 && id.len() <= crate::search::store::ID_BYTES,
        "{}",
        id.len()
    );
    let mut r = req(&id);
    r.scope = "0:1".into();
    r.outgoing = true;
    r.relations = vec!["instance_of".into()];
    let out = run(&index, &r).unwrap();
    let main = format!("0:{}", base + 2 * depth);
    assert_eq!(out["results"][0]["target"]["ref"]["nodeId"], json!(main));
    r.target = "k".repeat(crate::search::store::ID_BYTES + 1);
    assert!(r.validate().is_err());
    // Scopes stay stored nodes: the sublayer as a scope names its stored instance instead.
    let Err(err) = crate::search::index_tree(&tree, &id, crate::search::deadline()) else {
        panic!("a sublayer scope was accepted");
    };
    let err = err.downcast_ref::<FigError>().unwrap();
    assert_eq!(
        (
            err.kind(),
            err.to_string().contains("search its stored instance")
        ),
        (ErrorKind::UnsupportedNode, true)
    );
}

#[test]
fn a_paint_scan_is_charged_even_when_nothing_links() {
    // 600 plain gradient paints with stops: no alias, no image, so no edge.
    let plain = K::Array(
        (0..600)
            .map(|_| {
                obj(vec![
                    ("type", K::Enum("PaintType", "GRADIENT_LINEAR")),
                    ("stopsVar", K::Array(vec![obj(vec![]), obj(vec![])])),
                ])
            })
            .collect(),
    );
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(
                2,
                Some(1),
                "RECTANGLE",
                "Plain",
                vec![("fillPaints", plain)],
            ),
        ]),
    )]);
    let tree = Tree::new(&message);
    let (mut unverified, mut unread) = (0, 0);
    let mut roomy = bounds(WORK);
    let before = roomy.work;
    let edges = extract(
        &tree,
        "0:2",
        "RECTANGLE",
        None,
        &mut roomy,
        &mut unverified,
        &mut unread,
    );
    // 600 paints and 1,200 stops were visited and paid for although nothing linked.
    assert_eq!(
        (edges.map(|e| e.len()), before - roomy.work >= 1800),
        (Some(0), true)
    );
    let mut tight = bounds(100);
    assert_eq!(
        (
            extract(
                &tree,
                "0:2",
                "RECTANGLE",
                None,
                &mut tight,
                &mut unverified,
                &mut unread
            )
            .is_none(),
            tight.stop
        ),
        (true, Some(crate::expand::Stop::Work))
    );
    let mut late = bounds(WORK);
    late.deadline = std::time::Instant::now();
    assert!(
        extract(
            &tree,
            "0:2",
            "RECTANGLE",
            None,
            &mut late,
            &mut unverified,
            &mut unread
        )
        .is_none()
    );
}

#[test]
fn a_swapped_instance_s_own_prototype_links_are_partial() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Target", vec![]),
            node(10, Some(1), "SYMBOL", "Card", vec![]),
            // A nested instance with its own link, swapped by the outer instance to 0:30.
            instance(12, 10, "Slot", 20, vec![click_to(2)]),
            node(20, Some(1), "SYMBOL", "Original", vec![]),
            node(30, Some(1), "SYMBOL", "Replacement", vec![]),
            instance(
                40,
                1,
                "Use",
                10,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![obj(vec![
                        path(&[12]),
                        ("overriddenSymbolID", guid(30)),
                    ])]),
                )],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let mut r = req("I0:40;0:12");
    r.outgoing = true;
    r.relations = vec!["prototype_target".into()];
    let out = run(&index, &r).unwrap();
    // The link stays, with its evidence; that it applies over the swap is not claimed.
    assert_eq!(
        (
            &out["results"][0]["target"]["ref"]["nodeId"],
            &out["coverage"]["layers"]["relations"]["kinds"]["prototype_target"]
        ),
        (&json!("0:2"), &json!("partial"))
    );
}
