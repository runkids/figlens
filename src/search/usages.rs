//! Exact relations of one index and the `usages` query over them (issue #91). Edges are
//! extracted while the search index is built, from the same expansion: every edge names its
//! location record, relation, target, resolution state, basis and evidence. Similarity, names
//! and coordinates never make an edge.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;

use serde_json::{Map, Value as Json, json};

use super::{Index, Record, kiwi_name, short_hash};
use crate::expand::{Bounds, Expanded, json_bytes, kiwi_bytes, parent_of, plugin_type};
use crate::{ErrorKind, FigError, Tree, guid_key, to_json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Extraction version: a stored generation built with another one is rebuilt.
pub(crate) const RELATIONS: &str = "relations-2";
/// Relations extracted, in key order; `library_identity` (#93) is not.
pub(crate) const KINDS: [&str; 7] = [
    "binds_variable",
    "contains",
    "instance_of",
    "prototype_target",
    "uses_image",
    "uses_style",
    "variant_of",
];
/// `resolved`: the target exists in this file with the expected type. `missing`: a recorded id
/// absent from the file. `unresolved`: no usable target id. `unsupported`: a target of the wrong
/// type or a field figlens cannot read. `external`: a library identity this file only names.
/// `recorded`: an exact identity figlens does not check further (an image hash).
pub(crate) const STATES: [&str; 6] = [
    "resolved",
    "missing",
    "unresolved",
    "unsupported",
    "external",
    "recorded",
];
pub(crate) const BASES: [&str; 2] = ["recorded", "resolved"];
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;
/// Most component-set members one query lists and follows.
pub(crate) const MAX_MEMBERS: usize = 200;
const MAX_ID_BYTES: usize = 1024;
/// Visits one field size estimate may take.
const VISITS: usize = 1 << 16;
const FIELDS: [&str; 5] = [
    "fillPaints",
    "strokePaints",
    "effects",
    "parameterConsumptionMap",
    "variableConsumptionMap",
];
/// Fields whose `styleOverrideTable` holds per-range (text) or per-region (vector) paints.
const STYLED: [&str; 2] = ["textData", "vectorData"];
const STYLES: [&str; 5] = [
    "styleIdForText",
    "styleIdForFill",
    "styleIdForStrokeFill",
    "styleIdForEffect",
    "styleIdForGrid",
];

/// One relation from a location record to its target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edge {
    pub relation: &'static str,
    /// Orders several edges of one relation on one record.
    pub seq: u32,
    pub target: Option<String>,
    pub state: &'static str,
    pub basis: &'static str,
    /// The field or binding that established it.
    pub evidence: String,
}

/// What an index knows about an edge target that may lie outside its scope.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Definition {
    pub name: String,
    pub kind: String,
    /// The COMPONENT_SET a COMPONENT belongs to.
    pub set: Option<String>,
    pub page: Option<String>,
}

// ---- Extraction ----

