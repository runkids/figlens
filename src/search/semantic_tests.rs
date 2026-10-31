//! Model inputs beyond name and text: effective component properties and observed scope.

use super::*;
use crate::search::tests::{bounds, guid, instance, node, obj, path};
use kiwi_schema::Value as K;

fn def(id: u32, name: &str, kind: &'static str, initial: K<'static>) -> K<'static> {
    obj(vec![
        ("id", guid(id)),
        ("name", K::String(name.into())),
        ("type", K::Enum("ComponentPropType", kind)),
        ("initialValue", initial),
    ])
}

fn text_value(s: &str) -> K<'static> {
    obj(vec![(
        "textValue",
        obj(vec![("characters", K::String(s.into()))]),
    )])
}

fn bool_value(b: bool) -> K<'static> {
    obj(vec![("boolValue", K::Bool(b))])
}

fn swap_value(id: u32) -> K<'static> {
    obj(vec![("symbolIdValue", obj(vec![("guid", guid(id))]))])
}

fn assign(def: u32, value: K<'static>) -> K<'static> {
    obj(vec![("defID", guid(def)), ("value", value)])
}

fn assignments(list: Vec<K<'static>>) -> (&'static str, K<'static>) {
    ("componentPropAssignments", K::Array(list))
}

/// A variant of the `Button` set: its state and text, boolean and two instance-swap properties.
fn variant(id: u32, state: &str) -> K<'static> {
    node(
        id,
        Some(40),
        "SYMBOL",
        &format!("State={state}"),
        vec![
            (
                "variantPropSpecs",
                K::Array(vec![obj(vec![
                    ("propDefId", guid(80)),
                    ("value", K::String(state.into())),
                ])]),
            ),
            (
                "componentPropDefs",
                K::Array(vec![
                    def(90, "Label", "TEXT", text_value("Button")),
                    def(91, "Show icon", "BOOL", bool_value(true)),
                    def(92, "Icon", "INSTANCE_SWAP", swap_value(20)),
                    def(93, "Badge", "INSTANCE_SWAP", swap_value(20)),
                ]),
            ),
        ],
    )
}

/// `Pay button` overrides every property of a Hover button, one swap to a component outside the
/// file; `Promo card` overrides the label of the button its card nests, whose own assignments
/// also set `Show icon`.
fn message() -> K<'static> {
    swapped("Arrow icon", 99)
}

/// `message` with `Pay button`'s `Icon` swapped to `arrow` and its `Badge` to node `badge`.
fn swapped(arrow: &str, badge: u32) -> K<'static> {
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Checkout page", vec![]),
            node(20, Some(1), "SYMBOL", "Star icon", vec![]),
            node(25, Some(1), "SYMBOL", arrow, vec![]),
            node(
                40,
                Some(1),
                "FRAME",
                "Button",
                vec![
                    ("isStateGroup", K::Bool(true)),
                    (
                        "componentPropDefs",
                        K::Array(vec![def(80, "State", "VARIANT", text_value("Default"))]),
                    ),
                ],
            ),
            variant(41, "Default"),
            variant(42, "Hover"),
            node(30, Some(1), "SYMBOL", "Card", vec![]),
            instance(
                31,
                30,
                "Inner button",
                41,
                vec![assignments(vec![
                    assign(90, text_value("Inner")),
                    assign(91, bool_value(false)),
                ])],
            ),
            node(2, Some(1), "FRAME", "Checkout", vec![]),
            node(7, Some(2), "FRAME", "Footer", vec![]),
            instance(
                3,
                7,
                "Pay button",
                42,
                vec![assignments(vec![
                    assign(90, text_value("Complete purchase")),
                    assign(91, bool_value(false)),
                    assign(92, swap_value(25)),
                    assign(93, swap_value(badge)),
                ])],
            ),
            instance(
                5,
                2,
                "Promo card",
                30,
                vec![(
                    "symbolOverrides",
                    K::Array(vec![obj(vec![
                        path(&[31]),
                        assignments(vec![assign(90, text_value("Outer"))]),
                    ])]),
                )],
            ),
        ]),
    )])
}

