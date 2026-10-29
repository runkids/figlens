//! Exact diff of two revisions of one scope (#81), the engine durable changes (#92) reuse.
//!
//! Each side is a full relations build that also records, per record, the effective value of a
//! fixed field list (an instance override wins over the stored field, as in usages) grouped
//! into facets. Records match by exact id and kind only; top-level frames left unmatched may be
//! paired by name or geometry as a labelled `possible_match`, which never transfers identity.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use super::{
    BYTES, DEADLINE, DEPTH, Index, REHASH, Stamp, WORK, build_with, hex, require_scope,
    snapshot_id, source_id, stage, stamp,
};
use crate::expand::{Bounds, Expanded, json_bytes, kiwi_bytes};
use crate::{Document, ErrorKind, FigError, Tree, to_json};

/// Bumped whenever a field list, facet or digest stream changes; cursors carry it.
const VERSION: &str = "diff-v1";
pub(crate) const FACETS: [&str; 5] = ["text", "layout", "appearance", "properties", "structure"];
/// Effective kiwi fields per facet; `structure` (name, kind, parent, order, hidden) is the
/// record's own and bindings are edges, not fields.
const FIELDS: [&[&str]; 4] = [
    &[
        "textData",
        "fontSize",
        "fontName",
        "lineHeight",
        "letterSpacing",
        "textAlignHorizontal",
        "textAlignVertical",
        "textAutoResize",
        "textCase",
        "textDecoration",
        "paragraphSpacing",
    ],
    &[
        "size",
        "transform",
        "horizontalConstraint",
        "verticalConstraint",
        "stackMode",
        "stackSpacing",
        "stackHorizontalPadding",
        "stackVerticalPadding",
        "stackPaddingRight",
        "stackPaddingBottom",
        "stackPrimarySizing",
        "stackCounterSizing",
        "stackPrimaryAlignItems",
        "stackCounterAlignItems",
        "stackChildPrimaryGrow",
        "stackChildAlignSelf",
        "stackPositioning",
        "stackWrap",
        "frameMaskDisabled",
    ],
    &[
        "fillPaints",
        "strokePaints",
        "strokeWeight",
        "strokeAlign",
        "borderStrokeWeightsIndependent",
        "borderTopWeight",
        "borderRightWeight",
        "borderBottomWeight",
        "borderLeftWeight",
        "dashPattern",
        "effects",
        "opacity",
        "blendMode",
        "mask",
        "visible",
        "cornerRadius",
        "rectangleCornerRadiiIndependent",
        "rectangleTopLeftCornerRadius",
        "rectangleTopRightCornerRadius",
        "rectangleBottomLeftCornerRadius",
        "rectangleBottomRightCornerRadius",
    ],
    &[
        "componentPropAssignments",
        "componentPropRefs",
        "variantPropSpecs",
    ],
];
/// Largest value one field may scan; a larger one is `unread`, like a usages binding field.
const VISITS: usize = 1 << 16;
/// Retained value bytes: one field, one record. Over them the value is dropped and marked
/// truncated; its digest still decides whether it changed.
const FIELD_VALUE: usize = 512;
const RECORD_VALUES: usize = 4 * 1024;
/// Changes and candidate probes one pair may hold or spend, charged before they are kept.
const PAIR_WORK: usize = 2_000_000;
const PAIR_BYTES: usize = 64 * 1024 * 1024;
/// Possible matches returned; candidate probes are charged to the pair budget.
const POSSIBLE: usize = 200;
/// Value bytes one page may carry.
const PAGE_VALUES: usize = 256 * 1024;
pub const LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;

/// One effective field of one record.
pub(crate) struct Field {
    key: &'static str,
    facet: u8,
    /// SHA-256 of the field's key and length-delimited JSON; `None` when it was unread.
    digest: Option<[u8; 32]>,
    /// The value, when it fit `FIELD_VALUE` and the record's `RECORD_VALUES`.
    value: Option<Json>,
}

