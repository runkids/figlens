use super::super::tests::{bounds, guid, instance, node, obj, path, stored};
use super::*;
use crate::search::store::Stored;
use crate::search::{WORK, build, build_as};
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
fn a_text_range_takes_its_style_from_the_last_row_and_its_paints_from_the_first_painted_row() {
    let style = |id: u32| obj(vec![("guid", guid(id))]);
    let paints = |image: u8, var: u32| {
        K::Array(vec![
            obj(vec![
                ("type", K::Enum("PaintType", "IMAGE")),
                ("image", obj(vec![("hash", K::Bytes(vec![image]))])),
            ]),
            obj(vec![
                ("type", K::Enum("PaintType", "SOLID")),
                ("colorVar", alias(var)),
            ]),
        ])
    };
    let row = |sid: u32, fill: Option<(u8, u32)>, text: Option<u32>| {
        let mut fields = vec![("styleID", K::UInt(sid))];
        fields.extend(fill.map(|(i, v)| ("fillPaints", paints(i, v))));
        fields.extend(text.map(|t| ("styleIdForText", style(t))));
        obj(fields)
    };
    let rows = vec![
        // styleID 1: paints from row 0, the style from row 1.
        row(1, Some((0xa1, 20)), None),
        row(1, Some((0xb1, 21)), Some(11)),
        // styleID 2: row 2 has no paints and is not the last, so row 3 supplies both.
        row(2, None, Some(12)),
        row(2, Some((0xc2, 22)), None),
        // styleID 3 is used by no character.
        row(3, Some((0xd3, 23)), Some(12)),
    ];
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(11, Some(1), "TEXT", "Heading", vec![]),
            node(12, Some(1), "TEXT", "Body", vec![]),
            node(20, Some(1), "VARIABLE", "A", vec![]),
            node(21, Some(1), "VARIABLE", "B", vec![]),
            node(22, Some(1), "VARIABLE", "C", vec![]),
            node(23, Some(1), "VARIABLE", "D", vec![]),
            node(
                3,
                Some(1),
                "TEXT",
                "Label",
                vec![(
                    "textData",
                    obj(vec![
                        ("characterStyleIDs", K::Array(vec![K::UInt(1), K::UInt(2)])),
                        ("styleOverrideTable", K::Array(rows)),
                    ]),
                )],
            ),
        ]),
    )]);
    let tree = Tree::new(&message);
    let mut r = req("0:3");
    r.outgoing = true;
    let edges = |work| {
        // Too little work leaves the node unindexed, which also lists nothing as complete.
        let Ok(out) = run(&build(&tree, "0:0", bounds(work)), &r) else {
            return (vec![], json!(false));
        };
        let edges: Vec<String> = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{} {} {}", e["relation"], e["target"], e["evidence"]))
            .collect();
        (edges, out["coverage"]["complete"].clone())
    };
    let (all, complete) = edges(WORK);
    let rel = |e: &str| all.iter().filter(|x| x.contains(e)).count();
    assert_eq!(complete, true);
    assert_eq!(
        [
            rel("image:a1"),
            rel("image:c2"),
            rel("image:b1"),
            rel("image:d3")
        ],
        [1, 1, 0, 0]
    );
    assert_eq!(
        [
            rel("VariableID:0:20"),
            rel("VariableID:0:22"),
            rel("0:21"),
            rel("0:23")
        ],
        [1, 1, 0, 0]
    );
    assert_eq!(rel("uses_style"), 1);
    assert!(
        all.iter()
            .any(|e| e.contains("0:11") && e.contains("[1].styleIdForText"))
    );
    assert!(rel("styleOverrideTable[3].fillPaints[0]") == 1);
    // With too little work the side is partial, never complete with other rows' edges.
    for work in 0..400 {
        let (some, complete) = edges(work);
        assert!(complete == false || some == all, "{work}: {some:?}");
    }
}

#[test]
fn styled_text_and_vector_region_images_and_capped_prototypes_are_kept_or_partial() {
    let table = |b: u8| {
        (
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
        )
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
            node(
                3,
                Some(1),
                "TEXT",
                "Label",
                vec![(
                    "textData",
                    obj(vec![
                        ("characterStyleIDs", K::Array(vec![K::UInt(1)])),
                        table(0x01),
                    ]),
                )],
            ),
            node(
                4,
                Some(1),
                "VECTOR",
                "Shape",
                vec![
                    ("vectorData", obj(vec![table(0x02)])),
                    (
                        "fillGeometry",
                        K::Array(vec![obj(vec![("styleID", K::UInt(1))])]),
                    ),
                ],
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
        Some((&mut unverified, &mut unread)),
        None,
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
                Some((&mut unverified, &mut unread)),
                None,
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
            Some((&mut unverified, &mut unread)),
            None,
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

#[test]
fn an_unused_local_variable_is_a_known_resource_with_no_uses() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Plain", vec![]),
            node(9, Some(0), "CANVAS", "Internal", vec![]),
            node(90, Some(9), "VARIABLE", "Unused", vec![]),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:1", bounds(WORK));
    let conn = stored(&index);
    let mut r = req("VariableID:0:90");
    r.resource = true;
    r.scope = "0:1".into();
    let q = r.validate().unwrap();
    let src = source();
    let served = || Served {
        source: &src,
        gen_id: 1,
    };
    let whole = query(&mut Whole(&index), &q, &r, served()).unwrap();
    let mut disk = Stored {
        conn: &conn,
        gen_id: 1,
        index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
        spent: 0,
    };
    assert_eq!(whole, query(&mut disk, &q, &r, served()).unwrap());
    assert_eq!(
        (
            whole["results"].as_array().map(Vec::len),
            &whole["definition"]["name"]
        ),
        (Some(0), &json!("Unused"))
    );
}

#[test]
fn vector_region_fill_and_gradient_stop_variables_are_bound() {
    let region = obj(vec![(
        "styleOverrideTable",
        K::Array(vec![obj(vec![
            ("styleID", K::UInt(1)),
            (
                "fillPaints",
                K::Array(vec![
                    obj(vec![
                        ("type", K::Enum("PaintType", "SOLID")),
                        ("colorVar", alias(60)),
                    ]),
                    obj(vec![
                        ("type", K::Enum("PaintType", "GRADIENT_LINEAR")),
                        (
                            "stopsVar",
                            K::Array(vec![obj(vec![("colorVar", alias(61))])]),
                        ),
                    ]),
                ]),
            ),
        ])]),
    )]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(
                4,
                Some(1),
                "VECTOR",
                "Shape",
                vec![
                    ("vectorData", region),
                    (
                        "fillGeometry",
                        K::Array(vec![obj(vec![("styleID", K::UInt(1))])]),
                    ),
                ],
            ),
            node(60, Some(1), "VARIABLE", "Ink", vec![]),
            node(61, Some(1), "VARIABLE", "Stop", vec![]),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let conn = stored(&index);
    let src = source();
    let served = || Served {
        source: &src,
        gen_id: 1,
    };
    for variable in ["VariableID:0:60", "VariableID:0:61"] {
        let r = req(variable);
        let q = r.validate().unwrap();
        let whole = query(&mut Whole(&index), &q, &r, served()).unwrap();
        let mut disk = Stored {
            conn: &conn,
            gen_id: 1,
            index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
            spent: 0,
        };
        assert_eq!(
            (ids(&whole), &whole["coverage"]["complete"]),
            (vec!["0:4".to_owned()], &json!(true))
        );
        assert_eq!(
            whole["results"],
            query(&mut disk, &q, &r, served()).unwrap()["results"]
        );
        assert_eq!(whole["results"][0]["evidence"], "vectorRegionFills");
    }
}

#[test]
fn a_bound_field_too_large_to_read_charges_the_visits_that_measured_it() {
    let doc = |paints: usize| {
        let paint = obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            ("colorVar", alias(60)),
        ]);
        let fills = K::Array(vec![paint; paints]);
        obj(vec![(
            "nodeChanges",
            K::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", vec![]),
                node(
                    4,
                    Some(1),
                    "RECTANGLE",
                    "Shape",
                    vec![("fillPaints", fills)],
                ),
            ]),
        )])
    };
    let small = doc(1);
    let small = Tree::new(&small);
    let work = (1..10_000)
        .find(|w| build(&small, "0:0", bounds(*w)).stop.is_none())
        .unwrap();
    // The first paint binds a variable, so the prefilter passes at once; measuring the rest
    // walks VISITS values, which must come out of the same work budget.
    let big = doc(VISITS);
    let index = build(&Tree::new(&big), "0:0", bounds(work + 10_000));
    assert_eq!(index.stop, Some("work_limit"));
}

