use super::*;
use kiwi_schema::Value as K;

pub(super) fn obj(fields: Vec<(&'static str, K<'static>)>) -> K<'static> {
    K::Object("Test", fields.into_iter().collect())
}

pub(super) fn guid(id: u32) -> K<'static> {
    obj(vec![("sessionID", K::UInt(0)), ("localID", K::UInt(id))])
}

pub(super) fn text(s: &str) -> (&'static str, K<'static>) {
    ("textData", obj(vec![("characters", K::String(s.into()))]))
}

pub(super) fn path(ids: &[u32]) -> (&'static str, K<'static>) {
    (
        "guidPath",
        obj(vec![(
            "guids",
            K::Array(ids.iter().map(|&i| guid(i)).collect()),
        )]),
    )
}

pub(super) fn node(
    id: u32,
    parent: Option<u32>,
    kind: &'static str,
    name: &str,
    extra: Vec<(&'static str, K<'static>)>,
) -> K<'static> {
    let mut fields = vec![
        ("guid", guid(id)),
        ("type", K::Enum("NodeType", kind)),
        ("name", K::String(name.into())),
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

fn prop_ref(def: u32, field: &'static str) -> (&'static str, K<'static>) {
    (
        "componentPropRefs",
        K::Array(vec![obj(vec![
            ("defID", guid(def)),
            (
                "componentPropNodeField",
                K::Enum("ComponentPropNodeField", field),
            ),
        ])]),
    )
}

fn assignments(label: &str, show_hint: bool) -> (&'static str, K<'static>) {
    (
        "componentPropAssignments",
        K::Array(vec![
            obj(vec![
                ("defID", guid(90)),
                (
                    "value",
                    obj(vec![(
                        "textValue",
                        obj(vec![("characters", K::String(label.into()))]),
                    )]),
                ),
            ]),
            obj(vec![
                ("defID", guid(91)),
                ("value", obj(vec![("boolValue", K::Bool(show_hint))])),
            ]),
        ]),
    )
}

pub(super) fn instance(
    id: u32,
    parent: u32,
    name: &str,
    main: u32,
    extra: Vec<(&'static str, K<'static>)>,
) -> K<'static> {
    let mut symbol = vec![("symbolID", guid(main))];
    let mut rest = Vec::new();
    for (k, v) in extra {
        if k == "symbolOverrides" {
            symbol.push((k, v));
        } else {
            rest.push((k, v));
        }
    }
    rest.push(("symbolData", obj(symbol)));
    node(id, Some(parent), "INSTANCE", name, rest)
}

fn message() -> K<'static> {
    let long = format!("{} needle", "lorem ipsum ".repeat(30));
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Checkout", vec![]),
            // Components: a button with text/boolean properties, two icons and a card nesting them.
            node(10, Some(1), "SYMBOL", "Button", vec![]),
            node(
                11,
                Some(10),
                "TEXT",
                "Label",
                vec![text("Default label"), prop_ref(90, "TEXT_DATA")],
            ),
            node(
                12,
                Some(10),
                "TEXT",
                "Hint",
                vec![text("Secret hint"), prop_ref(91, "VISIBLE")],
            ),
            node(20, Some(1), "SYMBOL", "Icon", vec![]),
            node(21, Some(20), "TEXT", "Glyph", vec![text("Star glyph")]),
            node(25, Some(1), "SYMBOL", "Alt icon", vec![]),
            node(26, Some(25), "TEXT", "Glyph", vec![text("Swapped content")]),
            node(30, Some(1), "SYMBOL", "Card", vec![]),
            instance(31, 30, "Inner button", 10, vec![]),
            instance(32, 30, "Icon slot", 20, vec![]),
            instance(
                3,
                2,
                "Pay card",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![
                        obj(vec![path(&[31, 11]), text("Pay now 立即付款")]),
                        obj(vec![path(&[32]), ("overriddenSymbolID", guid(25))]),
                        obj(vec![path(&[31]), assignments("Nested label", false)]),
                    ]),
                )],
            ),
            instance(
                4,
                2,
                "Plain button",
                10,
                vec![assignments("Confirm order", false)],
            ),
            node(
                5,
                Some(2),
                "TEXT",
                "Hidden note",
                vec![text("Pay later"), ("visible", K::Bool(false))],
            ),
            node(6, Some(2), "TEXT", "Terms", vec![text(&long)]),
            node(7, Some(2), "FRAME", "Row", vec![]),
            node(8, Some(2), "FRAME", "Row", vec![]),
            node(9, Some(2), "TEXT", "Greeting", vec![text("مرحبا بالعالم")]),
            node(13, Some(2), "TEXT", "Party", vec![text("🎉 Party time 👍🏽")]),
            node(14, Some(2), "TEXT", "Menu", vec![text("Cafe\u{301} menu")]),
            node(
                15,
                Some(2),
                "TEXT",
                "Street",
                vec![text("STRASSE Straße ΟΔΟΣ ＰＡＹ ｶﾞｰﾄﾞ")],
            ),
            node(16, Some(2), "TEXT", "Notes", vec![text("設計稿 カード")]),
            node(17, Some(2), "FRAME", "Row header", vec![]),
            node(18, Some(2), "FRAME", "IconButton/Primary", vec![]),
            node(
                50,
                Some(0),
                "CANVAS",
                "Internal",
                vec![("internalOnly", K::Bool(true))],
            ),
            node(51, Some(50), "TEXT", "Copy", vec![text("Pay internal")]),
        ]),
    )])
}

pub(super) fn bounds(work: usize) -> Bounds {
    Bounds {
        work,
        bytes: BYTES,
        depth: DEPTH,
        deadline: Instant::now() + DEADLINE,
        stop: None,
    }
}

