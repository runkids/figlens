use super::super::tests::{guid, instance, node, obj, path, text};
use super::*;
use crate::search::build_as;
use kiwi_schema::Value as K;

fn fill(r: f64) -> (&'static str, K<'static>) {
    (
        "fillPaints",
        K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            (
                "color",
                obj(vec![
                    ("r", K::Float(r as f32)),
                    ("g", K::Float(0.0)),
                    ("b", K::Float(0.0)),
                    ("a", K::Float(1.0)),
                ]),
            ),
        ])]),
    )
}

fn size(x: f32, y: f32) -> (&'static str, K<'static>) {
    ("size", obj(vec![("x", K::Float(x)), ("y", K::Float(y))]))
}

fn doc(nodes: Vec<K<'static>>) -> K<'static> {
    let mut all = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
    ];
    all.extend(nodes);
    obj(vec![("nodeChanges", K::Array(all))])
}

fn screen(title: &str, red: f64, width: f32) -> Vec<K<'static>> {
    vec![
        node(10, Some(1), "FRAME", "Home", vec![size(width, 800.0)]),
        node(11, Some(10), "TEXT", "Title", vec![text(title)]),
        node(12, Some(10), "RECTANGLE", "Card", vec![fill(red)]),
        node(13, Some(10), "RECTANGLE", "Same", vec![fill(0.5)]),
    ]
}

fn req(limit: usize, cursor: Option<String>) -> Request {
    Request {
        node: "0:0".into(),
        limit,
        cursor,
    }
}

fn run(old: K<'static>, new: K<'static>) -> Json {
    let (a, b) = (Tree::new(&old), Tree::new(&new));
    page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0"),
        &req(LIMIT, None),
        false,
    )
    .unwrap()
}