#[test]
fn a_stale_snapshot_recovery_repeats_the_callers_selectors() {
    let index = index();
    let mut r = req("0:20");
    r.snapshot = Some("sha256:old".into());
    r.outgoing = true;
    r.relations = vec!["instance_of".into(), "uses_style".into()];
    r.include_hidden = true;
    r.limit = 7;
    let e = run(&index, &r).unwrap_err();
    assert!(
        e.to_string().contains("read from snapshot sha256:old"),
        "{e}"
    );
    let mut arguments =
        e.downcast_ref::<FigError>().unwrap().recovery().unwrap()[0]["arguments"].clone();
    assert_ne!(arguments["snapshotId"], "sha256:old");
    arguments.as_object_mut().unwrap().remove("snapshotId");
    assert_eq!(
        arguments,
        json!({"node": "0:20", "scope": r.scope, "limit": 7, "direction": "out",
            "relation": "instance_of,uses_style", "includeHidden": true})
    );
}

#[test]
fn text_styles_reach_through_used_range_rows_and_lend_their_font_bindings() {
    let font_size = |var: u32| {
        (
            "parameterConsumptionMap",
            obj(vec![(
                "entries",
                K::Array(vec![obj(vec![
                    ("variableField", K::Enum("VariableField", "FONT_SIZE")),
                    ("variableData", alias(var)),
                ])]),
            )]),
        )
    };
    let style = |id: u32| obj(vec![("guid", guid(id))]);
    let row = |sid: u64, id: u32| {
        obj(vec![
            ("styleID", K::UInt(sid as u32)),
            ("styleIdForText", style(id)),
        ])
    };
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Styles", vec![]),
            node(2, Some(0), "CANVAS", "Screen", vec![]),
            node(3, Some(2), "FRAME", "Frame", vec![]),
            node(10, Some(1), "VARIABLE", "Size", vec![]),
            node(13, Some(1), "VARIABLE", "Own size", vec![]),
            node(14, Some(1), "VARIABLE", "Weight", vec![]),
            node(
                11,
                Some(1),
                "TEXT",
                "Heading",
                vec![(
                    "parameterConsumptionMap",
                    obj(vec![(
                        "entries",
                        K::Array(vec![
                            obj(vec![
                                ("variableField", K::Enum("VariableField", "FONT_SIZE")),
                                ("variableData", alias(10)),
                            ]),
                            obj(vec![
                                ("variableField", K::Enum("VariableField", "FONT_STYLE")),
                                ("variableData", alias(14)),
                            ]),
                        ]),
                    )]),
                )],
            ),
            // Two rows of one styleID: the last applies.
            node(
                7,
                Some(3),
                "TEXT",
                "Repeated",
                vec![(
                    "textData",
                    obj(vec![
                        ("characterStyleIDs", K::Array(vec![K::UInt(1)])),
                        ("styleOverrideTable", K::Array(vec![row(1, 11), row(1, 12)])),
                    ]),
                )],
            ),
            node(12, Some(1), "TEXT", "Body", vec![]),
            node(
                4,
                Some(3),
                "TEXT",
                "Title",
                vec![("styleIdForText", style(11))],
            ),
            node(
                5,
                Some(3),
                "TEXT",
                "Mixed",
                vec![(
                    "textData",
                    obj(vec![
                        (
                            "characterStyleIDs",
                            K::Array(vec![K::UInt(0), K::UInt(1), K::UInt(1)]),
                        ),
                        ("styleOverrideTable", K::Array(vec![row(1, 12), row(2, 11)])),
                    ]),
                )],
            ),
            node(
                6,
                Some(3),
                "TEXT",
                "Sized",
                vec![("styleIdForText", style(11)), font_size(13)],
            ),
        ]),
    )]);
    // The styles and variables lie outside the indexed frame; their uses inside it are listed.
    let index = build(&Tree::new(&message), "0:3", bounds(WORK));
    let incoming = |target: &str| {
        let mut r = req(target);
        r.scope = "0:3".into();
        let out = run(&index, &r).unwrap();
        let evidence: Vec<Json> = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["evidence"].clone())
            .collect();
        (ids(&out), evidence)
    };
    // Only the row a character uses applies; row 1 (styleID 2) is unused.
    assert_eq!(
        incoming("0:12"),
        (
            vec!["0:5".to_owned(), "0:7".to_owned()],
            vec![
                json!("textData.styleOverrideTable[0].styleIdForText"),
                json!("textData.styleOverrideTable[1].styleIdForText")
            ]
        )
    );
    assert_eq!(
        incoming("0:11"),
        (
            vec!["0:4".to_owned(), "0:6".to_owned()],
            vec![json!("styleIdForText"), json!("styleIdForText")]
        )
    );
    // The style's font-size binding reaches the text without one of its own.
    assert_eq!(
        incoming("VariableID:0:10"),
        (
            vec!["0:4".to_owned()],
            vec![json!("styleIdForText.fontSize")]
        )
    );
    assert_eq!(incoming("VariableID:0:13").0, ["0:6"]);
    // The text with its own size still takes the style's weight.
    assert_eq!(incoming("VariableID:0:14").0, ["0:4", "0:6"]);
    let outgoing = |id: &str, relation: &str| {
        let mut r = req(id);
        r.scope = "0:3".into();
        r.outgoing = true;
        r.relations = vec![relation.into()];
        let out = run(&index, &r).unwrap();
        out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let t = &r["target"];
                let target = t["ref"]["nodeId"].as_str().or(t["resource"].as_str());
                (target.unwrap_or("").to_owned(), r["evidence"].clone())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        outgoing("0:5", "uses_style"),
        [(
            "0:12".to_owned(),
            json!("textData.styleOverrideTable[0].styleIdForText")
        )]
    );
    assert_eq!(
        outgoing("0:4", "binds_variable"),
        [
            (
                "VariableID:0:10".to_owned(),
                json!("styleIdForText.fontSize")
            ),
            (
                "VariableID:0:14".to_owned(),
                json!("styleIdForText.fontWeight")
            )
        ]
    );
}