fn stored_build(tree: &Tree) -> Index {
    build_with(tree, "0:0", bounds(WORK), true, Library::Stored, None)
}

fn props(index: &Index, id: &str) -> Vec<(String, Option<String>)> {
    let mut p = index
        .records
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .props
        .clone()
        .unwrap();
    p.sort();
    p
}

fn known(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(n, v)| (n.to_string(), v.map(str::to_owned)))
        .collect()
}

fn layer_of(index: &Index) -> Layer {
    let no = AtomicBool::new(false);
    let mut b = bounds(LAYER_WORK);
    b.bytes = LAYER_BYTES;
    Layer::new(index, json!(1), DIM, &mut b, &no).unwrap()
}

/// The `(field, chunk, text)` units of record `id`, in order.
fn units<'l>(index: &Index, layer: &'l Layer, id: &str) -> Vec<(&'static str, u32, &'l str)> {
    layer
        .units
        .iter()
        .filter(|u| index.record(u.0).id == id)
        .map(|u| (field_name(u.1), u.2, layer.texts[u.3].as_str()))
        .collect()
}

#[test]
fn overridden_instance_properties_are_model_inputs_with_exact_provenance() {
    let message = message();
    let tree = Tree::new(&message);
    let index = stored_build(&tree);
    // Text, boolean, swap (by the swapped component's name) and variant, as the instance sets
    // them; a swap to a component outside the file is unknown, never invented.
    assert_eq!(
        props(&index, "0:3"),
        known(&[
            ("Badge", None),
            ("Icon", Some("Arrow icon")),
            ("Label", Some("Complete purchase")),
            ("Show icon", Some("false")),
            ("State", Some("Hover")),
        ])
    );
    // The nested button takes the card instance's override; `Show icon`, which REST resolves to
    // the default while expansion keeps the nested instance's own `false`, is unknown.
    assert_eq!(
        props(&index, "I0:5;0:31"),
        known(&[
            ("Badge", Some("Star icon")),
            ("Icon", Some("Star icon")),
            ("Label", Some("Outer")),
            ("Show icon", None),
            ("State", Some("Default")),
        ])
    );
    let layer = layer_of(&index);
    // One chunk, in REST key order, without the unknown value.
    assert_eq!(
        units(&index, &layer, "0:3")
            .into_iter()
            .filter(|u| u.0 == "properties")
            .collect::<Vec<_>>(),
        [(
            "properties",
            0,
            "Icon: Arrow icon; Label: Complete purchase; Show icon: false; State: Hover"
        )]
    );
    assert_eq!(layer.unknown, 2);
    assert_eq!(layer_coverage(&layer, "p", [0, 0])["unknownProperties"], 2);
}

#[test]
fn swap_target_names_are_charged_before_they_are_copied() {
    // Both swaps name `Arrow icon`, which lies outside the definitions and assignments.
    let pay = |arrow: &str, bytes| {
        let message = swapped(arrow, 25);
        let mut b = bounds(WORK);
        b.bytes = bytes;
        let got = properties(&Tree::new(&message), "0:3", None, &mut Defs::new(), &mut b);
        (got, bytes - b.bytes)
    };
    let (_, spent) = pay("Arrow icon", BYTES);
    let long = "Arrow icon ".repeat(10_000);
    assert!(pay(&long, spent + long.len()).0.is_none());
    let (all, _) = pay(&long, spent + 2 * long.len());
    assert_eq!(
        all.unwrap()
            .into_iter()
            .filter(|p| p.1.as_ref() == Some(&long))
            .count(),
        2
    );
}

