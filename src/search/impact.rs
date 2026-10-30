//! Which layers a set of changed nodes reaches, over the indexes of both compared generations.
//! Pure and bounded: it walks the relations of two indexes and asks the diff, through
//! `Evidence`, whether a reached layer's effective value really differs in one facet. Nothing
//! here compares fields, so a dependency alone never proves a change and an override alone
//! never proves none.
// The host owner wires it into the tracked publication; until then only its tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use super::Index;
use super::usages::{pointing_at, rid_in};
use crate::expand::Bounds;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Relations through which a target's change can reach the layer that points at it.
const DEPENDS: [&str; 5] = [
    "binds_variable",
    "instance_of",
    "uses_image",
    "uses_style",
    "variant_of",
];
/// Dependency hops from a changed node.
pub(crate) const DEPTH: usize = 16;
/// Layers one computation reports.
pub(crate) const HITS: usize = 2_000;
/// Steps waiting to be walked.
pub(crate) const QUEUE: usize = 20_000;
/// Estimated bytes of one kept id beyond its text: the string, map entry or queued step.
const ENTRY: usize = 64;
const OLD: u8 = 1;
const NEW: u8 = 2;
/// The step of a row whose facets the diff did not name (an added or removed layer): never
/// asked about, so what it reaches stays possible.
const PRESENCE: u8 = 0;

/// How a diff row changed its node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Added,
    Removed,
    Changed,
    /// The diff could not tell (an unread field, a retyped node): never a confirmed change.
    Unknown,
}

/// One changed node from the diff, by exact id, with the facets (diff bits) it changed.
#[derive(Clone, Debug)]
pub(crate) struct Seed {
    pub node: String,
    pub change: Change,
    pub facets: u8,
}

/// The diff's answer for one reached layer and one facet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The facet of the layer's effective value differs between the generations.
    Changed,
    /// Every field of the facet was read on both sides and is equal.
    Equal,
    Unknown,
}

/// One question to the diff: does `recipient`'s effective value differ in `facet` (one diff
/// bit)? `through` (the id its edge points at) and `seed` explain the path; the verdict is
/// about the recipient's own record.
pub(crate) struct Probe<'a> {
    pub recipient: &'a str,
    pub through: &'a str,
    pub seed: &'a str,
    pub facet: u8,
}

/// The diff side of the comparison; it charges its own work to `bounds` before doing it.
pub(crate) trait Evidence {
    fn compare(&mut self, probe: &Probe, bounds: &mut Bounds) -> Result<Verdict>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    /// A diff row itself.
    Direct,
    /// A proven dependency whose effective value the diff found changed.
    Effective,
    /// A proven dependency whose effective value the diff read as equal on both sides.
    EffectiveUnchanged,
    /// Reached, but neither changed nor unchanged is proven.
    Possible,
}

/// One reached layer. `side` holds `OLD` and `NEW` bits for the generations it was reached in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub node: String,
    pub class: Class,
    pub side: u8,
    /// Facet bits per verdict, merged over every path: changed wins, then unknown, then equal.
    pub changed: u8,
    pub unknown: u8,
    pub equal: u8,
    /// The relation and target that first reached it with its current class.
    pub via: Option<(&'static str, String)>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Impact {
    pub hits: Vec<Hit>,
    /// True only when every reached layer is proven and nothing was left unwalked.
    pub complete: bool,
    /// Why it is not complete, with counts: `seed_unknown`, `relation_unverified` (an edge
    /// that does not resolve, or not in every generation of the change), `comparison_unknown`
    /// (the diff could not prove either way), `depth_limit`, and for a walk that stopped,
    /// `budget_unknown`, `hit_limit` or `queue_limit` with the seeds and steps still pending:
    /// a known pending count, not every omitted layer.
    pub reasons: BTreeMap<&'static str, u64>,
}

/// One traversal step: the layers that depend on `target` may change in `facet` with `seed`.
struct Step {
    target: String,
    seed: String,
    facet: u8,
    side: u8,
    depth: usize,
}

struct Run<'a> {
    graphs: [&'a Index; 2],
    evidence: &'a mut dyn Evidence,
    bounds: &'a mut Bounds,
    hits: BTreeMap<String, Hit>,
    /// Facet-and-side combinations already queued, per target and seed.
    walked: BTreeMap<String, BTreeMap<String, u32>>,
    queue: VecDeque<Step>,
    reasons: BTreeMap<&'static str, u64>,
    /// What stopped the walk, when something did.
    stopped: Option<&'static str>,
}

fn cost(id: &str) -> usize {
    id.len() + ENTRY
}

/// The `walked` bits of one facet (`PRESENCE` included) in each generation of `side`.
fn combos(facet: u8, side: u8) -> u32 {
    let at = match facet {
        PRESENCE => 8,
        f => f.trailing_zeros(),
    };
    [OLD, NEW]
        .into_iter()
        .filter(|b| side & b != 0)
        .map(|b| 1 << (at * 2 + u32::from(b == NEW)))
        .fold(0, |a, b| a | b)
}