#[test]
fn a_text_too_long_to_select_its_rows_leaves_its_range_styles_partial() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "TEXT", "Body", vec![]),
            node(
                3,
                Some(1),
                "TEXT",
                "Long",
                vec![(
                    "textData",
                    obj(vec![
                        ("characterStyleIDs", K::Array(vec![K::UInt(1); VISITS + 1])),
                        (
                            "styleOverrideTable",
                            K::Array(vec![obj(vec![
                                ("styleID", K::UInt(1)),
                                ("styleIdForText", obj(vec![("guid", guid(2))])),
                                (
                                    "fillPaints",
                                    K::Array(vec![obj(vec![
                                        ("type", K::Enum("PaintType", "IMAGE")),
                                        ("image", obj(vec![("hash", K::Bytes(vec![0x20]))])),
                                    ])]),
                                ),
                            ])]),
                        ),
                    ]),
                )],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let out = run(&index, &req("0:2")).unwrap();
    assert_eq!(
        (ids(&out).len(), &out["coverage"]["complete"]),
        (0, &json!(false))
    );
    // An image is no definition: with its only use unread, the answer is not found, never a
    // complete empty one.
    let mut image = req("image:20");
    image.resource = true;
    assert!(run(&index, &image).is_err());
}

#[test]
fn an_instance_text_override_applies_its_range_style_only_where_characters_use_it() {
    let text = |ids: Option<Vec<K<'static>>>| {
        let mut fields = vec![(
            "styleOverrideTable",
            K::Array(vec![obj(vec![
                ("styleID", K::UInt(1)),
                ("styleIdForText", obj(vec![("guid", guid(12))])),
                (
                    "fillPaints",
                    K::Array(vec![obj(vec![
                        ("type", K::Enum("PaintType", "IMAGE")),
                        ("image", obj(vec![("hash", K::Bytes(vec![0x30]))])),
                    ])]),
                ),
            ])]),
        )];
        if let Some(ids) = ids {
            fields.push(("characterStyleIDs", K::Array(ids)));
        }
        (
            "symbolOverrides",
            K::Array(vec![obj(vec![path(&[21]), ("textData", obj(fields))])]),
        )
    };
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Components", vec![]),
            node(2, Some(0), "CANVAS", "Screen", vec![]),
            node(3, Some(2), "FRAME", "Frame", vec![]),
            node(12, Some(1), "TEXT", "Body", vec![]),
            node(20, Some(1), "SYMBOL", "Chip", vec![]),
            node(21, Some(20), "TEXT", "Label", vec![]),
            instance(30, 3, "Used", 20, vec![text(Some(vec![K::UInt(1)]))]),
            // An override without characterStyleIDs uses no row, as in REST.
            instance(31, 3, "Unused", 20, vec![text(None)]),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:3", bounds(WORK));
    let mut r = req("0:12");
    r.scope = "0:3".into();
    let out = run(&index, &r).unwrap();
    assert_eq!(
        (ids(&out), &out["results"][0]["evidence"]),
        (
            vec!["I0:30;0:21".to_owned()],
            &json!("textData.styleOverrideTable[0].styleIdForText")
        )
    );
    let mut image = req("image:30");
    image.resource = true;
    image.scope = "0:3".into();
    assert_eq!(ids(&run(&index, &image).unwrap()), ["I0:30;0:21"]);
}

#[test]
fn only_text_rows_characters_use_lend_their_images_and_paint_bindings() {
    let image = |b: u8| {
        K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "IMAGE")),
            ("image", obj(vec![("hash", K::Bytes(vec![b]))])),
        ])])
    };
    let row =
        |sid: u32, paints: K<'static>| obj(vec![("styleID", K::UInt(sid)), ("fillPaints", paints)]);
    let rows = |used: Vec<u32>| {
        obj(vec![
            (
                "characterStyleIDs",
                K::Array(used.into_iter().map(K::UInt).collect()),
            ),
            (
                "styleOverrideTable",
                K::Array(vec![
                    row(1, image(0x10)),
                    row(2, image(0x11)),
                    row(4, solid(40)),
                    row(5, solid(41)),
                    // Two rows of one styleID: the first with fills paints, as rendered.
                    row(3, image(0x12)),
                    row(3, image(0x13)),
                ]),
            ),
        ])
    };
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(40, Some(1), "VARIABLE", "Used", vec![]),
            node(41, Some(1), "VARIABLE", "Unused", vec![]),
            node(
                2,
                Some(1),
                "TEXT",
                "Body",
                vec![("textData", rows(vec![1, 1, 3, 4]))],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let users = |target: &str| {
        let mut r = req(target);
        r.resource = true;
        // An image is no definition: with no use left it is not found, while a variable answers
        // with no uses.
        run(&index, &r).ok().map(|out| ids(&out))
    };
    assert_eq!(users("image:10"), Some(vec!["0:2".to_owned()]));
    assert_eq!(users("image:11"), None);
    assert_eq!(users("image:12"), Some(vec!["0:2".to_owned()]));
    assert_eq!(users("image:13"), None);
    assert_eq!(users("VariableID:0:40"), Some(vec!["0:2".to_owned()]));
    assert_eq!(users("VariableID:0:41"), Some(vec![]));
    let mut r = req("0:2");
    r.outgoing = true;
    r.relations = vec!["binds_variable".into(), "uses_image".into()];
    let out = run(&index, &r).unwrap();
    let edges: Vec<(&Json, &Json, &Json)> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (&e["relation"], &e["target"]["resource"], &e["evidence"]))
        .collect();
    assert_eq!(
        edges,
        [
            (
                &json!("binds_variable"),
                &json!("VariableID:0:40"),
                &json!("textRangeFills")
            ),
            (
                &json!("uses_image"),
                &json!("image:10"),
                &json!("textData.styleOverrideTable[0].fillPaints[0]")
            ),
            (
                &json!("uses_image"),
                &json!("image:12"),
                &json!("textData.styleOverrideTable[4].fillPaints[0]")
            ),
        ]
    );
}

