//! Compare stored definitions without guessing descendant correspondence or expanding instances.
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::path::Path;

use super::{Budget, MAX_BYTES, MAX_ID_BYTES, MAX_WORK, definition_id};
use crate::{ErrorKind, FigError, Tree, compare::normalized, expand::plugin_type, to_json};
use kiwi_schema::Value as K;
use serde_json::{Value, json};

const MAX_NODES: usize = 200;
const MAX_VALUE_BYTES: usize = 4096;
const FIELDS: &[&str] = &[
    "size",
    "visible",
    "opacity",
    "fillPaints",
    "strokePaints",
    "effects",
    "blendMode",
    "strokeWeight",
    "borderStrokeWeightsIndependent",
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
    "cornerRadius",
    "rectangleCornerRadiiIndependent",
    "rectangleTopLeftCornerRadius",
    "rectangleTopRightCornerRadius",
    "rectangleBottomRightCornerRadius",
    "rectangleBottomLeftCornerRadius",
    "stackMode",
    "stackSpacing",
    "stackVerticalPadding",
    "stackPaddingRight",
    "stackPaddingBottom",
    "stackHorizontalPadding",
    "stackPrimaryAlignItems",
    "stackCounterAlignItems",
    "textData.characters",
    "fontName",
    "fontSize",
    "fontStyle",
    "transform.m00",
    "transform.m01",
    "transform.m10",
    "transform.m11",
];

fn kind(raw: &K) -> Option<String> {
    match raw.get("type") {
        Some(K::Enum(_, name)) if name.len() <= 128 => Some(plugin_type(raw)),
        _ => None,
    }
}

fn component_set(tree: &Tree, id: &str) -> Result<String, Box<dyn Error>> {
    let raw = tree
        .nodes
        .get(id)
        .ok_or_else(|| FigError::new(ErrorKind::NodeNotFound, format!("variant {id} not found")))?;
    if kind(raw).as_deref() == Some("COMPONENT")
        && let Some(set) = raw
            .get("parentIndex")
            .and_then(|p| p.get("guid"))
            .and_then(definition_id)
        && tree
            .nodes
            .get(&set)
            .is_some_and(|n| kind(n).as_deref() == Some("COMPONENT_SET"))
    {
        return Ok(set);
    }
    Err(FigError::new(ErrorKind::UnsupportedNode,
        "variant comparison requires stored COMPONENT definitions with an explicit COMPONENT_SET parent").into())
}

// Check work, depth and bytes before to_json can copy any untrusted field.
fn bounded(value: &K, depth: usize, bytes: &mut usize, budget: &mut Budget, hash: bool) -> bool {
    if depth == 16 || !budget.step() {
        return false;
    }
    let length = match value {
        K::String(s) => s.len(),
        K::Enum(_, s) => s.len(),
        K::Bytes(items) if hash => items.len().saturating_mul(2),
        K::Bytes(_) => return false,
        K::Byte(_) if !hash => return false,
        K::Float(n) if !n.is_finite() => return false,
        K::Array(items) => {
            for item in items {
                if hash && !matches!(item, K::Byte(_)) {
                    return false;
                }
                if !bounded(item, depth + 1, bytes, budget, hash) {
                    return false;
                }
            }
            2
        }
        K::Object(_, fields) => {
            for (key, item) in fields {
                if key.len() > *bytes {
                    return false;
                }
                *bytes -= key.len();
                if !bounded(item, depth + 1, bytes, budget, *key == "hash") {
                    return false;
                }
            }
            2
        }
        _ => 16,
    };
    if length > *bytes {
        return false;
    }
    *bytes -= length;
    true
}