/// A set whose definitions are `parent`, a variant of it whose own definitions are `refs`
/// references to parent definition 90 and whose specs are `specs` references to 80, and an
/// instance of the variant with `assigned`.
fn shared_parent(
    parent: Vec<K<'static>>,
    refs: u32,
    specs: u32,
    assigned: Vec<K<'static>>,
) -> K<'static> {
    let own = (0..refs)
        .map(|i| obj(vec![("id", guid(1000 + i)), ("parentPropDefId", guid(90))]))
        .collect();
    let specs = (0..specs)
        .map(|_| {
            obj(vec![
                ("propDefId", guid(80)),
                ("value", K::String("On".into())),
            ])
        })
        .collect();
    obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(
                40,
                Some(1),
                "FRAME",
                "Set",
                vec![
                    ("isStateGroup", K::Bool(true)),
                    ("componentPropDefs", K::Array(parent)),
                ],
            ),
            node(
                41,
                Some(40),
                "SYMBOL",
                "State=On",
                vec![
                    ("componentPropDefs", K::Array(own)),
                    ("variantPropSpecs", K::Array(specs)),
                ],
            ),
            instance(3, 1, "Instance", 41, vec![assignments(assigned)]),
        ]),
    )])
}

#[test]
fn property_resolution_stops_before_constructing_guid_lookup_keys() {
    let cases = [
        (
            vec![def(90, "Unused", "TEXT", text_value("x"))],
            0,
            0,
            vec![],
        ),
        (vec![], 0, 0, vec![assign(90, text_value("x"))]),
        (vec![], 1, 0, vec![]),
        (vec![], 0, 1, vec![]),
    ];
    for (i, (parent, refs, specs, assigned)) in cases.into_iter().enumerate() {
        let message = shared_parent(parent, refs, specs, assigned);
        let tree = Tree::new(&message);
        let read = crate::rest::PropDefs::read(&tree, "0:41").unwrap();
        let assignments =
            crate::to_json(tree.nodes["0:3"].get("componentPropAssignments").unwrap());
        // None of these produces a property, but each still copies a GUID for a lookup.
        assert!(
            read.resolve_with(&assignments, |_, _| false).is_none(),
            "case {i}"
        );
        assert!(read.resolve(&assignments).is_empty());
    }
}

#[test]
fn definitions_sharing_one_large_parent_are_charged_per_copy() {
    let big = "x".repeat(64 * 1024);
    let small = || text_value("small");
    let preferred = (0..2000)
        .map(|i| {
            obj(vec![
                ("type", K::Enum("Kind", "COMPONENT")),
                ("key", K::String(format!("{i:0>32}"))),
            ])
        })
        .collect();
    let swap = obj(vec![
        ("id", guid(90)),
        ("name", K::String("Icon".into())),
        ("type", K::Enum("ComponentPropType", "INSTANCE_SWAP")),
        ("initialValue", swap_value(1)),
        (
            "preferredValues",
            obj(vec![("instanceSwapValues", K::Array(preferred))]),
        ),
    ]);
    let cases = [
        // A large default, copied once per reference.
        (
            vec![def(90, "Label", "TEXT", text_value(&big))],
            256,
            0,
            vec![],
        ),
        // A small default whose one large assignment through the parent applies to every reference.
        (
            vec![def(90, "Label", "TEXT", small())],
            256,
            0,
            vec![assign(90, text_value(&big))],
        ),
        // A large variant name, copied once per spec.
        (vec![def(80, &big, "VARIANT", small())], 0, 256, vec![]),
        // Large preferred values, copied once per reference.
        (vec![swap], 256, 0, vec![]),
    ];
    for (i, (parent, refs, specs, assigned)) in cases.into_iter().enumerate() {
        let message = shared_parent(parent, refs, specs, assigned);
        let tree = Tree::new(&message);
        let mut b = bounds(WORK);
        b.bytes = 4 * 1024 * 1024;
        let got = properties(&tree, "0:3", None, &mut Defs::new(), &mut b);
        assert!(got.is_none(), "case {i} copied past its bytes");
        // With room for every copy, the resolution is REST's: one property, however many refs.
        let got = properties(&tree, "0:3", None, &mut Defs::new(), &mut bounds(WORK)).unwrap();
        assert_eq!(got.len(), 1, "case {i}");
    }
}