/// A record's effective fields in `FIELDS` order; absent fields are not listed.
pub(crate) struct Facets {
    fields: Vec<Field>,
}

/// The effective value of `key` for record `id`: its override, else the stored field.
enum Value<'a> {
    Json(&'a Json),
    Kiwi(&'a kiwi_schema::Value<'a>),
    Owned(Json),
}

/// The facets of record `id` (expanded as `e`), charged to `bounds`; `None` once they stop.
pub(super) fn facets(
    tree: &Tree,
    id: &str,
    e: Option<&Expanded>,
    bounds: &mut Bounds,
) -> Option<Facets> {
    let node = tree.nodes.get(e.map_or(id, |e| e.source.as_str()));
    let mut fields = Vec::new();
    let mut kept = 0;
    for (facet, keys) in FIELDS.iter().enumerate() {
        for key in *keys {
            let value = match (e, *key) {
                (Some(e), "size") if e.size.is_some() => {
                    let s = e.size.unwrap();
                    Value::Owned(json!({"x": s.x, "y": s.y}))
                }
                (Some(e), "transform") => {
                    let m = e.matrix;
                    Value::Owned(
                        json!({"m00": m.a, "m01": m.c, "m02": m.tx, "m10": m.b, "m11": m.d, "m12": m.ty}),
                    )
                }
                _ => match (
                    e.and_then(|e| e.props.get(*key)),
                    node.and_then(|n| n.get(key)),
                ) {
                    (Some(v), _) => Value::Json(v),
                    (None, Some(v)) => Value::Kiwi(v),
                    (None, None) => continue,
                },
            };
            if !bounds.charge(0, 0) {
                return None;
            }
            let mut left = VISITS;
            match &value {
                Value::Json(v) => json_bytes(v, &mut left),
                Value::Owned(v) => json_bytes(v, &mut left),
                Value::Kiwi(v) => kiwi_bytes(v, &mut left),
            };
            if !bounds.charge(VISITS - left, 0) {
                return None;
            }
            if left == 0 {
                fields.push(Field {
                    key,
                    facet: facet as u8,
                    digest: None,
                    value: None,
                });
                continue;
            }
            let json = match value {
                Value::Json(v) => v.clone(),
                Value::Owned(v) => v,
                Value::Kiwi(v) => to_json(v),
            };
            let bytes = serde_json::to_vec(&json).ok()?;
            let mut h = Sha256::new();
            h.update(key.as_bytes());
            h.update([0]);
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(&bytes);
            let fits = bytes.len() <= FIELD_VALUE && kept + bytes.len() <= RECORD_VALUES;
            // The converted copy is transient; only the digest and a fitting value are kept.
            if !bounds.charge(1, 64 + key.len() + if fits { bytes.len() } else { 0 }) {
                return None;
            }
            if fits {
                kept += bytes.len();
            }
            fields.push(Field {
                key,
                facet: facet as u8,
                digest: Some(h.finalize().into()),
                value: fits.then_some(json),
            });
        }
    }
    Some(Facets { fields })
}

/// One revision of the selected scope: its index, facets and exact identity.
struct Side {
    index: Index,
    facets: Vec<Facets>,
    source: Json,
    /// The built corpus: generation and stop. Same bytes can build differently under load.
    signature: String,
    path: PathBuf,
    stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Added,
    Removed,
    Changed,
    /// Same id on both sides but another layer type: identity is not proven either way.
    Retyped,
    /// Present on one side while the other stopped before reaching it.
    Unknown,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Added => "added",
            Kind::Removed => "removed",
            Kind::Changed => "changed",
            Kind::Retyped => "kind_changed",
            Kind::Unknown => "unknown",
        }
    }
}