fn raw_field<'a>(raw: &'a K, path: &str) -> Result<Option<&'a K<'a>>, ()> {
    let mut current = raw;
    for key in path.split('.') {
        if !matches!(current, K::Object(..)) {
            return Err(());
        }
        let Some(next) = current.get(key) else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

fn field(raw: &K, path: &str, id: &str, budget: &mut Budget) -> Value {
    let mut cell = json!({"sourceNodeId":id,"sourceField":path,"state":"absent"});
    let value = match raw_field(raw, path) {
        Ok(Some(value)) => value,
        Ok(None) => return cell,
        Err(()) => {
            cell["state"] = "unsupported".into();
            cell["reason"] = "malformed_field_container".into();
            return cell;
        }
    };
    let mut remaining = MAX_VALUE_BYTES;
    let shape = match path {
        "visible" | "rectangleCornerRadiiIndependent" | "borderStrokeWeightsIndependent" => {
            matches!(value, K::Bool(_))
        }
        "size" => matches!(
            (value.get("x"), value.get("y")),
            (Some(K::Float(_)), Some(K::Float(_)))
        ),
        "textData.characters" => matches!(value, K::String(_)),
        "fillPaints" | "strokePaints" | "effects" => matches!(value, K::Array(_)),
        _ => true,
    };
    if shape && bounded(value, 0, &mut remaining, budget, false) {
        if matches!(path, "fillPaints" | "strokePaints" | "effects")
            && matches!(value,K::Array(items) if items.iter().any(|i| !matches!(i,K::Object(..))))
        {
            cell["state"] = "unsupported".into();
            cell["reason"] = "malformed_list_entry".into();
            return cell;
        }
        let value = normalized(&to_json(value));
        if serde_json::to_vec(&value).expect("JSON field").len() <= MAX_VALUE_BYTES {
            cell["state"] = "recorded".into();
            cell["value"] = value;
            return cell;
        }
    }
    cell["state"] = "unsupported".into();
    cell["reason"] = "unsupported_or_limited_value".into();
    cell
}

fn collect(tree: &Tree, root: &str, budget: &mut Budget) -> Vec<Value> {
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if nodes.len() == MAX_NODES || !budget.step() {
            budget.stop("comparison_node_or_work_limit");
            break;
        }
        if !seen.insert(id) {
            continue;
        }
        if !budget.id(id) {
            continue;
        }
        let Some(raw) = tree.nodes.get(id) else {
            budget.stop("missing_node");
            continue;
        };
        let mut node = budget.name(super::string(raw, "name").unwrap_or(""));
        node["nodeId"] = id.into();
        let kind = kind(raw);
        node["type"] = kind.as_deref().unwrap_or("UNKNOWN").into();
        let mut fields = serde_json::Map::new();
        if kind.is_none() {
            fields.insert("type".into(),json!({"sourceNodeId":id,"sourceField":"type","state":"unsupported","reason":"unsupported_or_limited_type"}));
        }
        for path in FIELDS {
            if id == root || !matches!(raw_field(raw, path), Ok(None)) {
                fields.insert((*path).into(), field(raw, path, id, budget));
            }
        }
        node["fields"] = fields.into();
        if !budget.push(&mut nodes, node) {
            break;
        }
        if let Some(list) = tree.children.get(id) {
            // At most MAX_NODES candidates can be visited; report omitted children explicitly.
            let available = MAX_NODES.saturating_sub(nodes.len() + stack.len());
            if list.len() > available {
                budget.stop("comparison_node_limit");
            }
            stack.extend(list.iter().take(available).rev().map(String::as_str));
        }
    }
    nodes
}

/// Descendants that are the only node of their name and type under a paired parent on each
/// side, from the roots down, as (left, right) indexes; names that were cut short never pair.
fn pairs(tree: &Tree, a: &[Value], b: &[Value]) -> Vec<(usize, usize)> {
    let index = |nodes: &[Value]| -> std::collections::HashMap<String, usize> {
        nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| Some((n["nodeId"].as_str()?.to_owned(), i)))
            .collect()
    };
    let (left, right) = (index(a), index(b));
    // Each side's scanned children by (type, name), with how many share that key.
    let children = |nodes: &[Value], at: &std::collections::HashMap<String, usize>, i: usize| {
        let mut keys: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let id = nodes[i]["nodeId"].as_str().unwrap_or("");
        for child in tree.children.get(id).into_iter().flatten() {
            let Some(&c) = at.get(child) else { continue };
            if nodes[c]["nameTruncated"] == true {
                continue;
            }
            let key = json!([nodes[c]["type"], nodes[c]["name"]]).to_string();
            let entry = keys.entry(key).or_insert((c, 0));
            entry.1 += 1;
        }
        keys
    };
    let mut out = Vec::new();
    let mut queue = vec![(0, 0)];
    while let Some((l, r)) = queue.pop() {
        if l >= a.len() || r >= b.len() {
            continue;
        }
        let theirs = children(b, &right, r);
        for (key, (lc, count)) in children(a, &left, l) {
            if let Some(&(rc, 1)) = theirs.get(&key)
                && count == 1
            {
                out.push((lc, rc));
                queue.push((lc, rc));
            }
        }
    }
    out
}