/// The edges of one record, `contains` excluded (each record's `parent` is that edge). `None`
/// when `bounds` ran out first. Prototype links the record inherits from its component
/// definition are not verified to apply, so they are counted in `unverified`, not kept; a
/// binding field too large to read is counted in `unread`.
pub(crate) fn extract(
    tree: &Tree,
    id: &str,
    kind: &str,
    e: Option<&Expanded>,
    bounds: &mut Bounds,
    unverified: &mut usize,
    unread: &mut usize,
) -> Option<Vec<Edge>> {
    let mut out: Vec<Edge> = Vec::new();
    let mut push = |bounds: &mut Bounds,
                    relation: &'static str,
                    target: Option<String>,
                    state: &'static str,
                    basis: &'static str,
                    evidence: String| {
        // A target longer than any stored id cannot be kept exactly; it stays explicit.
        let (target, state) = match target {
            Some(t) if t.len() > super::store::ID_BYTES => (None, "unsupported"),
            t => (t, state),
        };
        let bytes = 96 + target.as_ref().map_or(0, String::len) + evidence.len();
        if !bounds.charge(1, bytes) {
            return false;
        }
        // Each relation's edges are pushed together, so its seq follows the last edge's.
        let seq = out
            .last()
            .filter(|x| x.relation == relation)
            .map_or(0, |x| x.seq + 1);
        out.push(Edge {
            relation,
            seq,
            target,
            state,
            basis,
            evidence,
        });
        true
    };
    if kind == "INSTANCE" {
        let (target, basis, evidence) = match e {
            None => (
                tree.nodes
                    .get(id)
                    .and_then(|n| n.get("symbolData")?.get("symbolID").and_then(guid_key)),
                "recorded",
                "symbolData",
            ),
            Some(e) => (
                e.main.as_ref().map(|m| m.0.clone()),
                "resolved",
                e.main.as_ref().map_or("definition", |m| m.1),
            ),
        };
        let state = node_state(tree, target.as_deref(), "COMPONENT");
        if !push(bounds, "instance_of", target, state, basis, evidence.into()) {
            return None;
        }
    }
    if kind == "COMPONENT"
        && e.is_none()
        && let Some(set) = tree.nodes.get(id).and_then(|n| parent_of(n))
        && tree
            .nodes
            .get(&set)
            .is_some_and(|n| plugin_type(n) == "COMPONENT_SET")
        && !push(
            bounds,
            "variant_of",
            Some(set),
            "resolved",
            "recorded",
            "parentIndex".into(),
        )
    {
        return None;
    }
    let fields = effective(tree, id, e, bounds, unread)?;
    for (field, aliases) in crate::rest::bound_variables(&fields) {
        let mut ids = Vec::new();
        crate::variables::aliases(&aliases, &mut ids);
        for variable in ids {
            let state = match variable.strip_prefix("VariableID:") {
                Some(v) if v.contains('/') => "external",
                Some(v) => node_state(tree, Some(v), "VARIABLE"),
                None => "unsupported",
            };
            if !push(
                bounds,
                "binds_variable",
                Some(variable),
                state,
                "recorded",
                field.clone(),
            ) {
                return None;
            }
        }
    }
    for key in STYLES {
        let Some(r) = fields.get(key) else { continue };
        let (target, state) = match (r["assetRef"]["key"].as_str(), crate::rest::guid(&r["guid"])) {
            (Some(k), _) => (Some(format!("StyleKey:{k}")), "external"),
            (None, Some(g)) => {
                let state = if tree.nodes.contains_key(&g) {
                    "resolved"
                } else {
                    "missing"
                };
                (Some(g), state)
            }
            (None, None) => continue,
        };
        if !push(bounds, "uses_style", target, state, "recorded", key.into()) {
            return None;
        }
    }
    // Top-level paints, and the per-range text and per-region vector fills the renderer selects,
    // streamed from the fields `effective` already charged; only a kept edge names its path.
    let top = ["fillPaints", "strokePaints"]
        .into_iter()
        .map(|k| (None, k, fields.get(k)));
    let styled = STYLED.into_iter().flat_map(|table| {
        let rows = fields.get(table).map(|t| &t["styleOverrideTable"]);
        rows.and_then(Json::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(move |(i, row)| (Some((table, i)), "fillPaints", row.get("fillPaints")))
    });
    for (row, key, paints) in top.chain(styled) {
        let paints = paints.and_then(Json::as_array).into_iter().flatten();
        for (i, p) in paints.enumerate() {
            let Some(hash) = p["image"]["hash"].as_str().filter(|_| p["type"] == "IMAGE") else {
                continue;
            };
            let target = Some(format!("image:{hash}"));
            if !push(
                bounds,
                "uses_image",
                target,
                "recorded",
                "recorded",
                match row {
                    Some((table, r)) => format!("{table}.styleOverrideTable[{r}].{key}[{i}]"),
                    None => format!("{key}[{i}]"),
                },
            ) {
                return None;
            }
        }
    }
    // An instance can inherit its component's links without carrying any of its own.
    let has_prototype = kind == "INSTANCE"
        || e.is_some_and(|e| e.prototype_source.is_some())
        || tree
            .nodes
            .get(e.map_or(id, |e| e.source.as_str()))
            .is_some_and(|n| n.get("prototypeInteractions").is_some());
    // The evidence JSON is transient and capped by the prototype reader; only kept edges are
    // retained, each charged before it is.
    if has_prototype && let Some(p) = crate::prototype::node(tree, id, e, kind) {
        // Interactions a component defines are not verified to apply to its instances.
        let inherited = p["basis"] == "component-definition";
        // Actions past the reader's caps are not read, so their targets are not verified absent.
        if p["truncated"] == true {
            *unverified += 1;
        }
        let interactions = p["interactions"].as_array().into_iter().flatten();
        for action in interactions.flat_map(|i| i["actions"].as_array().into_iter().flatten()) {
            let d = &action["destination"];
            let state = match d["state"].as_str() {
                Some("resolved") => "resolved",
                Some("unresolved") => "missing",
                Some("unsupported") => "unsupported",
                _ if d["reason"].is_string() => "unresolved",
                _ => continue,
            };
            if inherited {
                *unverified += 1;
                continue;
            }
            let target = d["nodeId"].as_str().map(str::to_owned);
            let evidence = format!(
                "{}:{}",
                p["basis"].as_str().unwrap_or(""),
                d["sourceField"].as_str().unwrap_or("")
            );
            if !push(
                bounds,
                "prototype_target",
                target,
                state,
                "recorded",
                evidence,
            ) {
                return None;
            }
        }
    }
    Some(out)
}

fn budget() -> Box<dyn Error> {
    FigError::new(
        ErrorKind::ResponseBudget,
        format!(
            "usages results would copy more than {} bytes of names and paths; lower limit",
            super::REPLY_BYTES
        ),
    )
    .into()
}

fn node_state(tree: &Tree, target: Option<&str>, kind: &str) -> &'static str {
    match target.map(|t| tree.nodes.get(t)) {
        None => "unresolved",
        Some(None) => "missing",
        Some(Some(n)) if plugin_type(n) == kind => "resolved",
        Some(Some(_)) => "unsupported",
    }
}

