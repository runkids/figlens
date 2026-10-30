//! Exact diff of two revisions of one scope (#81), the engine durable changes (#92) reuse.
//!
//! Each side is a full relations build that also records, per record, the effective value of a
//! fixed field list (an instance override wins over the stored field, as in usages) grouped
//! into facets. Records match by exact id and kind only; top-level frames left unmatched may be
//! paired by name or geometry as a labelled `possible_match`, which never transfers identity.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use super::{
    BYTES, DEADLINE, DEPTH, Index, REHASH, Stamp, WORK, build_with, hex, missing_scope,
    require_scope, snapshot_id, source_id, stage, stamp,
};
use crate::expand::{Bounds, Expanded, json_bytes, kiwi_bytes};
use crate::{Document, ErrorKind, FigError, Tree, guid_key, to_json};

/// Bumped whenever a field list, facet or digest stream changes; cursors carry it.
const VERSION: &str = "diff-v7";
/// Estimated heap of an expanded size or transform built as JSON (a map of at most six numbers).
const OWNED_BYTES: usize = 512;
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
        "textTruncation",
        "maxLines",
        "fontVariantNumericFigure",
        "fontVariantNumericSpacing",
        "leadingTrim",
        "hyperlink",
        // Laid-out lines and glyphs: REST line heights and drawn text read them.
        "derivedTextData",
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
        "layoutGrids",
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
        // A page's background, which page exports draw.
        "backgroundEnabled",
        "backgroundColor",
        "backgroundOpacity",
        "mask",
        "maskType",
        "visible",
        "cornerRadius",
        "rectangleCornerRadiiIndependent",
        "rectangleTopLeftCornerRadius",
        "rectangleTopRightCornerRadius",
        "rectangleBottomLeftCornerRadius",
        "rectangleBottomRightCornerRadius",
        "fillGeometry",
        "strokeGeometry",
        // An expanded layer's outlines are scaled from the source size they were drawn at.
        "fillGeometrySize",
        "strokeGeometrySize",
        "vectorData",
        // Explicit modes re-resolve the bound colors of every layer below.
        "variableModeBySetMap",
    ],
    &[
        // A component's or set's property definitions name the instances' emitted properties.
        "componentPropDefs",
        "componentPropAssignments",
        "componentPropRefs",
        "variantPropSpecs",
        // A variable's or collection's own values, types and modes.
        "variableDataValues",
        "variableResolvedType",
        "variableSetID",
        "variableSetModes",
        "variableScopes",
        "codeSyntax",
        // Emitted component, set and style metadata.
        "description",
        "symbolDescription",
        "styleType",
        // Triggers and transitions; edges hold only their destinations.
        "prototypeInteractions",
    ],
];
/// Largest value one field may scan; a larger one is `unread`, like a usages binding field.
const VISITS: usize = 1 << 16;
/// Retained value bytes: one field, one record. Over them the value is dropped and marked
/// truncated; its digest still decides whether it changed.
const FIELD_VALUE: usize = 512;
const RECORD_VALUES: usize = 4 * 1024;
/// Largest estimated field converted and digested; a larger one is `unread`. Laid-out text
/// holds about 400 bytes a glyph, so it gets room for a long text layer (`VISITS` still caps it).
const FIELD_BYTES: usize = 64 * 1024;
const DERIVED_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Changes and candidate probes one pair may hold or spend, charged before they are kept.
const PAIR_WORK: usize = 2_000_000;
const PAIR_BYTES: usize = 64 * 1024 * 1024;
/// Possible matches returned; candidate probes are charged to the pair budget.
const POSSIBLE: usize = 200;
/// Value bytes one page may carry.
const PAGE_VALUES: usize = 256 * 1024;
/// Edge visits one page may spend comparing changed records' edges, and an estimate (64 bytes
/// an edge, not a measured heap cap) of the transient sets one record's comparison builds; both
/// are checked before the sets are built.
const PAGE_WORK: usize = 1_000_000;
const EDGE_SET_BYTES: usize = 8 * 1024 * 1024;
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

/// Feeds the bytes `to_json` shows only as `<N bytes>` into `h`, in key order, so binary values
/// of equal length still differ.
fn raw_bytes(v: &kiwi_schema::Value, h: &mut Sha256) {
    use kiwi_schema::Value as K;
    match v {
        K::Bytes(b) => {
            h.update((b.len() as u64).to_le_bytes());
            h.update(b);
        }
        K::Byte(b) => h.update([*b]),
        K::Array(a) => {
            h.update((a.len() as u64).to_le_bytes());
            a.iter().for_each(|x| raw_bytes(x, h));
        }
        K::Object(_, o) => {
            let mut keys: Vec<_> = o.iter().filter(|(k, _)| !UNCOMPARED.contains(k)).collect();
            keys.sort_by_key(|(k, _)| *k);
            keys.into_iter().for_each(|(_, x)| raw_bytes(x, h));
        }
        _ => {}
    }
}