fn request(query: &str) -> Request {
    Request {
        query: query.into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit: DEFAULT_LIMIT,
        cursor: None,
    }
}

fn run_with(req: &Request, work: usize) -> Json {
    let message = message();
    let tree = Tree::new(&message);
    let index = build(&tree, &req.scope, bounds(work));
    let q = req.validate().unwrap();
    reply(
        &index,
        &q,
        req,
        json!({"sourceId": "local:test", "snapshotId": "sha256:test"}),
    )
    .unwrap()
}

fn run(req: &Request) -> Json {
    run_with(req, WORK)
}

fn ids(reply: &Json) -> Vec<&str> {
    reply["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"]["nodeId"].as_str().unwrap())
        .collect()
}

fn first(reply: &Json) -> &Json {
    &reply["results"][0]
}

#[test]
fn instance_text_overrides_keep_exact_expanded_ids_and_origin() {
    let found = run(&request("pay now"));
    assert_eq!(ids(&found), ["I0:3;0:31;0:11"]);
    let hit = first(&found);
    assert_eq!(hit["origin"], "instance-effective");
    assert_eq!(hit["textSource"], "override");
    assert_eq!(hit["sourceNodeId"], "0:11");
    assert_eq!(hit["scopeRoot"], "0:2");
    assert_eq!(hit["page"], json!({"id": "0:1", "name": "Page"}));
    assert_eq!(
        hit["ref"],
        json!({"sourceId": "local:test", "snapshotId": "sha256:test", "nodeId": "I0:3;0:31;0:11"})
    );
}

#[test]
fn text_properties_on_direct_instances_are_effective_text() {
    let found = run(&request("confirm order"));
    assert_eq!(ids(&found), ["I0:4;0:11"]);
    assert_eq!(first(&found)["textSource"], "override");
}

#[test]
fn nested_swaps_index_the_swapped_component_content() {
    let found = run(&request("swapped content"));
    // Document order: the checkout instance's swapped slot precedes the component itself.
    assert_eq!(ids(&found), ["I0:3;0:32;0:26", "0:26"]);
    assert_eq!(first(&found)["textSource"], "component");
    assert!(
        ids(&run(&request("star glyph")))
            .iter()
            .all(|id| !id.starts_with("I0:3;"))
    );
}

#[test]
fn main_default_text_is_not_counted_as_overridden_instance_text() {
    let mut req = request("default label");
    req.include_hidden = true;
    let found = run(&req);
    // Both overridden copies are absent; the card main's unmodified button shows the default.
    assert_eq!(ids(&found), ["0:11", "I0:31;0:11"]);
    assert_eq!(found["results"][0]["textSource"], "stored");
    assert_eq!(found["results"][1]["textSource"], "component");
}

#[test]
fn boolean_properties_hide_instance_copies_unless_requested() {
    let found = run(&request("secret hint"));
    assert_eq!(ids(&found), ["0:12", "I0:31;0:12"]);
    assert_eq!(found["coverage"]["excluded"]["hidden"], 2);
    let mut req = request("secret hint");
    req.include_hidden = true;
    let all = run(&req);
    assert_eq!(
        ids(&all),
        ["I0:3;0:31;0:12", "I0:4;0:12", "0:12", "I0:31;0:12"]
    );
    assert_eq!(all["results"][0]["hidden"], true);
}

#[test]
fn hidden_stored_layers_need_the_hidden_filter() {
    assert!(ids(&run(&request("pay later"))).is_empty());
    let mut req = request("pay later");
    req.include_hidden = true;
    assert_eq!(ids(&run(&req)), ["0:5"]);
}

#[test]
fn internal_pages_need_the_internal_filter() {
    let found = run(&request("pay internal"));
    assert!(ids(&found).is_empty());
    assert_eq!(found["coverage"]["excluded"]["internal"], 1);
    let mut req = request("pay internal");
    req.include_internal = true;
    let found = run(&req);
    assert_eq!(ids(&found), ["0:51"]);
    assert_eq!(first(&found)["internal"], true);
}

#[test]
fn long_text_is_indexed_past_the_preview_length_with_a_bounded_snippet() {
    let found = run(&request("needle"));
    let hit = first(&found);
    assert_eq!(hit["ref"]["nodeId"], "0:6");
    let start = 30 * "lorem ipsum ".len() + 1;
    assert_eq!(hit["textLength"], start + 6);
    let text = &hit["matches"][0];
    assert_eq!(
        text["span"],
        json!({"start": start, "end": start + 6, "unit": "utf16"})
    );
    assert_eq!(text["snippet"]["truncatedStart"], true);
    let snippet = text["snippet"]["text"].as_str().unwrap();
    assert!(snippet.ends_with("needle") && snippet.chars().count() < 60);
    assert_eq!(text["snippet"]["sourceStart"], start - 40);
}

