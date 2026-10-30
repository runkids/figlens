use super::super::tests::{bounds, guid, instance, node, obj, stored};
use super::*;
use crate::Tree;
use crate::search::{Index, WORK, build, build_as};
use kiwi_schema::Value as K;
use std::collections::HashMap;

/// Two same-name Buttons, a Card holding a Button instance, and screens of instances. `extra`
/// adds layers to one generation only.
fn index(extra: Vec<K<'static>>) -> Index {
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some(0), "CANVAS", "Components", vec![]),
        node(2, Some(0), "CANVAS", "Screens", vec![]),
        node(20, Some(1), "SYMBOL", "Button", vec![]),
        node(22, Some(20), "TEXT", "Label", vec![]),
        node(21, Some(1), "SYMBOL", "Button", vec![]),
        node(30, Some(1), "SYMBOL", "Card", vec![]),
        instance(31, 30, "Action", 20, vec![]),
        node(32, Some(30), "TEXT", "Title", vec![]),
        node(33, Some(30), "TEXT", "Body", vec![]),
        node(60, Some(1), "SYMBOL", "Panel", vec![]),
        instance(40, 60, "Card", 30, vec![]),
        instance(61, 2, "Panel", 60, vec![]),
        instance(41, 2, "Other button", 21, vec![]),
    ];
    nodes.extend(extra);
    index_of(nodes)
}

fn index_of(nodes: Vec<K<'static>>) -> Index {
    let message = obj(vec![("nodeChanges", K::Array(nodes))]);
    build(&Tree::new(&message), "0:0", bounds(WORK))
}

/// Answers by (recipient, seed, facet), `Unknown` when unlisted, and records every probe.
struct Fixed {
    answers: HashMap<(&'static str, &'static str, u8), Verdict>,
    asked: Vec<(String, String, String, u8)>,
}

impl Evidence for Fixed {
    fn compare(&mut self, probe: &Probe, _: &mut Bounds) -> Result<Verdict> {
        let (r, t, s) = (probe.recipient, probe.through, probe.seed);
        self.asked
            .push((r.to_owned(), t.to_owned(), s.to_owned(), probe.facet));
        Ok(*self
            .answers
            .get(&(probe.recipient, probe.seed, probe.facet))
            .unwrap_or(&Verdict::Unknown))
    }
}

/// Answers about facet 1.
fn fixed(answers: &[(&'static str, &'static str, Verdict)]) -> Fixed {
    let answers: Vec<_> = answers.iter().map(|&(r, s, v)| (r, s, 1, v)).collect();
    per_facet(&answers)
}

fn per_facet(answers: &[(&'static str, &'static str, u8, Verdict)]) -> Fixed {
    Fixed {
        answers: answers.iter().map(|&(r, s, f, v)| ((r, s, f), v)).collect(),
        asked: Vec::new(),
    }
}

fn seed(node: &str, change: Change) -> Seed {
    Seed {
        node: node.to_owned(),
        change,
        facets: 1,
    }
}

fn run(old: &Index, new: &Index, seeds: &[Seed], evidence: &mut Fixed, work: usize) -> Impact {
    compute(old, new, seeds, evidence, &mut bounds(work)).unwrap()
}

fn class<'a>(impact: &'a Impact, node: &str) -> Option<&'a Hit> {
    impact.hits.iter().find(|h| h.node == node)
}

fn classes(impact: &Impact, nodes: &[&str]) -> Vec<Option<Class>> {
    nodes
        .iter()
        .map(|n| class(impact, n).map(|h| h.class))
        .collect()
}

const C: Verdict = Verdict::Changed;
const E: Verdict = Verdict::Equal;

#[test]
fn a_changed_nested_layer_reaches_instances_by_exact_id_through_its_component() {
    let i = index(vec![]);
    let mut e = fixed(&[
        ("0:31", "0:22", C),
        ("0:40", "0:22", C),
        ("0:61", "0:22", C),
    ]);
    let out = run(&i, &i, &[seed("0:22", Change::Changed)], &mut e, WORK);
    let effective = Some(Class::Effective);
    assert_eq!(
        classes(&out, &["0:22", "0:31", "0:40", "0:61", "0:41"]),
        [Some(Class::Direct), effective, effective, effective, None]
    );
}