/// Names each variable alias in recorded values by its id and, when the file holds it or a
/// library's cached copy, its name.
fn name_variables(tree: &Tree, nodes: &mut [Value]) {
    fn walk(value: &mut Value, vars: &mut Option<crate::rest::Vars>, tree: &Tree) {
        match value {
            Value::Array(items) => items.iter_mut().for_each(|v| walk(v, vars, tree)),
            Value::Object(map) => {
                let alias = &map.get("value").unwrap_or(&Value::Null)["alias"];
                // Normalized values hold a local guid's numbers as floats.
                let local = || match (
                    alias["guid"]["sessionID"].as_f64(),
                    alias["guid"]["localID"].as_f64(),
                ) {
                    (Some(s), Some(l)) => Some(format!("VariableID:{}:{}", s as u64, l as u64)),
                    _ => None,
                };
                if map.get("dataType").and_then(Value::as_str) == Some("ALIAS")
                    && let Some(id) = crate::rest::variable_id(alias).or_else(local)
                {
                    let vars = vars.get_or_insert_with(|| crate::rest::variables(tree));
                    map.insert(
                        "variableName".into(),
                        json!(crate::variables::name(vars, &id)),
                    );
                    map.insert("variableId".into(), json!(id));
                }
                map.values_mut().for_each(|v| walk(v, vars, tree));
            }
            _ => {}
        }
    }
    let mut vars = None;
    for node in nodes {
        for cell in node["fields"]
            .as_object_mut()
            .into_iter()
            .flat_map(|f| f.values_mut())
        {
            if cell["state"] == "recorded" {
                walk(&mut cell["value"], &mut vars, tree);
            }
        }
    }
}

/// Stored field changes between two nodes compared as one, unsupported values left out.
fn changes(before: &Value, after: &Value) -> Vec<Value> {
    let absent = |n: &Value, path: &str| json!({"sourceNodeId":n["nodeId"],"sourceField":path,"state":"absent"});
    FIELDS
        .iter()
        .filter_map(|path| {
            let cell = |n: &Value| match &n["fields"][path] {
                Value::Null => absent(n, path),
                cell => cell.clone(),
            };
            let (b, a) = (cell(before), cell(after));
            (b["state"] != "unsupported"
                && a["state"] != "unsupported"
                && (b["state"] != a["state"] || b["value"] != a["value"]))
                .then(|| json!({"field":path,"before":b,"after":a}))
        })
        .collect()
}

/// The descendants in `nodes` that no pairing covers.
fn without_pair<'a>(
    nodes: &'a [Value],
    paired: &HashSet<usize>,
) -> impl Iterator<Item = &'a Value> {
    nodes
        .iter()
        .enumerate()
        .skip(1)
        .filter(move |(i, _)| !paired.contains(i))
        .map(|(_, n)| n)
}

fn groups(nodes: &[Value], path: &str, paired: &HashSet<usize>) -> BTreeMap<String, Value> {
    let mut groups = BTreeMap::new();
    for node in without_pair(nodes, paired) {
        let cell = &node["fields"][path];
        if cell["state"] != "recorded" {
            continue;
        }
        let key = json!([node["type"], cell["value"]]).to_string();
        let group = groups
            .entry(key)
            .or_insert_with(|| json!({"type":node["type"],"value":cell["value"],"sources":[]}));
        group["sources"]
            .as_array_mut()
            .unwrap()
            .push(json!({"sourceNodeId":node["nodeId"],"sourceField":path}));
    }
    groups
}