/// One changed record, referencing both sides by record index.
struct Change {
    frame: u32,
    old: Option<u32>,
    new: Option<u32>,
    kind: Kind,
    /// Bit per `FACETS` entry that differs, plus `EDGES`; `UNREAD` when a compared field was
    /// unread on either side.
    facets: u8,
    /// Added or removed layers below this one that are reported with it, not listed.
    descendants: u32,
}

const EDGES: u8 = 1 << 5;
const UNREAD: u8 = 1 << 6;

/// A top-level frame (or a page, for layers directly on it) with changes.
struct Frame {
    id: String,
    name: String,
    page: String,
    /// The frame's own change, when the frame record itself changed.
    change: Option<Kind>,
    counts: [u32; 8],
    possible: Option<(String, &'static str)>,
}

struct Pair {
    old: Side,
    new: Side,
    node: String,
    changes: Vec<Change>,
    frames: Vec<Frame>,
    possible: usize,
    /// Why the pair itself stopped short (`diff_work_limit`).
    stop: Option<&'static str>,
    hashed: SystemTime,
}

// shortcut: one old/new pair per process (two indexes with their facets), reused while both
// paths, stamps and the selection match; durable revisions are #92's.
static PAIR: Mutex<Option<Arc<Pair>>> = Mutex::new(None);

/// A diff request: `node` selects the scope on both sides.
pub struct Request {
    pub node: String,
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Request {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(FigError::new(
                ErrorKind::InvalidArguments,
                format!("limit must be 1..={MAX_LIMIT}"),
            )
            .into());
        }
        if self.node.is_empty() || self.node.len() > super::store::ID_BYTES {
            return Err(FigError::new(
                ErrorKind::InvalidArguments,
                "node must be a stored node id",
            )
            .into());
        }
        Ok(())
    }
}

/// Diffs `node` of the `.fig` at `old` against the one at `new`. A later page of the same pair
/// reads the process cache after rechecking both sources; a cold call (one CLI process per
/// page) builds both sides again.
pub fn diff(old: &Path, new: &Path, req: &Request) -> Result<Json, Box<dyn Error>> {
    req.validate()?;
    let paths = [std::fs::canonicalize(old)?, std::fs::canonicalize(new)?];
    let stamps = [stamp(&paths[0])?, stamp(&paths[1])?];
    let fresh = |p: &Pair| {
        p.old.path == paths[0]
            && p.new.path == paths[1]
            && p.old.stamp == stamps[0]
            && p.new.stamp == stamps[1]
            && p.node == req.node
            && p.hashed.elapsed().is_ok_and(|age| age < REHASH)
    };
    let cached = PAIR
        .lock()
        .ok()
        .and_then(|c| c.clone().filter(|p| fresh(p)));
    let (pair, reused) = match cached {
        Some(pair) => {
            // Metadata alone does not prove either source is still readable.
            std::fs::File::open(&paths[0])?;
            std::fs::File::open(&paths[1])?;
            (pair, true)
        }
        None => {
            // Free the previous pair before building a new one.
            if let Ok(mut c) = PAIR.lock() {
                *c = None;
            }
            let deadline = Instant::now() + DEADLINE;
            let old = side(&paths[0], old, &req.node, deadline)?;
            let new = side(&paths[1], new, &req.node, deadline)?;
            let pair = Arc::new(compare(old, new, &req.node));
            let stopped = |s: &Side| s.index.stop == Some("deadline");
            if !stopped(&pair.old)
                && !stopped(&pair.new)
                && let Ok(mut c) = PAIR.lock()
            {
                *c = Some(pair.clone());
            }
            (pair, false)
        }
    };
    page(&pair, req, reused)
}