fn nodes(reply: &Json) -> Vec<(String, String)> {
    reply["frames"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["nodes"].as_array().unwrap())
        .map(|n| {
            (
                n["id"].as_str().unwrap().to_owned(),
                n["change"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn changed_text_fill_and_size_are_reported_per_facet_and_unchanged_layers_are_not() {
    let out = run(
        doc(screen("Hello", 1.0, 400.0)),
        doc(screen("Hi", 0.0, 420.0)),
    );
    assert_eq!(
        nodes(&out),
        [
            ("0:10".into(), "changed".into()),
            ("0:11".into(), "changed".into()),
            ("0:12".into(), "changed".into())
        ]
    );
    let frame = &out["frames"][0];
    assert_eq!(
        frame["summary"],
        "3 changed, text 1, layout 1, appearance 1"
    );
    let title = &frame["nodes"][1];
    assert_eq!(title["facets"], json!(["text"]));
    assert_eq!(
        title["fields"]["textData"]["old"]["value"]["characters"],
        "Hello"
    );
    assert_eq!(
        title["fields"]["textData"]["new"]["value"]["characters"],
        "Hi"
    );
    assert_eq!(
        frame["nodes"][0]["fields"]["size"]["new"]["value"]["x"],
        420.0
    );
    assert_eq!(out["coverage"]["complete"], true);
    assert_eq!(
        run(
            doc(screen("Same", 1.0, 400.0)),
            doc(screen("Same", 1.0, 400.0))
        )["frames"],
        json!([])
    );
}

#[test]
fn a_removed_subtree_is_one_change_and_a_same_name_frame_is_only_a_possible_match() {
    let mut new = screen("Hello", 1.0, 400.0);
    // Same name and children, new ids: never the same frame, only a labelled candidate.
    new[0] = node(20, Some(1), "FRAME", "Home", vec![size(400.0, 800.0)]);
    for (i, n) in new.iter_mut().enumerate().skip(1) {
        *n = node(20 + i as u32, Some(20), "RECTANGLE", "Child", vec![]);
    }
    let out = run(doc(screen("Hello", 1.0, 400.0)), doc(new));
    assert_eq!(
        nodes(&out),
        [
            ("0:10".into(), "removed".into()),
            ("0:20".into(), "added".into())
        ]
    );
    let (old, added) = (&out["frames"][0], &out["frames"][1]);
    assert_eq!(old["nodes"][0]["descendants"], 3);
    assert_eq!(
        old["possibleMatch"],
        json!({"id": "0:20", "basis": "name", "identity": "possible_match"})
    );
    assert_eq!(added["possibleMatch"]["id"], "0:10");
    assert_eq!(out["coverage"]["possibleMatches"], 1);
}

#[test]
fn a_same_id_with_another_kind_is_identity_uncertain() {
    let mut new = screen("Hello", 1.0, 400.0);
    new[2] = node(12, Some(10), "ELLIPSE", "Card", vec![fill(1.0)]);
    let out = run(doc(screen("Hello", 1.0, 400.0)), doc(new));
    let n = &out["frames"][0]["nodes"][0];
    assert_eq!(
        (&n["change"], &n["identity"], &n["oldKind"]),
        (
            &json!("kind_changed"),
            &json!("identity_uncertain"),
            &json!("RECTANGLE")
        )
    );
}

fn with_instance(main: u32, overrides: Vec<K<'static>>) -> K<'static> {
    doc(vec![
        node(30, Some(1), "SYMBOL", "A", vec![]),
        node(31, Some(30), "RECTANGLE", "Bg", vec![fill(1.0)]),
        node(40, Some(1), "SYMBOL", "B", vec![]),
        node(41, Some(40), "RECTANGLE", "Bg", vec![fill(1.0)]),
        node(10, Some(1), "FRAME", "Home", vec![]),
        instance(
            11,
            10,
            "Use",
            main,
            vec![("symbolOverrides", K::Array(overrides))],
        ),
    ])
}

#[test]
fn an_override_equal_to_the_main_value_is_no_change_and_a_swap_names_both_mains() {
    // The override is JSON, the main's field kiwi: both digest the same effective value.
    let same = obj(vec![path(&[31]), fill(1.0)]);
    assert_eq!(
        run(with_instance(30, vec![]), with_instance(30, vec![same]))["frames"],
        json!([])
    );
    let red = obj(vec![path(&[31]), fill(0.0)]);
    let out = run(with_instance(30, vec![]), with_instance(30, vec![red]));
    assert_eq!(nodes(&out), [("I0:11;0:31".into(), "changed".into())]);
    let swapped = run(with_instance(30, vec![]), with_instance(40, vec![]));
    let n = &swapped["frames"][0]["nodes"][0];
    assert_eq!(n["id"], "0:11");
    assert_eq!(n["edges"]["swap"], json!({"old": "0:30", "new": "0:40"}));
    assert!(
        n["edges"]["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["target"] == "0:30")
    );
}

#[test]
fn layers_past_a_side_that_stopped_are_unknown_not_removed() {
    let (a, b) = (
        doc(screen("Hello", 1.0, 400.0)),
        doc(screen("Hello", 1.0, 400.0)),
    );
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let work = (1..400)
        .find(|w| {
            build_as(&b, "0:0", super::super::tests::bounds(*w), true)
                .stop
                .is_some_and(|_| {
                    build_as(&b, "0:0", super::super::tests::bounds(*w + 1), true)
                        .stop
                        .is_none()
                })
        })
        .unwrap();
    let pair = compare_trees(&a, &b, [WORK, work, PAIR_WORK], "0:0");
    let out = page(&pair, &req(LIMIT, None), false).unwrap();
    assert!(nodes(&out).iter().all(|(_, c)| c == "unknown"), "{out}");
    assert_eq!(out["coverage"]["complete"], false);
}

#[test]
fn pages_walk_every_change_once_and_a_cursor_of_another_pair_is_stale() {
    let many = |r: f64| {
        doc((0..7)
            .map(|i| node(10 + i, Some(1), "RECTANGLE", "R", vec![fill(r)]))
            .collect())
    };
    let (a, b) = (many(1.0), many(0.0));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let pair = compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0");
    let (mut seen, mut cursor) = (Vec::new(), None);
    loop {
        let out = page(&pair, &req(3, cursor), false).unwrap();
        seen.extend(nodes(&out));
        match out["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 7);
    let first = page(&pair, &req(3, None), false).unwrap();
    // Another selection of the same revisions binds another cursor.
    let other = compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:1");
    let err = page(
        &other,
        &req(3, first["nextCursor"].as_str().map(str::to_owned)),
        false,
    )
    .unwrap_err();
    assert_eq!(
        err.downcast_ref::<FigError>().map(FigError::kind),
        Some(ErrorKind::CursorStale)
    );
}

#[test]
fn a_field_too_large_to_read_is_unread_never_unchanged() {
    let big = |r: f64| {
        let paint = obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            ("opacity", K::Float(r as f32)),
        ]);
        ("fillPaints", K::Array(vec![paint; VISITS]))
    };
    let a = doc(vec![node(
        10,
        Some(1),
        "RECTANGLE",
        "R",
        vec![big(1.0), size(1.0, 1.0)],
    )]);
    let b = doc(vec![node(
        10,
        Some(1),
        "RECTANGLE",
        "R",
        vec![big(0.5), size(2.0, 1.0)],
    )]);
    let out = run(a, b);
    let n = &out["frames"][0]["nodes"][0];
    assert_eq!(
        (&n["unread"], &n["facets"]),
        (&json!(true), &json!(["layout"]))
    );
    assert_eq!(
        out["coverage"]["reasons"],
        json!([{"reason": "unread_fields", "count": 1}])
    );
    let _ = guid(0);
}

#[test]
fn a_giant_field_is_unread_and_an_escaped_one_is_digested_without_keeping_its_value() {
    let one = |s: &str| doc(vec![node(10, Some(1), "TEXT", "T", vec![text(s)])]);
    // Past the conversion cap: never converted, listed as unread, not taken for unchanged.
    let giant = "x".repeat(FIELD_BYTES + 1);
    let out = run(one(&giant), one(&giant));
    let n = &out["frames"][0]["nodes"][0];
    assert_eq!((&n["unread"], &n["facets"]), (&json!(true), &json!([])));
    // Control characters escape to six bytes each; the stream is hashed, the value dropped.
    let escaped = "\u{1}".repeat(FIELD_BYTES / 8);
    let out = run(one(&escaped), one(&format!("{escaped}\u{2}")));
    let n = &out["frames"][0]["nodes"][0];
    assert_eq!((&n["facets"], n.get("unread")), (&json!(["text"]), None));
}

#[test]
fn content_a_now_hidden_instance_leaves_unexpanded_is_unknown_not_removed() {
    let hidden = doc(vec![
        node(30, Some(1), "SYMBOL", "A", vec![]),
        node(31, Some(30), "RECTANGLE", "Bg", vec![fill(1.0)]),
        node(40, Some(1), "SYMBOL", "B", vec![]),
        node(41, Some(40), "RECTANGLE", "Bg", vec![fill(1.0)]),
        node(10, Some(1), "FRAME", "Home", vec![]),
        instance(11, 10, "Use", 30, vec![("visible", K::Bool(false))]),
    ]);
    let changes = nodes(&run(with_instance(30, vec![]), hidden));
    assert!(
        changes.contains(&("I0:11;0:31".into(), "unknown".into())),
        "{changes:?}"
    );
    assert!(!changes.iter().any(|(_, c)| c == "removed"), "{changes:?}");
}

#[test]
fn a_selection_on_one_side_only_lists_its_children_for_drilling_in() {
    let (a, b) = (doc(screen("Hi", 1.0, 100.0)), doc(vec![]));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let out = page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:10"),
        &Request {
            node: "0:10".into(),
            limit: LIMIT,
            cursor: None,
        },
        false,
    )
    .unwrap();
    assert_eq!(
        nodes(&out),
        ["0:10", "0:11", "0:12", "0:13"].map(|id| (id.to_owned(), "removed".to_owned()))
    );
    assert_eq!(out["coverage"]["new"]["selected"], false);
}

#[test]
fn binary_values_of_equal_length_differ_and_an_override_holding_bytes_is_unread() {
    let blob = |b: u8| {
        (
            "fillPaints",
            K::Array(vec![obj(vec![
                ("type", K::Enum("PaintType", "SOLID")),
                ("blob", K::Bytes(vec![1, b])),
            ])]),
        )
    };
    let one = |b| doc(vec![node(10, Some(1), "RECTANGLE", "R", vec![blob(b)])]);
    let n = &run(one(2), one(3))["frames"][0]["nodes"][0];
    assert_eq!(n["facets"], json!(["appearance"]));
    // The override reaches the facets as JSON with `<2 bytes>`: unknown, not unchanged.
    let same = obj(vec![path(&[31]), blob(2)]);
    let changes = nodes(&run(
        with_instance(30, vec![same.clone()]),
        with_instance(30, vec![same]),
    ));
    assert_eq!(changes, [("I0:11;0:31".to_owned(), "changed".to_owned())]);
}

#[test]
fn an_unread_field_is_charged_its_metadata_like_a_read_one() {
    let held = (
        "fillPaints",
        K::Array(vec![obj(vec![
            ("type", K::Enum("PaintType", "SOLID")),
            ("blob", K::Bytes(vec![1, 2])),
        ])]),
    );
    let giant = text(&"x".repeat(FIELD_BYTES + 1));
    // Too large to read, a malformed blob, and an override holding bytes.
    for (message, id) in [
        (
            doc(vec![node(10, Some(1), "TEXT", "T", vec![giant])]),
            "0:10",
        ),
        (vector_with(Some(K::UInt(1))), "0:10"),
        (
            with_instance(30, vec![obj(vec![path(&[31]), held])]),
            "I0:11;0:31",
        ),
    ] {
        let tree = Tree::new(&message);
        let bounds = super::super::tests::bounds;
        let (ex, _) = crate::expand::expand_instances_bounded(&tree, "0:0", bounds(WORK));
        let mut b = bounds(WORK);
        let before = b.bytes;
        let f = facets(&tree, id, ex.nodes.get(id), &mut b).unwrap();
        assert!(f.fields.iter().any(|f| f.digest.is_none()), "{id}");
        // Each field pays its metadata, and a kept value its serialized length; the malformed
        // blob is found only after its stored geometry was converted and copied, which it pays.
        let converted = tree
            .nodes
            .get(id)
            .and_then(|n| n.get("fillGeometry"))
            .map_or(0, |g| 2 * crate::expand::kiwi_bytes(g, &mut VISITS.clone()));
        let kept: usize = f
            .fields
            .iter()
            .map(|f| 64 + f.key.len() + f.value.as_ref().map_or(0, |v| v.to_string().len()))
            .sum();
        // An expanded size and transform are each built after paying their fixed estimate.
        let e = ex.nodes.get(id);
        let built = usize::from(e.is_some_and(|e| e.size.is_some())) + usize::from(e.is_some());
        assert_eq!(
            before - b.bytes,
            kept + converted + built * OWNED_BYTES,
            "{id}"
        );
    }
}

#[test]
fn a_stored_field_pays_for_its_conversion_before_it_is_converted() {
    // Past what one value keeps, so only the conversion needs the bytes.
    let long = text(&"x".repeat(FIELD_BYTES / 4));
    let message = doc(vec![node(10, Some(1), "TEXT", "T", vec![long])]);
    let tree = Tree::new(&message);
    let mut b = super::super::tests::bounds(WORK);
    b.bytes = 1024;
    assert!(facets(&tree, "0:10", None, &mut b).is_none());
    assert_eq!(b.stop, Some(crate::expand::Stop::Bytes));
}

#[test]
fn an_expanded_size_or_transform_pays_before_it_is_built() {
    let message = doc(vec![
        node(30, Some(1), "SYMBOL", "A", vec![]),
        node(31, Some(30), "RECTANGLE", "Bg", vec![]),
        node(10, Some(1), "FRAME", "Home", vec![]),
        instance(11, 10, "Use", 30, vec![]),
    ]);
    let tree = Tree::new(&message);
    let bounds = super::super::tests::bounds;
    let (ex, _) = crate::expand::expand_instances_bounded(&tree, "0:0", bounds(WORK));
    let id = "I0:11;0:31";
    assert!(facets(&tree, id, ex.nodes.get(id), &mut bounds(WORK)).is_some());
    // Enough for what the record keeps, not for building its expanded transform first.
    let mut b = bounds(WORK);
    b.bytes = OWNED_BYTES - 1;
    assert!(facets(&tree, id, ex.nodes.get(id), &mut b).is_none());
    assert_eq!(b.stop, Some(crate::expand::Stop::Bytes));
}

#[test]
fn each_page_background_setting_is_an_appearance_change() {
    let red = |r: f32| {
        let channel = |k, v| (k, K::Float(v));
        obj(vec![
            channel("r", r),
            channel("g", 0.0),
            channel("b", 0.0),
            channel("a", 1.0),
        ])
    };
    let page = |fields: Vec<(&'static str, K<'static>)>| {
        obj(vec![(
            "nodeChanges",
            K::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", fields),
            ]),
        )])
    };
    let base = || {
        vec![
            ("backgroundColor", red(1.0)),
            ("backgroundEnabled", K::Bool(true)),
            ("backgroundOpacity", K::Float(1.0)),
        ]
    };
    let with = |key: &'static str, value: K<'static>| {
        let mut fields = base();
        fields.iter_mut().find(|(k, _)| *k == key).unwrap().1 = value;
        fields
    };
    for (setting, changed) in [
        ("backgroundColor", with("backgroundColor", red(0.0))),
        (
            "backgroundEnabled",
            with("backgroundEnabled", K::Bool(false)),
        ),
        (
            "backgroundOpacity",
            with("backgroundOpacity", K::Float(0.5)),
        ),
    ] {
        let out = run(page(base()), page(changed));
        let changes: Vec<_> = out["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|f| f["nodes"].as_array().unwrap())
            .map(|n| (n["id"].clone(), n["change"].clone(), n["facets"].clone()))
            .collect();
        assert_eq!(
            changes,
            [(json!("0:1"), json!("changed"), json!(["appearance"]))],
            "{setting}: {out}"
        );
        assert_eq!(out["coverage"]["complete"], true, "{setting}");
    }
}

#[test]
fn a_page_full_of_long_names_ends_early_and_its_cursor_reaches_every_change() {
    let name = "n".repeat(2000);
    let side = |red: f64| {
        let mut nodes = vec![node(10, Some(1), "FRAME", "Home", vec![])];
        nodes
            .extend((0..200).map(|i| node(100 + i, Some(10), "RECTANGLE", &name, vec![fill(red)])));
        doc(nodes)
    };
    let (a, b) = (side(1.0), side(0.5));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let pair = compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0");
    let (mut seen, mut cursor, mut pages) = (0, None, 0);
    loop {
        let out = page(&pair, &req(MAX_LIMIT, cursor), true).unwrap();
        let listed = nodes(&out).len();
        assert!(listed > 0 && listed < 200, "{listed}");
        seen += listed;
        pages += 1;
        match out["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!((seen, pages > 1), (200, true));
}

#[test]
fn possible_match_probes_are_charged_and_stop_the_pair_when_out_of_work() {
    let frames = |first: u32| {
        (0..300)
            .map(move |i| node(first + i, Some(1), "FRAME", "A", vec![]))
            .collect()
    };
    let (a, b) = (doc(frames(100)), doc(frames(1000)));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let run = |work| {
        page(
            &compare_trees(&a, &b, [WORK, WORK, work], "0:0"),
            &req(1, None),
            false,
        )
        .unwrap()
    };
    let full = run(PAIR_WORK);
    assert_eq!(full["coverage"]["possibleMatches"], POSSIBLE);
    // Enough for the maps and the 600 changes, not for every probe.
    let short = run(2 * 600 + 600 * 4 + 100);
    // Every change was kept; only the probes ran out.
    assert_eq!(
        (
            &short["coverage"]["changes"],
            &short["coverage"]["complete"]
        ),
        (&json!(600), &json!(false))
    );
    assert!(
        short["coverage"]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!({"reason": "diff_work_limit"}))
    );
}

#[test]
fn a_first_change_too_large_for_a_page_is_an_error_not_an_oversized_page() {
    let long = "n".repeat(PAGE_VALUES + 1);
    let one = |red| doc(vec![node(10, Some(1), "RECTANGLE", &long, vec![fill(red)])]);
    let (a, b) = (one(1.0), one(0.5));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let pair = compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0");
    let e = page(&pair, &req(LIMIT, None), false).unwrap_err();
    assert_eq!(
        e.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::ResponseBudget
    );
}

#[test]
fn edges_past_the_page_work_end_the_page_or_fail_the_first_change() {
    let side = |main: u32| {
        doc(vec![
            node(30, Some(1), "SYMBOL", "A", vec![]),
            node(40, Some(1), "SYMBOL", "B", vec![]),
            node(10, Some(1), "FRAME", "Home", vec![]),
            instance(11, 10, "One", main, vec![]),
            instance(12, 10, "Two", main, vec![]),
        ])
    };
    let (a, b) = (side(30), side(40));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let pair = compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0");
    let at = |work, cursor| page_with(&pair, &req(LIMIT, cursor), false, [PAGE_VALUES, work]);
    let e = at(0, None).unwrap_err();
    assert_eq!(
        e.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::QueryWorkLimit
    );
    // The least work that lists one changed instance lists only it; the cursor reaches the other.
    let work = (1..100).find(|w| at(*w, None).is_ok()).unwrap();
    let first = at(work, None).unwrap();
    let next = first["nextCursor"].as_str().map(str::to_owned);
    let instances = |page: &Json| {
        nodes(page)
            .into_iter()
            .filter(|(id, _)| id == "0:11" || id == "0:12")
            .count()
    };
    assert_eq!(instances(&first), 1);
    let second = at(work, next).unwrap();
    let all: Vec<_> = nodes(&first)
        .into_iter()
        .chain(nodes(&second))
        .map(|(id, _)| id)
        .collect();
    assert!(
        all.contains(&"0:11".to_owned()) && all.contains(&"0:12".to_owned()),
        "{all:?}"
    );
    assert!(
        first["frames"][0]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["edges"].is_object())
    );
}

#[test]
fn a_vector_path_is_compared_by_its_blob_bytes_not_its_blob_index() {
    let file = |at: usize, bytes: Vec<u8>| {
        let mut blobs = vec![obj(vec![("bytes", K::Bytes(vec![9]))]); 2];
        blobs[at] = obj(vec![("bytes", K::Bytes(bytes))]);
        let geometry = obj(vec![("commandsBlob", K::UInt(at as u32))]);
        let K::Object(name, mut fields) = doc(vec![node(
            10,
            Some(1),
            "VECTOR",
            "Icon",
            vec![("fillGeometry", K::Array(vec![geometry]))],
        )]) else {
            unreachable!()
        };
        fields.insert("blobs", K::Array(blobs));
        K::Object(name, fields)
    };
    let n = &run(file(0, vec![1, 2]), file(0, vec![1, 3]))["frames"][0]["nodes"][0];
    assert_eq!(n["facets"], json!(["appearance"]));
    assert_eq!(
        run(file(0, vec![1, 2]), file(1, vec![1, 2]))["frames"],
        json!([])
    );
}

#[test]
fn an_image_override_equal_to_the_main_fill_is_no_change() {
    let image = || {
        (
            "fillPaints",
            K::Array(vec![obj(vec![
                ("type", K::Enum("PaintType", "IMAGE")),
                ("image", obj(vec![("hash", K::Bytes(vec![0, 255]))])),
            ])]),
        )
    };
    let file = |overrides: Vec<K<'static>>| {
        doc(vec![
            node(30, Some(1), "SYMBOL", "A", vec![]),
            node(31, Some(30), "RECTANGLE", "Bg", vec![image()]),
            node(10, Some(1), "FRAME", "Home", vec![]),
            instance(
                11,
                10,
                "Use",
                30,
                vec![("symbolOverrides", K::Array(overrides))],
            ),
        ])
    };
    let same = obj(vec![path(&[31]), image()]);
    assert_eq!(run(file(vec![]), file(vec![same]))["frames"], json!([]));
}

#[test]
fn content_left_unexpanded_below_hidden_layers_makes_coverage_partial() {
    let hidden = |red| {
        doc(vec![
            node(30, Some(1), "SYMBOL", "A", vec![]),
            node(31, Some(30), "RECTANGLE", "Bg", vec![fill(red)]),
            node(10, Some(1), "FRAME", "Home", vec![]),
            instance(11, 10, "Use", 30, vec![("visible", K::Bool(false))]),
        ])
    };
    let coverage = &run(hidden(1.0), hidden(0.0))["coverage"];
    assert_eq!(
        (&coverage["complete"], &coverage["new"]["complete"]),
        (&json!(false), &json!(false))
    );
    assert!(
        coverage["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!({"reason": "hidden_content_not_expanded", "side": "new", "count": 1})),
        "{coverage}"
    );
}

fn vector_with(blob: Option<K<'static>>) -> K<'static> {
    let geometry = obj(vec![("commandsBlob", K::UInt(0))]);
    let K::Object(name, mut fields) = doc(vec![node(
        10,
        Some(1),
        "VECTOR",
        "Icon",
        vec![("fillGeometry", K::Array(vec![geometry]))],
    )]) else {
        unreachable!()
    };
    let blobs = blob.map(|b| obj(vec![("bytes", b)])).into_iter().collect();
    fields.insert("blobs", K::Array(blobs));
    K::Object(name, fields)
}

#[test]
fn a_missing_or_malformed_blob_is_unread_never_unchanged() {
    for blob in [None, Some(K::Array(vec![K::UInt(1)])), Some(K::UInt(1))] {
        let out = run(vector_with(blob.clone()), vector_with(blob));
        assert_eq!(nodes(&out), [("0:10".to_owned(), "changed".to_owned())]);
        assert_eq!(out["coverage"]["complete"], false);
    }
}

#[test]
fn compact_blob_bytes_and_a_byte_array_digest_alike() {
    let array = K::Array(vec![K::Byte(1), K::Byte(2)]);
    let out = run(
        vector_with(Some(K::Bytes(vec![1, 2]))),
        vector_with(Some(array)),
    );
    assert_eq!(out["frames"], json!([]));
}

#[test]
fn a_byte_array_blob_past_the_visit_cap_is_unread() {
    let blob = || Some(K::Array(vec![K::Byte(1); VISITS + 1]));
    let out = run(vector_with(blob()), vector_with(blob()));
    assert_eq!(nodes(&out), [("0:10".to_owned(), "changed".to_owned())]);
    assert_eq!(out["coverage"]["complete"], false);
}

#[test]
fn a_changed_truncation_line_count_is_a_text_change() {
    let one = |lines| {
        doc(vec![node(
            10,
            Some(1),
            "TEXT",
            "Clamp",
            vec![
                ("textTruncation", K::Enum("TextTruncation", "ENDING")),
                ("maxLines", K::UInt(lines)),
            ],
        )])
    };
    let n = &run(one(1), one(2))["frames"][0]["nodes"][0];
    assert_eq!(n["facets"], json!(["text"]));
}

#[test]
fn a_selected_node_moved_to_another_parent_is_a_structure_change() {
    let file = |parent| {
        doc(vec![
            node(10, Some(1), "FRAME", "Home", vec![]),
            node(20, Some(10), "FRAME", "Left", vec![]),
            node(30, Some(10), "FRAME", "Right", vec![]),
            node(21, Some(parent), "RECTANGLE", "Card", vec![fill(1.0)]),
        ])
    };
    let (a, b) = (file(20), file(30));
    let (a, b) = (Tree::new(&a), Tree::new(&b));
    let out = page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:21"),
        &Request {
            node: "0:21".into(),
            limit: LIMIT,
            cursor: None,
        },
        false,
    )
    .unwrap();
    assert_eq!(out["frames"][0]["nodes"][0]["facets"], json!(["structure"]));
}

#[test]
fn a_selected_context_past_the_side_budget_is_unknown_not_position_zero() {
    let mut nodes = vec![node(10, Some(1), "FRAME", "Home", vec![])];
    nodes.extend((20..30).map(|i| node(i, Some(10), "RECTANGLE", "Card", vec![])));
    let file = doc(nodes);
    let tree = Tree::new(&file);
    let out = page(
        &compare_trees(&tree, &tree, [2, WORK, PAIR_WORK], "0:29"),
        &Request {
            node: "0:29".into(),
            limit: LIMIT,
            cursor: None,
        },
        false,
    )
    .unwrap();
    let reasons = out["coverage"]["reasons"].as_array().unwrap();
    assert!(
        reasons.contains(&json!({"reason": "selection_context_unknown", "side": "old"})),
        "{reasons:?}"
    );
}

#[test]
fn an_explicit_variable_mode_switch_is_an_appearance_change() {
    // Switching the theme leaves every cached paint below as it was; only the frame's mode map
    // changes, and it re-resolves their bound colors.
    let themed = |mode: u32| {
        let modes = obj(vec![(
            "entries",
            K::Array(vec![obj(vec![
                ("variableSetID", obj(vec![("guid", guid(70))])),
                ("variableModeID", guid(mode)),
            ])]),
        )]);
        doc(vec![
            node(
                10,
                Some(1),
                "FRAME",
                "Home",
                vec![("variableModeBySetMap", modes)],
            ),
            node(12, Some(10), "RECTANGLE", "Card", vec![fill(1.0)]),
        ])
    };
    let out = run(themed(71), themed(72));
    assert_eq!(nodes(&out), [("0:10".into(), "changed".into())]);
    assert_eq!(
        out["frames"][0]["nodes"][0]["facets"],
        json!(["appearance"])
    );
}

#[test]
fn a_resized_source_child_under_a_fixed_instance_size_is_an_appearance_change() {
    // The instance keeps its child at 20x20 and the outline bytes stay the same, but the
    // expanded child's outline is scaled from the source size it was drawn at.
    let file = |source: f32| {
        let K::Object(name, mut fields) = doc(vec![
            node(30, Some(1), "SYMBOL", "Icon", vec![]),
            node(
                31,
                Some(30),
                "VECTOR",
                "Glyph",
                vec![
                    size(source, source),
                    (
                        "fillGeometry",
                        K::Array(vec![obj(vec![("commandsBlob", K::UInt(0))])]),
                    ),
                ],
            ),
            // A layer without an outline draws the same at any source size.
            node(
                32,
                Some(30),
                "RECTANGLE",
                "Plate",
                vec![size(source, source)],
            ),
            node(10, Some(1), "FRAME", "Home", vec![]),
            instance(
                11,
                10,
                "Use",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![
                        obj(vec![path(&[31]), size(20.0, 20.0)]),
                        obj(vec![path(&[32]), size(20.0, 20.0)]),
                    ]),
                )],
            ),
        ]) else {
            unreachable!()
        };
        fields.insert(
            "blobs",
            K::Array(vec![obj(vec![("bytes", K::Bytes(vec![1, 2]))])]),
        );
        K::Object(name, fields)
    };
    let (old, new) = (file(10.0), file(40.0));
    let (a, b) = (Tree::new(&old), Tree::new(&new));
    let out = page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:11"),
        &req(LIMIT, None),
        false,
    )
    .unwrap();
    assert_eq!(nodes(&out), [("I0:11;0:31".into(), "changed".into())]);
    assert_eq!(
        out["frames"][0]["nodes"][0]["facets"],
        json!(["appearance"])
    );
}

fn alias(id: u32) -> K<'static> {
    obj(vec![
        ("dataType", K::Enum("VariableDataType", "ALIAS")),
        ("value", obj(vec![("alias", obj(vec![("guid", guid(id))]))])),
    ])
}

