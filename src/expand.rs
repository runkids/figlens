//! Instance expansion: a `.fig` stores an instance as one node, so its children
//! are rebuilt from the main component plus the instance's overrides, with the
//! `I<instance>;<path>` ids the Figma plugin reports.

use std::collections::HashMap;

use kiwi_schema::Value;
use serde_json::{Map, Value as Json};

use crate::{Tree, guid_key, to_json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

/// A node transform: linear part `a b c d` plus translation `tx ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Mat {
    pub const IDENTITY: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn from_value(v: &Value) -> Mat {
        let f = |name: &str| v.get(name).map_or(0.0, |x| x.as_float() as f64);
        Mat {
            a: f("m00"),
            b: f("m10"),
            c: f("m01"),
            d: f("m11"),
            tx: f("m02"),
            ty: f("m12"),
        }
    }

    pub fn apply(self, p: Vec2) -> Vec2 {
        Vec2 {
            x: self.a * p.x + self.c * p.y + self.tx,
            y: self.b * p.x + self.d * p.y + self.ty,
        }
    }

    pub fn inverse(self) -> Mat {
        let det = self.a * self.d - self.b * self.c;
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Mat {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        }
    }

    /// Top-left corner of the axis-aligned box around a `size` box after this transform.
    pub fn origin(self, s: Vec2) -> Vec2 {
        Vec2 {
            x: self.tx + (self.a * s.x).min(0.0) + (self.c * s.y).min(0.0),
            y: self.ty + (self.b * s.x).min(0.0) + (self.d * s.y).min(0.0),
        }
    }

    /// Size of the axis-aligned box around a `size` box after this transform.
    pub fn bbox(self, s: Vec2) -> Vec2 {
        Vec2 {
            x: self.a.abs() * s.x + self.c.abs() * s.y,
            y: self.b.abs() * s.x + self.d.abs() * s.y,
        }
    }
}

/// `self * t`: `t` is the child's transform inside this space.
impl std::ops::Mul for Mat {
    type Output = Mat;

    fn mul(self, t: Mat) -> Mat {
        Mat {
            a: self.a * t.a + self.c * t.b,
            b: self.b * t.a + self.d * t.b,
            c: self.a * t.c + self.c * t.d,
            d: self.b * t.c + self.d * t.d,
            tx: self.a * t.tx + self.c * t.ty + self.tx,
            ty: self.b * t.tx + self.d * t.ty + self.ty,
        }
    }
}

/// A node produced by expanding an instance.
#[derive(Debug)]
pub struct Expanded {
    /// The main component's node this one copies.
    pub source: String,
    pub name: String,
    pub kind: String,
    pub size: Option<Vec2>,
    pub matrix: Mat,
    /// Effective placement and overridden kiwi fields, applied over `source`'s own.
    pub props: Map<String, Json>,
    pub(crate) prototype_source: Option<crate::prototype::Source>,
}

/// Expanded nodes plus their child order (keys: instance or expanded ids).
pub struct Expansion {
    pub nodes: HashMap<String, Expanded>,
    pub children: HashMap<String, Vec<String>>,
}

/// Fields an instance can override or Figma can derive for a nested node.
#[derive(Clone, Default)]
struct Patch {
    name: Option<String>,
    visible: Option<bool>,
    size: Option<Vec2>,
    transform: Option<Mat>,
    swap: Option<String>,
    props: Map<String, Json>,
    prototype_source: Option<crate::prototype::Source>,
}

impl Patch {
    fn from_value(v: &Value) -> Patch {
        Patch {
            name: v.get("name").map(|n| n.as_string().to_owned()),
            visible: v.get("visible").map(Value::as_bool),
            size: v.get("size").map(vec2),
            transform: v.get("transform").map(Mat::from_value),
            swap: v.get("overriddenSymbolID").and_then(guid_key),
            props: match to_json(v) {
                Json::Object(mut m) => {
                    m.remove("guidPath");
                    m
                }
                _ => Map::new(),
            },
            prototype_source: None,
        }
    }

    /// Field-wise merge where `self` wins over `inner`.
    fn over(self, inner: Patch) -> Patch {
        Patch {
            name: self.name.or(inner.name),
            visible: self.visible.or(inner.visible),
            size: self.size.or(inner.size),
            transform: self.transform.or(inner.transform),
            swap: self.swap.or(inner.swap),
            props: inner.props.into_iter().chain(self.props).collect(),
            prototype_source: self.prototype_source.or(inner.prototype_source),
        }
    }
}

type Patches = HashMap<String, Patch>;
type Resize = Option<(Vec2, Vec2)>;

/// What stopped a bounded run first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Work,
    Bytes,
    Depth,
    Deadline,
}

/// Work left for a bounded run, spent before the visits, clones and allocations it pays for.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    /// Node visits plus override entries converted, merged or walked.
    pub work: usize,
    /// Estimated bytes converted, copied or retained, including a caller's own records.
    pub bytes: usize,
    /// Nested expansion depth.
    pub depth: usize,
    pub deadline: std::time::Instant,
    pub stop: Option<Stop>,
}

impl Bounds {
    /// Spends `work` and `bytes`; false, with `stop` set, once any bound has run out.
    pub fn charge(&mut self, work: usize, bytes: usize) -> bool {
        if self.stop.is_none() {
            self.stop = if std::time::Instant::now() >= self.deadline {
                Some(Stop::Deadline)
            } else if work > self.work {
                Some(Stop::Work)
            } else if bytes > self.bytes {
                Some(Stop::Bytes)
            } else {
                self.work -= work;
                self.bytes -= bytes;
                return true;
            };
        }
        false
    }
}

/// Sums `f` over `items` until `left` runs out, without enumerating the rest of a collection.
fn sum_while<T>(
    items: impl IntoIterator<Item = T>,
    left: &mut usize,
    mut f: impl FnMut(T, &mut usize) -> usize,
) -> usize {
    let mut total = 0;
    for item in items {
        if *left == 0 {
            break;
        }
        total += f(item, left);
    }
    total
}

/// A cheap upper estimate of `v`'s retained size, computed without allocating; it visits at
/// most `left` values, so a payload larger than the remaining work stops early.
fn json_bytes(v: &Json, left: &mut usize) -> usize {
    if *left == 0 {
        return 0;
    }
    *left -= 1;
    match v {
        Json::String(s) => s.len() + 24,
        Json::Array(a) => 24 + sum_while(a, left, json_bytes),
        Json::Object(o) => {
            48 + sum_while(o, left, |(k, v), left| k.len() + 24 + json_bytes(v, left))
        }
        _ => 16,
    }
}

/// `json_bytes` of `v` once converted, estimated from the kiwi value without converting it.
fn kiwi_bytes(v: &Value, left: &mut usize) -> usize {
    if *left == 0 {
        return 0;
    }
    *left -= 1;
    match v {
        Value::String(s) => s.len() + 24,
        Value::Bytes(b) => 24 + 4 * b.len(),
        Value::Array(a) => 24 + sum_while(a, left, kiwi_bytes),
        Value::Object(_, o) => {
            48 + sum_while(o, left, |(k, v), left| k.len() + 24 + kiwi_bytes(v, left))
        }
        _ => 16,
    }
}

fn patches_bytes(patches: &Patches, left: &mut usize) -> usize {
    sum_while(patches, left, |(k, p), left| {
        128 + k.len() + sum_while(p.props.values(), left, json_bytes)
    })
}