#[test]
fn properties_survive_a_stored_generation_and_text_builds_never_pay_for_them() {
    let message = message();
    let tree = Tree::new(&message);
    let index = stored_build(&tree);
    let conn = crate::search::tests::stored(&index);
    let (mut spent, never) = (0, AtomicBool::new(false));
    let mut loaded = store::load(&conn, 1, &[], &mut spent).unwrap();
    let all: Vec<u32> = (0..index.records.len() as u32).collect();
    store::add(&conn, 1, &mut loaded, &all, &mut spent).unwrap();
    store::add_properties(&conn, 1, &mut loaded, &mut spent, &never).unwrap();
    let of = |i: &Index| -> Vec<_> { i.records.iter().map(|r| r.props.clone()).collect() };
    assert_eq!(of(&loaded), of(&index));
    // Every record of a full build is indexed, a layer that is no instance as known empty.
    assert!(of(&index).iter().all(Option::is_some));
    assert!(of(&index).iter().flatten().any(|p| !p.is_empty()));
    // A text-only build reads no properties.
    let text = build_as(&tree, "0:0", bounds(WORK), false);
    assert!(text.records.iter().all(|r| r.props.is_none()));
}

#[test]
fn a_layer_over_unindexed_properties_is_never_ready_and_says_so() {
    let message = message();
    let tree = Tree::new(&message);
    let mut index = stored_build(&tree);
    // As a generation stored before schema 11 loads: no property column.
    for r in &mut index.records {
        r.props = None;
    }
    let mut layer = layer_of(&index);
    layer.vectors = layer.texts.iter().map(|t| Some(embed(t))).collect();
    let cov = layer_coverage(&layer, "p", [0, 0]);
    let instances = index
        .records
        .iter()
        .filter(|r| r.kind == "INSTANCE")
        .count();
    assert_eq!(cov["state"], "partial");
    assert_eq!(
        cov["reasons"],
        json!([{"code": NOT_INDEXED, "counts": {"records": instances}}])
    );
    // Known-empty properties (a fresh build of instances with none) are complete.
    for r in &mut index.records {
        r.props = Some(Vec::new());
    }
    let mut known = layer_of(&index);
    known.vectors = known.texts.iter().map(|t| Some(embed(t))).collect();
    assert_eq!(layer_coverage(&known, "p", [0, 0])["state"], "ready");
}

/// A deterministic word-bag embedding: each lowercase word adds one to its hashed dimension.
const DIM: usize = 256;

fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; DIM];
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let h = Sha256::digest(word.to_lowercase().as_bytes());
        v[u16::from_le_bytes([h[0], h[1]]) as usize % DIM] += 1.0;
    }
    let n = v
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt()
        .max(f32::MIN_POSITIVE);
    v.into_iter().map(|x| x / n).collect()
}

fn best(index: &Index, layer: &Layer, query: &str) -> (String, &'static str, u32) {
    let req = Request {
        query: query.into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: true,
        include_internal: true,
        limit: DEFAULT_LIMIT,
        cursor: None,
    };
    let (order, _) = score(index, layer, &embed(query), &req, &AtomicBool::new(false)).unwrap();
    let (rid, _, field, chunk) = order[0];
    (index.record(rid).id.clone(), field_name(field), chunk)
}

#[test]
fn a_query_matching_only_a_property_value_or_the_scope_finds_that_layer() {
    let message = message();
    let tree = Tree::new(&message);
    let index = stored_build(&tree);
    let mut layer = layer_of(&index);
    layer.vectors = layer.texts.iter().map(|t| Some(embed(t))).collect();
    // Neither the instance's name nor any text holds these words.
    assert_eq!(
        best(&index, &layer, "Complete purchase"),
        ("0:3".into(), "properties", 0)
    );
    // Only its observed scope (page, top-level frame and container) holds both words.
    assert_eq!(
        best(&index, &layer, "Checkout Footer"),
        ("0:3".into(), "scope", 0)
    );
    let scope = units(&index, &layer, "0:3")
        .into_iter()
        .find(|u| u.0 == "scope")
        .unwrap()
        .2;
    assert_eq!(scope, "Checkout page / Checkout / Footer");
}