/// Compare two definitions in the same snapshot; source fields are not effective rendered values.
pub fn compare_variants(
    tree: &Tree,
    left: &str,
    right: &str,
    file: &Path,
) -> Result<Value, Box<dyn Error>> {
    if left.len() > MAX_ID_BYTES || right.len() > MAX_ID_BYTES {
        return Err(FigError::new(
            ErrorKind::InvalidArguments,
            "variant ID exceeds the comparison limit",
        )
        .into());
    }
    let set = component_set(tree, left)?;
    if left == right || component_set(tree, right)? != set {
        return Err(FigError::new(
            ErrorKind::UnsupportedNode,
            "select two different variants in the same component set",
        )
        .into());
    }
    let file = file.canonicalize().unwrap_or_else(|_| file.to_owned());
    let source = file.to_string_lossy();
    let mut input = Budget {
        work: 0,
        bytes: MAX_BYTES / 2 - 2,
        reasons: Default::default(),
    };
    let mut a = collect(tree, left, &mut input);
    let mut b = collect(tree, right, &mut input);
    name_variables(tree, &mut a);
    name_variables(tree, &mut b);
    let matched = pairs(tree, &a, &b);
    let paired = [
        matched.iter().map(|p| p.0).collect::<HashSet<_>>(),
        matched.iter().map(|p| p.1).collect::<HashSet<_>>(),
    ];
    let mut result = json!({"source":{"file":(source.len() <= MAX_ID_BYTES).then_some(source.as_ref())},
        "leftNodeId":left,"rightNodeId":right,"componentSetId":set,"interpretation":"stored-field-comparison",
        "verification":"not-compared-with-figma","businessConditions":"not-inferred","numberPrecision":4,"fields":FIELDS,
        "notCompared":["canvas-position","child-order-and-ambiguous-correspondence","instance-expansion","resolved-variables","text-style-ranges","vector-region-paints","geometry-and-pixels","prototype-and-business-conditions"]});
    let header = serde_json::to_vec(&result).expect("JSON header").len();
    let input_complete = input.reasons.is_empty();
    let mut output = Budget {
        work: 0,
        bytes: MAX_BYTES.saturating_sub(header + 1024),
        reasons: input.reasons,
    };
    if source.len() > MAX_ID_BYTES {
        output.stop("source_id_limit");
    }
    let mut roots = Vec::new();
    let mut collections = Vec::new();
    let mut unknown = Vec::new();
    let mut unpaired = Vec::new();
    let mut nested = 0;
    for ((root, nodes), paired) in [(left, &a), (right, &b)].into_iter().zip(&paired) {
        for (index, node) in nodes.iter().enumerate() {
            if node["type"] == "INSTANCE" {
                nested += 1;
            }
            if index != 0 && !paired.contains(&index) {
                output.push(&mut unpaired, json!({"variantNodeId":root,"nodeId":node["nodeId"],"name":node["name"],"nameTruncated":node["nameTruncated"],"type":node["type"],"state":"not-paired"}));
            }
            for cell in node["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, cell)| cell)
            {
                if cell["state"] == "unsupported" {
                    output.push(&mut unknown, cell.clone());
                }
            }
        }
    }
    for path in FIELDS {
        let before = a.first().map(|n| &n["fields"][path]);
        let after = b.first().map(|n| &n["fields"][path]);
        if let (Some(before), Some(after)) = (before, after)
            && before["state"] != "unsupported"
            && after["state"] != "unsupported"
            && (before["state"] != after["state"] || before["value"] != after["value"])
        {
            output.push(
                &mut roots,
                json!({"field":path,"before":before,"after":after}),
            );
        }
        let before = groups(&a, path, &paired[0]);
        let after = groups(&b, path, &paired[1]);
        if before.len() != after.len()
            || before.iter().any(|(key, g)| {
                after
                    .get(key)
                    .map(|v| v["sources"].as_array().unwrap().len())
                    != Some(g["sources"].as_array().unwrap().len())
            })
        {
            let complete = input_complete
                && without_pair(&a, &paired[0])
                    .chain(without_pair(&b, &paired[1]))
                    .all(|n| {
                        n["fields"][path]["state"] != "unsupported"
                            && n["fields"]["type"]["state"] != "unsupported"
                    });
            output.push(&mut collections,json!({"field":path,"state":if complete {"differences-recorded"} else {"unknown"},"scope":"stored-descendant-value-multiset","pairing":"not-inferred","before":before.into_values().collect::<Vec<_>>(),"after":after.into_values().collect::<Vec<_>>()}));
        }
    }
    let mut pair_list = Vec::new();
    for &(l, r) in &matched {
        let (before, after) = (&a[l], &b[r]);
        output.push(&mut pair_list, json!({"leftNodeId":before["nodeId"],"rightNodeId":after["nodeId"],
            "name":before["name"],"type":before["type"],"basis":"only-child-with-this-name-and-type-under-paired-parents",
            "changes":changes(before, after)}));
    }
    let pair_changed = pair_list
        .iter()
        .any(|p| p["changes"].as_array().is_some_and(|c| !c.is_empty()));
    result["state"] = if !roots.is_empty()
        || pair_changed
        || collections
            .iter()
            .any(|c| c["state"] == "differences-recorded")
    {
        "differences-recorded"
    } else if !unpaired.is_empty()
        || !unknown.is_empty()
        || nested != 0
        || !output.reasons.is_empty()
    {
        "unknown"
    } else {
        "no-differences-in-compared-fields"
    }
    .into();
    result["rootChanges"] = roots.into();
    result["pairedNodes"] = pair_list.into();
    result["collectionChanges"] = collections.into();
    result["unpairedNodes"] = unpaired.into();
    result["unknownFields"] = unknown.into();
    result["coverage"] = json!({"scope":"stored-definition-nodes","scannedLeft":a.len(),"scannedRight":b.len(),"nestedInstancesNotExpanded":nested,"workItems":input.work,"workLimit":MAX_WORK});
    result["truncated"] = (!output.reasons.is_empty()).into();
    result["reasons"] = json!(output.reasons);
    Ok(result)
}

#[cfg(test)]
mod tests;