#[test]
fn a_dependency_without_evidence_is_only_possible() {
    let i = index(vec![]);
    let out = run(
        &i,
        &i,
        &[seed("0:22", Change::Changed)],
        &mut fixed(&[]),
        WORK,
    );
    // An unproven layer still passes its possible change on to what depends on it.
    let possible = Some(Class::Possible);
    assert_eq!(classes(&out, &["0:31", "0:61"]), [possible, possible]);
}

#[test]
fn a_dependency_the_diff_reads_as_equal_is_effective_unchanged() {
    let i = index(vec![]);
    let mut e = fixed(&[("0:31", "0:22", E)]);
    let out = run(&i, &i, &[seed("0:22", Change::Changed)], &mut e, WORK);
    assert_eq!(classes(&out, &["0:31"]), [Some(Class::EffectiveUnchanged)]);
}

#[test]
fn a_masked_path_first_never_hides_a_later_changed_path_or_what_lies_beyond_it() {
    let i = index(vec![]);
    let mut e = fixed(&[
        ("0:40", "0:33", E),
        ("0:40", "0:32", C),
        ("0:61", "0:32", C),
    ]);
    let seeds = [seed("0:33", Change::Changed), seed("0:32", Change::Changed)];
    let out = run(&i, &i, &seeds, &mut e, WORK);
    let effective = Some(Class::Effective);
    assert_eq!(classes(&out, &["0:40", "0:61"]), [effective, effective]);
}

#[test]
fn a_dependent_in_one_generation_only_is_possible_with_that_side() {
    let old = index(vec![instance(43, 2, "Old", 20, vec![])]);
    let new = index(vec![instance(42, 2, "New", 20, vec![])]);
    let mut e = fixed(&[("0:42", "0:22", C), ("0:43", "0:22", C)]);
    let out = run(&old, &new, &[seed("0:22", Change::Changed)], &mut e, WORK);
    let sides: Vec<_> = ["0:43", "0:42"]
        .iter()
        .map(|n| class(&out, n).map(|h| (h.class, h.side)))
        .collect();
    assert_eq!(
        sides,
        [Some((Class::Possible, 1)), Some((Class::Possible, 2))]
    );
}

#[test]
fn a_removed_layer_reaches_only_the_old_generations_dependents() {
    let old = index(vec![instance(43, 2, "Old", 20, vec![])]);
    let new = index(vec![instance(42, 2, "New", 20, vec![])]);
    let mut e = fixed(&[("0:43", "0:22", C)]);
    let out = run(&old, &new, &[seed("0:22", Change::Removed)], &mut e, WORK);
    assert_eq!(
        classes(&out, &["0:43", "0:42"]),
        [Some(Class::Effective), None]
    );
}

#[test]
fn an_unknown_diff_row_is_possible_never_direct() {
    let i = index(vec![]);
    let out = run(
        &i,
        &i,
        &[seed("0:22", Change::Unknown)],
        &mut fixed(&[]),
        WORK,
    );
    assert_eq!(
        (classes(&out, &["0:22"]), out.complete),
        (vec![Some(Class::Possible)], false)
    );
}

#[test]
fn a_bound_that_runs_out_leaves_the_impact_incomplete_not_unchanged() {
    let i = index(vec![]);
    let mut e = fixed(&[("0:31", "0:22", E)]);
    let out = run(&i, &i, &[seed("0:22", Change::Changed)], &mut e, 10);
    assert_eq!(
        (out.complete, out.reasons.contains_key("budget_unknown")),
        (false, true),
        "{out:?}"
    );
}

#[test]
fn a_nested_layer_is_asked_about_through_its_component_with_its_seed_and_facets() {
    let i = index(vec![]);
    let mut e = fixed(&[]);
    run(&i, &i, &[seed("0:22", Change::Changed)], &mut e, WORK);
    let probe = ("0:31".to_owned(), "0:20".to_owned(), "0:22".to_owned(), 1u8);
    assert!(e.asked.contains(&probe), "{:?}", e.asked);
}

#[test]
fn an_added_layer_reaches_only_the_new_generations_dependents() {
    let old = index(vec![instance(43, 2, "Old", 20, vec![])]);
    let new = index(vec![instance(42, 2, "New", 20, vec![])]);
    let mut e = fixed(&[("0:42", "0:22", C)]);
    let out = run(&old, &new, &[seed("0:22", Change::Added)], &mut e, WORK);
    assert_eq!(
        classes(&out, &["0:42", "0:43"]),
        [Some(Class::Effective), None]
    );
}