#[test]
fn only_vector_regions_a_geometry_paints_lend_their_images_and_paint_bindings() {
    let image = |b: u8| {
        K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "IMAGE")),
            ("image", obj(vec![("hash", K::Bytes(vec![b]))])),
        ])])
    };
    let row = |sid: u32, paints: Option<K<'static>>| {
        let mut fields = vec![("styleID", K::UInt(sid))];
        fields.extend(paints.map(|p| ("fillPaints", p)));
        obj(fields)
    };
    let geometry = |ids: &[Option<u32>]| {
        (
            "fillGeometry",
            K::Array(
                ids.iter()
                    .map(|id| obj(id.map(|id| ("styleID", K::UInt(id))).into_iter().collect()))
                    .collect(),
            ),
        )
    };
    let vector = obj(vec![(
        "styleOverrideTable",
        K::Array(vec![
            row(7, Some(image(0x40))),
            // A later row of a painted styleID is not drawn.
            row(7, Some(image(0x41))),
            row(8, Some(image(0x42))),
            row(0, Some(image(0x43))),
            row(9, Some(solid(51))),
            // The first row of one styleID that has fill paints wins.
            row(10, None),
            row(10, Some(solid(50))),
        ]),
    )]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(50, Some(1), "VARIABLE", "Painted", vec![]),
            node(51, Some(1), "VARIABLE", "Unpainted", vec![]),
            node(20, Some(1), "SYMBOL", "Icon", vec![]),
            // A geometry without a styleID paints style 0.
            node(
                21,
                Some(20),
                "VECTOR",
                "Shape",
                vec![
                    ("vectorData", vector),
                    geometry(&[Some(7), Some(7), None, Some(10)]),
                ],
            ),
            // An instance without a derived outline paints the source's regions.
            instance(31, 1, "Plain", 20, vec![]),
            // The instance re-derives the outline to paint style 8 only.
            instance(
                30,
                1,
                "Resized",
                20,
                vec![(
                    "derivedSymbolData",
                    K::Array(vec![obj(vec![path(&[21]), geometry(&[Some(8)])])]),
                )],
            ),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let users = |target: &str| {
        let mut r = req(target);
        r.resource = true;
        // An image is no definition: with no use left it is not found, while a variable answers
        // with no uses.
        run(&index, &r).ok().map(|out| ids(&out))
    };
    assert_eq!(
        users("image:40"),
        Some(vec!["0:21".to_owned(), "I0:31;0:21".to_owned()])
    );
    assert_eq!(users("image:41"), None);
    assert_eq!(users("image:42"), Some(vec!["I0:30;0:21".to_owned()]));
    assert_eq!(
        users("image:43"),
        Some(vec!["0:21".to_owned(), "I0:31;0:21".to_owned()])
    );
    assert_eq!(
        users("VariableID:0:50"),
        Some(vec!["0:21".to_owned(), "I0:31;0:21".to_owned()])
    );
    assert_eq!(users("VariableID:0:51"), Some(vec![]));
    let mut r = req("0:21");
    r.outgoing = true;
    r.relations = vec!["binds_variable".into(), "uses_image".into()];
    let out = run(&index, &r).unwrap();
    let edges: Vec<(&Json, &Json, &Json)> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (&e["relation"], &e["target"]["resource"], &e["evidence"]))
        .collect();
    assert_eq!(
        edges,
        [
            (
                &json!("binds_variable"),
                &json!("VariableID:0:50"),
                &json!("vectorRegionFills")
            ),
            (
                &json!("uses_image"),
                &json!("image:40"),
                &json!("vectorData.styleOverrideTable[0].fillPaints[0]")
            ),
            (
                &json!("uses_image"),
                &json!("image:43"),
                &json!("vectorData.styleOverrideTable[3].fillPaints[0]")
            ),
        ]
    );
}

#[test]
fn a_queried_definition_too_long_for_the_reply_is_a_budget_error() {
    let long = "v".repeat(crate::search::REPLY_BYTES + 1);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Plain", vec![]),
            node(9, Some(0), "CANVAS", "Internal", vec![]),
            node(90, Some(9), "VARIABLE", &long, vec![]),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:1", bounds(WORK));
    let mut r = req("VariableID:0:90");
    r.scope = "0:1".into();
    let err = run(&index, &r).unwrap_err();
    assert_eq!(
        err.downcast_ref::<FigError>().map(FigError::kind),
        Some(ErrorKind::ResponseBudget)
    );
}

#[test]
fn an_outgoing_query_with_no_rows_still_checks_its_definition_against_the_budget() {
    let long = "n".repeat(crate::search::REPLY_BYTES + 1);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Short", vec![]),
        ]),
    )]);
    let mut index = build(&Tree::new(&message), "0:0", bounds(WORK));
    // A name too long for the reply, set after a quick build so only the reply budget refuses.
    let frame = index.records.iter_mut().find(|r| r.id == "0:2").unwrap();
    frame.name = long;
    // A relation the node has none of: the reply has no rows, only the definition.
    let mut r = req("0:2");
    r.outgoing = true;
    r.relations = vec!["binds_variable".into()];
    let err = run(&index, &r).unwrap_err();
    assert_eq!(
        err.downcast_ref::<FigError>().map(FigError::kind),
        Some(ErrorKind::ResponseBudget)
    );
}