/// Blob indexes in geometry, glyphs and vector networks: numbers local to one file, so the digest takes
/// the bytes they name instead.
const BLOB_KEYS: [&str; 2] = ["commandsBlob", "vectorNetworkBlob"];
/// Bytes no output reads, left out of the comparison: the digest of the font file a text layout
/// was measured with (`derivedTextData.fontMetaData[].fontDigest`). A re-derived instance holds
/// it only as a `<N bytes>` placeholder, which would otherwise make every such text unread.
const UNCOMPARED: [&str; 1] = ["fontDigest"];

/// `v` without its blob indexes, pushed to `refs` in key order, or its `UNCOMPARED` keys.
fn unblob(v: &Json, refs: &mut Vec<u64>) -> Json {
    match v {
        Json::Array(a) => Json::Array(a.iter().map(|x| unblob(x, refs)).collect()),
        Json::Object(o) => Json::Object(
            o.iter()
                .filter_map(|(k, x)| match BLOB_KEYS.contains(&k.as_str()) {
                    true => {
                        refs.push(x.as_u64().unwrap_or(u64::MAX));
                        None
                    }
                    false if UNCOMPARED.contains(&k.as_str()) => None,
                    false => Some((k.clone(), unblob(x, refs))),
                })
                .collect(),
        ),
        _ => v.clone(),
    }
}

/// `value`'s estimated bytes and the visits that took, walking at most `VISITS` or `work`; a
/// walk that used its whole allowance stopped there.
fn estimate(value: &Value, work: usize) -> (usize, usize) {
    let allowance = VISITS.min(work);
    let mut left = allowance;
    let bytes = match value {
        Value::Json(v) => json_bytes(v, &mut left),
        Value::Owned(v) => json_bytes(v, &mut left),
        Value::Kiwi(v) => kiwi_bytes(v, &mut left),
    };
    (bytes, allowance - left)
}

/// Whether `v` holds a `to_json` bytes placeholder, whose contents are gone; an `UNCOMPARED`
/// key's does not count, as `unblob` leaves it out.
fn placeholder(v: &Json) -> bool {
    match v {
        Json::String(s) => s.starts_with('<') && s.ends_with(" bytes>"),
        Json::Array(a) => a.iter().any(placeholder),
        Json::Object(o) => o
            .iter()
            .any(|(k, x)| !UNCOMPARED.contains(&k.as_str()) && placeholder(x)),
        _ => false,
    }
}

/// Bytes a kept field's record takes besides its key and value.
const FIELD_META: usize = std::mem::size_of::<Field>();

/// Bytes a kept value of serialized length `len` holds as JSON, which is larger than its text.
/// It is at most `FIELD_VALUE` long, so its walk is short.
fn held_value(v: &Json, len: usize) -> usize {
    json_bytes(v, &mut usize::MAX.clone()).max(len)
}

/// Hashes what is written and counts it, keeping nothing. A visit per started 64 bytes of the
/// whole stream is charged to `bounds` before each chunk is hashed, since escaping can make the
/// stream longer than its estimate; a chunk it cannot pay for fails the write. Charging the
/// stream's total, not each chunk, keeps small writes from paying more than one large one.
struct Digesting<'b> {
    hash: Sha256,
    len: usize,
    bounds: &'b mut Bounds,
}