#[test]
fn case_folding_and_compatibility_forms_match_with_original_spans() {
    let street = |q: &str| {
        let found = run(&request(q));
        let hit = found["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["ref"]["nodeId"] == "0:15")
            .cloned();
        hit.unwrap()["matches"][0]["span"].clone()
    };
    // ß folds to ss, final sigma to sigma, fullwidth and halfwidth voiced kana compose.
    assert_eq!(
        street("straße"),
        json!({"start": 0, "end": 7, "unit": "utf16"})
    );
    assert_eq!(
        street("οδος"),
        json!({"start": 15, "end": 19, "unit": "utf16"})
    );
    assert_eq!(
        street("pay"),
        json!({"start": 20, "end": 23, "unit": "utf16"})
    );
    assert_eq!(
        street("ガード"),
        json!({"start": 24, "end": 29, "unit": "utf16"})
    );
}

#[test]
fn combining_marks_match_their_precomposed_form() {
    let found = run(&request("café"));
    assert_eq!(ids(&found), ["0:14"]);
    assert_eq!(
        first(&found)["matches"][0]["span"],
        json!({"start": 0, "end": 5, "unit": "utf16"})
    );
}

#[test]
fn cjk_uses_bigrams_and_a_single_character_path_without_script_conversion() {
    assert_eq!(ids(&run(&request("付款"))), ["I0:3;0:31;0:11"]);
    assert_eq!(ids(&run(&request("付"))), ["I0:3;0:31;0:11"]);
    assert_eq!(ids(&run(&request("設計"))), ["0:16"]);
    // Simplified for traditional and hiragana for katakana are different text.
    assert!(ids(&run(&request("设计"))).is_empty());
    assert!(ids(&run(&request("かーど"))).is_empty());
    assert_eq!(ids(&run(&request("カード"))), ["0:16"]);
}

#[test]
fn emoji_and_rtl_text_match_with_utf16_spans() {
    let party = run(&request("👍🏽"));
    assert_eq!(ids(&party), ["0:13"]);
    assert_eq!(
        first(&party)["matches"][0]["span"],
        json!({"start": 14, "end": 18, "unit": "utf16"})
    );
    let time = run(&request("time"));
    assert_eq!(first(&time)["matches"][0]["span"]["start"], 9);
    let rtl = run(&request("بالعالم"));
    assert_eq!(ids(&rtl), ["0:9"]);
    assert_eq!(
        first(&rtl)["matches"][0]["span"],
        json!({"start": 6, "end": 13, "unit": "utf16"})
    );
}

#[test]
fn result_paths_are_charged_before_they_are_copied() {
    // 200 hits under 20 ancestors with 100 kB names would copy 400 MB of paths.
    let name = "a".repeat(100_000);
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
    ];
    nodes.extend((2..22).map(|id| node(id, Some(id - 1), "FRAME", &name, vec![])));
    nodes.extend((100..300).map(|id| node(id, Some(21), "TEXT", "Row", vec![])));
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let mut req = request("row");
    req.limit = MAX_LIMIT;
    let q = req.validate().unwrap();
    let error = reply(&index, &q, &req, source.clone()).unwrap_err();
    let error = error.downcast_ref::<FigError>().unwrap();
    assert_eq!(error.kind(), ErrorKind::ResponseBudget);
    req.limit = 3;
    let q = req.validate().unwrap();
    assert_eq!(ids(&reply(&index, &q, &req, source).unwrap()).len(), 3);
}

#[test]
fn snippets_stay_bounded_around_long_attached_runs_and_wide_matches() {
    // Attached runs on both sides of the match, then a match widened by composed jamo.
    let runs = format!("y{0}x{0}", "\u{FE0F}".repeat(100_000));
    let wide = "\u{1100}\u{1161}\u{11A8}".repeat(20_000);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "TEXT", "Copy", vec![text(&runs)]),
            node(3, Some(1), "TEXT", "Wide", vec![text(&wide)]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let snippet_of = |query: &str| {
        let req = request(query);
        let q = req.validate().unwrap();
        let found = reply(&index, &q, &req, source.clone()).unwrap();
        first(&found)["matches"][0]["snippet"].clone()
    };
    let runs = snippet_of("x");
    let text = runs["text"].as_str().unwrap();
    assert!(
        text.contains('x') && text.len() <= SNIPPET_BYTES,
        "{}",
        text.len()
    );
    assert_eq!(
        (runs["truncatedStart"].clone(), runs["truncatedEnd"].clone()),
        (json!(true), json!(true))
    );
    let wide = snippet_of(&"각".repeat(256));
    let text = wide["text"].as_str().unwrap();
    assert!(
        text.len() <= SNIPPET_BYTES && text.starts_with('\u{1100}'),
        "{}",
        text.len()
    );
    assert_eq!(wide["truncatedEnd"], json!(true));
}

#[test]
fn case_folding_composes_across_a_whole_segment() {
    // Folding J or T apart from its mark leaves it decomposed, unlike the precomposed query.
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "TEXT", "J\u{30C}", vec![]),
            node(3, Some(1), "TEXT", "T\u{308}", vec![]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    for (query, id) in [("\u{1F0}", "0:2"), ("\u{1E97}", "0:3")] {
        let req = request(query);
        let q = req.validate().unwrap();
        let found = reply(&index, &q, &req, source.clone()).unwrap();
        assert_eq!(ids(&found), [id], "{query}");
        assert_eq!(first(&found)["matches"][0]["reason"], "exact");
    }
}

#[test]
fn a_phrase_after_a_rejected_overlap_ranks_as_a_phrase() {
    // Same terms in both names; only the second holds "bar bar" as whole words, after an
    // occurrence that starts inside "foobar".
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "bar foobar bar", vec![]),
            node(3, Some(1), "FRAME", "foobar bar bar", vec![]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let req = request("bar bar");
    let q = req.validate().unwrap();
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let found = reply(&index, &q, &req, source).unwrap();
    assert_eq!(ids(&found), ["0:3", "0:2"]);
    let matched = &first(&found)["matches"][0];
    assert_eq!(matched["reason"], "phrase");
    assert_eq!(
        matched["span"],
        json!({"start": 7, "end": 14, "unit": "utf16"})
    );
}

#[test]
fn names_split_at_slashes_and_camel_case() {
    assert_eq!(ids(&run(&request("button primary"))), ["0:18"]);
    assert_eq!(
        first(&run(&request("icon/primary")))["ref"]["nodeId"],
        "0:18"
    );
    // Case folding alone decides an exact name, whichever side carries the camel case.
    let exact = run(&request("iconbutton/primary"));
    assert_eq!(first(&exact)["matches"][0]["reason"], "exact");
    assert_eq!(ids(&run(&request("ICONBUTTON"))), ["0:18"]);
    // A mixed-case query finds a name or text without that capitalization.
    let greeting = run(&request("GreeTing"));
    assert_eq!(first(&greeting)["matches"][0]["reason"], "exact");
    assert_eq!(ids(&run(&request("StrAsse"))), ["0:15"]);
}