#[test]
fn a_masked_path_after_a_changed_one_leaves_the_layer_changed() {
    let i = index(vec![]);
    let mut e = fixed(&[("0:40", "0:32", C), ("0:40", "0:33", E)]);
    let seeds = [seed("0:32", Change::Changed), seed("0:33", Change::Changed)];
    let out = run(&i, &i, &seeds, &mut e, WORK);
    assert_eq!(classes(&out, &["0:40"]), [Some(Class::Effective)]);
}

#[test]
fn one_changed_facet_never_marks_another_facet_changed() {
    let i = index(vec![]);
    let mut e = per_facet(&[("0:31", "0:22", 1, C), ("0:31", "0:22", 2, E)]);
    let mut text_and_fill = seed("0:22", Change::Changed);
    text_and_fill.facets = 1 | 2;
    let out = run(&i, &i, &[text_and_fill], &mut e, WORK);
    let hit = class(&out, "0:31").unwrap();
    assert_eq!(
        (hit.class, hit.changed, hit.equal),
        (Class::Effective, 1, 2)
    );
}

#[test]
fn a_facet_walked_in_one_generation_is_still_walked_in_the_other() {
    let walked = combos(1, OLD) | combos(2, NEW);
    assert_ne!(walked & combos(1, NEW), combos(1, NEW));
}

#[test]
fn unproven_layers_leave_the_impact_incomplete_with_their_reasons() {
    let old = index(vec![instance(43, 2, "Old", 20, vec![])]);
    let new = index(vec![]);
    let out = run(
        &old,
        &new,
        &[seed("0:22", Change::Changed)],
        &mut fixed(&[]),
        WORK,
    );
    let reasons: Vec<_> = out.reasons.keys().copied().collect();
    assert_eq!(
        (out.complete, reasons),
        (false, vec!["comparison_unknown", "relation_unverified"])
    );
}

#[test]
fn proven_layers_alone_make_the_impact_complete() {
    let i = index(vec![]);
    let proven = [
        "0:31",
        "0:40",
        "0:61",
        "I0:40;0:31",
        "I0:61;0:40",
        "I0:61;0:40;0:31",
    ];
    let answers: Vec<_> = proven.iter().map(|&r| (r, "0:22", C)).collect();
    let mut e = fixed(&answers);
    let out = run(&i, &i, &[seed("0:22", Change::Changed)], &mut e, WORK);
    assert!(out.complete, "{out:?}");
}

#[test]
fn the_hit_cap_stops_before_keeping_more_layers_and_says_so() {
    let mut extra: Vec<_> = (0..HITS as u32 + 10)
        .map(|n| instance(10_000 + n, 2, "Many", 20, vec![]))
        .collect();
    extra.push(node(9, Some(2), "FRAME", "Pad", vec![]));
    let i = index(extra);
    let out = run(
        &i,
        &i,
        &[seed("0:22", Change::Changed)],
        &mut fixed(&[]),
        WORK,
    );
    assert_eq!(
        (
            out.hits.len(),
            out.complete,
            out.reasons.contains_key("hit_limit")
        ),
        (HITS, false, true)
    );
}

#[test]
fn a_changed_component_set_reaches_its_members_and_their_instances() {
    let set = ("isStateGroup", K::Bool(true));
    let i = index(vec![
        node(50, Some(1), "FRAME", "State", vec![set]),
        node(51, Some(50), "SYMBOL", "State=On", vec![]),
        instance(55, 2, "Toggle", 51, vec![]),
    ]);
    let out = run(
        &i,
        &i,
        &[seed("0:50", Change::Changed)],
        &mut fixed(&[]),
        WORK,
    );
    let possible = Some(Class::Possible);
    assert_eq!(classes(&out, &["0:51", "0:55"]), [possible, possible]);
}

fn fill_style(target: u32) -> (&'static str, K<'static>) {
    ("styleIdForFill", obj(vec![("guid", guid(target))]))
}