/// The record's effective binding fields as JSON: an expanded sublayer's override wins over
/// its source's own field. Each is charged at its estimated size before it is converted.
fn effective(
    tree: &Tree,
    id: &str,
    e: Option<&Expanded>,
    bounds: &mut Bounds,
    unread: &mut usize,
) -> Option<Map<String, Json>> {
    let node = tree.nodes.get(e.map_or(id, |e| e.source.as_str()));
    let mut out = Map::new();
    for key in FIELDS
        .iter()
        .chain(&STYLES)
        .chain(["textData", "vectorData"].iter())
    {
        let own = e.and_then(|e| e.props.get(*key));
        if own.is_some_and(|v| !linked_json(v)) {
            continue;
        }
        let mut left = VISITS;
        let (bytes, value) = match own {
            Some(v) => (json_bytes(v, &mut left), None),
            None => match node.and_then(|n| n.get(key)) {
                // Only the per-range / per-region style table of text and vectors is read.
                Some(v) if STYLED.contains(key) => match v.get("styleOverrideTable") {
                    Some(t) if linked(t) => (kiwi_bytes(t, &mut left), Some(t)),
                    _ => continue,
                },
                Some(v) if linked(v) => (kiwi_bytes(v, &mut left), Some(v)),
                _ => continue,
            },
        };
        // One field past the per-value cap is left out and counted, not the rest of the index.
        if left == 0 {
            *unread += 1;
            continue;
        }
        if !bounds.charge(VISITS - left, bytes) {
            return None;
        }
        let converted = match (own, value) {
            (Some(v), _) if STYLED.contains(key) => {
                json!({"styleOverrideTable": v["styleOverrideTable"]})
            }
            (Some(v), _) => v.clone(),
            (None, Some(v)) if STYLED.contains(key) => json!({"styleOverrideTable": to_json(v)}),
            (None, Some(v)) => to_json(v),
            (None, None) => continue,
        };
        out.insert((*key).into(), converted);
    }
    Some(out)
}

/// Whether a stored field can yield an edge: a paint list (or text style table) with a bound
/// color, gradient stop or image; any other field is kept whole. Skips converting the plain
/// paints most layers carry.
fn linked(v: &kiwi_schema::Value) -> bool {
    use kiwi_schema::Value;
    // Plain paints carry an unbound colorVar too; only an alias names a variable.
    fn alias(c: &Value) -> bool {
        matches!(c.get("dataType"), Some(Value::Enum(_, "ALIAS")))
    }
    fn paint(p: &Value) -> bool {
        p.get("colorVar").is_some_and(alias)
            || matches!(p.get("stopsVar"), Some(Value::Array(s)) if s.iter().any(|s| s.get("colorVar").is_some_and(alias)))
            || matches!(p.get("type"), Some(Value::Enum(_, "IMAGE")))
            || p.get("fillPaints").is_some_and(linked)
    }
    match v {
        Value::Array(items) => items.iter().any(paint),
        _ => true,
    }
}

/// `linked` for an override already in JSON.
fn linked_json(v: &Json) -> bool {
    fn paint(p: &Json) -> bool {
        p["colorVar"]["dataType"] == "ALIAS"
            || p["stopsVar"]
                .as_array()
                .is_some_and(|s| s.iter().any(|s| s["colorVar"]["dataType"] == "ALIAS"))
            || p["type"] == "IMAGE"
            || p.get("fillPaints").is_some_and(linked_json)
    }
    match v {
        Json::Array(items) => items.iter().any(paint),
        _ => true,
    }
}