#[test]
fn a_prototype_projection_is_reserved_from_the_build_bounds_even_without_destinations() {
    // A scroll action has no destination, so no edge would ever charge for reading it.
    let scroll = obj(vec![(
        "actions",
        K::Array(vec![obj(vec![(
            "navigationType",
            K::Enum("NavigationType", "SCROLL_TO"),
        )])]),
    )]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(
                2,
                Some(1),
                "FRAME",
                "Menu",
                vec![("prototypeInteractions", K::Array(vec![scroll]))],
            ),
        ]),
    )]);
    let tree = Tree::new(&message);
    let tight = crate::expand::Bounds {
        bytes: 8 * 1024,
        ..bounds(WORK)
    };
    assert_eq!(build_as(&tree, "0:0", tight, false).stop, None);
    assert!(build(&tree, "0:0", tight).stop.is_some());
}

#[test]
fn a_hidden_use_before_the_visible_ones_is_reported_by_some_page() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(20, Some(1), "COMPONENT", "Button", vec![]),
            instance(30, 1, "Hidden", 20, vec![("visible", K::Bool(false))]),
            instance(31, 1, "One", 20, vec![]),
            instance(32, 1, "Two", 20, vec![]),
        ]),
    )]);
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let conn = stored(&index);
    // Its instances, and the page's children through `contains`, each from memory and SQLite.
    for (target, want) in [
        ("0:20", vec!["0:31", "0:32"]),
        ("0:1", vec!["0:20", "0:31", "0:32"]),
    ] {
        for disk in [false, true] {
            let mut r = req(target);
            r.limit = 1;
            if target == "0:1" {
                r.relations = vec!["contains".into()];
            }
            let (mut seen, mut reported) = (Vec::new(), false);
            loop {
                let q = r.validate().unwrap();
                let served = Served {
                    source: &source(),
                    gen_id: 1,
                };
                let out = if disk {
                    let graph = &mut Stored {
                        conn: &conn,
                        gen_id: 1,
                        index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
                        spent: 0,
                    };
                    query(graph, &q, &r, served)
                } else {
                    query(&mut Whole(&index), &q, &r, served)
                }
                .unwrap();
                seen.extend(ids(&out));
                let reasons = out["coverage"]["reasons"].as_array().unwrap();
                reported |= reasons
                    .iter()
                    .any(|x| x["code"] == "hidden_locations_excluded");
                match out["nextCursor"].as_str() {
                    Some(next) => r.cursor = Some(next.to_owned()),
                    None => break,
                }
            }
            assert_eq!(
                (seen, reported),
                (want.iter().map(|s| s.to_string()).collect(), true),
                "{target} disk={disk}"
            );
        }
    }
}

#[test]
fn a_row_paint_estimate_pays_for_every_row_even_an_inactive_one() {
    // Mostly blanked inactive rows, then one with paints: each row is a visit.
    let mut rows = vec![Json::Null; 1000];
    rows.push(json!({"fillPaints": [1, 2]}));
    let mut left = 4;
    assert_eq!(row_paints([&rows, &[]], &mut left), (0, true));
    assert_eq!(left, 0);
    // With room for every row, the paints are estimated after them.
    let mut left = 2 * VISITS;
    let (bytes, cut) = row_paints([&[], &rows], &mut left);
    assert_eq!((bytes > 0, cut, 2 * VISITS - left), (true, false, 1001 + 3));
}

#[test]
fn a_field_estimate_with_little_work_left_spends_it_and_stops_the_side() {
    // The image paint links at once, so only the size estimate walks the 5,000 values.
    let mut paints = vec![obj(vec![("type", K::Enum("PaintType", "IMAGE"))])];
    paints.extend(vec![K::Float(0.0); 5000]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(
                2,
                Some(1),
                "RECTANGLE",
                "R",
                vec![("fillPaints", K::Array(paints))],
            ),
        ]),
    )]);
    let tree = Tree::new(&message);
    for work in [1, 100] {
        let mut b = bounds(work);
        let mut unread = 0;
        assert!(effective(&tree, "0:2", None, &mut b, &mut unread, false).is_none());
        assert_eq!(
            (b.work, b.stop, unread),
            (0, Some(crate::expand::Stop::Work), 0)
        );
    }
}

#[test]
fn a_library_alias_key_is_charged_for_its_projected_copies_before_they_are_made() {
    let doc = |key: &str| {
        let lib = obj(vec![
            ("dataType", K::Enum("VariableDataType", "ALIAS")),
            (
                "value",
                obj(vec![(
                    "alias",
                    obj(vec![(
                        "assetRef",
                        obj(vec![
                            ("key", K::String(key.into())),
                            ("version", K::String("1".into())),
                        ]),
                    )]),
                )]),
            ),
        ]);
        let fills = K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            ("colorVar", lib),
        ])]);
        obj(vec![(
            "nodeChanges",
            K::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", vec![]),
                node(
                    2,
                    Some(1),
                    "RECTANGLE",
                    "Shape",
                    vec![("fillPaints", fills)],
                ),
            ]),
        )])
    };
    let budget = |bytes| crate::expand::Bounds {
        bytes,
        ..bounds(WORK)
    };
    let short = doc("k");
    let short = Tree::new(&short);
    let fits = (1..64)
        .map(|kib| kib * 1024)
        .find(|b| build(&short, "0:0", budget(*b)).stop.is_none())
        .unwrap();
    // The key's field fits the margin once; its projected copies do not. A control character
    // serializes to six bytes in the dedup key.
    for (key, margin) in [("k".repeat(64 * 1024), 2), ("\u{1}".repeat(16 * 1024), 7)] {
        let message = doc(&key);
        let tree = Tree::new(&message);
        let index = build(&tree, "0:0", budget(fits + margin * key.len()));
        assert_eq!(index.stop, Some("byte_limit"), "margin {margin}");
    }
}