/// Decodes and builds one side, dropping its decoded tree before returning.
fn side(
    canonical: &Path,
    path: &Path,
    node: &str,
    deadline: Instant,
) -> Result<Side, Box<dyn Error>> {
    let (bytes, staged) = stage(canonical)?;
    let snapshot = snapshot_id(&bytes);
    let mut doc = Document::from_bytes(&bytes, &path.display().to_string())?;
    drop(bytes);
    let message = doc.take_message()?;
    let tree = Tree::new(&message);
    require_scope(&tree, node)?;
    let bounds = Bounds {
        work: WORK,
        bytes: BYTES,
        depth: DEPTH,
        deadline,
        stop: None,
    };
    let mut facets = Vec::new();
    let index = build_with(&tree, node, bounds, true, Some(&mut facets));
    if stamp(canonical)? != staged {
        return Err(FigError::new(
            ErrorKind::SourceUnreadable,
            "a .fig changed while it was diffed; retry after the save finishes",
        )
        .into());
    }
    Ok(Side {
        signature: format!("{}:{}", index.generation, index.stop.unwrap_or("")),
        source: json!({"sourceId": source_id(canonical), "snapshotId": snapshot}),
        index,
        facets,
        path: canonical.to_owned(),
        stamp: staged,
    })
}

/// A record's structure digest input: name, kind, parent id, order among siblings and hidden.
fn structure<'a>(
    index: &'a Index,
    rid: usize,
    order: &[u32],
) -> (&'a str, &'a str, Option<&'a str>, u32, bool) {
    let r = &index.records[rid];
    let parent = r.parent.map(|p| index.records[p as usize].id.as_str());
    (
        r.name.as_str(),
        r.kind.as_str(),
        parent,
        order[rid],
        r.hidden,
    )
}

/// Each record's position among its parent's indexed children.
fn orders(index: &Index) -> Vec<u32> {
    let mut next: HashMap<Option<u32>, u32> = HashMap::new();
    index
        .records
        .iter()
        .map(|r| {
            let n = next.entry(r.parent).or_default();
            *n += 1;
            *n - 1
        })
        .collect()
}

fn by_id(s: &Side) -> HashMap<&str, usize> {
    s.index
        .records
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect()
}

fn edge_keys(s: &Side, r: usize) -> Vec<(&str, Option<&str>, &str, &str)> {
    s.index.records[r]
        .edges
        .iter()
        .map(|e| {
            (
                e.relation,
                e.target.as_deref(),
                e.state,
                e.evidence.as_str(),
            )
        })
        .collect()
}

/// The facet bits on which two records of the same id and kind differ.
fn differs(old: &Side, o: usize, oo: &[u32], new: &Side, n: usize, no: &[u32]) -> u8 {
    let mut bits = 0;
    let (a, b) = (&old.facets[o].fields, &new.facets[n].fields);
    for facet in 0..FIELDS.len() as u8 {
        let pick = |f: &[Field]| {
            f.iter()
                .filter(|x| x.facet == facet)
                .map(|x| (x.key, x.digest))
                .collect::<Vec<_>>()
        };
        let (x, y) = (pick(a), pick(b));
        if x.iter().chain(&y).any(|(_, d)| d.is_none()) {
            bits |= UNREAD;
        }
        if x != y {
            bits |= 1 << facet;
        }
    }
    if structure(&old.index, o, oo) != structure(&new.index, n, no) {
        bits |= 1 << 4;
    }
    if edge_keys(old, o) != edge_keys(new, n) {
        bits |= EDGES;
    }
    bits
}

/// Groups changes by top-level frame (or page) while charging each to the pair budget.
struct Collect {
    budget: Bounds,
    frames: Vec<Frame>,
    at: HashMap<String, u32>,
    changes: Vec<Change>,
}

