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