#[test]
fn an_overlong_library_style_key_near_the_byte_limit_stays_an_unsupported_edge() {
    let doc = |key: &str| {
        let style = obj(vec![(
            "assetRef",
            obj(vec![("key", K::String(key.into()))]),
        )]);
        obj(vec![(
            "nodeChanges",
            K::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", vec![]),
                node(
                    2,
                    Some(1),
                    "RECTANGLE",
                    "Shape",
                    vec![("styleIdForFill", style)],
                ),
            ]),
        )])
    };
    let budget = |bytes| crate::expand::Bounds {
        bytes,
        ..bounds(WORK)
    };
    let edge = |index: &Index| {
        let mut r = req("0:2");
        r.outgoing = true;
        let out = run(index, &r).unwrap();
        let e = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["relation"] == "uses_style")
            .cloned()
            .unwrap();
        (e["target"].clone(), e["state"].clone())
    };
    let short = doc("k");
    let short = Tree::new(&short);
    let fits = (1..64)
        .map(|kib| kib * 1024)
        .find(|b| build(&short, "0:0", budget(*b)).stop.is_none())
        .unwrap();
    let normal = build(&short, "0:0", budget(fits));
    // The long key's field fits the margin; its target is measured, never copied or charged.
    let long = "k".repeat(64 * 1024);
    let message = doc(&long);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", budget(fits + 4 * long.len()));
    assert_eq!(
        (index.stop, edge(&normal), edge(&index)),
        (
            None,
            (json!({"resource": "StyleKey:k"}), json!("external")),
            (Json::Null, json!("unsupported"))
        )
    );
}

#[test]
fn an_overlong_image_hash_stays_an_unsupported_edge_and_a_short_one_is_kept() {
    let doc = |hash: Vec<u8>| {
        let fills = K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "IMAGE")),
            ("image", obj(vec![("hash", K::Bytes(hash))])),
        ])]);
        obj(vec![(
            "nodeChanges",
            K::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", vec![]),
                node(
                    2,
                    Some(1),
                    "RECTANGLE",
                    "Shape",
                    vec![("fillPaints", fills)],
                ),
            ]),
        )])
    };
    let edge = |message: &K| {
        let index = build(&Tree::new(message), "0:0", bounds(WORK));
        let mut r = req("0:2");
        r.outgoing = true;
        let out = run(&index, &r).unwrap();
        let e = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["relation"] == "uses_image")
            .cloned()
            .unwrap();
        (index.stop, e["state"].clone(), e["target"].is_null())
    };
    // Hex doubles the hash, so half of ID_BYTES plus one is already too long to keep.
    let long = vec![0xab; super::super::store::ID_BYTES / 2 + 1];
    assert_eq!(
        (edge(&doc(vec![0xab, 0xcd])), edge(&doc(long))),
        (
            (None, json!("recorded"), false),
            (None, json!("unsupported"), true)
        )
    );
}

/// Library copies: mains whose naive `namespace/key` joins collide, a main with no namespace,
/// one style key under two namespaces, one under one, one with none, and one with no copy; a
/// variable with one of those keys, and one with no copy.
fn libraries_design() -> K<'static> {
    let s = |v: &str| K::String(v.into());
    let named = |mut extra: Vec<(&'static str, K<'static>)>, ns: Option<&str>| {
        if let Some(ns) = ns {
            extra.push(("sourceLibraryKey", s(ns)));
        }
        extra
    };
    let main = |id, key: &str, ns| {
        node(
            id,
            Some(1),
            "SYMBOL",
            "Button",
            named(vec![("componentKey", s(key))], ns),
        )
    };
    let style = |id, key: &str, ns| {
        let extra = vec![("styleType", K::Enum("StyleType", "FILL")), ("key", s(key))];
        node(id, Some(1), "ROUNDED_RECTANGLE", "Brand", named(extra, ns))
    };
    let variable = |id, key: &str, ns| {
        node(
            id,
            Some(1),
            "VARIABLE",
            "brand",
            named(vec![("key", s(key))], ns),
        )
    };
    let binds = |id, key: &str| {
        let r = obj(vec![("key", s(key)), ("version", s("1:2"))]);
        let alias = obj(vec![
            ("dataType", K::Enum("VariableDataType", "ALIAS")),
            ("value", obj(vec![("alias", obj(vec![("assetRef", r)]))])),
        ]);
        let paint = obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            ("colorVar", alias),
        ]);
        node(
            id,
            Some(2),
            "RECTANGLE",
            "Tint",
            vec![("fillPaints", K::Array(vec![paint]))],
        )
    };
    let uses = |id, key: &str| {
        let r = obj(vec![("key", s(key)), ("version", s("v1"))]);
        let extra = vec![("styleIdForFill", obj(vec![("assetRef", r)]))];
        node(id, Some(2), "RECTANGLE", "Box", extra)
    };
    // Two library styles on one record: each relation's edges keep their own sequence.
    let both = |id| {
        let r = |k: &str| {
            obj(vec![(
                "assetRef",
                obj(vec![("key", s(k)), ("version", s("v1"))]),
            )])
        };
        let extra = vec![
            ("styleIdForFill", r("solo")),
            ("styleIdForStrokeFill", r("bare")),
        ];
        node(id, Some(2), "RECTANGLE", "Outlined", extra)
    };
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Library copies", vec![]),
            node(2, Some(0), "CANVAS", "Screens", vec![]),
            main(10, "c", Some("a/Component:b")),
            main(11, "b/Component:c", Some("a")),
            main(12, "k", None),
            instance(20, 2, "One", 10, vec![]),
            instance(21, 2, "Two", 11, vec![]),
            instance(22, 2, "Three", 12, vec![]),
            style(30, "shared", Some("x")),
            style(31, "shared", Some("y")),
            style(32, "solo", Some("x")),
            style(33, "bare", None),
            uses(40, "shared"),
            uses(41, "solo"),
            uses(42, "bare"),
            uses(43, "absent"),
            // A variable sharing a style's key keeps its own namespace.
            variable(34, "solo", Some("v")),
            binds(44, "solo"),
            binds(45, "absent"),
            both(46),
        ]),
    )])
}

fn library_edges(index: &Index) -> Vec<(String, Json, &'static str, &'static str, Json)> {
    let mut out = Vec::new();
    for r in &index.records {
        assert!(r.edges.iter().all(|e| e.relation != LIBRARY));
        let keys: BTreeSet<_> = r
            .edges
            .iter()
            .chain(&r.library)
            .map(|e| (e.relation, e.seq))
            .collect();
        assert_eq!(keys.len(), r.edges.len() + r.library.len(), "{}", r.id);
        for e in &r.library {
            let target = serde_json::from_str(e.target.as_deref().unwrap()).unwrap();
            let evidence = serde_json::from_str(&e.evidence).unwrap();
            out.push((r.id.clone(), target, e.state, e.basis, evidence));
        }
    }
    out
}

