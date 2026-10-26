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
    "cornerRadius",
    "rectangleCornerRadiiIndependent",
    "rectangleTopLeftCornerRadius",
    "rectangleTopRightCornerRadius",
    "rectangleBottomRightCornerRadius",
    "rectangleBottomLeftCornerRadius",
    "stackMode",
    "stackSpacing",
    "stackPaddingTop",
    "stackPaddingRight",
    "stackPaddingBottom",
    "stackPaddingLeft",
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
        "visible" | "rectangleCornerRadiiIndependent" => matches!(value, K::Bool(_)),
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

fn groups(nodes: &[Value], path: &str) -> BTreeMap<String, Value> {
    let mut groups = BTreeMap::new();
    for node in nodes.iter().skip(1) {
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
    let a = collect(tree, left, &mut input);
    let b = collect(tree, right, &mut input);
    let mut result = json!({"source":{"file":(source.len() <= MAX_ID_BYTES).then_some(source.as_ref())},
        "leftNodeId":left,"rightNodeId":right,"componentSetId":set,"interpretation":"stored-field-comparison",
        "verification":"not-compared-with-figma","businessConditions":"not-inferred","numberPrecision":4,"fields":FIELDS,
        "notCompared":["canvas-position","child-order-and-correspondence","instance-expansion","resolved-variables","text-style-ranges","vector-region-paints","geometry-and-pixels","prototype-and-business-conditions"]});
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
    for (root, nodes) in [(left, &a), (right, &b)] {
        for (index, node) in nodes.iter().enumerate() {
            if node["type"] == "INSTANCE" {
                nested += 1;
            }
            if index != 0 {
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
        let before = groups(&a, path);
        let after = groups(&b, path);
        if before.len() != after.len()
            || before.iter().any(|(key, g)| {
                after
                    .get(key)
                    .map(|v| v["sources"].as_array().unwrap().len())
                    != Some(g["sources"].as_array().unwrap().len())
            })
        {
            let complete = input_complete
                && a.iter().skip(1).chain(b.iter().skip(1)).all(|n| {
                    n["fields"][path]["state"] != "unsupported"
                        && n["fields"]["type"]["state"] != "unsupported"
                });
            output.push(&mut collections,json!({"field":path,"state":if complete {"differences-recorded"} else {"unknown"},"scope":"stored-descendant-value-multiset","pairing":"not-inferred","before":before.into_values().collect::<Vec<_>>(),"after":after.into_values().collect::<Vec<_>>()}));
        }
    }
    result["state"] = if !roots.is_empty()
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