/// Bytes `bind_properties` converts and copies for property assignments: each converts once,
/// and each property reference may copy the largest one into its layer.
fn assignment_bytes(
    own: Option<&Value>,
    inherited: Option<&Json>,
    refs: usize,
    left: &mut usize,
) -> usize {
    let (mut total, mut largest) = (0usize, 0usize);
    let own = match own {
        Some(Value::Array(items)) => &items[..],
        _ => &[],
    };
    let mut add = |n: usize| {
        total += n;
        largest = largest.max(n);
    };
    for v in own {
        add(kiwi_bytes(v, left));
    }
    for v in inherited.and_then(Json::as_array).into_iter().flatten() {
        add(json_bytes(v, left));
    }
    total.saturating_add(largest.saturating_mul(refs))
}

fn list_len(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Array(items)) => items.len(),
        _ => 0,
    }
}

fn vec2(v: &Value) -> Vec2 {
    let f = |name: &str| v.get(name).map_or(0.0, |x| x.as_float() as f64);
    Vec2 {
        x: f("x"),
        y: f("y"),
    }
}

fn enum_name<'a>(v: &Value<'a>, field: &str) -> Option<&'a str> {
    match v.get(field) {
        Some(Value::Enum(_, name)) => Some(*name),
        _ => None,
    }
}

fn flag(v: &Value, field: &str) -> Option<bool> {
    v.get(field).map(Value::as_bool)
}

/// Plugin type names, including component-set and resize-to-fit frames.
pub fn plugin_type(node: &Value) -> String {
    match enum_name(node, "type").unwrap_or("?") {
        "FRAME" if flag(node, "isStateGroup") == Some(true) => "COMPONENT_SET",
        "FRAME" if flag(node, "resizeToFit") == Some(true) => "GROUP",
        "ROUNDED_RECTANGLE" => "RECTANGLE",
        "SYMBOL" => "COMPONENT",
        other => other,
    }
    .to_owned()
}

/// `a:b;c:d` key of an override's `guidPath`.
fn path_key(entry: &Value) -> String {
    let guids = entry.get("guidPath").and_then(|p| p.get("guids"));
    match guids {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(guid_key)
            .collect::<Vec<_>>()
            .join(";"),
        _ => String::new(),
    }
}

/// One patch per guidPath; a path can have several entries (e.g. a swap, then a size), merged with the later one winning per field.
fn entries(list: Option<&Value>, source: Option<&str>) -> Patches {
    let mut patches = Patches::new();
    if let Some(Value::Array(items)) = list {
        for (index, entry) in items.iter().enumerate() {
            let key = path_key(entry);
            let previous = patches.remove(&key).unwrap_or_default();
            let mut patch = Patch::from_value(entry);
            if entry.get("prototypeInteractions").is_some() {
                patch.prototype_source = source.map(|node_id| crate::prototype::Source {
                    node_id: node_id.into(),
                    override_index: Some(index),
                });
            }
            patches.insert(key, patch.over(previous));
        }
    }
    patches
}

/// Adds a nested instance's own patches (relative to it, at `prefix`) under the outer ones.
/// Merges field by field: an outer entry that only carries other fields (e.g. fillGeometry) must not hide the inner size.
fn nest(outer: &Patches, prefix: &str, inner: Patches) -> Patches {
    let mut merged = outer.clone();
    for (k, patch) in inner {
        let full = format!("{prefix};{k}");
        let outer_patch = outer.get(&full).cloned().unwrap_or_default();
        merged.insert(full, outer_patch.over(patch));
    }
    merged
}

/// Resolves visibility, text and instance-swap component properties within one instance.
fn bind_properties(
    tree: &Tree,
    symbol: &str,
    prefix: &str,
    mut overrides: Patches,
    instance: &Value,
    patch: &Patch,
) -> Patches {
    let own = instance
        .get("componentPropAssignments")
        .map(to_json)
        .unwrap_or(Json::Null);
    let assignments: Vec<_> = own
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            patch
                .props
                .get("componentPropAssignments")
                .and_then(Json::as_array)
                .into_iter()
                .flatten(),
        )
        .collect();
    if assignments.is_empty() {
        return overrides;
    }
    for id in subtree(tree, symbol).into_iter().skip(1) {
        let child = tree.nodes[&id];
        let key = child.get("overrideKey").and_then(guid_key).unwrap_or(id);
        let key = if prefix.is_empty() {
            key
        } else {
            format!("{prefix};{key}")
        };
        if let Some(Value::Array(refs)) = child.get("componentPropRefs") {
            for reference in refs {
                let Some(def) = reference.get("defID") else {
                    continue;
                };
                let Some(assignment) = assignments
                    .iter()
                    .rev()
                    .find(|a| a["defID"] == to_json(def))
                else {
                    continue;
                };
                let value = assignment
                    .get("varValue")
                    .and_then(|v| v.get("value"))
                    .or_else(|| assignment.get("value"));
                let patch = overrides.entry(key.clone()).or_default();
                if enum_name(reference, "componentPropNodeField") == Some("TEXT_DATA")
                    && !patch.props.contains_key("textData")
                    && let Some(text) = [&assignment["varValue"]["value"], &assignment["value"]]
                        .into_iter()
                        .flat_map(|v| [&v["textDataValue"], &v["textValue"]])
                        .find(|v| v["characters"].is_string() || v.is_string())
                {
                    let mut data = match child.get("textData").map(to_json) {
                        Some(Json::Object(data)) => data,
                        _ => Map::new(),
                    };
                    if let Some(fields) = text.as_object() {
                        data.extend(fields.clone());
                    } else {
                        data.insert("characters".into(), text.clone());
                    }
                    patch.props.insert("textData".into(), Json::Object(data));
                }
                if enum_name(reference, "componentPropNodeField") == Some("VISIBLE")
                    && patch.visible.is_none()
                    && let Some(visible) = value.and_then(|v| v["boolValue"].as_bool())
                {
                    patch.visible = Some(visible);
                    patch.props.insert("visible".into(), Json::Bool(visible));
                }
                if enum_name(reference, "componentPropNodeField") == Some("OVERRIDDEN_SYMBOL_ID")
                    && patch.swap.is_none()
                    && let Some(guid) = value
                        .and_then(|v| v.get("symbolIdValue"))
                        .and_then(|s| s.get("guid"))
                    && let (Some(session), Some(local)) =
                        (guid["sessionID"].as_u64(), guid["localID"].as_u64())
                {
                    patch.swap = Some(format!("{session}:{local}"));
                }
            }
        }
    }
    overrides
}

/// Resizes `size` by the child's constraints when its container changed size.
fn constrain(size: Option<Vec2>, h: Option<&str>, v: Option<&str>, resize: Resize) -> Option<Vec2> {
    let (Some(s), Some((from, to))) = (size, resize) else {
        return size;
    };
    let axis = |c: Option<&str>, len: f64, from: f64, to: f64| match c {
        Some("SCALE") => len * (to / from),
        Some("STRETCH") => len + (to - from),
        _ => len,
    };
    Some(Vec2 {
        x: axis(h, s.x, from.x, to.x),
        y: axis(v, s.y, from.y, to.y),
    })
}

fn resized(from: Option<Vec2>, to: Option<Vec2>) -> Resize {
    match (from, to) {
        (Some(f), Some(t)) if f != t => Some((f, t)),
        _ => None,
    }
}

pub(crate) fn parent_of(node: &Value) -> Option<String> {
    node.get("parentIndex")?.get("guid").and_then(guid_key)
}

/// `root` and every descendant, in tree order.
pub fn subtree(tree: &Tree, root: &str) -> Vec<String> {
    subtree_while(tree, root, || true)
}