/// A FLOAT variable 60 holding `value` in one mode, on the page.
fn variable(value: f32) -> K<'static> {
    let data = obj(vec![
        ("dataType", K::Enum("VariableDataType", "FLOAT")),
        ("value", obj(vec![("floatValue", K::Float(value))])),
    ]);
    let entry = obj(vec![("modeID", guid(70)), ("variableData", data)]);
    node(
        60,
        Some(1),
        "VARIABLE",
        "Gap",
        vec![(
            "variableDataValues",
            obj(vec![("entries", K::Array(vec![entry]))]),
        )],
    )
}

/// Frame 10 holds a layer filled from variable 60.
fn bound_screen(value: f32) -> K<'static> {
    let paint = obj(vec![
        ("type", K::Enum("PaintType", "SOLID")),
        ("colorVar", alias(60)),
    ]);
    doc(vec![
        variable(value),
        node(10, Some(1), "FRAME", "Home", vec![]),
        node(
            12,
            Some(10),
            "RECTANGLE",
            "Card",
            vec![("fillPaints", K::Array(vec![paint]))],
        ),
    ])
}

fn scoped(old: K<'static>, new: K<'static>, node: &str) -> Json {
    let (a, b) = (Tree::new(&old), Tree::new(&new));
    let mut r = req(LIMIT, None);
    r.node = node.into();
    page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], node),
        &r,
        false,
    )
    .unwrap()
}

