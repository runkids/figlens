//! Canonical fingerprints of definitions and cached library copies (#93), captured while a
//! stored generation is built and compared by the workspace library query without parsing.
//!
//! A fingerprint roots at every stored COMPONENT, COMPONENT_SET, VARIABLE or style node the
//! build indexes. Each of `FACETS` folds, in document order over the root and its indexed
//! subtree, every record's depth below the root, kind, name and own visibility, then the digest
//! of each of its `diff::FIELDS` fields in that facet (diff's bounded per-field digests). The
//! root's placement in its parent (`placement`) and every file-local id are left out: a field
//! whose value holds a GUID cannot be mapped to a confirmed library key, so it makes its facet
//! `unsupported`, never stripped. A field over diff's caps, hidden content left unexpanded, a
//! subtree the build cut short or an expansion stop make it `unread`.

use sha2::{Digest, Sha256};

use super::{Record, diff};
use crate::expand::Bounds;

/// The compared facets: `diff::FACETS` without `structure`, which every facet folds instead.
pub(crate) const FACETS: [&str; 4] = ["text", "layout", "appearance", "properties"];
pub(crate) const STATES: [&str; 3] = ["read", "unread", "unsupported"];
/// Bumped whenever the fold changes; stored with `usages::RELATIONS`.
pub(crate) const VERSION: &str = "fingerprint-1";
/// A root's place in its parent, which a cached copy does not share with its definition: its
/// transform and the fields instance expansion keeps as placement.
fn placement(key: &str) -> bool {
    key == "transform" || crate::expand::PLACEMENT.contains(&key)
}

/// Retained bytes of one stored facet row.
pub(super) const ROW_BYTES: usize = 96;

/// One facet: its state and, when read, its digest.
pub(crate) type Print = (&'static str, Option<[u8; 32]>);

/// What the fold reads of one record inside a fingerprinted subtree.
pub(super) struct Input {
    pub facets: diff::Facets,
    pub hidden: bool,
    /// Hidden content REST omits was not expanded below it.
    pub unexpanded: bool,
}

/// Whether a stored node roots a fingerprint.
pub(super) fn applicable(kind: &str, node: Option<&kiwi_schema::Value>) -> bool {
    matches!(kind, "COMPONENT" | "COMPONENT_SET" | "VARIABLE")
        || node.is_some_and(|n| n.get("styleType").is_some())
}

/// The fingerprint of each of `roots`, charged to `bounds` per visited record and field and per
/// retained row first; `None` once they run out. `cut` says the build stopped before the last
/// record's subtree ended, `expansion` that instance expansion stopped somewhere.
pub(super) fn prints(
    records: &[Record],
    inputs: &[Option<Input>],
    roots: &[u32],
    cut: bool,
    expansion: bool,
    bounds: &mut Bounds,
) -> Option<Vec<(u32, [Print; 4])>> {
    if !bounds.charge(
        records.len(),
        4 * records.len() + roots.len() * 4 * ROW_BYTES,
    ) {
        return None;
    }
    // Pre-order: a root's subtree is the records after it that sit deeper.
    let mut depth = vec![0u32; records.len()];
    for (i, r) in records.iter().enumerate() {
        depth[i] = r.parent.map_or(0, |p| depth[p as usize] + 1);
    }
    let mut out = Vec::with_capacity(roots.len());
    for &root in roots {
        let root = root as usize;
        let mut h: Vec<Sha256> = FACETS
            .iter()
            .map(|f| {
                let mut h = Sha256::new();
                h.update(VERSION);
                h.update(f);
                h
            })
            .collect();
        let mut unread = [expansion; 4];
        let mut local = [false; 4];
        let mut end = root;
        while end < records.len() && (end == root || depth[end] > depth[root]) {
            let r = &records[end];
            let Some(input) = &inputs[end] else {
                unread = [true; 4];
                break;
            };
            if !bounds.charge(1 + input.facets.fields.len(), 0) {
                return None;
            }
            for h in &mut h {
                for part in [r.kind.as_bytes(), r.name.as_bytes()] {
                    h.update((part.len() as u64).to_le_bytes());
                    h.update(part);
                }
                h.update((depth[end] - depth[root]).to_le_bytes());
                h.update([u8::from(input.hidden)]);
            }
            if input.unexpanded {
                unread = [true; 4];
            }
            for f in &input.facets.fields {
                let i = f.facet as usize;
                if i >= FACETS.len() || (end == root && placement(f.key)) {
                    continue;
                }
                match f.digest {
                    None => unread[i] = true,
                    Some(_) if f.local => local[i] = true,
                    Some(d) => {
                        h[i].update(f.key);
                        h[i].update(d);
                    }
                }
            }
            end += 1;
        }
        if cut && end == records.len() {
            unread = [true; 4];
        }
        let mut print = [("read", None); 4];
        for (i, h) in h.into_iter().enumerate() {
            print[i] = match (unread[i], local[i]) {
                (true, _) => ("unread", None),
                (_, true) => ("unsupported", None),
                _ => ("read", Some(h.finalize().into())),
            };
        }
        out.push((root as u32, print));
    }
    Some(out)
}

/// How a cached copy compares with the selected definition: `definition_mismatch` when a facet
/// read and supported on both sides differs, `equal` when every facet is and none does, else
/// `unknown`. Records of different kinds (a set and one variant) are never compared.
pub(crate) fn compare(
    definition: &(String, [Print; 4]),
    copy: &(String, [Print; 4]),
) -> &'static str {
    if definition.0 != copy.0 {
        return "unknown";
    }
    let mut all = true;
    for (a, b) in definition.1.iter().zip(&copy.1) {
        match (a.1, b.1) {
            (Some(x), Some(y)) if x != y => return "definition_mismatch",
            (Some(_), Some(_)) => {}
            _ => all = false,
        }
    }
    if all { "equal" } else { "unknown" }
}