/// `subtree`, stopping before the first node `more` refuses.
fn subtree_while(tree: &Tree, root: &str, mut more: impl FnMut() -> bool) -> Vec<String> {
    let mut out = vec![root.to_owned()];
    // A crafted file can make a node its own descendant; visit each id once.
    let mut seen: std::collections::HashSet<String> = out.iter().cloned().collect();
    let mut i = 0;
    while i < out.len() {
        for c in tree.children.get(&out[i]).into_iter().flatten() {
            if !seen.contains(c) && !more() {
                return out;
            }
            if seen.insert(c.clone()) {
                out.push(c.clone());
            }
        }
        i += 1;
    }
    out
}

/// Product of transforms from `id` up to (excluding) `root`.
pub fn abs_matrix(tree: &Tree, id: &str, root: &str) -> Mat {
    let mut m = Mat::IDENTITY;
    let mut cur = Some(id.to_owned());
    while let Some(node) = cur
        .as_deref()
        .filter(|c| *c != root)
        .and_then(|c| tree.nodes.get(c))
    {
        if let Some(t) = node.get("transform") {
            m = Mat::from_value(t) * m;
        }
        cur = parent_of(node);
    }
    m
}

/// False when `id` or any ancestor up to `root` is hidden.
fn shown(tree: &Tree, id: &str, root: &str) -> bool {
    let mut cur = Some(id.to_owned());
    while let Some(node) = cur.as_deref().and_then(|c| tree.nodes.get(c)) {
        if flag(node, "visible") == Some(false) {
            return false;
        }
        if cur.as_deref() == Some(root) {
            break;
        }
        cur = parent_of(node);
    }
    true
}

fn main_of<'t>(tree: &Tree<'t>, instance: &Value) -> Option<String> {
    instance
        .get("symbolData")?
        .get("symbolID")
        .and_then(guid_key)
        .filter(|id| tree.nodes.contains_key(id))
}

struct Expander<'t, 'a> {
    tree: &'t Tree<'a>,
    out: Expansion,
    /// The components being expanded, outermost first: one that contains itself stops here.
    symbols: Vec<String>,
    /// Bytes left for the `id`, `name` and `type` every expanded node emits; `None` is unbounded.
    budget: Option<usize>,
    /// Shared work bounds of `expand_instances_bounded`; `None` is unbounded.
    bounds: Option<Bounds>,
    depth: usize,
    /// `symbol_cost` per component.
    sizes: HashMap<String, (usize, usize, usize)>,
    exceeded: bool,
}

/// Expands every visible instance under `root` into `I<instance>;<path>` nodes.
pub fn expand_instances(tree: &Tree, root: &str) -> Expansion {
    expand_all(tree, root, None, None).0
}

/// `expand_instances`, or `None` once the expanded nodes' emitted `id`, `name` and `type`
/// alone exceed `bytes`: a lower bound of `root`'s REST JSON, so a fitting scope still expands.
pub fn expand_instances_within(tree: &Tree, root: &str, bytes: usize) -> Option<Expansion> {
    let (ex, _, exceeded) = expand_all(tree, root, Some(bytes), None);
    (!exceeded).then_some(ex)
}

/// `expand_instances` that spends `bounds` before each stored visit, override conversion or
/// merge, expanded node and nesting level, and keeps what it expanded when they run out
/// (`Bounds::stop`). Stored children still win over an expansion, as in REST output.
pub fn expand_instances_bounded(tree: &Tree, root: &str, bounds: Bounds) -> (Expansion, Bounds) {
    let (ex, bounds, _) = expand_all(tree, root, None, Some(bounds));
    (ex, bounds.unwrap())
}

fn expand_all(
    tree: &Tree,
    root: &str,
    budget: Option<usize>,
    bounds: Option<Bounds>,
) -> (Expansion, Option<Bounds>, bool) {
    let mut ex = Expander {
        tree,
        out: Expansion {
            nodes: HashMap::new(),
            children: HashMap::new(),
        },
        symbols: Vec::new(),
        budget,
        bounds,
        depth: 0,
        sizes: HashMap::new(),
        exceeded: false,
    };
    let stored = subtree_while(tree, root, || ex.spend(1, 0));
    for id in stored {
        if ex.exceeded {
            break;
        }
        let inst = tree.nodes[&id];
        if enum_name(inst, "type") != Some("INSTANCE") || !shown(tree, &id, root) {
            continue;
        }
        // REST output keeps stored children over an expansion, so a bounded run skips it unread.
        if (budget.is_some() || ex.bounds.is_some()) && tree.children.contains_key(&id) {
            continue;
        }
        let Some(main) = main_of(tree, inst) else {
            continue;
        };
        let sd = inst.get("symbolData");
        if ex.bounds.is_some() {
            let (nodes, text, refs) = ex.symbol_cost(&main);
            let (overrides, derived, assignments) = (
                sd.and_then(|s| s.get("symbolOverrides")),
                inst.get("derivedSymbolData"),
                inst.get("componentPropAssignments"),
            );
            let work = nodes
                + list_len(overrides)
                + list_len(derived)
                + refs.saturating_mul(list_len(assignments));
            let estimate = |left: &mut usize| {
                [overrides, derived]
                    .into_iter()
                    .flatten()
                    .map(|v| kiwi_bytes(v, left))
                    .sum::<usize>()
                    + assignment_bytes(assignments, None, refs, left)
            };
            if ex.spend_estimated(work, text, estimate).is_none() {
                break;
            }
        }
        let ov = bind_properties(
            tree,
            &main,
            "",
            entries(sd.and_then(|s| s.get("symbolOverrides")), Some(&id)),
            inst,
            &Patch::default(),
        );
        let dv = entries(inst.get("derivedSymbolData"), None);
        let resize = resized(
            tree.nodes[&main].get("size").map(vec2),
            inst.get("size").map(vec2),
        );
        ex.symbols = vec![main.clone()];
        ex.expand(
            &id,
            &id,
            &main,
            &[],
            &ov,
            &dv,
            abs_matrix(tree, &id, root),
            resize,
        );
    }
    (ex.out, ex.bounds, ex.exceeded)
}

impl Expander<'_, '_> {
    /// Charges the shared bounds, if any; false once they ran out.
    fn spend(&mut self, work: usize, bytes: usize) -> bool {
        if let Some(b) = &mut self.bounds
            && !b.charge(work, bytes)
        {
            self.exceeded = true;
            return false;
        }
        true
    }

    /// Enters one more nesting level; false once a bounded run is too deep.
    fn descend(&mut self) -> bool {
        if let Some(b) = &mut self.bounds
            && self.depth >= b.depth
        {
            b.stop.get_or_insert(Stop::Depth);
            self.exceeded = true;
            return false;
        }
        self.depth += 1;
        true
    }

    /// Spends `work` and `bytes` plus what `estimate` measures; the estimate's value visits
    /// are charged as work and stop at the remaining work. The estimated bytes, or None.
    fn spend_estimated(
        &mut self,
        work: usize,
        bytes: usize,
        estimate: impl FnOnce(&mut usize) -> usize,
    ) -> Option<usize> {
        let start = self.bounds.as_ref()?.work.saturating_add(1);
        let mut left = start;
        let estimated = estimate(&mut left);
        self.spend(work + (start - left), bytes + estimated)
            .then_some(estimated)
    }

    /// Stored nodes, text bytes and property references `bind_properties` walks and may copy,
    /// measured once per component by walks charged as they go. Only bounded runs measure.
    fn symbol_cost(&mut self, symbol: &str) -> (usize, usize, usize) {
        if self.bounds.is_none() {
            return (0, 0, 0);
        }
        if let Some(cost) = self.sizes.get(symbol) {
            return *cost;
        }
        let tree = self.tree;
        let ids = subtree_while(tree, symbol, || self.spend(1, 0));
        let texts = ids.iter().filter_map(|id| tree.nodes[id].get("textData"));
        let text = self
            .spend_estimated(0, 0, |left| texts.map(|t| kiwi_bytes(t, left)).sum())
            .unwrap_or(0);
        let refs = ids
            .iter()
            .map(|id| list_len(tree.nodes[id].get("componentPropRefs")))
            .sum();
        let cost = (ids.len(), text, refs);
        self.sizes.insert(symbol.to_owned(), cost);
        cost
    }