#[test]
fn exact_names_lead_and_duplicates_keep_document_order() {
    let found = run(&request("row"));
    assert_eq!(ids(&found), ["0:7", "0:8", "0:17"]);
    assert_eq!(first(&found)["matches"][0]["reason"], "exact");
    assert_eq!(found["results"][2]["matches"][0]["reason"], "phrase");
}

#[test]
fn terms_match_whole_tokens_only() {
    assert!(ids(&run(&request("pa"))).is_empty());
    assert_eq!(
        first(&run(&request("label")))["matches"][0]["terms"],
        json!(["label"])
    );
}

#[test]
fn pages_continue_with_a_cursor_bound_to_the_query() {
    let mut req = request("row");
    req.limit = 2;
    let page = run(&req);
    assert_eq!(page["hasMore"], true);
    let cursor = page["nextCursor"].as_str().unwrap().to_owned();
    req.cursor = Some(cursor.clone());
    let next = run(&req);
    assert_eq!(ids(&next), ["0:17"]);
    assert_eq!(next["results"][0]["rank"], 3);
    assert_eq!(next["hasMore"], false);
    assert_eq!(next["nextCursor"], Json::Null);
    let message = message();
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let rejects = |index: &Index, req: &Request, source: Json| {
        reply(index, &req.validate().unwrap(), req, source).is_err()
    };
    assert!(!rejects(&index, &req, source.clone()));
    let mut other = req.clone();
    other.query = "row header".into();
    assert!(rejects(&index, &other, source.clone()));
    assert!(rejects(
        &index,
        &req,
        json!({"sourceId": "local:other", "snapshotId": "sha256:test"})
    ));
    assert!(rejects(
        &index,
        &req,
        json!({"sourceId": "local:test", "snapshotId": "sha256:other"})
    ));
    // A partial rebuild of the same snapshot is another generation, so its pages differ.
    assert!(rejects(&build(&tree, "0:0", bounds(10)), &req, source));
}

#[test]
fn types_filter_candidates() {
    let mut req = request("row");
    req.types = vec!["TEXT".into()];
    assert!(ids(&run(&req)).is_empty());
}

#[test]
fn invalid_requests_fail_before_any_work() {
    let bad = |edit: &dyn Fn(&mut Request)| {
        let mut req = request("pay");
        edit(&mut req);
        req.validate().err().unwrap().to_string()
    };
    assert!(bad(&|r| r.query = " / ".into()).contains("needs a query"));
    assert!(bad(&|r| r.query = "x".repeat(257)).contains("256"));
    assert!(bad(&|r| r.limit = 0).contains("1-200"));
    assert!(bad(&|r| r.limit = 201).contains("1-200"));
    assert!(bad(&|r| r.types = vec!["text".into()]).contains("node types"));
    assert!(bad(&|r| r.cursor = Some("s1.zz.2".into())).contains("cursor"));
    assert!(bad(&|r| r.query = (0..65).map(|i| format!("w{i} ")).collect()).contains("64"));
}

#[test]
fn a_budget_stop_reports_partial_coverage_without_a_continuation() {
    let found = run_with(&request("row"), 10);
    let coverage = &found["coverage"];
    assert_eq!(coverage["complete"], false);
    assert_eq!(coverage["layers"]["text"]["state"], "partial");
    assert_eq!(coverage["totalRecords"], Json::Null);
    assert_eq!(coverage["reasons"][0]["code"], "work_limit");
    assert_eq!(found["hasMore"], false);
    assert_eq!(found["nextCursor"], Json::Null);
    let full = run(&request("row"))["coverage"].clone();
    assert_eq!(full["layers"]["text"]["state"], "ready");
    assert_eq!(full["layers"]["relations"]["kinds"]["instance_of"], "ready");
    assert_eq!(
        full["layers"]["relations"]["reasons"][0]["code"],
        "relation_unavailable"
    );
    assert_eq!(full["totalRecords"], full["scannedRecords"]);
}