/// Text style 20 at `font`, used by a layer of frame 10 but outside it.
fn styled(font: f32) -> K<'static> {
    doc(vec![
        node(
            20,
            Some(1),
            "TEXT",
            "Body",
            vec![("fontSize", K::Float(font))],
        ),
        node(10, Some(1), "FRAME", "Home", vec![]),
        node(
            11,
            Some(10),
            "TEXT",
            "Title",
            vec![
                text("Hi"),
                ("styleIdForText", obj(vec![("guid", guid(20))])),
            ],
        ),
    ])
}

#[test]
fn a_text_style_outside_the_scope_makes_coverage_partial_instead_of_unchanged() {
    // The style's own record changes, but only a layer referencing it is in scope.
    let out = scoped(styled(12.0), styled(16.0), "0:10");
    assert_eq!(
        (&out["coverage"]["complete"], &out["coverage"]["reasons"]),
        (
            &json!(false),
            &json!([
                {"reason": "definition_not_compared", "side": "old", "count": 1},
                {"reason": "definition_not_compared", "side": "new", "count": 1},
            ])
        )
    );
}

#[test]
fn a_variable_value_change_over_the_whole_file_is_a_properties_change() {
    let out = run(bound_screen(8.0), bound_screen(12.0));
    let facets = out["frames"][0]["nodes"][0]["facets"].clone();
    assert_eq!(
        (nodes(&out), facets, &out["coverage"]["complete"]),
        (
            vec![("0:60".into(), "changed".into())],
            json!(["properties"]),
            &json!(true)
        )
    );
}