impl Collect {
    fn push(
        &mut self,
        side: &Side,
        ids: &HashMap<&str, usize>,
        rid: usize,
        (old, new): (Option<u32>, Option<u32>),
        kind: Kind,
        facets: u8,
    ) -> bool {
        if !self.budget.charge(1, 96) {
            return false;
        }
        let index = &side.index;
        let r = &index.records[rid];
        let page = r.page.map_or("", |p| index.pages[p as usize].id.as_str());
        let (id, name) = match r.root {
            Some(root) => {
                let id = index.roots[root as usize].as_str();
                (
                    id,
                    ids.get(id).map_or("", |&i| index.records[i].name.as_str()),
                )
            }
            None => (
                page,
                r.page.map_or("", |p| index.pages[p as usize].name.as_str()),
            ),
        };
        if !self.at.contains_key(id) {
            if !self
                .budget
                .charge(1, 128 + id.len() + name.len() + page.len())
            {
                return false;
            }
            self.frames.push(Frame {
                id: id.to_owned(),
                name: name.to_owned(),
                page: page.to_owned(),
                change: None,
                counts: [0; 8],
                possible: None,
            });
            self.at.insert(id.to_owned(), self.frames.len() as u32 - 1);
        }
        let f = self.at[id];
        let frame = &mut self.frames[f as usize];
        if r.id == frame.id {
            frame.change = Some(kind);
        }
        for bit in 0..7 {
            if facets & (1 << bit) != 0 {
                frame.counts[bit] += 1;
            }
        }
        frame.counts[7] += 1;
        self.changes.push(Change {
            frame: f,
            old,
            new,
            kind,
            facets,
            descendants: 0,
        });
        true
    }
}

fn compare(old: Side, new: Side, node: &str) -> Pair {
    let mut c = Collect {
        budget: Bounds {
            work: PAIR_WORK,
            bytes: PAIR_BYTES,
            depth: DEPTH,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        },
        frames: Vec::new(),
        at: HashMap::new(),
        changes: Vec::new(),
    };
    let (oo, no) = (orders(&old.index), orders(&new.index));
    let (old_ids, new_ids) = (by_id(&old), by_id(&new));
    // Removed and added subtrees are reported at their top record, counting what lies below.
    let mut reported: HashMap<(bool, usize), usize> = HashMap::new();
    let mut full = true;
    for (o, r) in old.index.records.iter().enumerate() {
        let kept = match new_ids.get(r.id.as_str()) {
            Some(&n) if new.index.records[n].kind != r.kind => c.push(
                &old,
                &old_ids,
                o,
                (Some(o as u32), Some(n as u32)),
                Kind::Retyped,
                0,
            ),
            Some(&n) => {
                let bits = differs(&old, o, &oo, &new, n, &no);
                c.budget.charge(1, 0)
                    && (bits & !UNREAD == 0
                        || c.push(
                            &new,
                            &new_ids,
                            n,
                            (Some(o as u32), Some(n as u32)),
                            Kind::Changed,
                            bits,
                        ))
            }
            None => match r
                .parent
                .and_then(|p| reported.get(&(false, p as usize)).copied())
            {
                Some(top) => {
                    c.changes[top].descendants += 1;
                    reported.insert((false, o), top);
                    true
                }
                None => {
                    let kind = if new.index.stop.is_none() {
                        Kind::Removed
                    } else {
                        Kind::Unknown
                    };
                    let kept = c.push(&old, &old_ids, o, (Some(o as u32), None), kind, 0);
                    reported.insert((false, o), c.changes.len().saturating_sub(1));
                    kept
                }
            },
        };
        if !kept {
            full = false;
            break;
        }
    }
    for (n, r) in new.index.records.iter().enumerate().filter(|_| full) {
        if old_ids.contains_key(r.id.as_str()) {
            continue;
        }
        if let Some(top) = r
            .parent
            .and_then(|p| reported.get(&(true, p as usize)).copied())
        {
            c.changes[top].descendants += 1;
            reported.insert((true, n), top);
            continue;
        }
        let kind = if old.index.stop.is_none() {
            Kind::Added
        } else {
            Kind::Unknown
        };
        if !c.push(&new, &new_ids, n, (None, Some(n as u32)), kind, 0) {
            full = false;
            break;
        }
        reported.insert((true, n), c.changes.len() - 1);
    }
    let possible = match full {
        true => pair_frames(&old, &new, &old_ids, &new_ids, &mut c.frames, &mut c.budget),
        false => 0,
    };
    let stop = (!full || c.budget.stop.is_some()).then_some("diff_work_limit");
    let Collect {
        frames,
        mut changes,
        ..
    } = c;
    // Pages list changes frame by frame, in first-seen frame order.
    changes.sort_by_key(|c| c.frame);
    Pair {
        old,
        new,
        node: node.to_owned(),
        changes,
        frames,
        possible,
        stop,
        hashed: SystemTime::now(),
    }
}