impl std::io::Write for Digesting<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let visits = (self.len + buf.len()).div_ceil(64) - self.len.div_ceil(64);
        if !self.bounds.charge(visits, 0) {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        self.hash.update(buf);
        self.len += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
            // A geometry size only scales outlines, so a layer without that outline has none.
            if let Some(outline) = key.strip_suffix("Size").filter(|k| k.ends_with("Geometry"))
                && e.and_then(|e| e.props.get(outline)).is_none()
                && node.and_then(|n| n.get(outline)).is_none()
            {
                continue;
            }
            // An expanded size or transform is built here, so its heap is paid for first.
            let built = matches!((e, *key), (Some(e), "size") if e.size.is_some())
                || (e.is_some() && *key == "transform");
            if built && !bounds.charge(0, OWNED_BYTES) {
                return None;
            }
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
            let (size, walked) = estimate(&value, bounds.work);
            // Running out of the side's work stops the side, visits kept; only a walk that
            // reached `VISITS` makes this one field unread.
            if walked == bounds.work && walked < VISITS {
                bounds.charge(walked, 0);
                bounds.charge(1, 0);
                return None;
            }
            // Converting and hashing is paid for before it happens: a visit per 64 bytes.
            let cap = match *key {
                "derivedTextData" => DERIVED_TEXT_BYTES,
                _ => FIELD_BYTES,
            };
            let unread = walked == VISITS || size > cap;
            if !bounds.charge(walked + if unread { 0 } else { size / 64 }, 0) {
                return None;
            }
            // An override's bytes were already shown as `<N bytes>`; equal lengths prove nothing.
            // An unread value is never scanned for one.
            let gone = !unread
                && match &value {
                    Value::Json(v) => placeholder(v),
                    Value::Owned(v) => placeholder(v),
                    Value::Kiwi(_) => false,
                };
            // A field kept unread retains its metadata, charged as a read one's is.
            if unread || gone {
                if !bounds.charge(0, FIELD_META + key.len()) {
                    return None;
                }
                fields.push(Field {
                    key,
                    facet: facet as u8,
                    digest: None,
                    value: None,
                });
                continue;
            }
            // Converting a stored value, and a geometry field's unblobbed copy, each allocate
            // about its estimate; both are paid for first.
            let blobbed = [
                "fillGeometry",
                "strokeGeometry",
                "vectorData",
                "derivedTextData",
            ]
            .contains(key);
            let copies = usize::from(matches!(value, Value::Kiwi(_))) + usize::from(blobbed);
            if !bounds.charge(0, copies * size) {
                return None;
            }
            let owned;
            let json = match &value {
                Value::Json(v) => *v,
                Value::Owned(v) => v,
                Value::Kiwi(v) => {
                    owned = to_json(v);
                    &owned
                }
            };
            let mut refs = Vec::new();
            let unblobbed;
            let json = match blobbed {
                true => {
                    unblobbed = unblob(json, &mut refs);
                    &unblobbed
                }
                false => json,
            };
            // Each named blob is hashed in place, never converted, read as `Tree::blob` reads
            // it: compact bytes (a visit per 64) or an array of bytes (a visit per element,
            // charged before it is checked). A missing or malformed blob, or one needing more
            // than `VISITS`, makes the field unread.
            let mut blobs = Vec::with_capacity(refs.len());
            let mut large = false;
            for i in refs {
                use kiwi_schema::Value as K;
                let b = tree.blobs.get(i as usize).and_then(|b| b.get("bytes"));
                let cost = match b {
                    Some(K::Bytes(x)) => x.len() / 64,
                    Some(K::Array(x)) => x.len(),
                    _ => usize::MAX,
                };
                large |= cost > VISITS;
                if !bounds.charge(1 + if large { 0 } else { cost }, 0) {
                    return None;
                }
                // `||` keeps an array past the cap from being walked.
                large = large
                    || matches!(b, Some(K::Array(x)) if !x.iter().all(|v| matches!(v, K::Byte(_))));
                if large {
                    break;
                }
                blobs.extend(b);
            }
            if large {
                if !bounds.charge(0, FIELD_META + key.len()) {
                    return None;
                }
                fields.push(Field {
                    key,
                    facet: facet as u8,
                    digest: None,
                    value: None,
                });
                continue;
            }
            // Streamed into the digest; only the first `FIELD_VALUE` bytes are ever buffered.
            let mut w = Digesting {
                hash: Sha256::new(),
                len: 0,
                bounds,
            };
            w.hash.update(VERSION.as_bytes());
            w.hash.update(key.as_bytes());
            w.hash.update([0]);
            serde_json::to_writer(&mut w, json).ok()?;
            let Digesting { mut hash, len, .. } = w;
            hash.update((len as u64).to_le_bytes());
            // Only bytes `to_json` dropped are added, so a stored value and an equal override
            // digest alike.
            if let (Value::Kiwi(v), true) = (&value, placeholder(json)) {
                raw_bytes(v, &mut hash);
            }
            // Bytes and an array of bytes hash alike: their length, then each byte.
            blobs.into_iter().for_each(|b| raw_bytes(b, &mut hash));
            let fits = len <= FIELD_VALUE && kept + len <= RECORD_VALUES;
            let held = FIELD_META + key.len() + if fits { held_value(json, len) } else { 0 };
            if !bounds.charge(0, held) {
                return None;
            }
            if fits {
                kept += len;
            }
            fields.push(Field {
                key,
                facet: facet as u8,
                digest: Some(hash.finalize().into()),
                value: fits.then(|| json.clone()),
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
    /// Whether the selected node exists on this side; a side without it is complete and empty.
    present: bool,
    /// The selected node's own parent and position among its siblings, which the index's
    /// root record leaves out; `Err` when it could not be verified within the side's bounds.
    origin: Result<Option<(String, u32)>, ()>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Added,
    Removed,
    Changed,
    /// Same id on both sides but another layer type: identity is not proven either way.
    Retyped,
    /// Present on one side while the other stopped before reaching it, or on both with a
    /// compared field unread and no proven difference.
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
    /// Changes with a field unread on either side, counted once when the pair is built.
    unread: usize,
    /// Why the pair itself stopped short (`diff_work_limit`).
    stop: Option<&'static str>,
    /// Per side, style and variable uses whose definition lies outside the compared records.
    outside: [usize; 2],
    hashed: SystemTime,
    /// Digest of the retained ordered result, which cursors bind (`result`).
    result: String,
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
            if !old.present && !new.present {
                return Err(missing_scope(&req.node));
            }
            let pair = Arc::new(compare(old, new, &req.node, PAIR_WORK));
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
    // Instance sublayers are never scopes; a stored node missing here only empties this side.
    let present = tree.nodes.contains_key(node);
    if node.starts_with('I') {
        require_scope(&tree, node)?;
    }
    let mut bounds = Bounds {
        work: WORK,
        bytes: BYTES,
        depth: DEPTH,
        deadline,
        stop: None,
    };
    let origin = origin(&tree, node, &mut bounds);
    let mut facets = Vec::new();
    let index = match present {
        true => build_with(&tree, node, bounds, true, Some(&mut facets)),
        false => Index::empty(),
    };
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
        present,
        origin,
    })
}