#[test]
fn requested_hidden_content_that_rest_omits_is_reported_partial() {
    let mut req = request("star glyph");
    let visible = run(&req);
    assert_eq!(visible["coverage"]["layers"]["text"]["state"], "ready");
    req.include_hidden = true;
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(20, Some(1), "SYMBOL", "Icon", vec![]),
            node(21, Some(20), "TEXT", "Glyph", vec![text("Star glyph")]),
            instance(2, 1, "Hidden icon", 20, vec![("visible", K::Bool(false))]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let found = reply(&index, &req.validate().unwrap(), &req, json!({})).unwrap();
    let coverage = &found["coverage"];
    assert_eq!(coverage["layers"]["text"]["state"], "partial");
    assert_eq!(coverage["totalRecords"], Json::Null);
    assert_eq!(
        coverage["reasons"][0],
        json!({"code": "hidden_content_not_expanded", "counts": {"layers": 1}})
    );
}

#[test]
fn a_byte_bound_stops_before_normalizing_a_large_field() {
    let message = message();
    let tree = Tree::new(&message);
    let index = build(
        &tree,
        "0:0",
        Bounds {
            bytes: 64 * 1024,
            ..bounds(WORK)
        },
    );
    assert_eq!(index.stop, Some("byte_limit"));
    assert!(index.records.iter().all(|r| r.id != "0:6"));
}

#[test]
fn bounded_expansion_matches_unbounded_rest_selection() {
    let message = message();
    let tree = Tree::new(&message);
    let full = crate::expand::expand_instances(&tree, "0:0");
    let (bounded, left) = expand_instances_bounded(&tree, "0:0", bounds(WORK));
    assert_eq!(left.stop, None);
    let mut a: Vec<_> = full.nodes.keys().collect();
    let mut b: Vec<_> = bounded.nodes.keys().collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    let (_, left) = expand_instances_bounded(
        &tree,
        "0:0",
        Bounds {
            depth: 0,
            ..bounds(WORK)
        },
    );
    assert_eq!(left.stop, Some(Stop::Depth));
}

#[test]
fn normalization_matches_whole_string_nfkc_fold_nfkc() {
    let whole = |s: &str| -> String {
        s.nfkc()
            .collect::<String>()
            .chars()
            .default_case_fold()
            .collect::<String>()
            .nfkc()
            .collect()
    };
    for s in [
        "\u{1100}\u{1161}\u{11A8}",
        "\u{AC01}",
        "ｶﾞｰﾄﾞ",
        "Cafe\u{301}",
        "STRASSE Straße",
        "ﬁ ㎏ Ⅻ",
        "\u{3131}\u{314F}",
        "x ＰＡＹ",
    ] {
        let units: String = normalize(s).iter().map(|u| u.c).collect();
        assert_eq!(units, whole(s), "{s:?}");
    }
    let jamo = normalize("\u{1100}\u{1161}");
    assert_eq!(
        (jamo.len(), jamo[0].c, jamo[0].start, jamo[0].end),
        (1, '가', 0, 6)
    );
    // Compatibility jamo compose too; fullwidth letters keep their own spans.
    let jamo = normalize("\u{3131}\u{314F}");
    assert_eq!(
        (jamo.len(), jamo[0].c, jamo[0].start, jamo[0].end),
        (1, '가', 0, 6)
    );
    let spans: Vec<_> = normalize("x ＰＡＹ")
        .iter()
        .map(|u| (u.c, u.start, u.end))
        .collect();
    assert_eq!(
        spans,
        [
            ('x', 0, 1),
            (' ', 1, 2),
            ('p', 2, 5),
            ('a', 5, 8),
            ('y', 8, 11)
        ]
    );
}

#[test]
fn normalized_copies_are_charged_at_their_expanded_size() {
    // U+FDFA is 3 bytes and normalizes to 33: 1,000 names of 100 keep about 3.6 MB.
    let name = "\u{FDFA}".repeat(100);
    let mut nodes = vec![node(0, None, "DOCUMENT", "Document", vec![])];
    nodes.extend((1..=1000).map(|id| node(id, Some(0), "FRAME", &name, vec![])));
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let tree = Tree::new(&message);
    let index = |bytes| {
        build(
            &tree,
            "0:0",
            Bounds {
                bytes,
                ..bounds(WORK)
            },
        )
    };
    assert_eq!(index(2_500_000).stop, Some("byte_limit"));
    assert_eq!(index(8 << 20).stop, None);
}

#[test]
fn a_long_combining_run_is_refused_before_normalization() {
    // Past the deadline, any normalization would stop at its first check with Deadline.
    let marks = "a".repeat(100_000) + &"\u{301}".repeat(MAX_MARKS + 1);
    let past = Instant::now() - Duration::from_secs(1);
    assert_eq!(transient(&marks, past), Err(None));
    assert_eq!(
        transient(&"a".repeat(100_000), past),
        Err(Some(Stop::Deadline))
    );
    // The build reports the run as its own reason, not as an exhausted byte budget.
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "TEXT", "Marks", vec![text(&marks)]),
        ]),
    )]);
    let tree = Tree::new(&message);
    assert_eq!(
        build(&tree, "0:0", bounds(WORK)).stop,
        Some("combining_run_limit")
    );
}

#[test]
fn a_cursor_does_not_page_another_partial_build_with_equal_counts() {
    let req = Request {
        limit: 1,
        ..request("pay")
    };
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let first = message();
    let tree = Tree::new(&first);
    // The smallest work that stops the build after more than one page of matches.
    let (work, index, cursor) = (1..WORK)
        .map(|work| (work, build(&tree, "0:0", bounds(work))))
        .filter(|(_, index)| index.stop.is_some())
        .find_map(|(work, index)| {
            let next = reply(&index, &req.validate().unwrap(), &req, source.clone()).unwrap()
                ["nextCursor"]
                .as_str()
                .map(str::to_owned);
            next.map(|cursor| (work, index, cursor))
        })
        .unwrap();
    // Same shape with another frame name: the build stops at the same record, other contents.
    let mut changed = message();
    let K::Object(_, fields) = &mut changed else {
        unreachable!()
    };
    let K::Array(nodes) = fields.get_mut("nodeChanges").unwrap() else {
        unreachable!()
    };
    nodes[2] = node(2, Some(1), "FRAME", "Other frame", vec![]);
    let other_tree = Tree::new(&changed);
    let other = build(&other_tree, "0:0", bounds(work));
    assert_eq!(
        (other.records.len(), other.stop),
        (index.records.len(), index.stop)
    );
    let paged = Request {
        cursor: Some(cursor),
        ..req
    };
    let q = paged.validate().unwrap();
    assert!(reply(&index, &q, &paged, source.clone()).is_ok());
    assert!(reply(&other, &q, &paged, source).is_err());
}

#[test]
fn an_expansion_stop_still_indexes_stored_layers() {
    let message = message();
    let tree = Tree::new(&message);
    let index = build(
        &tree,
        "0:0",
        Bounds {
            depth: 0,
            ..bounds(WORK)
        },
    );
    assert_eq!(index.stop, Some("depth_limit"));
    assert!(index.records.iter().any(|r| r.id == "0:2"));
}