/// Labels removed and added top-level frames of one page that share a name, else an exact
/// rounded size and position, as possible matches. At most `POSSIBLE`; every probe is charged.
fn pair_frames(
    old: &Side,
    new: &Side,
    old_ids: &HashMap<&str, usize>,
    new_ids: &HashMap<&str, usize>,
    frames: &mut [Frame],
    budget: &mut Bounds,
) -> usize {
    let geometry = |s: &Side, id: &str| -> Option<String> {
        let ids = if std::ptr::eq(s, old) {
            old_ids
        } else {
            new_ids
        };
        let rid = *ids.get(id)?;
        let f = &s.facets[rid].fields;
        let v = |k| f.iter().find(|x| x.key == k).and_then(|x| x.value.as_ref());
        let (size, t) = (v("size")?, v("transform")?);
        let r = |x: &Json| x.as_f64().map_or(i64::MIN, |x| x.round() as i64);
        Some(format!(
            "{},{},{},{}",
            r(&size["x"]),
            r(&size["y"]),
            r(&t["m02"]),
            r(&t["m12"])
        ))
    };
    let mut by: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, f) in frames.iter().enumerate() {
        if f.change == Some(Kind::Added) {
            if !budget.charge(2, 128) {
                return 0;
            }
            by.entry((f.page.clone(), format!("n:{}", f.name)))
                .or_default()
                .push(i);
            if let Some(g) = geometry(new, &f.id) {
                by.entry((f.page.clone(), format!("g:{g}")))
                    .or_default()
                    .push(i);
            }
        }
    }
    let mut found = 0;
    for i in 0..frames.len() {
        if found == POSSIBLE {
            break;
        }
        if frames[i].change != Some(Kind::Removed) {
            continue;
        }
        if !budget.charge(2, 0) {
            break;
        }
        let page = frames[i].page.clone();
        let keys = [
            (format!("n:{}", frames[i].name), "name"),
            (
                geometry(old, &frames[i].id).map_or(String::new(), |g| format!("g:{g}")),
                "position",
            ),
        ];
        for (key, basis) in keys {
            let hit = by.get_mut(&(page.clone(), key)).and_then(|v| {
                v.iter()
                    .position(|j| frames[*j].possible.is_none())
                    .map(|p| v.remove(p))
            });
            if let Some(j) = hit {
                frames[i].possible = Some((frames[j].id.clone(), basis));
                frames[j].possible = Some((frames[i].id.clone(), basis));
                found += 1;
                break;
            }
        }
    }
    found
}

fn binding(pair: &Pair) -> String {
    let s = |s: &Side| {
        format!(
            "{}|{}|{}",
            s.source["sourceId"], s.source["snapshotId"], s.signature
        )
    };
    format!("{VERSION}|{}|{}|{}", s(&pair.old), s(&pair.new), pair.node)
}

fn cursor(pair: &Pair, offset: usize) -> String {
    format!("d1.{}.{offset}", &hex(&Sha256::digest(binding(pair)))[..32])
}