#[test]
fn a_later_proven_edge_supersedes_an_earlier_unproven_one() {
    // In the old generation only, the instance also uses the changed layer as a fill style.
    let old = index(vec![instance(45, 2, "Styled", 20, vec![fill_style(22)])]);
    let new = index(vec![instance(45, 2, "Styled", 20, vec![])]);
    let mut e = fixed(&[("0:45", "0:22", C)]);
    let out = run(&old, &new, &[seed("0:22", Change::Changed)], &mut e, WORK);
    assert_eq!(classes(&out, &["0:45"]), [Some(Class::Effective)]);
}

#[test]
fn a_hop_beyond_a_one_generation_layer_stays_unproven() {
    let old = index(vec![
        instance(43, 2, "Old", 20, vec![]),
        node(46, Some(2), "FRAME", "Styled by old", vec![fill_style(43)]),
    ]);
    let new = index(vec![]);
    let mut e = fixed(&[("0:46", "0:22", C)]);
    let out = run(&old, &new, &[seed("0:22", Change::Changed)], &mut e, WORK);
    assert_eq!(classes(&out, &["0:46"]), [Some(Class::Possible)]);
}

#[test]
fn a_facet_keeps_only_its_strongest_verdict() {
    let i = index(vec![]);
    let mut e = fixed(&[("0:40", "0:33", E), ("0:40", "0:32", C)]);
    let seeds = [seed("0:33", Change::Changed), seed("0:32", Change::Changed)];
    let out = run(&i, &i, &seeds, &mut e, WORK);
    let hit = class(&out, "0:40").unwrap();
    assert_eq!((hit.changed, hit.equal), (1, 0));
}

#[test]
fn a_node_that_is_also_a_confirmed_row_stays_direct() {
    let i = index(vec![]);
    let seeds = [seed("0:22", Change::Unknown), seed("0:22", Change::Changed)];
    let out = run(&i, &i, &seeds, &mut fixed(&[]), WORK);
    assert_eq!(classes(&out, &["0:22"]), [Some(Class::Direct)]);
}

#[test]
fn an_unread_facet_is_never_asked_about() {
    let i = index(vec![]);
    let mut e = per_facet(&[("0:31", "0:22", UNREAD, E)]);
    let mut unread = seed("0:22", Change::Changed);
    unread.facets = UNREAD;
    let out = run(&i, &i, &[unread], &mut e, WORK);
    assert_eq!(classes(&out, &["0:31"]), [Some(Class::Possible)]);
}

#[test]
fn an_index_without_relations_is_not_walked() {
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![node(0, None, "DOCUMENT", "Document", vec![])]),
    )]);
    let text = build_as(&Tree::new(&message), "0:0", bounds(WORK), false);
    let out = run(
        &text,
        &text,
        &[seed("0:0", Change::Changed)],
        &mut fixed(&[]),
        WORK,
    );
    assert_eq!(
        (
            out.complete,
            out.reasons.keys().copied().collect::<Vec<_>>()
        ),
        (false, vec!["relations_unavailable"])
    );
}

/// `index` saved to and fully reloaded from a workspace database, as the diff reads it.
fn reloaded(index: &Index) -> Index {
    let conn = stored(index);
    let mut spent = 0;
    let mut loaded = crate::search::store::load(&conn, 1, &[], &mut spent).unwrap();
    let all: Vec<u32> = (0..loaded.total as u32).collect();
    crate::search::store::add(&conn, 1, &mut loaded, &all, &mut spent).unwrap();
    loaded
}

#[test]
fn a_reloaded_index_reaches_the_same_layers() {
    let i = index(vec![]);
    let r = reloaded(&i);
    let ids = |out: Impact| out.hits.into_iter().map(|h| h.node).collect::<Vec<_>>();
    let seeds = [seed("0:22", Change::Changed)];
    let live = run(&i, &i, &seeds, &mut fixed(&[]), WORK);
    assert_eq!(ids(run(&r, &r, &seeds, &mut fixed(&[]), WORK)), ids(live));
}

#[test]
fn a_reloaded_index_whose_reverse_map_does_not_fit_is_not_built() {
    let r = reloaded(&index(vec![]));
    let out = run(&r, &r, &[seed("0:22", Change::Changed)], &mut fixed(&[]), 3);
    assert_eq!(
        (
            out.hits.len(),
            out.reasons.get("budget_unknown"),
            r.reverse.get().is_none()
        ),
        (0, Some(&1), true)
    );
}