#[test]
fn a_bound_variable_outside_the_scope_makes_coverage_partial() {
    let out = scoped(bound_screen(8.0), bound_screen(8.0), "0:10");
    assert_eq!(
        out["coverage"]["reasons"],
        json!([
            {"reason": "definition_not_compared", "side": "old", "count": 1},
            {"reason": "definition_not_compared", "side": "new", "count": 1},
        ])
    );
}

#[test]
fn numeric_variants_trim_links_mask_type_and_grids_are_compared() {
    let link = |url: &str| obj(vec![("url", K::String(url.to_owned()))]);
    for (key, old, new, facet) in [
        (
            "fontVariantNumericFigure",
            K::Enum("FontVariantNumericFigure", "LINING"),
            K::Enum("FontVariantNumericFigure", "OLDSTYLE"),
            "text",
        ),
        (
            "fontVariantNumericSpacing",
            K::Enum("FontVariantNumericSpacing", "TABULAR"),
            K::Enum("FontVariantNumericSpacing", "PROPORTIONAL"),
            "text",
        ),
        (
            "leadingTrim",
            K::Enum("LeadingTrim", "NONE"),
            K::Enum("LeadingTrim", "CAP_HEIGHT"),
            "text",
        ),
        (
            "hyperlink",
            link("https://a.test"),
            link("https://b.test"),
            "text",
        ),
        (
            "maskType",
            K::Enum("MaskType", "ALPHA"),
            K::Enum("MaskType", "OUTLINE"),
            "appearance",
        ),
        (
            "layoutGrids",
            K::Array(vec![obj(vec![("sectionSize", K::Float(8.0))])]),
            K::Array(vec![obj(vec![("sectionSize", K::Float(12.0))])]),
            "layout",
        ),
    ] {
        let one = |v: K<'static>| doc(vec![node(10, Some(1), "TEXT", "Layer", vec![(key, v)])]);
        let n = &run(one(old), one(new))["frames"][0]["nodes"][0];
        assert_eq!(n["facets"], json!([facet]), "{key}");
    }
}