/// Stored node `node`'s parent and position among its siblings, charged to `bounds` before
/// the parent id is copied and the siblings scanned; `Err` when out of bounds or not found.
fn origin(tree: &Tree, node: &str, bounds: &mut Bounds) -> Result<Option<(String, u32)>, ()> {
    let Some(guid) = tree
        .nodes
        .get(node)
        .and_then(|n| n.get("parentIndex")?.get("guid"))
    else {
        return Ok(None);
    };
    if !bounds.charge(1, 64) {
        return Err(());
    }
    let parent = guid_key(guid).ok_or(())?;
    let siblings = tree.children.get(&parent).map_or(&[][..], Vec::as_slice);
    if !bounds.charge(siblings.len(), 0) {
        return Err(());
    }
    let at = siblings.iter().position(|c| c == node).ok_or(())?;
    Ok(Some((parent, at as u32)))
}

/// A record's structure digest input: name, kind, parent id, order among siblings and hidden.
fn structure<'a>(
    side: &'a Side,
    rid: usize,
    order: &[u32],
) -> (&'a str, &'a str, Option<&'a str>, u32, bool) {
    let index = &side.index;
    let r = &index.records[rid];
    // The selected root has no indexed parent; its stored one stands in.
    let (parent, at) = match (r.parent, &side.origin) {
        (Some(p), _) => (Some(index.records[p as usize].id.as_str()), order[rid]),
        (None, Ok(Some((p, at)))) => (Some(p.as_str()), *at),
        (None, _) => (None, order[rid]),
    };
    (r.name.as_str(), r.kind.as_str(), parent, at, r.hidden)
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

fn edge_keys(s: &Side, r: usize) -> impl Iterator<Item = (&str, Option<&str>, &str, &str)> {
    s.index.records[r].edges.iter().map(edge_key)
}

/// Whether layer `rid` of `from`, missing on `side`, may be content `side` left unexpanded
/// below a hidden layer rather than a deletion. Out of budget counts as may.
fn unexpanded(
    side: &Side,
    ids: &HashMap<&str, usize>,
    from: &Side,
    rid: usize,
    budget: &mut Bounds,
) -> bool {
    if side.index.hidden_content == 0 {
        return false;
    }
    // The nearest ancestor present on `side`, then whether it or one above it is hidden there.
    let mut up = from.index.records[rid].parent;
    let mut at = loop {
        let Some(p) = up else { return false };
        if !budget.charge(1, 0) {
            return true;
        }
        let r = &from.index.records[p as usize];
        if let Some(&i) = ids.get(r.id.as_str()) {
            break Some(i as u32);
        }
        up = r.parent;
    };
    while let Some(i) = at {
        if !budget.charge(1, 0) {
            return true;
        }
        let r = &side.index.records[i as usize];
        if r.hidden {
            return true;
        }
        at = r.parent;
    }
    false
}

fn in_facet(f: &[Field], facet: u8) -> impl Iterator<Item = (&str, Option<[u8; 32]>)> {
    f.iter()
        .filter(move |x| x.facet == facet)
        .map(|x| (x.key, x.digest))
}

/// The facet bits on which two records of the same id and kind differ.
fn differs(old: &Side, o: usize, oo: &[u32], new: &Side, n: usize, no: &[u32]) -> u8 {
    let mut bits = 0;
    let (a, b) = (&old.facets[o].fields, &new.facets[n].fields);
    if a.iter().chain(b).any(|x| x.digest.is_none()) {
        bits |= UNREAD;
    }
    // A facet differs only where that is proven: a field present on one side only, or read on
    // both with other digests. An unread value proves nothing either way.
    for facet in 0..FIELDS.len() as u8 {
        let proven = in_facet(a, facet).count() != in_facet(b, facet).count()
            || in_facet(a, facet)
                .zip(in_facet(b, facet))
                .any(|(x, y)| x.0 != y.0 || matches!((x.1, y.1), (Some(p), Some(q)) if p != q));
        if proven {
            bits |= 1 << facet;
        }
    }
    if structure(old, o, oo) != structure(new, n, no) {
        bits |= 1 << 4;
    }
    if !edge_keys(old, o).eq(edge_keys(new, n)) {
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
        // A second visit for each change and frame pays for hashing it into `result`.
        if !self.budget.charge(2, 96) {
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
                .charge(2, 128 + id.len() + name.len() + page.len())
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

fn compare(old: Side, new: Side, node: &str, work: usize) -> Pair {
    let mut c = Collect {
        budget: Bounds {
            work,
            bytes: PAIR_BYTES,
            depth: DEPTH,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        },
        frames: Vec::new(),
        at: HashMap::new(),
        changes: Vec::new(),
    };
    // Sibling orders and id maps of both sides are paid for before they are built.
    let records = old.index.records.len() + new.index.records.len();
    if !c.budget.charge(records, records * 48) {
        return Pair {
            old,
            new,
            node: node.to_owned(),
            changes: Vec::new(),
            frames: Vec::new(),
            possible: 0,
            unread: 0,
            stop: Some("diff_work_limit"),
            outside: [0; 2],
            hashed: SystemTime::now(),
            result: String::new(),
        };
    }
    let (oo, no) = (orders(&old.index), orders(&new.index));
    let (old_ids, new_ids) = (by_id(&old), by_id(&new));
    // Removed and added subtrees are reported at their top record, counting what lies below;
    // the selected node's children are always listed, so a selection drills into a subtree.
    let mut reported: HashMap<(bool, usize), usize> = HashMap::new();
    let below = |reported: &HashMap<(bool, usize), usize>, new: bool, parent: Option<u32>| {
        parent
            .filter(|&p| p != 0)
            .and_then(|p| reported.get(&(new, p as usize)).copied())
    };
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
                let work = 1
                    + old.facets[o].fields.len()
                    + new.facets[n].fields.len()
                    + r.edges.len()
                    + new.index.records[n].edges.len();
                c.budget.charge(work, 0) && {
                    // A record with a field unread and no proven difference is unknown: listed,
                    // never taken for unchanged nor reported as changed.
                    let bits = differs(&old, o, &oo, &new, n, &no);
                    let kind = match bits {
                        UNREAD => Kind::Unknown,
                        _ => Kind::Changed,
                    };
                    bits == 0
                        || c.push(
                            &new,
                            &new_ids,
                            n,
                            (Some(o as u32), Some(n as u32)),
                            kind,
                            bits,
                        )
                }
            }
            None => match below(&reported, false, r.parent) {
                Some(top) => {
                    c.budget.charge(1, 32) && {
                        c.changes[top].descendants += 1;
                        reported.insert((false, o), top);
                        true
                    }
                }
                None => {
                    let kind = if new.index.stop.is_none()
                        && !unexpanded(&new, &new_ids, &old, o, &mut c.budget)
                    {
                        Kind::Removed
                    } else {
                        Kind::Unknown
                    };
                    c.push(&old, &old_ids, o, (Some(o as u32), None), kind, 0)
                        && c.budget.charge(1, 32)
                        && {
                            reported.insert((false, o), c.changes.len() - 1);
                            true
                        }
                }
            },
        };
        if !kept {
            full = false;
            break;
        }
    }
    for (n, r) in new.index.records.iter().enumerate().filter(|_| full) {
        if !c.budget.charge(1, 0) {
            full = false;
            break;
        }
        if old_ids.contains_key(r.id.as_str()) {
            continue;
        }
        let kept = match below(&reported, true, r.parent) {
            Some(top) => {
                c.budget.charge(1, 32) && {
                    c.changes[top].descendants += 1;
                    reported.insert((true, n), top);
                    true
                }
            }
            None => {
                let kind = if old.index.stop.is_none()
                    && !unexpanded(&old, &old_ids, &new, n, &mut c.budget)
                {
                    Kind::Added
                } else {
                    Kind::Unknown
                };
                c.push(&new, &new_ids, n, (None, Some(n as u32)), kind, 0)
                    && c.budget.charge(1, 32)
                    && {
                        reported.insert((true, n), c.changes.len() - 1);
                        true
                    }
            }
        };
        if !kept {
            full = false;
            break;
        }
    }
    let mut outside = [0; 2];
    for (i, (s, ids)) in [(&old, &old_ids), (&new, &new_ids)].into_iter().enumerate() {
        if !full {
            break;
        }
        match outside_uses(s, ids, &mut c.budget) {
            Some(n) => outside[i] = n,
            None => full = false,
        }
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
    let unread = changes.iter().filter(|c| c.facets & UNREAD != 0).count();
    // Pages list changes frame by frame, in first-seen frame order.
    changes.sort_by_key(|c| c.frame);
    let mut pair = Pair {
        old,
        new,
        node: node.to_owned(),
        changes,
        frames,
        possible,
        unread,
        stop,
        outside,
        hashed: SystemTime::now(),
        result: String::new(),
    };
    pair.result = result(&pair);
    pair
}

/// Style, variable and main component uses on `s` whose definition is not one of its records:
/// its values, or a main's property definitions, may change while every compared record stays
/// the same. Each record and edge is charged before
/// it is read.
fn outside_uses(s: &Side, ids: &HashMap<&str, usize>, budget: &mut Bounds) -> Option<usize> {
    let mut n = 0;
    for r in &s.index.records {
        if !budget.charge(1, 0) {
            return None;
        }
        for e in &r.edges {
            if !budget.charge(1, 0) {
                return None;
            }
            if let ("uses_style" | "binds_variable" | "instance_of", Some(t)) =
                (e.relation, e.target.as_deref())
                && !matches!(e.state, "missing" | "unresolved")
            {
                n += usize::from(!ids.contains_key(t.strip_prefix("VariableID:").unwrap_or(t)));
            }
        }
    }
    Some(n)
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
    // Candidates per key, with how far they were scanned: each is probed once in all.
    let mut by: HashMap<(String, String), (Vec<usize>, usize)> = HashMap::new();
    for (i, f) in frames.iter().enumerate() {
        if f.change == Some(Kind::Added) {
            if !budget.charge(2, 192 + 2 * (f.page.len() + f.name.len())) {
                return 0;
            }
            by.entry((f.page.clone(), format!("n:{}", f.name)))
                .or_default()
                .0
                .push(i);
            if let Some(g) = geometry(new, &f.id) {
                by.entry((f.page.clone(), format!("g:{g}")))
                    .or_default()
                    .0
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
        // The lookup keys are copies of the page and name, paid for before they are made.
        let key_bytes = 192 + 2 * (frames[i].page.len() + frames[i].name.len());
        if !budget.charge(2, key_bytes) {
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
            let mut hit = None;
            if let Some((v, next)) = by.get_mut(&(page.clone(), key)) {
                while *next < v.len() {
                    if !budget.charge(1, 0) {
                        return found;
                    }
                    let j = v[*next];
                    *next += 1;
                    if frames[j].possible.is_none() {
                        hit = Some(j);
                        break;
                    }
                }
            }
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

/// The retained ordered result a cursor's offset indexes into: a rebuild over the same sides that
/// stopped elsewhere (the pair's own deadline or work) can change it.
fn result(pair: &Pair) -> String {
    let mut h = Sha256::new();
    let n = |v: u64| v.to_le_bytes();
    let text = |h: &mut Sha256, t: &str| {
        h.update(n(t.len() as u64));
        h.update(t);
    };
    // Each list is prefixed with its domain and count, so its rows cannot run into the next.
    h.update(b"changes");
    h.update(n(pair.changes.len() as u64));
    for c in &pair.changes {
        h.update(n(c.frame.into()));
        h.update(n(c.old.map_or(u64::MAX, u64::from)));
        h.update(n(c.new.map_or(u64::MAX, u64::from)));
        h.update([c.kind as u8, c.facets]);
        h.update(n(c.descendants.into()));
    }
    h.update(b"frames");
    h.update(n(pair.frames.len() as u64));
    for f in &pair.frames {
        text(&mut h, &f.id);
        h.update([f.change.map_or(u8::MAX, |k| k as u8)]);
        f.counts.iter().for_each(|c| h.update(n((*c).into())));
        match &f.possible {
            Some((id, basis)) => {
                h.update([1]);
                text(&mut h, id);
                text(&mut h, basis);
            }
            None => h.update([0]),
        }
    }
    h.update(n(pair.possible as u64));
    h.update(n(pair.unread as u64));
    text(&mut h, pair.stop.unwrap_or(""));
    pair.outside.iter().for_each(|o| h.update(n(*o as u64)));
    hex(&h.finalize())
}

fn binding(pair: &Pair) -> String {
    let s = |s: &Side| {
        format!(
            "{}|{}|{}",
            s.source["sourceId"], s.source["snapshotId"], s.signature
        )
    };
    format!(
        "{VERSION}|{}|{}|{}|{}",
        s(&pair.old),
        s(&pair.new),
        pair.node,
        pair.result
    )
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
    let mut parts = vec![format!("{} listed", counts[7])];
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

/// Page bytes one listed field takes besides its key and values: its facet and markers.
const FIELD_ENVELOPE: usize = 96;

/// Changed fields with old and new values, within `left` page value bytes; `None` when even
/// their keys and markers do not fit.
fn fields(pair: &Pair, c: &Change, left: &mut usize) -> Option<Json> {
    let (Some(o), Some(n)) = (c.old, c.new) else {
        return Some(Json::Null);
    };
    let (a, b) = (
        &pair.old.facets[o as usize].fields,
        &pair.new.facets[n as usize].fields,
    );
    let differing: Vec<_> = FIELDS
        .iter()
        .flat_map(|k| k.iter())
        .map(|key| {
            let x = a.iter().find(|f| f.key == *key);
            (key, x, b.iter().find(|f| f.key == *key))
        })
        .filter(|(_, x, y)| x.map(|f| f.digest) != y.map(|f| f.digest))
        .collect();
    let envelope: usize = differing
        .iter()
        .map(|(k, ..)| FIELD_ENVELOPE + k.len())
        .sum();
    *left = left.checked_sub(envelope)?;
    let mut out = Map::new();
    for (key, x, y) in differing {
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
    Some(Json::Object(out))
}

/// The length of `s` as a JSON string, quotes included, as serde_json escapes it: control
/// characters grow to six bytes, so a name's raw length can understate its reply size sixfold.
fn escaped(s: &str) -> usize {
    let grown: usize = s
        .bytes()
        .map(|b| match b {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 1,
            0..=0x1f => 5,
            _ => 0,
        })
        .sum();
    s.len() + grown + 2
}

fn edge_key(e: &super::usages::Edge) -> (&str, Option<&str>, &str, &str) {
    (
        e.relation,
        e.target.as_deref(),
        e.state,
        e.evidence.as_str(),
    )
}

/// Edge differences of a changed record: relation, target, state and evidence, old minus new
/// and new minus old, listed while they fit `left` page bytes; the rest are counted.
/// `None` when comparing them would pass the page's remaining `work` or `EDGE_SET_BYTES`.
fn edges(pair: &Pair, c: &Change, left: &mut usize, work: &mut usize) -> Option<Json> {
    let (Some(o), Some(n)) = (c.old, c.new) else {
        return Some(Json::Null);
    };
    let (a, b) = (
        &pair.old.index.records[o as usize].edges,
        &pair.new.index.records[n as usize].edges,
    );
    let count = a.len() + b.len();
    if count > *work || count * 64 > EDGE_SET_BYTES {
        return None;
    }
    *work -= count;
    let (sa, sb): (HashSet<_>, HashSet<_>) = (
        a.iter().map(edge_key).collect(),
        b.iter().map(edge_key).collect(),
    );
    let mut out = json!({"added": [], "removed": []});
    let mut omitted = 0;
    for (name, list, other) in [("removed", a, &sb), ("added", b, &sa)] {
        for e in list.iter().filter(|e| !other.contains(&edge_key(e))) {
            let cost = 64
                + escaped(e.relation)
                + e.target.as_deref().map_or(4, escaped)
                + escaped(e.state)
                + escaped(&e.evidence);
            if cost > *left {
                omitted += 1;
                continue;
            }
            *left -= cost;
            out[name].as_array_mut().unwrap().push(
                json!({"relation": e.relation, "target": e.target, "state": e.state, "evidence": e.evidence}),
            );
        }
    }
    if omitted > 0 {
        out["truncated"] = json!(omitted);
    }
    let main = |v: &[super::usages::Edge]| {
        v.iter()
            .find(|e| e.relation == "instance_of")
            .and_then(|e| e.target.clone())
    };
    if main(a) != main(b) {
        let side = |m: &Option<String>| m.as_deref().map_or(4, escaped);
        *left = left.checked_sub(32 + side(&main(a)) + side(&main(b)))?;
        out["swap"] = json!({"old": main(a), "new": main(b)});
    }
    Some(out)
}

fn page(pair: &Pair, req: &Request, reused: bool) -> Result<Json, Box<dyn Error>> {
    page_with(pair, req, reused, [PAGE_VALUES, PAGE_WORK])
}

/// `page` within `budget`: page bytes and edge visits.
fn page_with(
    pair: &Pair,
    req: &Request,
    reused: bool,
    budget: [usize; 2],
) -> Result<Json, Box<dyn Error>> {
    let start = req
        .cursor
        .as_deref()
        .map(|c| offset(pair, c))
        .transpose()?
        .unwrap_or(0);
    let [mut left, mut work] = budget;
    let mut frames: Vec<Json> = Vec::new();
    let mut last = None;
    let mut end = start;
    for c in &pair.changes[start..(start + req.limit).min(pair.changes.len())] {
        let side = if c.new.is_some() {
            &pair.new
        } else {
            &pair.old
        };
        let rid = c.new.or(c.old).unwrap() as usize;
        let r = &side.index.records[rid];
        // Names and frame headers count against the page too; a full page ends early and the
        // cursor resumes at the next change, so none is dropped.
        let f = &pair.frames[c.frame as usize];
        let header = match (last == Some(c.frame), &f.possible) {
            (true, _) => 0,
            (false, p) => {
                let possible = p
                    .as_ref()
                    .map_or(0, |(id, basis)| 64 + escaped(id) + escaped(basis));
                192 + escaped(&f.id) + escaped(&f.name) + escaped(&f.page) + possible
            }
        };
        let old_kind = match c.kind {
            Kind::Retyped => 32 + escaped(&pair.old.index.records[c.old.unwrap() as usize].kind),
            _ => 0,
        };
        let cost = 96 + escaped(&r.id) + escaped(&r.kind) + escaped(&r.name) + header + old_kind;
        if cost > left {
            if end > start {
                break;
            }
            return Err(FigError::new(
                ErrorKind::ResponseBudget,
                format!("change {} needs more than {PAGE_VALUES} bytes for its name and frame; inspect it on each side with `figlens inspect`", r.id),
            )
            .into());
        }
        left -= cost;
        // A changed record whose edges do not fit this page ends it before it, or fails when it
        // is the first: never listed without them.
        let edge_list = match c.kind == Kind::Changed && c.facets & EDGES != 0 {
            true => match edges(pair, c, &mut left, &mut work) {
                Some(e) => Some(e),
                None if end > start => break,
                None => {
                    return Err(FigError::new(
                        ErrorKind::QueryWorkLimit,
                        format!("the edges of change {} are too many to compare in one page; list them on each side with `figlens usages <file> {} --out`", r.id, r.id),
                    )
                    .into())
                }
            },
            false => None,
        };
        // So do its fields' keys and markers; values that do not fit are marked truncated.
        let field_list = match c.kind == Kind::Changed || c.facets & UNREAD != 0 {
            true => match fields(pair, c, &mut left) {
                Some(f) => Some(f),
                None if end > start => break,
                None => {
                    return Err(FigError::new(
                        ErrorKind::ResponseBudget,
                        format!("the changed fields of {} need more than {PAGE_VALUES} bytes; inspect it on each side with `figlens inspect`", r.id),
                    )
                    .into())
                }
            },
            false => None,
        };
        end += 1;
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
            node["fields"] = field_list.unwrap_or_default();
            if let Some(e) = edge_list {
                node["edges"] = e;
            }
        } else if c.facets & UNREAD != 0 {
            node["unread"] = json!(true);
            node["fields"] = field_list.unwrap_or_default();
        }
        if c.descendants > 0 {
            node["descendants"] = json!(c.descendants);
        }
        if last != Some(c.frame) {
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
    // Links the index could not verify or read, and layers below hidden ones it never expanded,
    // may hide changes; each side counts them.
    let gaps = |s: &Side| {
        (
            s.index.unverified_links + s.index.unread_fields,
            s.index.hidden_content,
        )
    };
    let known = |s: &Side| s.origin.is_ok();
    let coverage = |s: &Side, outside: usize| json!({"stop": s.index.stop, "scannedRecords": s.index.records.len(), "complete": s.index.stop.is_none() && gaps(s) == (0, 0) && known(s) && outside == 0, "selected": s.present});
    let unread = pair.unread;
    let mut reasons = Vec::new();
    for (name, s, outside) in [
        ("old", &pair.old, pair.outside[0]),
        ("new", &pair.new, pair.outside[1]),
    ] {
        if let Some(stop) = s.index.stop {
            reasons.push(json!({"reason": stop, "side": name}));
        }
        let (links, hidden) = gaps(s);
        if links > 0 {
            reasons.push(json!({"reason": "relation_unverified", "side": name, "count": links}));
        }
        if hidden > 0 {
            reasons.push(
                json!({"reason": "hidden_content_not_expanded", "side": name, "count": hidden}),
            );
        }
        if !known(s) {
            reasons.push(json!({"reason": "selection_context_unknown", "side": name}));
        }
        if outside > 0 {
            reasons
                .push(json!({"reason": "definition_not_compared", "side": name, "count": outside}));
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
            "old": coverage(&pair.old, pair.outside[0]),
            "new": coverage(&pair.new, pair.outside[1]),
            "changes": pair.changes.len(),
            "possibleMatches": pair.possible,
            "reasons": reasons,
        },
        "frames": frames,
        "nextCursor": (end < pair.changes.len()).then(|| cursor(pair, end)),
    }))
}

#[cfg(test)]
fn compare_trees(old: &Tree, new: &Tree, works: [usize; 3], node: &str) -> Pair {
    let build = |tree: &Tree, label: &str, work: usize| {
        let mut bounds = Bounds {
            work,
            bytes: BYTES,
            depth: DEPTH,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        };
        let origin = origin(tree, node, &mut bounds);
        let mut facets = Vec::new();
        let present = tree.nodes.contains_key(node);
        let index = match present {
            true => build_with(tree, node, bounds, true, Some(&mut facets)),
            false => Index::empty(),
        };
        Side {
            present,
            signature: format!("{}:{}", index.generation, index.stop.unwrap_or("")),
            source: json!({"sourceId": format!("local:{label}"), "snapshotId": format!("sha256:{label}")}),
            index,
            facets,
            path: PathBuf::new(),
            stamp: (SystemTime::UNIX_EPOCH, 0),
            origin,
        }
    };
    compare(
        build(old, "old", works[0]),
        build(new, "new", works[1]),
        node,
        works[2],
    )
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