    /// `path` holds only instance boundaries: Figma ids skip plain frames in between.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &mut self,
        outer: &str,
        parent: &str,
        symbol: &str,
        path: &[String],
        ov: &Patches,
        dv: &Patches,
        m: Mat,
        resize: Resize,
    ) {
        let tree = self.tree;
        for child_id in tree.children.get(symbol).into_iter().flatten() {
            if self.exceeded {
                return;
            }
            let child = tree.nodes[child_id];
            // Library component copies keep the library's ids in overrideKey, and paths use those.
            let key = child
                .get("overrideKey")
                .and_then(guid_key)
                .unwrap_or_else(|| child_id.clone());
            let p: Vec<String> = path.iter().cloned().chain([key]).collect();
            let pk = p.join(";");
            let none = Patch::default();
            let o = ov.get(&pk).unwrap_or(&none);
            let d = dv.get(&pk).unwrap_or(&none);
            // Swaps reset the inherited name; variants use their component set's name.
            let swapped = o
                .swap
                .as_ref()
                .filter(|_| o.name.is_none())
                .and_then(|id| tree.nodes.get(id))
                .and_then(|main| {
                    let set = parent_of(main)
                        .and_then(|id| tree.nodes.get(&id).copied())
                        .filter(|p| flag(p, "isStateGroup") == Some(true));
                    set.unwrap_or(main).get("name")
                });
            let name = o
                .name
                .as_deref()
                .or(swapped.map(Value::as_string))
                .unwrap_or_else(|| child.get("name").map_or("", Value::as_string));
            if self.bounds.is_some() {
                let derived = ["fillGeometry", "strokeGeometry", "derivedTextData"]
                    .iter()
                    .filter_map(|f| d.props.get(*f));
                let estimate = |left: &mut usize| {
                    o.props
                        .values()
                        .chain(derived)
                        .map(|v| json_bytes(v, left))
                        .sum()
                };
                if self
                    .spend_estimated(1, 200 + outer.len() + pk.len() + name.len(), estimate)
                    .is_none()
                {
                    return;
                }
            }
            let kind = enum_name(child, "type");
            let own_size = child.get("size").map(vec2);
            // Auto-layout owns normal-flow child geometry; constraints only apply to absolute children.
            let resize = if matches!(
                enum_name(tree.nodes[symbol], "stackMode"),
                Some("HORIZONTAL" | "VERTICAL")
            ) && enum_name(child, "stackPositioning") != Some("ABSOLUTE")
            {
                None
            } else {
                resize
            };
            // Groups and boolean ops have no constraints of their own: children resolve against the outer container.
            // shortcut: the container takes its first child's constraints instead of re-fitting the children's bounds; fit bounds once positions are tracked.
            let transparent = kind == Some("BOOLEAN_OPERATION")
                || (kind == Some("FRAME") && flag(child, "resizeToFit") == Some(true));
            let size = d.size.or(o.size).or_else(|| {
                let first = tree
                    .children
                    .get(child_id)
                    .and_then(|c| c.first())
                    .filter(|_| transparent);
                let src = first.map_or(child, |f| tree.nodes[f]);
                constrain(
                    own_size,
                    enum_name(src, "horizontalConstraint"),
                    enum_name(src, "verticalConstraint"),
                    resize,
                )
            });
            let child_resize = if transparent {
                resize
            } else {
                resized(own_size, size)
            };

            let name = name.to_owned();

            let matrix = m
                * (d.transform.or(o.transform).unwrap_or_else(|| {
                    child
                        .get("transform")
                        .map_or(Mat::IDENTITY, Mat::from_value)
                }));
            // A swapped instance looks like its new main; only how it sits in its parent stays its own.
            let source = o
                .swap
                .as_ref()
                .filter(|s| tree.nodes.contains_key(*s))
                .unwrap_or(child_id)
                .clone();
            // Keep placement complete for semantic readers, including fields absent on the child.
            let mut props: Map<String, Json> = [
                "stackChildPrimaryGrow",
                "stackChildAlignSelf",
                "stackPositioning",
                "horizontalConstraint",
                "verticalConstraint",
            ]
            .into_iter()
            .filter_map(|f| Some((f.to_owned(), to_json(child.get(f)?))))
            .collect();
            props.extend(o.props.clone());
            // Figma re-derives the outlines of resized and re-texted nodes; without them the
            // source's outlines are scaled from the size they were drawn at.
            for f in ["fillGeometry", "strokeGeometry", "derivedTextData"] {
                if let Some(v) = d.props.get(f) {
                    props.insert(f.into(), v.clone());
                }
            }
            for (f, at) in [
                ("fillGeometry", "fillGeometrySize"),
                ("strokeGeometry", "strokeGeometrySize"),
            ] {
                if !d.props.contains_key(f)
                    && let Some(s) = tree.nodes[&source].get("size")
                {
                    props.insert(at.into(), to_json(s));
                }
            }
            let visible = o.visible.or_else(|| flag(tree.nodes[&source], "visible"));
            let id = format!("I{outer};{pk}");
            // Figma ids are unique; a repeat would make the tree contain itself, so skip it.
            if self.out.nodes.contains_key(&id) {
                continue;
            }
            let rest_type = plugin_type(child);
            if let Some(left) = self.budget {
                let emitted = serde_json::to_vec(
                    &serde_json::json!({"id": id, "name": name, "type": rest_type}),
                )
                .map_or(0, |v| v.len() - 2);
                match left.checked_sub(emitted) {
                    Some(left) => self.budget = Some(left),
                    None => {
                        self.exceeded = true;
                        return;
                    }
                }
            }
            self.out
                .children
                .entry(parent.to_owned())
                .or_default()
                .push(id.clone());
            let prototype_source = o.prototype_source.clone().or_else(|| {
                (source != *child_id
                    && kind == Some("INSTANCE")
                    && child.get("prototypeInteractions").is_some())
                .then(|| crate::prototype::Source {
                    node_id: child_id.clone(),
                    override_index: None,
                })
            });
            self.out.nodes.insert(
                id.clone(),
                Expanded {
                    source,
                    name,
                    kind: rest_type,
                    size,
                    matrix,
                    props,
                    prototype_source,
                },
            );
            // Inside an expanded instance, hidden nodes keep the node but omit its children.
            if visible == Some(false) {
                continue;
            }
            if kind == Some("INSTANCE") {
                let sd = child.get("symbolData");
                let Some(nested) = o
                    .swap
                    .clone()
                    .or_else(|| sd.and_then(|s| s.get("symbolID")).and_then(guid_key))
                else {
                    continue;
                };
                // Figma forbids a component containing itself; a crafted file would recurse forever.
                if self.symbols.contains(&nested) {
                    continue;
                }
                if !self.descend() {
                    return;
                }
                // Converting and merging the nested overrides copies the outer ones too.
                if self.bounds.is_some() {
                    let (nodes, text, refs) = self.symbol_cost(&nested);
                    let (overrides, derived, own, inherited) = (
                        sd.and_then(|s| s.get("symbolOverrides")),
                        child.get("derivedSymbolData"),
                        child.get("componentPropAssignments"),
                        o.props.get("componentPropAssignments"),
                    );
                    let assignments =
                        list_len(own) + inherited.and_then(Json::as_array).map_or(0, Vec::len);
                    let work = nodes
                        + ov.len()
                        + dv.len()
                        + list_len(overrides)
                        + list_len(derived)
                        + refs.saturating_mul(assignments);
                    let estimate = |left: &mut usize| {
                        patches_bytes(ov, left)
                            + patches_bytes(dv, left)
                            + [overrides, derived]
                                .into_iter()
                                .flatten()
                                .map(|v| kiwi_bytes(v, left))
                                .sum::<usize>()
                            + assignment_bytes(own, inherited, refs, left)
                    };
                    if self.spend_estimated(work, text, estimate).is_none() {
                        return;
                    }
                }
                let merged = bind_properties(
                    tree,
                    &nested,
                    &pk,
                    nest(
                        ov,
                        &pk,
                        entries(sd.and_then(|s| s.get("symbolOverrides")), Some(child_id)),
                    ),
                    child,
                    o,
                );
                let merged_derived = nest(dv, &pk, entries(child.get("derivedSymbolData"), None));
                let main_size = tree
                    .nodes
                    .get(&nested)
                    .and_then(|n| n.get("size"))
                    .map(vec2);
                self.symbols.push(nested.clone());
                self.expand(
                    outer,
                    &id,
                    &nested,
                    &p,
                    &merged,
                    &merged_derived,
                    matrix,
                    resized(main_size, size),
                );
                self.symbols.pop();
                self.depth -= 1;
            } else {
                if !self.descend() {
                    return;
                }
                self.expand(outer, &id, child_id, path, ov, dv, matrix, child_resize);
                self.depth -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(x: f64, y: f64) -> Vec2 {
        Vec2 { x, y }
    }

    fn obj(fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object("Test", fields.into_iter().collect())
    }

    fn size(x: f32, y: f32) -> Value<'static> {
        obj(vec![("x", Value::Float(x)), ("y", Value::Float(y))])
    }

    #[test]
    fn auto_layout_children_do_not_apply_constraints_again() {
        for (mode, positioning, expected) in [
            ("HORIZONTAL", "AUTO", 15.0),
            ("NONE", "AUTO", 17.5),
            ("HORIZONTAL", "ABSOLUTE", 17.5),
        ] {
            let main = obj(vec![("stackMode", Value::Enum("StackMode", mode))]);
            let group = obj(vec![
                ("type", Value::Enum("NodeType", "FRAME")),
                ("resizeToFit", Value::Bool(true)),
                (
                    "stackPositioning",
                    Value::Enum("StackPositioning", positioning),
                ),
                ("size", size(15.0, 15.0)),
            ]);
            let vector = obj(vec![
                ("type", Value::Enum("NodeType", "VECTOR")),
                ("size", size(10.0, 10.0)),
                ("horizontalConstraint", Value::Enum("Constraint", "SCALE")),
                ("verticalConstraint", Value::Enum("Constraint", "SCALE")),
            ]);
            let tree = Tree {
                nodes: HashMap::from([
                    ("main".into(), &main),
                    ("group".into(), &group),
                    ("vector".into(), &vector),
                ]),
                children: HashMap::from([
                    ("main".into(), vec!["group".into()]),
                    ("group".into(), vec!["vector".into()]),
                ]),
                blobs: Vec::new(),
            };
            let mut ex = Expander {
                bounds: None,
                depth: 0,
                sizes: HashMap::new(),
                tree: &tree,
                out: Expansion {
                    nodes: HashMap::new(),
                    children: HashMap::new(),
                },
                symbols: Vec::new(),
                budget: None,
                exceeded: false,
            };
            let derived = Patches::from([(
                "group".into(),
                Patch {
                    transform: Some(Mat {
                        tx: 6.5,
                        ty: 6.5,
                        ..Mat::IDENTITY
                    }),
                    ..Patch::default()
                },
            )]);
            ex.expand(
                "instance",
                "instance",
                "main",
                &[],
                &Patches::new(),
                &derived,
                Mat::IDENTITY,
                Some((v2(24.0, 24.0), v2(28.0, 28.0))),
            );
            assert_eq!(
                ex.out.nodes["Iinstance;group"].size,
                Some(v2(expected, expected))
            );
            let expected_vector = expected / 15.0 * 10.0;
            assert!(
                (ex.out.nodes["Iinstance;vector"].size.unwrap().x - expected_vector).abs() < 0.001
            );
        }
    }

    #[test]
    fn swapped_names_use_the_component_set_unless_explicitly_overridden() {
        let main = obj(vec![]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("name", Value::String("custom old name".into())),
        ]);
        let component = obj(vec![
            ("name", Value::String("State=on".into())),
            ("parentIndex", obj(vec![("guid", guid(1, 1))])),
        ]);
        let set = obj(vec![
            ("name", Value::String("Flag".into())),
            ("isStateGroup", Value::Bool(true)),
        ]);
        let tree = Tree {
            nodes: HashMap::from([
                ("main".into(), &main),
                ("child".into(), &child),
                ("component".into(), &component),
                ("1:1".into(), &set),
            ]),
            children: HashMap::from([("main".into(), vec!["child".into()])]),
            blobs: Vec::new(),
        };
        for name in [None, Some("explicit name".to_owned())] {
            let mut ex = Expander {
                bounds: None,
                depth: 0,
                sizes: HashMap::new(),
                tree: &tree,
                out: Expansion {
                    nodes: HashMap::new(),
                    children: HashMap::new(),
                },
                symbols: Vec::new(),
                budget: None,
                exceeded: false,
            };
            let ov = Patches::from([(
                "child".into(),
                Patch {
                    name: name.clone(),
                    swap: Some("component".into()),
                    ..Patch::default()
                },
            )]);
            ex.expand(
                "outer",
                "outer",
                "main",
                &[],
                &ov,
                &Patches::new(),
                Mat::IDENTITY,
                None,
            );
            assert_eq!(
                ex.out.nodes["Iouter;child"].name,
                name.as_deref().unwrap_or("Flag")
            );
        }
    }

    fn guid(s: u32, l: u32) -> Value<'static> {
        obj(vec![
            ("sessionID", Value::UInt(s)),
            ("localID", Value::UInt(l)),
        ])
    }

    #[test]
    fn a_component_containing_itself_stops_at_the_repeat() {
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
        ]);
        let main = obj(vec![]);
        let inner = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
        ]);
        let tree = Tree {
            nodes: HashMap::from([
                ("inst".into(), &inst),
                ("1:1".into(), &main),
                ("2:2".into(), &inner),
            ]),
            children: HashMap::from([("1:1".into(), vec!["2:2".into()]), ("inst".into(), vec![])]),
            blobs: Vec::new(),
        };
        let ex = expand_instances(&tree, "inst");
        assert_eq!((ex.nodes.len(), ex.children.get("Iinst;2:2")), (1, None));
    }

    #[test]
    fn a_reply_budget_counts_only_the_selected_sublayer() {
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
        ]);
        let main = obj(vec![]);
        let rect = |name: &str| {
            obj(vec![
                ("type", Value::Enum("NodeType", "RECTANGLE")),
                ("name", Value::String(name.to_owned())),
            ])
        };
        let (small, large) = (rect("small"), rect(&"x".repeat(4096)));
        let tree = Tree {
            nodes: HashMap::from([
                ("inst".into(), &inst),
                ("1:1".into(), &main),
                ("1:4".into(), &small),
                ("1:5".into(), &large),
            ]),
            children: HashMap::from([("1:1".into(), vec!["1:4".into(), "1:5".into()])]),
            blobs: Vec::new(),
        };
        let within = |id, bytes| crate::rest::resolve_nodes_within(&tree, id, bytes);
        for id in ["Iinst;1:4", "inst"] {
            let full = crate::rest::resolve_nodes(&tree, id).unwrap();
            // The complete selection's own size always fits: the count is a lower bound.
            let size = serde_json::to_vec(&full["document"]).unwrap().len();
            assert_eq!(within(id, size).unwrap(), full);
        }
        let error = within("inst", 1024).unwrap_err();
        let error = error.downcast_ref::<crate::FigError>().unwrap();
        assert_eq!(error.kind(), crate::ErrorKind::ResponseBudget);
        assert!(within("Iinst;1:4", 1024).is_ok());
    }

    #[test]
    fn a_bounded_expansion_stops_exponential_fan_out_but_keeps_fitting_scopes() {
        // Component d holds ten instances of component d + 1: 10^6 nested copies in all.
        let instance = |d: u32| {
            obj(vec![
                ("type", Value::Enum("NodeType", "INSTANCE")),
                ("symbolData", obj(vec![("symbolID", guid(d, 0))])),
            ])
        };
        let mut owned = vec![
            ("inst".to_owned(), instance(10)),
            ("16:0".to_owned(), obj(vec![])),
        ];
        let mut children = HashMap::new();
        for d in 10..16 {
            owned.push((format!("{d}:0"), obj(vec![])));
            let kids: Vec<String> = (1..=10).map(|i| format!("{d}:{i}")).collect();
            for kid in &kids {
                owned.push((kid.clone(), instance(d + 1)));
            }
            children.insert(format!("{d}:0"), kids);
        }
        let mut tree = Tree {
            nodes: owned.iter().map(|(id, v)| (id.clone(), v)).collect(),
            children,
            blobs: Vec::new(),
        };
        assert!(expand_instances_within(&tree, "inst", 1 << 20).is_none());
        let error = crate::rest::resolve_nodes_within(&tree, "inst", 1 << 20).unwrap_err();
        let error = error.downcast_ref::<crate::FigError>().unwrap();
        assert_eq!(error.kind(), crate::ErrorKind::ResponseBudget);
        // Two levels fit: the bounded run matches the full one.
        tree.children.insert("12:0".into(), Vec::new());
        let full = crate::rest::resolve_nodes(&tree, "inst").unwrap();
        let size = serde_json::to_vec(&full["document"]).unwrap().len();
        let bounded = expand_instances_within(&tree, "inst", size).unwrap();
        assert_eq!(bounded.nodes.len(), 110);
        assert_eq!(bounded.children, expand_instances(&tree, "inst").children);
        assert_eq!(
            crate::rest::resolve_nodes_within(&tree, "inst", size).unwrap(),
            full
        );
        // REST keeps stored children over an expansion, so a bounded run neither counts nor builds it.
        tree.children.insert("inst".into(), Vec::new());
        let full = crate::rest::resolve_nodes(&tree, "inst").unwrap();
        let size = serde_json::to_vec(&full["document"]).unwrap().len();
        assert!(
            expand_instances_within(&tree, "inst", 0)
                .unwrap()
                .nodes
                .is_empty()
        );
        assert_eq!(
            crate::rest::resolve_nodes_within(&tree, "inst", size).unwrap(),
            full
        );
    }

    fn bounded(tree: &Tree, bytes: usize) -> (Expansion, Bounds) {
        let bounds = Bounds {
            work: 1 << 20,
            bytes,
            depth: 64,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
            stop: None,
        };
        expand_instances_bounded(tree, "page", bounds)
    }

    #[test]
    fn a_bounded_expansion_charges_text_property_copies_before_binding_them() {
        // One large assignment bound into 100 children copies it 100 times.
        let big = "x".repeat(10_000);
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
            (
                "componentPropAssignments",
                Value::Array(vec![obj(vec![
                    ("defID", guid(2, 1)),
                    ("value", obj(vec![("textValue", Value::String(big))])),
                ])]),
            ),
        ]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "TEXT")),
            (
                "componentPropRefs",
                Value::Array(vec![obj(vec![
                    ("defID", guid(2, 1)),
                    ("componentPropNodeField", Value::Enum("Field", "TEXT_DATA")),
                ])]),
            ),
        ]);
        let (page, main) = (obj(vec![]), obj(vec![]));
        let kids: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
        let mut nodes: HashMap<String, &Value> = kids.iter().map(|k| (k.clone(), &child)).collect();
        nodes.extend([
            ("page".into(), &page),
            ("inst".into(), &inst),
            ("1:1".into(), &main),
        ]);
        let tree = Tree {
            nodes,
            children: HashMap::from([("page".into(), vec!["inst".into()]), ("1:1".into(), kids)]),
            blobs: Vec::new(),
        };
        let (ex, bounds) = bounded(&tree, 200_000);
        assert_eq!((bounds.stop, ex.nodes.len()), (Some(Stop::Bytes), 0));
        assert_eq!(bounded(&tree, 4 << 20).0.nodes.len(), 100);
    }

    #[test]
    fn a_bounded_expansion_charges_the_names_it_copies() {
        // Twenty instances each copy the main child's 10 kB default name.
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
        ]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "FRAME")),
            ("name", Value::String("n".repeat(10_000))),
        ]);
        let (page, main) = (obj(vec![]), obj(vec![]));
        let insts: Vec<String> = (0..20).map(|i| format!("i{i}")).collect();
        let mut nodes: HashMap<String, &Value> = insts.iter().map(|k| (k.clone(), &inst)).collect();
        nodes.extend([
            ("page".into(), &page),
            ("1:1".into(), &main),
            ("child".into(), &child),
        ]);
        let tree = Tree {
            nodes,
            children: HashMap::from([("page".into(), insts), ("1:1".into(), vec!["child".into()])]),
            blobs: Vec::new(),
        };
        let (ex, bounds) = bounded(&tree, 100_000);
        assert_eq!(bounds.stop, Some(Stop::Bytes));
        assert!(ex.nodes.len() < 10, "{}", ex.nodes.len());
        assert_eq!(bounded(&tree, 1 << 20).0.nodes.len(), 20);
    }

    #[test]
    fn subtree_visits_a_cycle_once() {
        let node = obj(vec![]);
        let tree = Tree {
            nodes: HashMap::from([("a".into(), &node), ("b".into(), &node)]),
            children: HashMap::from([
                ("a".into(), vec!["b".into()]),
                ("b".into(), vec!["a".into()]),
            ]),
            blobs: Vec::new(),
        };
        assert_eq!(subtree(&tree, "a"), vec!["a", "b"]);
    }

    #[test]
    fn a_repeated_sublayer_id_is_expanded_once() {
        // Two sublayers sharing one overrideKey would give one id twice and a node its own child.
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
        ]);
        let main = obj(vec![]);
        let frame = || {
            obj(vec![
                ("type", Value::Enum("NodeType", "FRAME")),
                ("overrideKey", guid(5, 5)),
            ])
        };
        let (a, b) = (frame(), frame());
        let tree = Tree {
            nodes: HashMap::from([
                ("inst".into(), &inst),
                ("1:1".into(), &main),
                ("a".into(), &a),
                ("b".into(), &b),
            ]),
            children: HashMap::from([
                ("1:1".into(), vec!["a".into()]),
                ("a".into(), vec!["b".into()]),
            ]),
            blobs: Vec::new(),
        };
        let ex = expand_instances(&tree, "inst");
        assert_eq!(
            (ex.children["inst"].clone(), ex.children.get("Iinst;5:5")),
            (vec!["Iinst;5:5".to_owned()], None)
        );
    }

    #[test]
    fn boolean_component_properties_hide_nested_subtrees() {
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 1))])),
            (
                "componentPropAssignments",
                Value::Array(vec![obj(vec![
                    ("defID", guid(2, 1)),
                    (
                        "varValue",
                        obj(vec![(
                            "value",
                            obj(vec![("boolValue", Value::Bool(false))]),
                        )]),
                    ),
                ])]),
            ),
        ]);
        let main = obj(vec![]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "FRAME")),
            (
                "componentPropRefs",
                Value::Array(vec![obj(vec![
                    ("defID", guid(2, 1)),
                    ("componentPropNodeField", Value::Enum("Field", "VISIBLE")),
                ])]),
            ),
        ]);
        let leaf = obj(vec![]);
        let tree = Tree {
            nodes: HashMap::from([
                ("inst".into(), &inst),
                ("1:1".into(), &main),
                ("child".into(), &child),
                ("leaf".into(), &leaf),
            ]),
            children: HashMap::from([
                ("1:1".into(), vec!["child".into()]),
                ("child".into(), vec!["leaf".into()]),
            ]),
            blobs: Vec::new(),
        };
        let ex = expand_instances(&tree, "inst");
        assert_eq!(
            ex.nodes["Iinst;child"].props.get("visible"),
            Some(&Json::Bool(false))
        );
        assert!(!ex.nodes.contains_key("Iinst;leaf"));
    }

    #[test]
    fn text_component_properties_reach_expanded_children_without_replacing_overrides() {
        for (form, text, explicit) in [
            (0, "查看全部", false),
            (1, "Plain label", false),
            (2, "String label", false),
            (2, "按钮文字", false),
            (3, "Fallback label", false),
            (0, "", false),
            (0, "Property label", true),
        ] {
            let text_data =
                |characters: &str| obj(vec![("characters", Value::String(characters.into()))]);
            let assignment = |characters: &str| {
                let value = obj(vec![(
                    if form == 0 {
                        "textDataValue"
                    } else {
                        "textValue"
                    },
                    if form == 2 {
                        Value::String(characters.into())
                    } else {
                        text_data(characters)
                    },
                )]);
                let mut fields = vec![
                    ("defID", guid(2, 1)),
                    if form == 0 {
                        ("varValue", obj(vec![("value", value)]))
                    } else {
                        ("value", value)
                    },
                ];
                if form == 3 {
                    fields.push(("varValue", obj(vec![("value", obj(vec![]))])));
                }
                Value::Array(vec![obj(fields)])
            };
            let inst = obj(vec![
                ("type", Value::Enum("NodeType", "INSTANCE")),
                (
                    "symbolData",
                    obj(vec![
                        ("symbolID", guid(1, 1)),
                        (
                            "symbolOverrides",
                            Value::Array(vec![obj(vec![
                                (
                                    "guidPath",
                                    obj(vec![("guids", Value::Array(vec![guid(1, 3)]))]),
                                ),
                                ("componentPropAssignments", assignment(text)),
                            ])]),
                        ),
                    ]),
                ),
                ("componentPropAssignments", assignment(text)),
            ]);
            let main = obj(vec![]);
            let nested = obj(vec![
                ("type", Value::Enum("NodeType", "INSTANCE")),
                (
                    "symbolData",
                    obj(vec![
                        ("symbolID", guid(1, 2)),
                        (
                            "symbolOverrides",
                            Value::Array(if explicit {
                                vec![obj(vec![
                                    (
                                        "guidPath",
                                        obj(vec![("guids", Value::Array(vec![guid(1, 4)]))]),
                                    ),
                                    ("textData", text_data("Explicit label")),
                                ])]
                            } else {
                                vec![]
                            }),
                        ),
                    ]),
                ),
                ("componentPropAssignments", assignment("Nested default")),
            ]);
            let child = obj(vec![
                ("type", Value::Enum("NodeType", "TEXT")),
                (
                    "textData",
                    obj(vec![
                        ("characters", Value::String("按钮文字".into())),
                        ("characterStyleIDs", Value::Array(vec![Value::UInt(1); 4])),
                        (
                            "styleOverrideTable",
                            Value::Array(vec![obj(vec![
                                ("styleID", Value::UInt(1)),
                                ("fontSize", Value::Float(22.0)),
                            ])]),
                        ),
                        (
                            "lines",
                            Value::Array(vec![obj(vec![
                                ("lineType", Value::Enum("LineType", "UNORDERED_LIST")),
                                ("indentationLevel", Value::UInt(2)),
                            ])]),
                        ),
                    ]),
                ),
                (
                    "componentPropRefs",
                    Value::Array(vec![obj(vec![
                        ("defID", guid(2, 1)),
                        ("componentPropNodeField", Value::Enum("Field", "TEXT_DATA")),
                    ])]),
                ),
            ]);
            let tree = Tree {
                nodes: HashMap::from([
                    ("inst".into(), &inst),
                    ("1:1".into(), &main),
                    ("1:2".into(), &main),
                    ("1:3".into(), &nested),
                    ("1:4".into(), &child),
                ]),
                children: HashMap::from([
                    ("1:1".into(), vec!["1:4".into(), "1:3".into()]),
                    ("1:2".into(), vec!["1:4".into()]),
                ]),
                blobs: Vec::new(),
            };
            let ex = expand_instances(&tree, "inst");
            assert_eq!(ex.nodes["Iinst;1:4"].props["textData"]["characters"], text);
            let doc = crate::rest::nodes(&tree, "inst");
            let direct = &doc["document"]["children"][0];
            assert_eq!(direct["characters"], text);
            assert_eq!(
                direct["characterStyleOverrides"],
                serde_json::json!([1, 1, 1, 1])
            );
            assert_eq!(direct["styleOverrideTable"]["1"]["fontSize"], 22.0);
            assert_eq!(direct["lineTypes"], serde_json::json!(["UNORDERED"]));
            assert_eq!(direct["lineIndentations"], serde_json::json!([2]));
            assert_eq!(
                ex.nodes["Iinst;1:3;1:4"].props["textData"]["characters"],
                if explicit { "Explicit label" } else { text }
            );
        }
    }

    #[test]
    fn swaps_use_the_new_mains_visibility_unless_overridden() {
        let main = obj(vec![]);
        let child = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("visible", Value::Bool(false)),
        ]);
        let target = obj(vec![("visible", Value::Bool(true))]);
        let leaf = obj(vec![]);
        let tree = Tree {
            nodes: HashMap::from([
                ("main".into(), &main),
                ("child".into(), &child),
                ("target".into(), &target),
                ("leaf".into(), &leaf),
            ]),
            children: HashMap::from([
                ("main".into(), vec!["child".into()]),
                ("target".into(), vec!["leaf".into()]),
            ]),
            blobs: Vec::new(),
        };
        for visible in [None, Some(false)] {
            let mut ex = Expander {
                bounds: None,
                depth: 0,
                sizes: HashMap::new(),
                tree: &tree,
                out: Expansion {
                    nodes: HashMap::new(),
                    children: HashMap::new(),
                },
                symbols: Vec::new(),
                budget: None,
                exceeded: false,
            };
            let ov = Patches::from([(
                "child".into(),
                Patch {
                    swap: Some("target".into()),
                    visible,
                    ..Patch::default()
                },
            )]);
            ex.expand(
                "outer",
                "outer",
                "main",
                &[],
                &ov,
                &Patches::new(),
                Mat::IDENTITY,
                None,
            );
            assert_eq!(
                ex.out.nodes.contains_key("Iouter;child;leaf"),
                visible != Some(false)
            );
        }
    }

    #[test]
    fn symbol_properties_on_nested_overrides_swap_the_referenced_instance() {
        let assignment = Value::Array(vec![obj(vec![
            ("defID", guid(2, 1)),
            (
                "varValue",
                obj(vec![(
                    "value",
                    obj(vec![("symbolIdValue", obj(vec![("guid", guid(1, 4))]))]),
                )]),
            ),
        ])]);
        let inst = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            (
                "symbolData",
                obj(vec![
                    ("symbolID", guid(1, 1)),
                    (
                        "symbolOverrides",
                        Value::Array(vec![obj(vec![
                            (
                                "guidPath",
                                obj(vec![("guids", Value::Array(vec![guid(1, 3)]))]),
                            ),
                            ("componentPropAssignments", assignment),
                        ])]),
                    ),
                ]),
            ),
        ]);
        let main = obj(vec![]);
        let nested = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("symbolData", obj(vec![("symbolID", guid(1, 2))])),
        ]);
        let icon = obj(vec![
            ("type", Value::Enum("NodeType", "INSTANCE")),
            ("name", Value::String("Refresh".into())),
            (
                "componentPropRefs",
                Value::Array(vec![obj(vec![
                    ("defID", guid(2, 1)),
                    (
                        "componentPropNodeField",
                        Value::Enum("Field", "OVERRIDDEN_SYMBOL_ID"),
                    ),
                ])]),
            ),
        ]);
        let target = obj(vec![("name", Value::String("Close".into()))]);
        let leaf = obj(vec![]);
        let tree = Tree {
            nodes: HashMap::from([
                ("inst".into(), &inst),
                ("1:1".into(), &main),
                ("1:2".into(), &main),
                ("1:3".into(), &nested),
                ("icon".into(), &icon),
                ("1:4".into(), &target),
                ("leaf".into(), &leaf),
            ]),
            children: HashMap::from([
                ("1:1".into(), vec!["1:3".into()]),
                ("1:2".into(), vec!["icon".into()]),
                ("1:4".into(), vec!["leaf".into()]),
            ]),
            blobs: Vec::new(),
        };
        let ex = expand_instances(&tree, "inst");
        assert_eq!(ex.nodes["Iinst;1:3;icon"].source, "1:4");
        assert_eq!(ex.nodes["Iinst;1:3;icon"].name, "Close");
        assert!(ex.nodes.contains_key("Iinst;1:3;icon;leaf"));
    }

    #[test]
    fn state_group_frames_are_component_sets() {
        let node = Value::Object(
            "NodeChange",
            HashMap::from([
                ("type", Value::Enum("NodeType", "FRAME")),
                ("isStateGroup", Value::Bool(true)),
            ]),
        );
        assert_eq!(plugin_type(&node), "COMPONENT_SET");
    }

    #[test]
    fn stretch_adds_the_container_delta_and_scale_multiplies() {
        let resize = Some((v2(100.0, 100.0), v2(150.0, 200.0)));
        let size = Some(v2(40.0, 40.0));
        assert_eq!(
            constrain(size, Some("STRETCH"), Some("SCALE"), resize),
            Some(v2(90.0, 80.0))
        );
    }

    #[test]
    fn other_constraints_keep_the_size() {
        let resize = Some((v2(100.0, 100.0), v2(150.0, 200.0)));
        assert_eq!(
            constrain(Some(v2(40.0, 40.0)), Some("MIN"), None, resize),
            Some(v2(40.0, 40.0))
        );
    }

    #[test]
    fn quarter_turn_swaps_bbox_axes() {
        let turn = Mat {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            ..Mat::IDENTITY
        };
        assert_eq!(turn.bbox(v2(10.0, 20.0)), v2(20.0, 10.0));
    }

    #[test]
    fn child_origin_adds_parent_translation_and_rotation() {
        let parent = Mat {
            tx: 100.0,
            ty: 50.0,
            ..Mat::IDENTITY
        };
        let turned_child = Mat {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            tx: 30.0,
            ty: 10.0,
        };
        // A quarter turn about the child's own origin puts its box left of it.
        assert_eq!(
            (parent * turned_child).origin(v2(10.0, 20.0)),
            v2(110.0, 60.0)
        );
    }

    #[test]
    fn inverse_undoes_the_transform() {
        let m = Mat {
            a: 0.0,
            b: 2.0,
            c: -1.0,
            d: 0.0,
            tx: 4.0,
            ty: -2.0,
        };
        assert_eq!(m.inverse().apply(m.apply(v2(1.0, 2.0))), v2(1.0, 2.0));
    }

    #[test]
    fn outer_override_wins_per_field() {
        let outer = Patch {
            name: Some("outer".into()),
            ..Patch::default()
        };
        let inner = Patch {
            name: Some("inner".into()),
            visible: Some(false),
            ..Patch::default()
        };
        let merged = outer.over(inner);
        assert_eq!(
            (merged.name.as_deref(), merged.visible),
            (Some("outer"), Some(false))
        );
    }

    #[test]
    fn outer_entry_without_size_keeps_the_nested_size() {
        // Seen in a real file: the outer derived entry only holds fillGeometry.
        let outer = Patches::from([("a;b".to_owned(), Patch::default())]);
        let inner = Patch {
            size: Some(v2(36.0, 36.0)),
            ..Patch::default()
        };
        let merged = nest(&outer, "a", Patches::from([("b".to_owned(), inner)]));
        assert_eq!(merged["a;b"].size, Some(v2(36.0, 36.0)));
    }

    #[test]
    fn entries_on_the_same_path_keep_both_swap_and_size() {
        // Seen in a real file: one entry swaps the node, a later one sets its size.
        let guid = |s, l| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(s)), ("localID", Value::UInt(l))]),
            )
        };
        let path = || {
            Value::Object(
                "GUIDPath",
                HashMap::from([("guids", Value::Array(vec![guid(7, 42)]))]),
            )
        };
        let size = Value::Object(
            "Vector",
            HashMap::from([("x", Value::Float(10.0)), ("y", Value::Float(20.0))]),
        );
        let list = Value::Array(vec![
            Value::Object(
                "SymbolOverride",
                HashMap::from([("guidPath", path()), ("overriddenSymbolID", guid(18, 1))]),
            ),
            Value::Object(
                "SymbolOverride",
                HashMap::from([("guidPath", path()), ("size", size)]),
            ),
        ]);
        let patch = &entries(Some(&list), None)["7:42"];
        assert_eq!(
            (patch.swap.as_deref(), patch.size),
            (Some("18:1"), Some(v2(10.0, 20.0)))
        );
    }
    #[test]
    fn an_estimate_stops_at_the_remaining_work() {
        let tree = Tree {
            nodes: HashMap::new(),
            children: HashMap::new(),
            blobs: Vec::new(),
        };
        let mut ex = Expander {
            bounds: Some(Bounds {
                work: 10,
                bytes: usize::MAX,
                depth: 8,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
                stop: None,
            }),
            depth: 0,
            sizes: HashMap::new(),
            tree: &tree,
            out: Expansion {
                nodes: HashMap::new(),
                children: HashMap::new(),
            },
            symbols: Vec::new(),
            budget: None,
            exceeded: false,
        };
        let payload = Value::Array(vec![Value::Int(0); 100_000]);
        let mut visited = 0;
        let estimate = |left: &mut usize| {
            let before = *left;
            let bytes = kiwi_bytes(&payload, left);
            visited = before - *left;
            bytes
        };
        assert_eq!(ex.spend_estimated(0, 0, estimate), None);
        assert_eq!((visited, ex.bounds.unwrap().stop), (11, Some(Stop::Work)));
    }

    #[test]
    fn an_exhausted_estimate_stops_enumerating_its_collection() {
        let mut yielded = 0;
        let items = (0..100_000).inspect(|_| yielded += 1);
        let mut left = 10;
        sum_while(items, &mut left, |_, left| {
            *left -= 1;
            1
        });
        assert_eq!(yielded, 11);
    }
}