#[test]
fn library_references_keep_namespace_and_key_apart_and_never_borrow_a_namespace() {
    let message = libraries_design();
    let tree = Tree::new(&message);
    let index = crate::search::build_with(
        &tree,
        "0:0",
        bounds(WORK),
        true,
        crate::search::Library::Stored,
        None,
    );
    let field = json!({"field": "styleIdForFill", "version": "v1"});
    let tint = json!({"field": "fills", "version": "1:2"});
    let unresolved = |key: &str| {
        (
            json!(["style", null, key]),
            "unresolved",
            "cached_copy",
            field.clone(),
        )
    };
    let mut edges = library_edges(&index);
    edges.sort_by(|a, b| a.0.cmp(&b.0));
    let got: Vec<_> = edges
        .into_iter()
        .map(|(id, t, s, b, e)| (id, (t, s, b, e)))
        .collect();
    assert_eq!(
        got,
        [
            (
                "0:20".into(),
                (
                    json!(["component", "a/Component:b", "c"]),
                    "external",
                    "cached_copy",
                    json!({"copy": "0:10"})
                )
            ),
            (
                "0:21".into(),
                (
                    json!(["component", "a", "b/Component:c"]),
                    "external",
                    "cached_copy",
                    json!({"copy": "0:11"})
                )
            ),
            (
                "0:22".into(),
                (
                    json!(["component", null, "k"]),
                    "unresolved",
                    "cached_copy",
                    json!({"copy": "0:12"})
                )
            ),
            ("0:40".into(), unresolved("shared")),
            (
                "0:41".into(),
                (
                    json!(["style", "x", "solo"]),
                    "external",
                    "cached_copy",
                    field.clone()
                )
            ),
            ("0:42".into(), unresolved("bare")),
            ("0:43".into(), unresolved("absent")),
            (
                "0:44".into(),
                (
                    json!(["variable", "v", "solo"]),
                    "external",
                    "cached_copy",
                    tint.clone()
                )
            ),
            (
                "0:45".into(),
                (
                    json!(["variable", null, "absent"]),
                    "unresolved",
                    "cached_copy",
                    tint
                )
            ),
            (
                "0:46".into(),
                (
                    json!(["style", "x", "solo"]),
                    "external",
                    "cached_copy",
                    field.clone()
                )
            ),
            (
                "0:46".into(),
                (
                    json!(["style", null, "bare"]),
                    "unresolved",
                    "cached_copy",
                    json!({"field": "styleIdForStrokeFill", "version": "v1"})
                )
            ),
        ]
    );
}

#[test]
fn library_references_stay_off_record_edges_and_no_query_lists_them_as_a_relation() {
    let message = libraries_design();
    let tree = Tree::new(&message);
    let stateless = build(&tree, "0:0", bounds(WORK));
    assert_eq!(library_edges(&stateless), []);
    let index = crate::search::build_with(
        &tree,
        "0:0",
        bounds(WORK),
        true,
        crate::search::Library::Stored,
        None,
    );
    // Every other reader of a record's edges sees exactly what a build without them sees.
    let same = |a: &Index, b: &Index| {
        a.records
            .iter()
            .map(|r| &r.edges)
            .eq(b.records.iter().map(|r| &r.edges))
    };
    assert!(same(&index, &stateless));
    let conn = stored(&index);
    let mut r = req("0:41");
    r.outgoing = true;
    let q = r.validate().unwrap();
    let graph = &mut Stored {
        conn: &conn,
        gen_id: 1,
        index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
        spent: 0,
    };
    let served = Served {
        source: &source(),
        gen_id: 1,
    };
    // The stored edge loads back, and the default outgoing relations still leave it out.
    let out = query(graph, &q, &r, served).unwrap();
    let relations: Vec<_> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["relation"].clone())
        .collect();
    assert_eq!(relations, [json!("contains"), json!("uses_style")]);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM edges WHERE relation = 'uses_library'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        11
    );
    let fresh = build(&tree, "0:0", bounds(WORK));
    assert_eq!(out["coverage"], run(&fresh, &r).unwrap()["coverage"]);
}

#[test]
fn a_cached_copy_use_names_its_library_reference_in_memory_and_stored_alike() {
    let message = libraries_design();
    let tree = Tree::new(&message);
    let index = crate::search::build_with(
        &tree,
        "0:0",
        bounds(WORK),
        true,
        crate::search::Library::Stored,
        None,
    );
    let conn = stored(&index);
    let named = |ns: Json, key: &str, kind: &str| {
        let mut l = json!({"kind": kind, "namespace": ns, "key": key, "basis": "cached_copy"});
        if ns.is_null() {
            l["unknownReason"] = json!("namespace_unconfirmed");
        }
        l
    };
    // A main's own namespace; none recorded; two copies that disagree; a style; a variable.
    let cases = [
        (
            "0:20",
            "instance_of",
            named(json!("a/Component:b"), "c", "component"),
        ),
        ("0:22", "instance_of", named(Json::Null, "k", "component")),
        ("0:40", "uses_style", named(Json::Null, "shared", "style")),
        ("0:41", "uses_style", named(json!("x"), "solo", "style")),
        (
            "0:44",
            "binds_variable",
            named(json!("v"), "solo", "variable"),
        ),
    ];
    for (node, relation, want) in cases {
        let mut r = req(node);
        r.outgoing = true;
        let q = r.validate().unwrap();
        let picked = |out: Json| -> Vec<Json> {
            out["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| (e["relation"].clone(), e.get("library").cloned()))
                .filter(|(rel, _)| rel != "contains")
                .map(|(rel, l)| json!([rel, l]))
                .collect()
        };
        let memory = picked(query(&mut Whole(&index), &q, &r, served_at(0)).unwrap());
        let graph = &mut Stored {
            conn: &conn,
            gen_id: 1,
            index: crate::search::store::load(&conn, 1, &[], &mut 0).unwrap(),
            spent: 0,
        };
        let disk = picked(query(graph, &q, &r, served_at(1)).unwrap());
        assert_eq!(memory, [json!([relation, want])], "{node}");
        assert_eq!(disk, memory, "{node}");
    }
}

#[test]
fn a_stateless_library_scan_that_runs_out_of_bytes_keeps_what_it_spent() {
    let message = libraries_design();
    let tree = Tree::new(&message);
    let with = |bytes| {
        let mut b = bounds(WORK);
        b.bytes = bytes;
        let found = crate::search::named_libraries(&tree, &mut b);
        (found, b)
    };
    // The most bytes that still stop the scan, so it stops after reading at least one copy.
    let fits = (0..1 << 16).find(|&b| with(b).0.is_some()).unwrap();
    let (found, left) = with(fits - 1);
    assert_eq!(
        (
            found,
            left.stop,
            left.work < WORK - tree.nodes.len(),
            left.bytes < fits - 1
        ),
        (None, None, true, true)
    );
}

#[test]
fn a_stateless_library_scan_that_does_not_fit_names_nothing_instead_of_stopping_the_build() {
    use crate::search::{Library, build_with};
    let message = libraries_design();
    let tree = Tree::new(&message);
    // The scan reads the whole file, the build only the screens page: some budget stops a stored
    // build at its scan, before any record, yet the stateless one keeps records without it.
    let kept = (1..20 * tree.nodes.len()).any(|work| {
        let stored = build_with(&tree, "0:2", bounds(work), true, Library::Stored, None);
        let named = build_with(&tree, "0:2", bounds(work), true, Library::Named, None);
        stored.records.is_empty() && !named.records.is_empty() && named.library_unread
    });
    assert!(kept);
    // Such a build says why a cached-copy use names no library.
    let mut index = build_with(&tree, "0:0", bounds(WORK), true, Library::Named, None);
    index.library_unread = true;
    index.records.iter_mut().for_each(|r| r.library.clear());
    let mut r = req("0:20");
    r.outgoing = true;
    let out = run(&index, &r).unwrap();
    let named = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e.get("library").is_some());
    let codes: Vec<_> = out["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].clone())
        .collect();
    assert_eq!(
        (named, codes.contains(&json!("library_reference_unread"))),
        (false, true)
    );
}

fn served_at(gen_id: i64) -> Served<'static> {
    static SOURCE: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
    Served {
        source: SOURCE.get_or_init(source),
        gen_id,
    }
}

