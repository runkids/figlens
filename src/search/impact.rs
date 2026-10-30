//! Which layers a set of changed nodes reaches, over the relation graphs of both compared
//! generations. Pure and bounded: it reads two `Graph`s and asks the diff, through `Evidence`,
//! whether a reached layer's effective fields really differ. Nothing here compares fields, so
//! a dependency alone never proves a change and an override alone never proves none.
// The host owner wires it into the tracked publication; until then only its tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use super::usages::{Graph, Key};
use crate::expand::Bounds;
use crate::{ErrorKind, FigError};
use std::collections::{BTreeMap, VecDeque};
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Relations through which a target's change can reach the layer that points at it.
const DEPENDS: [&str; 4] = ["binds_variable", "instance_of", "uses_image", "uses_style"];
/// Inbound edges read per graph call.
const PAGE: usize = 200;
/// Dependency hops from a changed node.
pub(crate) const DEPTH: usize = 16;
/// Layers one computation reports.
pub(crate) const HITS: usize = 2_000;
const OLD: u8 = 1;
const NEW: u8 = 2;

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

/// The diff's answer for one reached layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Some requested facet of the layer's effective value differs between the generations.
    Changed,
    /// Every requested facet was read on both sides and is equal.
    Equal,
    Unknown,
}

/// One question to the diff: does `recipient`'s effective value differ in `facets`, where it
/// depends on `through`, which `seed`'s change reached?
pub(crate) struct Probe<'a> {
    pub recipient: &'a str,
    pub through: &'a str,
    pub seed: &'a str,
    pub facets: u8,
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
    /// Facets per class, merged over every path: changed wins, then unknown, then equal.
    pub changed: u8,
    pub unknown: u8,
    pub equal: u8,
    /// The first relation and target that reached it with its strongest evidence.
    pub via: Option<(&'static str, String)>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Impact {
    pub hits: Vec<Hit>,
    pub complete: bool,
    /// Why it is not complete, with counts: `budget_unknown` (queued targets left when a bound
    /// ran out; a known pending count, not every omitted layer), `depth_limit`, `hit_limit`,
    /// `seed_unknown`.
    pub reasons: BTreeMap<&'static str, u64>,
}

/// One traversal step: the layers that depend on `target` may change with `seed`.
struct Step {
    target: String,
    seed: String,
    facets: u8,
    side: u8,
    depth: usize,
}

fn budget() -> Box<dyn Error> {
    FigError::new(ErrorKind::QueryWorkLimit, "impact budget".to_owned()).into()
}