#[test]
fn renaming_a_component_property_definition_is_a_properties_change() {
    let def = |name: &str| {
        (
            "componentPropDefs",
            K::Array(vec![obj(vec![
                ("id", guid(70)),
                ("name", K::String(name.to_owned())),
                ("type", K::Enum("ComponentPropType", "BOOL")),
                ("initialValue", obj(vec![("boolValue", K::Bool(true))])),
            ])]),
        )
    };
    let one = |name: &str| {
        doc(vec![node(
            20,
            Some(1),
            "COMPONENT",
            "Button",
            vec![def(name)],
        )])
    };
    let out = run(one("Show icon"), one("Has icon"));
    assert_eq!(
        (nodes(&out), &out["frames"][0]["nodes"][0]["facets"]),
        (
            vec![("0:20".into(), "changed".into())],
            &json!(["properties"])
        )
    );
}

/// A main component outside frame `0:10` with one Boolean definition, and an instance of it in
/// the frame.
fn used(name: &str, default: bool) -> K<'static> {
    doc(vec![
        node(
            20,
            Some(1),
            "COMPONENT",
            "Button",
            vec![(
                "componentPropDefs",
                K::Array(vec![obj(vec![
                    ("id", guid(70)),
                    ("name", K::String(name.to_owned())),
                    ("type", K::Enum("ComponentPropType", "BOOL")),
                    ("initialValue", obj(vec![("boolValue", K::Bool(default))])),
                ])]),
            )],
        ),
        node(10, Some(1), "FRAME", "Home", vec![]),
        instance(11, 10, "Button", 20, vec![]),
    ])
}