#[test]
fn phrases_match_whole_words_except_in_cjk() {
    assert_eq!(phrase_at("arrow", "row"), None);
    assert_eq!(phrase_at("arrow row", "row"), Some(6));
    assert_eq!(phrase_at("pay now", "pay now"), Some(0));
    assert_eq!(phrase_at("立即購買", "購買"), Some(6));
    // A rejected occurrence inside a word must not hide an overlapping whole-word one.
    assert_eq!(phrase_at("foobar bar bar", "bar bar"), Some(7));
}

#[test]
fn a_cjk_sentence_fits_the_term_limit() {
    assert!(
        request("設計稿中的按鈕文字需要與實作保持一致才能交付")
            .validate()
            .is_ok()
    );
}

#[test]
fn queries_up_to_the_term_limit_retrieve_only_records_with_every_term() {
    let words: Vec<String> = (0..MAX_TERMS).map(|i| format!("w{i}")).collect();
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "TEXT", "All", vec![text(&words.join(" "))]),
            node(
                2,
                Some(0),
                "TEXT",
                "Most",
                vec![text(&words[1..].join(" "))],
            ),
        ]),
    )]);
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    for n in [32, 33, MAX_TERMS] {
        let req = request(&words[..n].join(" "));
        let (hits, ..) = retrieve(&[&index], &req.validate().unwrap(), &req, &NEVER);
        let ids: Vec<&str> = hits
            .iter()
            .map(|h| index.records[h.record as usize].id.as_str())
            .collect();
        assert_eq!(ids, ["0:1"], "{n} terms");
    }
}

/// Instances stored, swapped to a missing main and swapped to a non-component.
fn swaps() -> K<'static> {
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(20, Some(1), "SYMBOL", "Icon", vec![]),
            node(40, Some(1), "FRAME", "Not a component", vec![]),
            node(30, Some(1), "SYMBOL", "Card", vec![]),
            instance(31, 30, "Default slot", 20, vec![]),
            instance(32, 30, "Frame slot", 20, vec![]),
            instance(
                3,
                1,
                "Card copy",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![
                        obj(vec![path(&[31]), ("overriddenSymbolID", guid(99))]),
                        obj(vec![path(&[32]), ("overriddenSymbolID", guid(40))]),
                    ]),
                )],
            ),
            instance(33, 1, "Stored", 20, vec![]),
        ]),
    )])
}

#[test]
fn a_swap_to_a_missing_main_keeps_its_id_as_missing_not_the_definitions_main() {
    let message = swaps();
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let of = |id: &str| {
        let r = index.records.iter().find(|r| r.id == id).unwrap();
        let e = r
            .edges
            .iter()
            .find(|e| e.relation == "instance_of")
            .unwrap();
        (e.target.clone(), e.state, e.basis, e.evidence.clone())
    };
    let swap = |target: &str, state| {
        (
            Some(target.to_owned()),
            state,
            "resolved",
            "swap".to_owned(),
        )
    };
    assert_eq!(of("I0:3;0:31"), swap("0:99", "missing"));
    assert_eq!(of("I0:3;0:32"), swap("0:40", "unsupported"));
    assert_eq!(
        of("0:33"),
        (
            Some("0:20".into()),
            "resolved",
            "recorded",
            "symbolData".into()
        )
    );
}

#[test]
fn a_set_cancel_flag_stops_loading_and_ranking_at_their_next_step() {
    let index = build(&Tree::new(&swaps()), "0:0", bounds(WORK));
    let conn = stored(&index);
    let all: Vec<u32> = (0..index.records.len() as u32).collect();
    let set = AtomicBool::new(true);
    let kind = |r: Result<(), Box<dyn Error>>| {
        r.unwrap_err()
            .downcast_ref::<FigError>()
            .map(FigError::kind)
    };
    let load = kind(store::load_with(&conn, 1, &["card".into()], &mut 0, &set).map(drop));
    let mut partial = store::load(&conn, 1, &[], &mut 0).unwrap();
    let add = kind(store::add_with(&conn, 1, &mut partial, &all, &mut 0, &set));
    let req = request("card");
    let q = req.validate().unwrap();
    let ranked = |cancel| retrieve(&[&index], &q, &req, cancel).0.len();
    let cancelled = Some(ErrorKind::Cancelled);
    assert_eq!(
        (load, add, ranked(&set), ranked(&NEVER) > 0),
        (cancelled, cancelled, 0, true)
    );
}

#[test]
fn one_query_budget_covers_every_load_and_is_checked_before_copying() {
    let index = build(&Tree::new(&swaps()), "0:0", bounds(WORK));
    let conn = stored(&index);
    let all: Vec<u32> = (0..index.records.len() as u32).collect();
    let mut one = 0;
    let mut first = store::load(&conn, 1, &[], &mut one).unwrap();
    store::add(&conn, 1, &mut first, &all, &mut one).unwrap();
    // A second load that fits alone, after others spent all but one byte of it.
    let mut spent = BYTES - one + 1;
    let second = store::load(&conn, 1, &[], &mut spent)
        .and_then(|mut i| store::add(&conn, 1, &mut i, &all, &mut spent));
    let kind = second
        .unwrap_err()
        .downcast_ref::<FigError>()
        .map(FigError::kind);
    assert_eq!((one > 0, kind), (true, Some(ErrorKind::QueryWorkLimit)));
}