/// One step per changed facet, so a verdict never covers a facet it was not asked about.
fn bits(facets: u8) -> Vec<u8> {
    match facets {
        0 => vec![PRESENCE],
        f => (0..8).map(|i| 1 << i).filter(|b| f & b != 0).collect(),
    }
}

/// The layers `seeds` reach through `old` and `new`, classified by `evidence`, within `bounds`.
/// Both indexes must be relations builds, whose reverse maps their build prepaid. A bound or
/// cap that runs out stops the walk and leaves it incomplete; it is never an error.
pub(crate) fn compute(
    old: &Index,
    new: &Index,
    seeds: &[Seed],
    evidence: &mut dyn Evidence,
    bounds: &mut Bounds,
) -> Result<Impact> {
    let mut run = Run {
        graphs: [old, new],
        evidence,
        bounds,
        hits: BTreeMap::new(),
        walked: BTreeMap::new(),
        queue: VecDeque::new(),
        reasons: BTreeMap::new(),
        stopped: None,
    };
    let mut left = seeds.len();
    for seed in seeds {
        let side = match seed.change {
            Change::Added => NEW,
            Change::Removed => OLD,
            Change::Changed | Change::Unknown => OLD | NEW,
        };
        let class = if seed.change == Change::Unknown {
            run.reason("seed_unknown");
            Class::Possible
        } else {
            Class::Direct
        };
        if !run.hit(&seed.node, class, seed.facets, side, None) {
            break;
        }
        if !bits(seed.facets)
            .into_iter()
            .all(|facet| run.push(&seed.node, &seed.node, facet, side, 0))
        {
            break;
        }
        left -= 1;
    }
    while run.stopped.is_none()
        && let Some(step) = run.queue.pop_front()
    {
        if !run.expand(&step)? {
            run.queue.push_front(step);
        }
    }
    let mut reasons = run.reasons;
    if let Some(why) = run.stopped {
        *reasons.entry(why).or_default() += (left + run.queue.len()) as u64;
    }
    Ok(Impact {
        hits: run.hits.into_values().collect(),
        complete: reasons.is_empty(),
        reasons,
    })
}