#[test]
fn a_main_outside_the_scope_makes_coverage_partial_when_its_property_definitions_change() {
    // Only the instance is in scope; its main's Boolean definition is renamed and its default
    // flipped, which changes the instance's emitted componentProperties.
    let out = scoped(used("Show icon", true), used("Has icon", false), "0:10");
    assert_eq!(
        (
            &out["coverage"]["complete"],
            nodes(&out),
            &out["coverage"]["reasons"]
        ),
        (
            &json!(false),
            vec![],
            &json!([
                {"reason": "definition_not_compared", "side": "old", "count": 1},
                {"reason": "definition_not_compared", "side": "new", "count": 1},
            ])
        )
    );
}

/// A workspace schema in memory, with `trees` saved as generations 1, 2, … of scope `0:0`.
fn saved(trees: &[&Tree], scope: &str) -> rusqlite::Connection {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
         CREATE TABLE sources (source_id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE,
         added_at_ms INTEGER NOT NULL) STRICT; {} {} {}",
        crate::workspace::V2,
        crate::workspace::V3,
        crate::workspace::V4
    ))
    .unwrap();
    for (i, tree) in trees.iter().enumerate() {
        let gen_id = i as i64 + 1;
        conn.execute(
            "INSERT INTO generations (gen, source_id, membership, scope, versions, job_id, state,
             created_ms) VALUES (?1, 's', 'm', ?2, 'v', 'j', 'building', 0)",
            rusqlite::params![gen_id, scope],
        )
        .unwrap();
        let (index, tracked) = tracked(tree, scope, Instant::now() + DEADLINE);
        crate::search::store::save(
            &mut conn,
            gen_id,
            &index,
            &|_| Ok(()),
            &mut |_, _, _| Ok(()),
        )
        .unwrap();
        save_facets(&mut conn, gen_id, &tracked, &|_| Ok(()), &mut |_, _, _| {
            Ok(())
        })
        .unwrap();
    }
    conn
}