#[test]
fn metadata_that_escapes_past_the_read_limit_is_never_published() {
    let mut index = build(&Tree::new(&swaps()), "0:0", bounds(WORK));
    // Each U+0001 is one byte in the name and six once JSON-escaped.
    let name = "\u{1}".repeat(12 * 1024 * 1024);
    index.above.push(("1:1".into(), name));
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    // `V8` also extends `subscriptions`, which only needs to exist here.
    conn.execute_batch(&format!(
        "CREATE TABLE sources (source_id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE,
         added_at_ms INTEGER NOT NULL) STRICT; {}",
        crate::workspace::V2
    ))
    .unwrap();
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, versions, job_id, state, created_ms)
         VALUES ('s', 'm', '0:0', 'v', 'j', 'building', 0)",
        [],
    )
    .unwrap();
    let e = store::save(&mut conn, 1, &index, &|_| Ok(()), &mut |_, _, _| Ok(())).unwrap_err();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (e.downcast_ref::<FigError>().map(FigError::kind), rows),
        (Some(ErrorKind::ParseBudget), 0)
    );
}

#[test]
fn index_metadata_serializes_as_before_within_its_cap_and_never_holds_more() {
    let index = build(&Tree::new(&swaps()), "0:0", bounds(WORK));
    let info = store::info_json(&index, usize::MAX).unwrap();
    let pages: Vec<Json> = index
        .pages
        .iter()
        .map(|p| json!([p.id, p.name, p.internal]))
        .collect();
    let before = json!({"pages": pages, "roots": index.roots, "above": index.above,
        "lens": index.lens, "stop": index.stop, "hiddenContent": index.hidden_content,
        "textBytes": index.text_bytes, "generation": index.generation,
        "unverifiedLinks": index.unverified_links, "unreadFields": index.unread_fields});
    assert_eq!(info, before.to_string());
    assert!(store::info_json(&index, info.len()).is_ok());
    // The write that would pass the cap is refused whole: nothing past it is ever held.
    let (head, tail) = info.as_bytes().split_at(info.len() / 2);
    let mut w = store::Capped::new(info.len() - 1);
    let wrote = [head, tail].map(|part| std::io::Write::write_all(&mut w, part).is_ok());
    assert_eq!((wrote, w.out.len()), ([true, false], head.len()));
    let kind = store::info_json(&index, info.len() - 1).unwrap_err();
    assert_eq!(
        kind.downcast_ref::<FigError>().map(FigError::kind),
        Some(ErrorKind::ParseBudget)
    );
}

#[test]
fn edges_and_definitions_survive_a_stored_generation() {
    let message = swaps();
    let tree = Tree::new(&message);
    let index = build(&tree, "0:0", bounds(WORK));
    let conn = stored(&index);
    let mut spent = 0;
    let mut loaded = store::load(&conn, 1, &[], &mut spent).unwrap();
    let all: Vec<u32> = (0..index.records.len() as u32).collect();
    store::add(&conn, 1, &mut loaded, &all, &mut spent).unwrap();
    let relations = |i: &Index| -> Vec<(String, Option<u32>, Vec<usages::Edge>)> {
        i.records
            .iter()
            .map(|r| (r.id.clone(), r.parent, r.edges.clone()))
            .collect()
    };
    assert_eq!(relations(&loaded), relations(&index));
    assert!(
        index
            .records
            .iter()
            .any(|r| r.edges.iter().any(|e| e.state == "missing"))
    );
}

/// A workspace schema in memory holding `index` as generation 1.
pub(super) fn stored(index: &Index) -> rusqlite::Connection {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE sources (source_id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE,
         added_at_ms INTEGER NOT NULL) STRICT; CREATE TABLE subscriptions (id TEXT) STRICT;
         {} {} {}",
        crate::workspace::V2,
        crate::workspace::V3,
        crate::workspace::V8
    ))
    .unwrap();
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, versions, job_id, state, created_ms)
         VALUES ('s', 'm', '0:0', 'v', 'j', 'building', 0)",
        [],
    )
    .unwrap();
    store::save(&mut conn, 1, index, &|_| Ok(()), &mut |_, _, _| Ok(())).unwrap();
    conn
}

#[test]
fn a_text_only_build_ranks_and_pages_like_a_full_one() {
    let message = message();
    let tree = Tree::new(&message);
    let text = build_as(&tree, "0:0", bounds(WORK), false);
    let full = build(&tree, "0:0", bounds(WORK));
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let mut req = request("label");
    req.limit = 1;
    let page = |index: &Index, req: &Request| {
        reply(index, &req.validate().unwrap(), req, source.clone()).unwrap()
    };
    let (a, b) = (page(&text, &req), page(&full, &req));
    assert_eq!(
        (&a["results"], &a["coverage"]["complete"]),
        (&b["results"], &b["coverage"]["complete"])
    );
    assert_eq!(
        (
            &a["coverage"]["layers"]["relations"]["kinds"]["uses_style"],
            &b["coverage"]["layers"]["relations"]["kinds"]["uses_style"]
        ),
        (&json!("not_built"), &json!("ready"))
    );
    // A cursor from the text-only build pages the full one that replaces it, and vice versa.
    req.cursor = Some(
        a["nextCursor"]
            .as_str()
            .expect("several matches")
            .to_owned(),
    );
    assert_eq!(page(&full, &req)["results"], page(&text, &req)["results"]);
}