fn offset(pair: &Pair, cursor: &str) -> Result<usize, Box<dyn Error>> {
    let stale = || -> Box<dyn Error> {
        FigError::new(
            ErrorKind::CursorStale,
            "this cursor belongs to another pair of revisions or selection; run the diff again without --cursor",
        )
        .into()
    };
    let mut parts = cursor.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("d1"), Some(h), Some(n), None) if *h == hex(&Sha256::digest(binding(pair)))[..32] => {
            n.parse::<usize>()
                .ok()
                .filter(|n| *n <= pair.changes.len())
                .ok_or_else(stale)
        }
        _ => Err(stale()),
    }
}

fn summary(counts: &[u32; 8]) -> String {
    let mut parts = vec![format!("{} changed", counts[7])];
    for (i, name) in FACETS
        .iter()
        .chain(["bindings", "unread"].iter())
        .enumerate()
    {
        if counts[i] > 0 {
            parts.push(format!("{name} {}", counts[i]));
        }
    }
    parts.join(", ")
}

/// Changed fields with old and new values, within `left` page value bytes.
fn fields(pair: &Pair, c: &Change, left: &mut usize) -> Json {
    let (Some(o), Some(n)) = (c.old, c.new) else {
        return Json::Null;
    };
    let (a, b) = (
        &pair.old.facets[o as usize].fields,
        &pair.new.facets[n as usize].fields,
    );
    let mut out = Map::new();
    for key in FIELDS.iter().flat_map(|k| k.iter()) {
        let (x, y) = (
            a.iter().find(|f| f.key == *key),
            b.iter().find(|f| f.key == *key),
        );
        if x.map(|f| f.digest) == y.map(|f| f.digest) {
            continue;
        }
        let mut value = |f: Option<&Field>| match f {
            None => json!({"present": false}),
            Some(Field { digest: None, .. }) => json!({"unread": true}),
            Some(Field { value: Some(v), .. }) => {
                let len = v.to_string().len();
                if len <= *left {
                    *left -= len;
                    json!({"value": v})
                } else {
                    json!({"truncated": true})
                }
            }
            Some(_) => json!({"truncated": true}),
        };
        let facet = FACETS[x.or(y).map_or(0, |f| f.facet) as usize];
        out.insert(
            (*key).into(),
            json!({"facet": facet, "old": value(x), "new": value(y)}),
        );
    }
    Json::Object(out)
}

/// Edge differences of a changed record: relation, target, state and evidence, old∪new.
fn edges(pair: &Pair, c: &Change) -> Json {
    let (Some(o), Some(n)) = (c.old, c.new) else {
        return Json::Null;
    };
    let list = |s: &Side, r: u32| {
        s.index.records[r as usize]
            .edges
            .iter()
            .map(|e| json!({"relation": e.relation, "target": e.target, "state": e.state, "evidence": e.evidence}))
            .collect::<Vec<_>>()
    };
    let (a, b) = (list(&pair.old, o), list(&pair.new, n));
    let removed: Vec<_> = a.iter().filter(|e| !b.contains(e)).cloned().collect();
    let added: Vec<_> = b.iter().filter(|e| !a.contains(e)).cloned().collect();
    let main = |v: &[Json]| {
        v.iter()
            .find(|e| e["relation"] == "instance_of")
            .map(|e| e["target"].clone())
    };
    let mut out = json!({"added": added, "removed": removed});
    if main(&a) != main(&b) {
        out["swap"] = json!({"old": main(&a), "new": main(&b)});
    }
    out
}