#[test]
fn stored_generations_diff_exactly_like_the_live_trees() {
    let mut moved = screen("Hi", 0.0, 420.0);
    moved.push(node(14, Some(1), "FRAME", "Other", vec![]));
    moved[3] = node(13, Some(14), "RECTANGLE", "Same", vec![fill(0.5)]);
    let (old, new) = (doc(screen("Hello", 1.0, 400.0)), doc(moved));
    let (a, b) = (Tree::new(&old), Tree::new(&new));
    let live = page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:0"),
        &req(LIMIT, None),
        false,
    )
    .unwrap();
    let side = |label: &str| json!({"sourceId": format!("local:{label}"), "snapshotId": format!("sha256:{label}")});
    let conn = saved(&[&a, &b], "0:0");
    let stored = stored_diff(&conn, (1, side("old")), (2, side("new")), &req(LIMIT, None));
    let unstamped = |mut v: Json| {
        v["source"]["freshness"]["hashedAtMs"].take();
        v
    };
    assert_eq!(unstamped(stored.unwrap()), unstamped(live));
}

#[test]
fn stored_generations_report_an_outside_definition_like_the_live_trees() {
    outside_parity(styled(12.0), styled(16.0));
    outside_parity(used("Show icon", true), used("Has icon", false));
}

/// Diffs `old` and `new` from stored generations of frame `0:10` and asserts the same partial
/// coverage reasons as the live trees.
fn outside_parity(old: K<'static>, new: K<'static>) {
    let (a, b) = (Tree::new(&old), Tree::new(&new));
    let mut r = req(LIMIT, None);
    r.node = "0:10".into();
    let live = page(
        &compare_trees(&a, &b, [WORK, WORK, PAIR_WORK], "0:10"),
        &r,
        false,
    )
    .unwrap();
    let side = |label: &str| json!({"sourceId": format!("local:{label}"), "snapshotId": format!("sha256:{label}")});
    let conn = saved(&[&a, &b], "0:10");
    let stored = stored_diff(&conn, (1, side("old")), (2, side("new")), &r).unwrap();
    let coverage = |v: &Json| {
        (
            v["coverage"]["complete"].clone(),
            v["coverage"]["reasons"].clone(),
        )
    };
    assert_eq!(
        coverage(&stored),
        (json!(false), coverage(&live).1),
        "{stored}"
    );
}

#[test]
fn tampered_or_other_layout_facets_are_refused_not_compared() {
    let old = doc(screen("Hello", 1.0, 400.0));
    let tree = Tree::new(&old);
    let source = json!({"sourceId": "local:a", "snapshotId": "sha256:a"});
    let run = |conn: &rusqlite::Connection| {
        let err = stored_diff(
            conn,
            (1, source.clone()),
            (1, source.clone()),
            &req(LIMIT, None),
        )
        .unwrap_err();
        err.downcast_ref::<FigError>().map(FigError::kind)
    };
    let conn = saved(&[&tree], "0:0");
    conn.execute(
        "UPDATE facets SET blob = CAST('[[\"bogus\",null]]' AS BLOB) WHERE rid = 1",
        [],
    )
    .unwrap();
    assert_eq!(run(&conn), Some(ErrorKind::IndexIncompatible));
    let conn = saved(&[&tree], "0:0");
    conn.execute("UPDATE facet_sets SET facet_version = 'diff-v0:0'", [])
        .unwrap();
    assert_eq!(run(&conn), Some(ErrorKind::IndexIncompatible));
}