/// What the index knows about each node or variable its edges point at, and about every
/// component-set member it indexes, wherever the node lies; `None` when `bounds` ran out.
pub(crate) fn definitions(
    tree: &Tree,
    records: &[Record],
    bounds: &mut Bounds,
) -> Option<BTreeMap<String, Definition>> {
    let mut wanted = BTreeSet::new();
    for r in records {
        if !bounds.charge(1 + r.edges.len(), 0) {
            return None;
        }
        for e in &r.edges {
            if let Some(t) = &e.target
                && e.state == "resolved"
            {
                // One set entry per target, and one more per variant member, charged first.
                let entries = 1 + usize::from(e.relation == "variant_of");
                if !bounds.charge(0, 64 * entries) {
                    return None;
                }
                wanted.insert(t.as_str());
                if e.relation == "variant_of" {
                    wanted.insert(r.id.as_str());
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut sets = Vec::new();
    for target in wanted {
        let node_id = target.strip_prefix("VariableID:").unwrap_or(target);
        let Some(node) = tree.nodes.get(node_id) else {
            continue;
        };
        let kind = plugin_type(node);
        let set = parent_of(node).filter(|p| {
            kind == "COMPONENT"
                && tree
                    .nodes
                    .get(p)
                    .is_some_and(|n| plugin_type(n) == "COMPONENT_SET")
        });
        // The page: the nearest CANVAS ancestor, one charged step per level.
        let mut page = None;
        let mut cur = parent_of(node);
        let mut steps = 0;
        while let Some(p) = cur {
            let Some(n) = tree.nodes.get(&p) else { break };
            steps += 1;
            if steps > crate::expand::MAX_DEPTH || !bounds.charge(1, 0) {
                break;
            }
            if plugin_type(n) == "CANVAS" {
                page = Some(p);
                break;
            }
            cur = parent_of(n);
        }
        let name = kiwi_name(node).to_owned();
        if !bounds.charge(1, 160 + target.len() + name.len() + kind.len()) {
            return None;
        }
        if let Some(s) = &set {
            sets.push(s.clone());
        }
        out.insert(
            target.to_owned(),
            Definition {
                name,
                kind,
                set,
                page,
            },
        );
    }
    // A member's set is described too, so a set query names it even outside the scope.
    for s in sets {
        if out.contains_key(&s) {
            continue;
        }
        let node = tree.nodes[&s];
        let name = kiwi_name(node).to_owned();
        if !bounds.charge(1, 160 + s.len() + name.len()) {
            return None;
        }
        let page = parent_of(node).filter(|p| {
            tree.nodes
                .get(p)
                .is_some_and(|n| plugin_type(n) == "CANVAS")
        });
        out.insert(
            s,
            Definition {
                name,
                kind: "COMPONENT_SET".into(),
                set: None,
                page,
            },
        );
    }
    Some(out)
}

// ---- Query ----

/// One `usages` call: an exact node or resource of one source and scope, its relations,
/// direction and page.
#[derive(Clone, Debug)]
pub struct Request {
    /// A node id (stored or expanded `I…;…`) or a resource id (`VariableID:…`, `StyleKey:…`,
    /// `image:…`).
    pub target: String,
    pub resource: bool,
    pub scope: String,
    pub relations: Vec<String>,
    pub outgoing: bool,
    /// The snapshot the caller's reference came from; another served snapshot is `source_stale`.
    pub snapshot: Option<String>,
    pub include_hidden: bool,
    pub limit: usize,
    pub cursor: Option<String>,
}

/// The ordering key of one result: location record, relation and sequence.
pub(crate) type Key = (u32, String, u32);

/// A request that passed validation, before any source is read.
pub struct Prepared {
    pub(crate) relations: Vec<&'static str>,
    pub(crate) after: Option<Key>,
    cursor: Option<(String, String, i64)>,
}

impl Prepared {
    /// The generation a workspace cursor paged, `0` for a stateless cursor.
    pub(crate) fn cursor_gen(&self) -> Option<i64> {
        self.cursor.as_ref().map(|c| c.2)
    }
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(ErrorKind::InvalidArguments, message).into()
}

impl Request {
    /// Checks ids, relations, limit and cursor syntax; cheap, so callers run it before I/O.
    pub fn validate(&self) -> Result<Prepared> {
        for (what, v) in [("node", &self.target), ("scope", &self.scope)] {
            if v.is_empty() || v.len() > MAX_ID_BYTES {
                return Err(invalid(format!(
                    "usages {what} takes 1 to {MAX_ID_BYTES} bytes"
                )));
            }
        }
        let resource = ["VariableID:", "StyleKey:", "image:"]
            .iter()
            .any(|p| self.target.starts_with(p));
        if resource != self.resource {
            return Err(invalid(if self.resource {
                "resource takes a VariableID:…, StyleKey:… or image:… id; pass a layer id as node"
            } else {
                "node takes a layer id; pass VariableID:…, StyleKey:… or image:… as resource"
            }));
        }
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(invalid(format!("usages limit takes 1 to {MAX_LIMIT}")));
        }
        let mut relations = Vec::new();
        for r in &self.relations {
            let Some(k) = KINDS.iter().find(|k| **k == r) else {
                return Err(invalid(format!(
                    "unknown relation {r:?}; use {}",
                    KINDS.join(", ")
                )));
            };
            if !relations.contains(k) {
                relations.push(*k);
            }
        }
        if relations.is_empty() {
            // Containment is a usage only when asked for; outgoing, a node's parent is listed.
            relations = KINDS
                .iter()
                .copied()
                .filter(|k| self.outgoing || *k != "contains")
                .collect();
        }
        relations.sort_unstable();
        let mut cursor = None;
        let mut after = None;
        if let Some(c) = &self.cursor {
            let bad = || invalid("invalid usages cursor; pass nextCursor from the previous reply");
            let parts: Vec<&str> = c.split('.').collect();
            let [tag, fp, snap, gen_id, rid, relation, seq] = parts[..] else {
                return Err(bad());
            };
            let relation = KINDS.iter().find(|k| **k == relation).ok_or_else(bad)?;
            if tag != "u1" || fp.len() != 16 || snap.len() != 16 {
                return Err(bad());
            }
            let num = |s: &str| s.parse::<u32>().map_err(|_| bad());
            after = Some((num(rid)?, (*relation).to_owned(), num(seq)?));
            cursor = Some((
                fp.to_owned(),
                snap.to_owned(),
                gen_id.parse::<i64>().map_err(|_| bad())?,
            ));
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.len() > MAX_ID_BYTES)
        {
            return Err(invalid("snapshotId is too long"));
        }
        Ok(Prepared {
            relations,
            after,
            cursor,
        })
    }
}

/// The relations one query reads, behind either a whole in-memory index or a stored generation.
pub(crate) trait Graph {
    fn index(&self) -> &Index;
    /// The record of `node`, if the index has it.
    fn rid_of(&mut self, node: &str) -> Result<Option<u32>>;
    fn definition(&mut self, id: &str) -> Result<Option<Definition>>;
    /// Up to `limit` known members of `set`, in id order.
    fn members(&mut self, set: &str, limit: usize) -> Result<Vec<String>>;
    /// The state of any edge that points at `target`.
    fn targeted(&mut self, target: &str) -> Result<Option<&'static str>>;
    /// Up to `limit` edges after `after` whose target is in `targets`, in key order, with
    /// hidden locations left out unless `hidden`. A `contains` relation matches children of
    /// `parent`.
    fn inbound(
        &mut self,
        targets: &[String],
        relations: &[&'static str],
        parent: Option<u32>,
        hidden: bool,
        after: Option<&Key>,
        limit: usize,
    ) -> Result<Vec<(u32, Edge)>>;
    /// The edges of record `rid`, its parent as `contains`, in key order.
    fn outbound(&mut self, rid: u32) -> Result<Vec<Edge>>;
    /// Makes records `rids` and their ancestors readable through `index`.
    fn load(&mut self, rids: &[u32]) -> Result<()>;
}

/// The in-memory index of a stateless call: every record is loaded.
/// Edges or children one page may pass over before it is refused, so a page costs what it
/// returns plus what its filters skip, never every earlier match.
pub(crate) const SCAN: usize = 50_000;

pub(crate) fn scan_limit() -> Box<dyn Error> {
    FigError::new(
        ErrorKind::QueryWorkLimit,
        format!("one usages page would pass over more than {SCAN} filtered-out edges; pass includeHidden or fewer relations"),
    )
    .into()
}

/// Lookups from a target to what points at it, over a whole built index.
#[derive(Default)]
pub(crate) struct Reverse {
    /// `(rid, edge)` positions per target, in key order.
    by_target: HashMap<String, Vec<(u32, u32)>>,
    /// Child rids per parent rid, ascending.
    children: HashMap<u32, Vec<u32>>,
    rids: HashMap<String, u32>,
    /// Members per component set, in id order.
    members: HashMap<String, Vec<String>>,
}

impl Reverse {
    // The lookups copy ids and positions the build already charged, about as many again.
    fn of(index: &Index) -> &Reverse {
        index.reverse.get_or_init(|| {
            let mut out = Reverse::default();
            for (rid, r) in index.records.iter().enumerate() {
                let rid = rid as u32;
                out.rids.insert(r.id.clone(), rid);
                if let Some(p) = r.parent {
                    out.children.entry(p).or_default().push(rid);
                }
                for (i, e) in r.edges.iter().enumerate() {
                    if let Some(t) = &e.target {
                        out.by_target
                            .entry(t.clone())
                            .or_default()
                            .push((rid, i as u32));
                    }
                }
            }
            for (id, d) in &index.definitions {
                if let Some(set) = &d.set {
                    out.members.entry(set.clone()).or_default().push(id.clone());
                }
            }
            out
        })
    }
}

pub(crate) struct Whole<'a>(pub &'a Index);

impl Graph for Whole<'_> {
    fn index(&self) -> &Index {
        self.0
    }
    fn rid_of(&mut self, node: &str) -> Result<Option<u32>> {
        Ok(Reverse::of(self.0).rids.get(node).copied())
    }
    fn definition(&mut self, id: &str) -> Result<Option<Definition>> {
        Ok(self.0.definitions.get(id).cloned())
    }
    fn members(&mut self, set: &str, limit: usize) -> Result<Vec<String>> {
        let members = Reverse::of(self.0).members.get(set);
        Ok(members.into_iter().flatten().take(limit).cloned().collect())
    }
    fn targeted(&mut self, target: &str) -> Result<Option<&'static str>> {
        let first = Reverse::of(self.0)
            .by_target
            .get(target)
            .and_then(|l| l.first());
        Ok(first.map(|&(rid, i)| self.0.record(rid).edges[i as usize].state))
    }
    fn inbound(
        &mut self,
        targets: &[String],
        relations: &[&'static str],
        parent: Option<u32>,
        hidden: bool,
        after: Option<&Key>,
        limit: usize,
    ) -> Result<Vec<(u32, Edge)>> {
        let index = self.0;
        let rev = Reverse::of(index);
        let past = |rid: u32, relation: &str, seq: u32| {
            after.is_none_or(|a| (rid, relation, seq) > (a.0, a.1.as_str(), a.2))
        };
        let mut out = Vec::new();
        let mut scanned = 0;
        for t in targets {
            let list = rev.by_target.get(t).map_or(&[][..], Vec::as_slice);
            let edge = |&(rid, i): &(u32, u32)| (rid, &index.record(rid).edges[i as usize]);
            let start = list.partition_point(|p| {
                let (rid, e) = edge(p);
                !past(rid, e.relation, e.seq)
            });
            let mut kept = 0;
            for p in &list[start..] {
                if kept == limit {
                    break;
                }
                scanned += 1;
                if scanned > SCAN {
                    return Err(scan_limit());
                }
                let (rid, e) = edge(p);
                if (index.record(rid).hidden && !hidden) || !relations.contains(&e.relation) {
                    continue;
                }
                out.push((rid, e.clone()));
                kept += 1;
            }
        }
        if let Some(parent) = parent.filter(|_| relations.contains(&"contains")) {
            let kids = rev.children.get(&parent).map_or(&[][..], Vec::as_slice);
            let start = kids.partition_point(|&rid| !past(rid, "contains", 0));
            let mut kept = 0;
            for &rid in &kids[start..] {
                if kept == limit {
                    break;
                }
                scanned += 1;
                if scanned > SCAN {
                    return Err(scan_limit());
                }
                let r = index.record(rid);
                if !r.hidden || hidden {
                    out.push((rid, contains(r)));
                    kept += 1;
                }
            }
        }
        out.sort_by(|a, b| (a.0, a.1.relation, a.1.seq).cmp(&(b.0, b.1.relation, b.1.seq)));
        out.truncate(limit);
        Ok(out)
    }
    fn outbound(&mut self, rid: u32) -> Result<Vec<Edge>> {
        let r = self.0.record(rid);
        let mut out: Vec<Edge> = r
            .parent
            .map(|p| parent_edge(self.0, r, p))
            .into_iter()
            .collect();
        out.extend(r.edges.iter().cloned());
        out.sort_by(|a, b| (a.relation, a.seq).cmp(&(b.relation, b.seq)));
        Ok(out)
    }
    fn load(&mut self, _: &[u32]) -> Result<()> {
        Ok(())
    }
}

/// A child's `contains` edge: its parent contains it.
fn contains(r: &Record) -> Edge {
    Edge {
        relation: "contains",
        seq: 0,
        target: None,
        state: "resolved",
        basis: if r.source.is_some() {
            "resolved"
        } else {
            "recorded"
        },
        evidence: "parent".into(),
    }
}

fn parent_edge(index: &Index, r: &Record, parent: u32) -> Edge {
    Edge {
        target: Some(index.record(parent).id.clone()),
        ..contains(r)
    }
}

/// The served source: its id, snapshot and, for a workspace, the generation paged.
pub(crate) struct Served<'a> {
    pub source: &'a Json,
    pub gen_id: i64,
}

/// The usages reply for `req` over `graph`.
pub(crate) fn query(
    graph: &mut dyn Graph,
    q: &Prepared,
    req: &Request,
    served: Served,
) -> Result<Json> {
    let snapshot = served.source["snapshotId"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    if let Some(asked) = &req.snapshot
        && *asked != snapshot
    {
        return Err(FigError::new(
            ErrorKind::SourceStale,
            format!("{} was read from snapshot {asked}, but the source now serves {snapshot}; query again with the current snapshotId, and reopen the node in it",
                req.target),
        )
        .with_recovery(json!([{"action": "use_current_snapshot", "requires": "none", "tool": "usages",
            "arguments": {if req.resource { "resource" } else { "node" }: req.target,
                "scope": req.scope, "snapshotId": snapshot},
            "detail": "keep the file or workspaceId and sourceId you sent; the same id may name another layer in the new snapshot, so reopen it there first"}]))
        .into());
    }
    let index_generation = graph.index().generation.clone();
    let fingerprint = short_hash(
        json!([
            served.source["sourceId"],
            snapshot,
            served.gen_id,
            index_generation,
            req.scope,
            req.target,
            q.relations,
            req.outgoing,
            req.include_hidden,
            RELATIONS
        ])
        .to_string(),
    );
    // A rebuilt index of the same snapshot can stop elsewhere, so cursors bind to the build too.
    let snap = short_hash(format!("{snapshot}.{index_generation}"));
    if let Some((fp, s, gen_id)) = &q.cursor {
        if *s != snap || *gen_id != served.gen_id {
            return Err(FigError::new(
                ErrorKind::CursorStale,
                "the source was reindexed or changed since this cursor; query usages again without cursor",
            )
            .into());
        }
        if *fp != fingerprint {
            return Err(invalid(
                "usages cursor belongs to another query; query again without cursor",
            ));
        }
    }
    let target = req.target.as_str();
    let rid = if req.resource {
        None
    } else {
        graph.rid_of(target)?
    };
    let definition = match (graph.definition(target)?, rid) {
        (Some(d), _) => Some(d),
        (None, Some(rid)) => {
            graph.load(&[rid])?;
            let r = graph.index().record(rid);
            Some(Definition {
                name: r.name.clone(),
                kind: r.kind.clone(),
                set: None,
                page: None,
            })
        }
        (None, None) => None,
    };
    let state = match (&rid, &definition) {
        (Some(_), _) => "indexed",
        (None, Some(_)) => "outside_scope",
        (None, None) => match graph.targeted(target)? {
            Some(s) => s,
            None => {
                return Err(FigError::new(
                    ErrorKind::NodeNotFound,
                    format!("{target} is neither indexed in scope {} nor the target of any relation it holds; check the id and snapshot, or index a scope that has it", req.scope),
                )
                .into());
            }
        },
    };
    let kind = definition.as_ref().map(|d| d.kind.as_str()).unwrap_or("");
    let mut members = Vec::new();
    let mut members_complete = true;
    let mut members_truncated = false;
    if kind == "COMPONENT_SET" && !req.outgoing {
        members = graph.members(target, MAX_MEMBERS + 1)?;
        // Every member is known only when the set itself was indexed whole; a set outside the
        // scope is known by the members the scope uses.
        members_truncated = members.len() > MAX_MEMBERS;
        members_complete = !members_truncated && rid.is_some() && graph.index().stop.is_none();
        members.truncate(MAX_MEMBERS);
    }
    let after = q.after.as_ref();
    let (hits, more, results): (Vec<(u32, Edge)>, bool, Vec<Json>);
    let mut hidden_excluded = false;
    let source = served.source;
    if req.outgoing {
        let Some(rid) = rid else {
            return Err(invalid(format!(
                "outgoing usages need a node indexed in scope {}; {target} is not",
                req.scope
            )));
        };
        let mut all: Vec<(u32, Edge)> = graph
            .outbound(rid)?
            .into_iter()
            .filter(|e| q.relations.contains(&e.relation))
            .filter(|e| after.is_none_or(|a| (rid, e.relation, e.seq) > (a.0, a.1.as_str(), a.2)))
            .map(|e| (rid, e))
            .collect();
        more = all.len() > req.limit;
        all.truncate(req.limit);
        hits = all;
        let mut defs = Vec::new();
        for (_, e) in &hits {
            defs.push(match &e.target {
                Some(t) => graph.definition(t)?,
                None => None,
            });
        }
        let bytes: usize = hits
            .iter()
            .zip(&defs)
            .map(|((_, e), d)| {
                let name = d.as_ref().map_or(0, |d| d.name.len());
                256 + e.target.as_ref().map_or(0, String::len) + e.evidence.len() + name
            })
            .sum();
        if bytes > super::REPLY_BYTES {
            return Err(budget());
        }
        results = hits
            .iter()
            .zip(defs)
            .map(|((_, e), d)| outgoing(e, d.as_ref(), source))
            .collect();
    } else {
        let mut targets = vec![target.to_owned()];
        targets.extend(members.iter().cloned());
        let mut found = graph.inbound(
            &targets,
            &q.relations,
            rid,
            req.include_hidden,
            after,
            req.limit + 1,
        )?;
        more = found.len() > req.limit;
        found.truncate(req.limit);
        // On the last page every visible match after the cursor is here, so a longer answer
        // with hidden layers included means hidden locations were left out.
        if !more && !req.include_hidden {
            let all = graph.inbound(&targets, &q.relations, rid, true, after, found.len() + 1)?;
            hidden_excluded = all.len() > found.len();
        }
        let rids: Vec<u32> = found.iter().map(|h| h.0).collect();
        graph.load(&rids)?;
        let index = graph.index();
        let bytes: usize = found
            .iter()
            .map(|(rid, _)| super::location_bytes(index, *rid))
            .sum();
        if bytes > super::REPLY_BYTES {
            return Err(budget());
        }
        results = found
            .iter()
            .map(|(rid, e)| {
                let mut out = super::location(index, *rid, source);
                edge_fields(&mut out, e);
                if let Some(t) = e
                    .target
                    .as_ref()
                    .filter(|t| *t != target && e.relation != "contains")
                {
                    out["via"] = json!(t);
                }
                out
            })
            .collect();
        hits = found;
    }
    let index = graph.index();
    let last = hits
        .last()
        .map(|(rid, e)| format!("{rid}.{}.{}", e.relation, e.seq));
    let mut reasons = Vec::new();
    if let Some(stop) = index.stop {
        reasons.push(json!({"code": stop}));
    }
    if index.hidden_content > 0 {
        reasons.push(json!({"code": "hidden_content_not_expanded",
            "counts": {"hiddenLayers": index.hidden_content}}));
    }
    if hidden_excluded {
        reasons.push(json!({"code": "hidden_locations_excluded",
            "detail": "pass includeHidden to list usages on hidden layers"}));
    }
    if members_truncated {
        reasons.push(json!({"code": "members_truncated", "limits": {"members": MAX_MEMBERS}}));
    } else if !members_complete && index.stop.is_none() {
        reasons.push(json!({"code": "members_outside_scope",
            "detail": "the set lies outside the scope; members are the ones the scope uses"}));
    }
    let fielded = ["binds_variable", "uses_style", "uses_image"];
    let unread = index.unread_fields > 0 && q.relations.iter().any(|r| fielded.contains(r));
    if unread {
        reasons.push(json!({"code": "relation_unverified", "relations": fielded,
            "counts": {"fields": index.unread_fields},
            "detail": "binding fields too large to read were left out"}));
    }
    let unverified = index.unverified_links > 0 && q.relations.contains(&"prototype_target");
    if unverified {
        reasons.push(
            json!({"code": "relation_unverified", "relations": ["prototype_target"],
            "counts": {"links": index.unverified_links},
            "detail": "prototype links a component defines are not attributed to its instances"}),
        );
    }
    let complete = index.stop.is_none() && members_complete && !unverified && !unread;
    let mut layer = super::relations(if complete { "ready" } else { "partial" });
    if index.unverified_links > 0 {
        layer["kinds"]["prototype_target"] = json!("partial");
    }
    if index.unread_fields > 0 {
        for r in fielded {
            layer["kinds"][r] = json!("partial");
        }
    }
    if let Some(r) = layer["reasons"].as_array() {
        reasons.extend(r.iter().cloned());
    }
    let definition_json = json!({
        "ref": if req.resource { Json::Null } else {
            json!({"sourceId": source["sourceId"], "snapshotId": source["snapshotId"], "nodeId": target})
        },
        "resource": if req.resource { json!(target) } else { Json::Null },
        "state": state,
        "name": definition.as_ref().map(|d| &d.name),
        "type": definition.as_ref().map(|d| &d.kind),
        "setId": definition.as_ref().and_then(|d| d.set.as_ref()),
        "page": definition.as_ref().and_then(|d| d.page.as_ref()),
    });
    let mut out = json!({
        "definition": definition_json,
        "direction": if req.outgoing { "out" } else { "in" },
        "relations": q.relations,
        "scope": {"nodeId": req.scope},
        "results": results,
        "coverage": {"complete": complete, "layers": {"relations": layer}, "reasons": reasons},
        "hasMore": more,
        "nextCursor": more.then(|| last.map(|l| format!("u1.{fingerprint}.{snap}.{}.{l}", served.gen_id))).flatten(),
    });
    if kind == "COMPONENT_SET" && !req.outgoing {
        out["members"] = json!({"nodeIds": members, "complete": members_complete,
            "basis": "the set's members this index describes: every member in scope and each one an edge points at"});
    }
    Ok(out)
}

fn edge_fields(out: &mut Json, e: &Edge) {
    out["relation"] = json!(e.relation);
    out["state"] = json!(e.state);
    out["basis"] = json!(e.basis);
    out["evidence"] = json!(e.evidence);
}

fn outgoing(e: &Edge, d: Option<&Definition>, source: &Json) -> Json {
    let resource = e.target.as_ref().is_some_and(|t| {
        ["VariableID:", "StyleKey:", "image:"]
            .iter()
            .any(|p| t.starts_with(p))
    });
    let mut out = json!({
        "target": match (&e.target, resource) {
            (None, _) => Json::Null,
            (Some(t), true) => json!({"resource": t}),
            (Some(t), false) => json!({"ref": {"sourceId": source["sourceId"],
                "snapshotId": source["snapshotId"], "nodeId": t}}),
        },
        "definition": d.map(|d| json!({"name": d.name, "type": d.kind, "setId": d.set, "page": d.page})),
    });
    edge_fields(&mut out, e);
    out
}

#[cfg(test)]
#[path = "usages_tests.rs"]
mod tests;