fn page(pair: &Pair, req: &Request, reused: bool) -> Result<Json, Box<dyn Error>> {
    let start = req
        .cursor
        .as_deref()
        .map(|c| offset(pair, c))
        .transpose()?
        .unwrap_or(0);
    let end = (start + req.limit).min(pair.changes.len());
    let mut left = PAGE_VALUES;
    let mut frames: Vec<Json> = Vec::new();
    let mut last = None;
    for c in &pair.changes[start..end] {
        let side = if c.new.is_some() {
            &pair.new
        } else {
            &pair.old
        };
        let rid = c.new.or(c.old).unwrap() as usize;
        let r = &side.index.records[rid];
        let mut node = json!({"id": r.id, "kind": r.kind, "name": r.name, "change": c.kind.name()});
        if c.kind == Kind::Retyped {
            node["identity"] = json!("identity_uncertain");
            node["oldKind"] = json!(pair.old.index.records[c.old.unwrap() as usize].kind);
        }
        if c.kind == Kind::Changed {
            let names: Vec<_> = FACETS
                .iter()
                .chain(["bindings"].iter())
                .enumerate()
                .filter(|(i, _)| c.facets & (1 << i) != 0)
                .map(|(_, n)| *n)
                .collect();
            node["facets"] = json!(names);
            if c.facets & UNREAD != 0 {
                node["unread"] = json!(true);
            }
            node["fields"] = fields(pair, c, &mut left);
            if c.facets & EDGES != 0 {
                node["edges"] = edges(pair, c);
            }
        }
        if c.descendants > 0 {
            node["descendants"] = json!(c.descendants);
        }
        if last != Some(c.frame) {
            let f = &pair.frames[c.frame as usize];
            let mut frame = json!({"id": f.id, "name": f.name, "page": f.page, "summary": summary(&f.counts), "nodes": []});
            if let Some(k) = f.change {
                frame["change"] = json!(k.name());
            }
            if let Some((id, basis)) = &f.possible {
                frame["possibleMatch"] =
                    json!({"id": id, "basis": basis, "identity": "possible_match"});
            }
            frames.push(frame);
            last = Some(c.frame);
        }
        frames.last_mut().unwrap()["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node);
    }
    let coverage = |s: &Side| json!({"stop": s.index.stop, "scannedRecords": s.index.records.len(), "complete": s.index.stop.is_none()});
    let unread = pair
        .changes
        .iter()
        .filter(|c| c.facets & UNREAD != 0)
        .count();
    let mut reasons = Vec::new();
    for (name, s) in [("old", &pair.old), ("new", &pair.new)] {
        if let Some(stop) = s.index.stop {
            reasons.push(json!({"reason": stop, "side": name}));
        }
    }
    if let Some(stop) = pair.stop {
        reasons.push(json!({"reason": stop}));
    }
    if unread > 0 {
        reasons.push(json!({"reason": "unread_fields", "count": unread}));
    }
    let mut source = json!({"old": pair.old.source, "new": pair.new.source, "freshness": {"reusedPair": reused}});
    source["freshness"]["hashedAtMs"] = json!(super::ms(pair.hashed));
    Ok(json!({
        "source": source,
        "node": pair.node,
        // Two caller-chosen files: ids match within this pair, never a proven file lineage.
        "matching": {"basis": "node_id+kind", "lineage": "unverified"},
        "coverage": {
            "complete": reasons.is_empty(),
            "old": coverage(&pair.old),
            "new": coverage(&pair.new),
            "changes": pair.changes.len(),
            "possibleMatches": pair.possible,
            "reasons": reasons,
        },
        "frames": frames,
        "nextCursor": (end < pair.changes.len()).then(|| cursor(pair, end)),
    }))
}

#[cfg(test)]
fn compare_trees(old: &Tree, new: &Tree, works: [usize; 2], node: &str) -> Pair {
    let build = |tree: &Tree, label: &str, work: usize| {
        let bounds = Bounds {
            work,
            bytes: BYTES,
            depth: DEPTH,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        };
        let mut facets = Vec::new();
        let index = build_with(tree, node, bounds, true, Some(&mut facets));
        Side {
            signature: format!("{}:{}", index.generation, index.stop.unwrap_or("")),
            source: json!({"sourceId": format!("local:{label}"), "snapshotId": format!("sha256:{label}")}),
            index,
            facets,
            path: PathBuf::new(),
            stamp: (SystemTime::UNIX_EPOCH, 0),
        }
    };
    compare(
        build(old, "old", works[0]),
        build(new, "new", works[1]),
        node,
    )
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