impl Run<'_> {
    fn reason(&mut self, why: &'static str) {
        *self.reasons.entry(why).or_default() += 1;
    }

    /// Spends from the bounds before the work it pays for; false, stopping the walk, once any
    /// bound has run out.
    fn pay(&mut self, work: usize, bytes: usize) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        let paid = self.bounds.charge(work, bytes);
        if !paid {
            self.stopped = Some("budget_unknown");
        }
        paid
    }

    /// Merges one path's verdict for `facets` into `node`'s hit; false once the walk stopped.
    fn hit(
        &mut self,
        node: &str,
        class: Class,
        facets: u8,
        side: u8,
        via: Option<(&'static str, &str)>,
    ) -> bool {
        if !self.hits.contains_key(node) {
            if self.hits.len() == HITS {
                self.stopped = Some("hit_limit");
                return false;
            }
            if !self.pay(1, 2 * cost(node)) {
                return false;
            }
            let hit = Hit {
                node: node.to_owned(),
                class,
                side: 0,
                changed: 0,
                unknown: 0,
                equal: 0,
                via: None,
            };
            self.hits.insert(node.to_owned(), hit);
        }
        let hit = &self.hits[node];
        let (mut changed, mut unknown, mut equal) = (hit.changed, hit.unknown, hit.equal);
        match class {
            Class::Direct | Class::Effective => changed |= facets,
            Class::Possible => unknown |= facets,
            Class::EffectiveUnchanged => equal |= facets,
        }
        let strongest = if hit.class == Class::Direct {
            Class::Direct
        } else if changed != 0 {
            Class::Effective
        } else if unknown != 0 || hit.class == Class::Possible || class == Class::Possible {
            Class::Possible
        } else {
            Class::EffectiveUnchanged
        };
        let replace =
            via.filter(|_| hit.via.is_none() || strongest != hit.class && strongest == class);
        if let Some((_, target)) = replace
            && !self.pay(0, cost(target))
        {
            return false;
        }
        let hit = self.hits.get_mut(node).expect("inserted above");
        (hit.changed, hit.unknown, hit.equal) = (changed, unknown, equal);
        hit.side |= side;
        hit.class = strongest;
        if let Some((relation, target)) = replace {
            hit.via = Some((relation, target.to_owned()));
        }
        true
    }

    /// Queues a step unless each of its generations was already queued for its target, seed
    /// and facet; false once the walk stopped.
    fn push(&mut self, target: &str, seed: &str, facet: u8, side: u8, depth: usize) -> bool {
        let want = combos(facet, side);
        let seen = self.walked.get(target).and_then(|m| m.get(seed)).copied();
        if seen.is_some_and(|s| s & want == want) {
            return true;
        }
        if self.queue.len() == QUEUE {
            self.stopped = Some("queue_limit");
            return false;
        }
        let keys = match seen {
            Some(_) => 0,
            None => cost(target) + cost(seed),
        };
        if !self.pay(1, keys + cost(target) + cost(seed)) {
            return false;
        }
        let by_seed = self.walked.entry(target.to_owned()).or_default();
        *by_seed.entry(seed.to_owned()).or_default() |= want;
        self.queue.push_back(Step {
            target: target.to_owned(),
            seed: seed.to_owned(),
            facet,
            side,
            depth,
        });
        true
    }

    /// `node` and its ancestors' ids in `index`, nearest first; None once the walk stopped. A
    /// target that is no layer (a variable, a style key) is pointed at only by its own id.
    fn lineage(&mut self, index: &Index, node: &str) -> Option<Vec<String>> {
        if !self.pay(1, cost(node)) {
            return None;
        }
        let mut out = vec![node.to_owned()];
        let mut at = rid_in(index, node);
        while let Some(rid) = at {
            let Some(parent) = index.record(rid).parent else {
                break;
            };
            let id = &index.record(parent).id;
            if !self.pay(1, cost(id)) {
                return None;
            }
            out.push(id.clone());
            at = Some(parent);
        }
        Some(out)
    }

    /// Asks the diff about one proven dependency; None once the walk stopped.
    fn verdict(&mut self, probe: &Probe) -> Result<Option<Class>> {
        if probe.facet == PRESENCE {
            self.reason("comparison_unknown");
            return Ok(Some(Class::Possible));
        }
        if !self.pay(1, 0) {
            return Ok(None);
        }
        let verdict = self.evidence.compare(probe, self.bounds)?;
        if self.bounds.stop.is_some() {
            self.stopped = Some("budget_unknown");
            return Ok(None);
        }
        Ok(Some(match verdict {
            Verdict::Changed => Class::Effective,
            Verdict::Equal => Class::EffectiveUnchanged,
            Verdict::Unknown => {
                self.reason("comparison_unknown");
                Class::Possible
            }
        }))
    }

    /// Classifies every layer that depends on `step.target` in the generations of `step.side`
    /// and queues those that may have changed; false once the walk stopped.
    fn expand(&mut self, step: &Step) -> Result<bool> {
        if step.depth == DEPTH {
            self.reason("depth_limit");
            return Ok(true);
        }
        // Recipients already classified for this step, so another anchor or generation adds
        // nothing.
        let mut done = BTreeSet::new();
        for (at, bit) in [(0, OLD), (1, NEW)] {
            if step.side & bit == 0 {
                continue;
            }
            let (index, other) = (self.graphs[at], self.graphs[1 - at]);
            let Some(anchors) = self.lineage(index, &step.target) else {
                return Ok(false);
            };
            for anchor in &anchors {
                for &(rid, i) in pointing_at(index, anchor) {
                    if !self.pay(1, 0) {
                        return Ok(false);
                    }
                    let r = index.record(rid);
                    let e = &r.edges[i as usize];
                    if !DEPENDS.contains(&e.relation)
                        || r.id == step.target
                        || done.contains(r.id.as_str())
                    {
                        continue;
                    }
                    // Proven only when the same edge resolves in every generation of the step.
                    let mut side = bit;
                    let mut resolved = e.state == "resolved";
                    if step.side & !bit != 0
                        && let Some(o) = rid_in(other, &r.id)
                    {
                        let edges = &other.record(o).edges;
                        if !self.pay(edges.len(), 0) {
                            return Ok(false);
                        }
                        let same = edges.iter().find(|x| {
                            x.relation == e.relation && x.target.as_deref() == Some(anchor)
                        });
                        if let Some(x) = same {
                            side |= !bit & (OLD | NEW);
                            resolved &= x.state == "resolved";
                        }
                    }
                    if !self.pay(1, cost(&r.id)) {
                        return Ok(false);
                    }
                    done.insert(r.id.clone());
                    let class = if side != step.side || !resolved {
                        self.reason("relation_unverified");
                        Class::Possible
                    } else {
                        let probe = Probe {
                            recipient: &r.id,
                            through: anchor,
                            seed: &step.seed,
                            facet: step.facet,
                        };
                        match self.verdict(&probe)? {
                            Some(class) => class,
                            None => return Ok(false),
                        }
                    };
                    if !self.hit(&r.id, class, step.facet, side, Some((e.relation, anchor))) {
                        return Ok(false);
                    }
                    if class != Class::EffectiveUnchanged
                        && !self.push(&r.id, &step.seed, step.facet, side, step.depth + 1)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "impact_tests.rs"]
mod tests;