#[test]
fn a_full_build_that_stops_earlier_never_takes_text_search_from_the_text_build() {
    // Many plain gradients cost a full build work a text-only build never spends.
    let plain = K::Array(
        (0..400)
            .map(|_| {
                obj(vec![
                    ("type", K::Enum("PaintType", "GRADIENT_LINEAR")),
                    ("stopsVar", K::Array(vec![obj(vec![]), obj(vec![])])),
                ])
            })
            .collect(),
    );
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Page", vec![]),
    ];
    for i in 0..40 {
        let name = format!("Label {i}");
        nodes.push(node(
            100 + i,
            Some(1),
            "RECTANGLE",
            &name,
            vec![("fillPaints", plain.clone())],
        ));
    }
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    let tree = Tree::new(&message);
    // The least work that lets the text-only build finish; the full one stops under it.
    let work = (1..2000)
        .map(|k| k * 50)
        .find(|w| build_as(&tree, "0:0", bounds(*w), false).stop.is_none())
        .unwrap();
    let text = Arc::new(build_as(&tree, "0:0", bounds(work), false));
    let full = Arc::new(build_as(&tree, "0:0", bounds(work), true));
    assert_eq!(full.stop, Some("work_limit"));
    let mut cached = Cached {
        path: PathBuf::new(),
        stamp: (SystemTime::UNIX_EPOCH, 0),
        scope: "0:0".into(),
        snapshot: "sha256:test".into(),
        hashed: SystemTime::UNIX_EPOCH,
        text: None,
        full: None,
    };
    cached.keep(text.clone());
    let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
    let mut req = request("label");
    req.limit = 5;
    let first = search_index(&cached.pick(false).unwrap(), &req, source.clone()).unwrap();
    cached.keep(full.clone());
    // Usages reads its own partial build and says so; search keeps the text build and cursor.
    let partial = usages_index(
        &cached.pick(true).unwrap(),
        &usages::Request {
            target: "0:1".into(),
            resource: false,
            scope: "0:0".into(),
            relations: vec!["contains".into()],
            outgoing: false,
            include_hidden: false,
            limit: 5,
            cursor: None,
            snapshot: None,
        },
        source.clone(),
    )
    .unwrap();
    assert_eq!(partial["coverage"]["complete"], false);
    let after = cached.pick(false).unwrap();
    assert!(Arc::ptr_eq(&after, &text));
    assert_eq!(search_index(&after, &req, source.clone()).unwrap(), first);
    req.cursor = Some(first["nextCursor"].as_str().unwrap().to_owned());
    assert!(search_index(&after, &req, source.clone()).is_ok());
    // A later text build of the same revision keeps the full one for usages.
    cached.keep(Arc::new(build_as(&tree, "0:0", bounds(work), false)));
    assert!(Arc::ptr_eq(&cached.pick(true).unwrap(), &full));
    // With room for both, one full build serves both and no second copy is held.
    let whole = Arc::new(build_as(&tree, "0:0", bounds(WORK), true));
    let mut roomy = Cached {
        text: None,
        full: None,
        ..cached
    };
    roomy.keep(Arc::new(build_as(&tree, "0:0", bounds(WORK), false)));
    roomy.keep(whole.clone());
    assert!(Arc::ptr_eq(&roomy.pick(false).unwrap(), &whole));
}

#[test]
fn definitions_are_described_within_the_callers_bounds() {
    // The only instance in scope points at a definition outside it whose name alone passes the
    // caller's byte budget; describing it must stop the build, not draw on a fresh allowance.
    let long = "n".repeat(1 << 20);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(0), "CANVAS", "Library", vec![]),
            node(20, Some(2), "SYMBOL", &long, vec![]),
            node(10, Some(1), "FRAME", "Home", vec![]),
            instance(11, 10, "Use", 20, vec![]),
        ]),
    )]);
    let tree = Tree::new(&message);
    let small = Bounds {
        bytes: 600 * 1024,
        ..bounds(WORK)
    };
    let full = build_as(&tree, "0:10", small, true);
    assert_eq!(
        (full.stop, full.definitions.contains_key("0:20")),
        (Some("byte_limit"), false)
    );
    let text = build_as(&tree, "0:10", small, false);
    assert_eq!(text.stop, None);
}

#[test]
fn search_coverage_marks_the_kinds_extraction_left_unverified_partial() {
    // 17 actions in one interaction: the prototype reader keeps 16 and counts the link.
    let action = || {
        obj(vec![
            ("transitionNodeID", guid(2)),
            ("navigationType", K::Enum("NavigationType", "NAVIGATE")),
        ])
    };
    let click = obj(vec![
        (
            "event",
            obj(vec![(
                "interactionType",
                K::Enum("InteractionType", "ON_CLICK"),
            )]),
        ),
        ("actions", K::Array((0..17).map(|_| action()).collect())),
    ]);
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Target", vec![]),
            node(
                5,
                Some(1),
                "FRAME",
                "Menu",
                vec![("prototypeInteractions", K::Array(vec![click]))],
            ),
        ]),
    )]);
    let index = build_as(&Tree::new(&message), "0:0", bounds(WORK), true);
    let layer = built(&index, relations("ready"));
    assert_eq!(
        (
            &layer["kinds"]["prototype_target"],
            &layer["kinds"]["contains"]
        ),
        (&json!("partial"), &json!("ready"))
    );
}

#[test]
fn loading_records_charges_their_edges_to_the_query_bytes() {
    let message = message();
    let index = build(&Tree::new(&message), "0:0", bounds(WORK));
    let conn = stored(&index);
    let mut partial = store::load(&conn, 1, &[], &mut 0).unwrap();
    let all: Vec<u32> = (0..index.records.len() as u32).collect();
    let text: usize = index
        .records
        .iter()
        .map(|r| {
            r.name.len()
                + r.text.as_ref().map_or(0, String::len)
                + r.norm[0].len()
                + r.norm[1].len()
        })
        .sum();
    assert!(index.records.iter().any(|r| !r.edges.is_empty()));
    // Room for every record's text, none for its edges.
    let mut spent = BYTES - text;
    let err = store::add(&conn, 1, &mut partial, &all, &mut spent).unwrap_err();
    assert_eq!(
        err.downcast_ref::<FigError>().map(FigError::kind),
        Some(ErrorKind::QueryWorkLimit)
    );
}