fn limited(e: &(dyn Error + 'static)) -> bool {
    e.downcast_ref::<FigError>()
        .is_some_and(|f| f.kind() == ErrorKind::QueryWorkLimit)
}

/// The layers `seeds` reach through `old` and `new`, classified by `evidence`, within `bounds`.
/// A bound that runs out stops the walk and leaves it incomplete; it is never an error.
pub(crate) fn compute(
    old: &mut dyn Graph,
    new: &mut dyn Graph,
    seeds: &[Seed],
    evidence: &mut dyn Evidence,
    bounds: &mut Bounds,
) -> Result<Impact> {
    let mut out = Impact::default();
    let mut hits: BTreeMap<String, Hit> = BTreeMap::new();
    // Facets and sides already walked per (target, seed): a later path walks again only with
    // more, so an earlier masked path never hides a later changed one.
    let mut walked: BTreeMap<(String, String), (u8, u8)> = BTreeMap::new();
    let mut queue = VecDeque::new();
    let reason = |out: &mut Impact, why: &'static str, n: u64| {
        *out.reasons.entry(why).or_default() += n;
    };
    for (i, seed) in seeds.iter().enumerate() {
        let side = match seed.change {
            Change::Added => NEW,
            Change::Removed => OLD,
            Change::Changed | Change::Unknown => OLD | NEW,
        };
        let class = if seed.change == Change::Unknown {
            reason(&mut out, "seed_unknown", 1);
            Class::Possible
        } else {
            Class::Direct
        };
        if !bounds.charge(1, seed.node.len() * 2 + 64) {
            reason(&mut out, "budget_unknown", (seeds.len() - i) as u64);
            return Ok(finish(out, hits));
        }
        merge(&mut hits, &seed.node, class, seed.facets, side, None);
        queue.push_back(Step {
            target: seed.node.clone(),
            seed: seed.node.clone(),
            facets: seed.facets,
            side,
            depth: 0,
        });
    }
    while let Some(step) = queue.pop_front() {
        let key = (step.target.clone(), step.seed.clone());
        if !walked.contains_key(&key) && !bounds.charge(1, (key.0.len() + key.1.len()) * 2 + 64) {
            reason(&mut out, "budget_unknown", 1 + queue.len() as u64);
            return Ok(finish(out, hits));
        }
        let walk = walked.entry(key).or_default();
        if walk.0 & step.facets == step.facets && walk.1 & step.side == step.side {
            continue;
        }
        *walk = (walk.0 | step.facets, walk.1 | step.side);
        if step.depth == DEPTH {
            reason(&mut out, "depth_limit", 1);
            continue;
        }
        match expand(old, new, &step, evidence, bounds, &mut hits) {
            Ok(next) => {
                if hits.len() > HITS {
                    reason(&mut out, "hit_limit", 1);
                    break;
                }
                queue.extend(next);
            }
            Err(e) if limited(e.as_ref()) => {
                reason(&mut out, "budget_unknown", 1 + queue.len() as u64);
                return Ok(finish(out, hits));
            }
            Err(e) => return Err(e),
        }
    }
    if !queue.is_empty() {
        reason(&mut out, "budget_unknown", queue.len() as u64);
    }
    Ok(finish(out, hits))
}

fn finish(mut out: Impact, hits: BTreeMap<String, Hit>) -> Impact {
    out.hits = hits.into_values().take(HITS).collect();
    out.complete = out.reasons.is_empty();
    out
}