#[test]
fn long_property_and_scope_inputs_are_chunked_to_their_end() {
    let message = message();
    let tree = Tree::new(&message);
    let mut index = stored_build(&tree);
    let long = format!("{} tail", "word ".repeat(model::CHUNK_CHARS));
    let at = |i: &Index, id: &str| i.records.iter().position(|r| r.id == id).unwrap();
    let (pay, footer) = (at(&index, "0:3"), at(&index, "0:7"));
    index.records[pay].props = Some(vec![("Label".into(), Some(long.clone()))]);
    index.records[footer].name = long.clone();
    let layer = layer_of(&index);
    let joined = |field: &str| {
        let parts: Vec<_> = units(&index, &layer, "0:3")
            .into_iter()
            .filter(|u| u.0 == field)
            .collect();
        assert!(parts.len() > 1, "{field} is one chunk");
        assert!(parts.iter().enumerate().all(|(i, u)| u.1 == i as u32));
        parts.iter().map(|u| u.2).collect::<String>()
    };
    assert_eq!(joined("properties"), format!("Label: {}", long.trim()));
    assert_eq!(
        joined("scope"),
        format!("Checkout page / Checkout / {long}").trim()
    );
}

#[test]
fn properties_leave_text_search_unchanged() {
    let fixture = std::fs::read("examples/first-success/design.fig").unwrap();
    let mut doc = Document::from_bytes(&fixture, "fixture").unwrap();
    let fixture = doc.take_message().unwrap();
    let message = message();
    for (tree, query) in [
        (Tree::new(&fixture), "ready"),
        (Tree::new(&message), "button"),
    ] {
        let req = Request {
            query: query.into(),
            scope: "0:0".into(),
            types: Vec::new(),
            include_hidden: true,
            include_internal: true,
            limit: DEFAULT_LIMIT,
            cursor: None,
        };
        let q = req.validate().unwrap();
        let source = json!({"sourceId": "local:test", "snapshotId": "sha256:test"});
        let with = stored_build(&tree);
        let without = build_with(&tree, "0:0", bounds(WORK), true, Library::Skip, None);
        assert_eq!(
            reply(&with, &q, &req, source.clone()).unwrap(),
            reply(&without, &q, &req, source).unwrap()
        );
    }
}

#[test]
fn a_stopped_property_build_never_masks_plain_text_even_when_no_record_fit() {
    use crate::search::Cached;
    use std::sync::Arc;
    let message = message();
    let tree = Tree::new(&message);
    let build = |work, library| build_with(&tree, "0:3", bounds(work), false, library, None);
    let work = (1..4000)
        .find(|&w| {
            let plain = build(w, Library::Skip);
            let model = build(w, Library::Stored);
            plain.stop.is_none() && model.stop == Some("work_limit") && model.records.is_empty()
        })
        .expect("property definitions can stop before a record that fits plain text");
    let plain = Arc::new(build(work, Library::Skip));
    let model = Arc::new(build(work, Library::Stored));
    assert!(plain.records.iter().any(|r| r.id == "0:3"));
    let mut cached = Cached {
        path: Default::default(),
        stamp: (SystemTime::UNIX_EPOCH, 0),
        scope: "0:3".into(),
        snapshot: "s".into(),
        hashed: SystemTime::UNIX_EPOCH,
        text: None,
        text_inputs: false,
        full: None,
    };
    cached.keep(model.clone(), true);
    assert!(
        cached.pick(false, false).is_none(),
        "plain text must rebuild its own budget"
    );
    assert!(Arc::ptr_eq(&cached.pick(false, true).unwrap(), &model));
    cached.keep(plain.clone(), false);
    assert!(Arc::ptr_eq(&cached.pick(false, false).unwrap(), &plain));
    assert!(
        cached.pick(false, true).is_none(),
        "a plain index has no model inputs"
    );
}