#[test]
fn a_library_scan_that_runs_out_stops_the_build() {
    let message = libraries_design();
    let tree = Tree::new(&message);
    let mut tight = bounds(tree.nodes.len() + 1);
    assert_eq!(libraries(&tree, &mut tight), None);
    assert_eq!(tight.stop, Some(crate::expand::Stop::Work));
}

/// A Button component at `base` (label text `base + 1`, a filled box `base + 2`) placed at `x`
/// under `parent`; `extra` goes on the root, `fill` on the box.
fn button(
    base: u32,
    parent: u32,
    x: f32,
    label: &str,
    fill: K<'static>,
    extra: Vec<(&'static str, K<'static>)>,
) -> Vec<K<'static>> {
    let f = K::Float;
    let at = obj(vec![
        ("m00", f(1.0)),
        ("m01", f(0.0)),
        ("m02", f(x)),
        ("m10", f(0.0)),
        ("m11", f(1.0)),
        ("m12", f(0.0)),
    ]);
    let mut root = vec![
        ("transform", at),
        ("size", obj(vec![("x", f(80.0)), ("y", f(32.0))])),
        ("horizontalConstraint", K::Enum("ConstraintType", "MIN")),
    ];
    root.extend(extra);
    vec![
        node(base, Some(parent), "SYMBOL", "Button", root),
        node(
            base + 1,
            Some(base),
            "TEXT",
            "Label",
            vec![super::super::tests::text(label)],
        ),
        node(
            base + 2,
            Some(base),
            "RECTANGLE",
            "Box",
            vec![("fillPaints", K::Array(vec![fill]))],
        ),
    ]
}

fn red(r: f32) -> K<'static> {
    let f = K::Float;
    obj(vec![
        ("type", K::Enum("PaintType", "SOLID")),
        (
            "color",
            obj(vec![
                ("r", f(r)),
                ("g", f(0.0)),
                ("b", f(0.0)),
                ("a", f(1.0)),
            ]),
        ),
    ])
}

/// The library's Button at `0:10` and a consumer's cached copy of it at `0:50` (other ids,
/// another place and constraint), used by an instance; each built as a stored generation and
/// compared from its stored rows.
fn copy_matches(copy_label: &str, copy_fill: K<'static>) -> &'static str {
    use crate::search::{Library, build_with, fingerprint, store};
    let design = |nodes: Vec<K<'static>>| {
        let mut all = vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Components", vec![]),
        ];
        all.extend(nodes);
        obj(vec![("nodeChanges", K::Array(all))])
    };
    let library = design(button(10, 1, 0.0, "Go", red(1.0), vec![]));
    let s = |v: &str| K::String(v.into());
    let cached = vec![("componentKey", s("k")), ("sourceLibraryKey", s("ns"))];
    let mut copy = button(50, 1, 300.0, copy_label, copy_fill, cached);
    if let K::Object(_, o) = &mut copy[0] {
        o.insert("horizontalConstraint", K::Enum("ConstraintType", "MAX"));
    }
    copy.push(instance(60, 1, "Use", 50, vec![]));
    let consumer = design(copy);
    let print = |message: &K<'static>, node: &str| {
        let tree = Tree::new(message);
        let index = build_with(&tree, "0:0", bounds(WORK), true, Library::Stored, None);
        store::print(&stored(&index), 1, node).unwrap().unwrap()
    };
    // The consumer's use names the copy it compares, never the instance itself.
    let tree = Tree::new(&consumer);
    let index = build_with(&tree, "0:0", bounds(WORK), true, Library::Stored, None);
    assert!(index.records.iter().any(|r| r.id == "0:60"
        && r.library.iter().any(|e| e.evidence == r#"{"copy":"0:50"}"#)));
    fingerprint::compare(&print(&library, "0:10"), &print(&consumer, "0:50"))
}

#[test]
fn a_cached_copy_under_other_ids_and_another_place_equals_its_definition() {
    assert_eq!(copy_matches("Go", red(1.0)), "equal");
}

#[test]
fn a_child_text_or_fill_change_under_unchanged_root_fields_is_a_mismatch() {
    assert_eq!(
        [copy_matches("Stop", red(1.0)), copy_matches("Go", red(0.5))],
        ["definition_mismatch"; 2]
    );
}

#[test]
fn a_fill_bound_to_a_file_local_variable_is_unknown() {
    let alias = obj(vec![
        ("dataType", K::Enum("VariableDataType", "ALIAS")),
        ("value", obj(vec![("alias", obj(vec![("guid", guid(7))]))])),
    ]);
    let mut bound = red(1.0);
    if let K::Object(_, o) = &mut bound {
        o.insert("colorVar", alias);
    }
    assert_eq!(copy_matches("Go", bound), "unknown");
}

#[test]
fn a_facet_over_its_read_cap_is_unknown() {
    // The same label on both sides, but a copy's too long to digest is never called equal.
    let long = "Go ".repeat(30_000);
    assert_eq!(copy_matches(&long, red(1.0)), "unknown");
}