fn merge(
    hits: &mut BTreeMap<String, Hit>,
    node: &str,
    class: Class,
    facets: u8,
    side: u8,
    via: Option<(&'static str, String)>,
) {
    let hit = hits.entry(node.to_owned()).or_insert_with(|| Hit {
        node: node.to_owned(),
        class,
        side: 0,
        changed: 0,
        unknown: 0,
        equal: 0,
        via: None,
    });
    hit.side |= side;
    match class {
        Class::Direct | Class::Effective => hit.changed |= facets,
        Class::Possible => hit.unknown |= facets,
        Class::EffectiveUnchanged => hit.equal |= facets,
    }
    let strongest = if hit.class == Class::Direct {
        Class::Direct
    } else if hit.changed != 0 {
        Class::Effective
    } else if hit.unknown != 0 || hit.class == Class::Possible || class == Class::Possible {
        Class::Possible
    } else {
        Class::EffectiveUnchanged
    };
    if hit.via.is_none() || strongest != hit.class && strongest == class {
        hit.via = via.or(hit.via.take());
    }
    hit.class = strongest;
}

/// Where `node` sits in `graph`: its own id and its ancestors' ids, nearest first.
fn lineage(graph: &mut dyn Graph, node: &str, bounds: &mut Bounds) -> Result<Vec<String>> {
    // A target that is no layer (a variable, a style key) is pointed at only by its own id.
    let mut out = vec![node.to_owned()];
    let Some(mut rid) = graph.rid_of(node)? else {
        return Ok(out);
    };
    loop {
        if !bounds.charge(1, 64) {
            return Err(budget());
        }
        let parent = graph
            .outbound(rid)?
            .into_iter()
            .find(|e| e.relation == "contains")
            .and_then(|e| e.target);
        let Some(parent) = parent else {
            return Ok(out);
        };
        let Some(next) = graph.rid_of(&parent)? else {
            return Ok(out);
        };
        if !bounds.charge(1, parent.len() * 2) {
            return Err(budget());
        }
        out.push(parent);
        rid = next;
    }
}

/// The layers that point at `target` (or a containing ancestor of it) in `graph`, with the
/// relation, the pointed-at id and whether the edge resolved there.
fn dependents(
    graph: &mut dyn Graph,
    target: &str,
    bounds: &mut Bounds,
) -> Result<BTreeMap<String, (&'static str, String, bool)>> {
    let mut out = BTreeMap::new();
    for anchor in lineage(graph, target, bounds)? {
        let mut after: Option<Key> = None;
        loop {
            if !bounds.charge(PAGE, 0) {
                return Err(budget());
            }
            let page = graph.inbound(
                std::slice::from_ref(&anchor),
                &DEPENDS,
                None,
                true,
                after.as_ref(),
                PAGE,
            )?;
            let rids: Vec<u32> = page.iter().map(|(rid, _)| *rid).collect();
            graph.load(&rids)?;
            for (rid, edge) in &page {
                let id = graph.index().record(*rid).id.clone();
                if !bounds.charge(1, id.len() * 2 + anchor.len() * 2 + 64) {
                    return Err(budget());
                }
                let resolved = edge.state == "resolved";
                let entry = out
                    .entry(id)
                    .or_insert((edge.relation, anchor.clone(), resolved));
                entry.2 &= resolved;
            }
            match page.last() {
                Some((rid, e)) if page.len() == PAGE => {
                    after = Some((*rid, e.relation.to_owned(), e.seq));
                }
                _ => break,
            }
        }
    }
    Ok(out)
}

/// Classifies every layer that depends on `step.target` in the generations of `step.side`, and
/// returns the steps from those that may have changed.
fn expand(
    old: &mut dyn Graph,
    new: &mut dyn Graph,
    step: &Step,
    evidence: &mut dyn Evidence,
    bounds: &mut Bounds,
    hits: &mut BTreeMap<String, Hit>,
) -> Result<Vec<Step>> {
    let mut seen: BTreeMap<String, (u8, &'static str, String, bool)> = BTreeMap::new();
    let mut read = |bit: u8, graph: &mut dyn Graph, bounds: &mut Bounds| -> Result<()> {
        if step.side & bit != 0 {
            for (id, (relation, anchor, resolved)) in dependents(graph, &step.target, bounds)? {
                let entry = seen.entry(id).or_insert((0, relation, anchor, true));
                entry.0 |= bit;
                entry.3 &= resolved;
            }
        }
        Ok(())
    };
    read(OLD, old, bounds)?;
    read(NEW, new, bounds)?;
    let mut next = Vec::new();
    for (recipient, (side, relation, anchor, resolved)) in seen {
        if recipient == step.target {
            continue;
        }
        // Proven only when the edge resolves in every generation the change is in.
        let class = if side == step.side && resolved {
            if !bounds.charge(1, 0) {
                return Err(budget());
            }
            let probe = Probe {
                recipient: &recipient,
                through: &anchor,
                seed: &step.seed,
                facets: step.facets,
            };
            match evidence.compare(&probe, bounds)? {
                Verdict::Changed => Class::Effective,
                Verdict::Equal => Class::EffectiveUnchanged,
                Verdict::Unknown => Class::Possible,
            }
        } else {
            Class::Possible
        };
        merge(
            hits,
            &recipient,
            class,
            step.facets,
            side,
            Some((relation, anchor)),
        );
        if class != Class::EffectiveUnchanged {
            next.push(Step {
                target: recipient,
                seed: step.seed.clone(),
                facets: step.facets,
                side,
                depth: step.depth + 1,
            });
        }
    }
    Ok(next)
}

#[cfg(test)]
#[path = "impact_tests.rs"]
mod tests;
