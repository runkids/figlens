//! Bounded component-set evidence from an already resolved subtree; no alternate-state expansion.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use kiwi_schema::Value as Kiwi;
use serde_json::{Value, json};

use crate::{Tree, expand::plugin_type, guid_key};

const MAX_WORK: usize = 10_000;
const MAX_INSTANCES: usize = 200;
const MAX_SETS: usize = 32;
const MAX_VARIANTS: usize = 256;
const MAX_SET_VARIANTS: usize = 64;
const MAX_PROPERTIES: usize = 32;
const MAX_BYTES: usize = 128 * 1024;
const MAX_ID_BYTES: usize = 4096;
const MAX_TEXT_CHARS: usize = 256;

struct Budget {
    work: usize,
    bytes: usize,
    reasons: BTreeSet<&'static str>,
}

impl Budget {
    fn stop(&mut self, reason: &'static str) {
        self.reasons.insert(reason);
    }

    fn step(&mut self) -> bool {
        if self.work == MAX_WORK {
            self.stop("work_limit");
            return false;
        }
        self.work += 1;
        true
    }

    fn push(&mut self, list: &mut Vec<Value>, item: Value) -> bool {
        let bytes = serde_json::to_vec(&item).expect("JSON evidence").len() + 1;
        if bytes > self.bytes {
            self.stop("byte_limit");
            return false;
        }
        self.bytes -= bytes;
        list.push(item);
        true
    }

    fn name(&mut self, text: &str) -> Value {
        let mut chars = text.chars();
        let name: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
        let truncated = chars.next().is_some();
        if truncated {
            self.stop("text_limit");
        }
        json!({"name": name, "nameTruncated": truncated})
    }

    fn id(&mut self, id: &str) -> bool {
        if id.len() > MAX_ID_BYTES {
            self.stop("id_limit");
            return false;
        }
        true
    }

    fn prototype(
        &mut self,
        nodes: &mut Vec<Value>,
        seen: &mut HashSet<String>,
        id: &str,
        mut evidence: Value,
    ) {
        if !self.id(id) || !seen.insert(id.to_owned()) {
            return;
        }
        if evidence["truncated"] == true {
            self.stop("prototype_limit");
        }
        evidence["nodeId"] = id.into();
        self.push(nodes, evidence);
    }
}

fn string<'a>(node: &'a Kiwi, field: &str) -> Option<&'a str> {
    match node.get(field)? {
        Kiwi::String(s) => Some(s),
        _ => None,
    }
}

fn array<'a>(node: &'a Kiwi, field: &str) -> Option<&'a [Kiwi<'a>]> {
    match node.get(field)? {
        Kiwi::Array(items) => Some(items),
        _ => None,
    }
}

fn current_properties(node: &Value, budget: &mut Budget) -> Vec<Value> {
    let mut out = Vec::new();
    for (key, property) in node["componentProperties"]
        .as_object()
        .into_iter()
        .flatten()
    {
        if out.len() == MAX_PROPERTIES || !budget.step() {
            budget.stop("property_limit");
            break;
        }
        let mut item = budget.name(key);
        item["type"] = property["type"]
            .as_str()
            .filter(|kind| matches!(*kind, "VARIANT" | "BOOLEAN" | "TEXT" | "INSTANCE_SWAP"))
            .unwrap_or("unknown")
            .into();
        let value = &property["value"];
        let supported = match item["type"].as_str() {
            Some("BOOLEAN") => value.is_boolean(),
            Some("VARIANT" | "TEXT" | "INSTANCE_SWAP") => value
                .as_str()
                .is_some_and(|s| s.chars().take(MAX_TEXT_CHARS + 1).count() <= MAX_TEXT_CHARS),
            _ => false,
        };
        if supported {
            item["value"] = value.clone();
            item["state"] = "recorded".into();
        } else {
            item["state"] = "unsupported".into();
            item["reason"] = "missing_or_oversized_scalar_value".into();
        }
        out.push(item);
    }
    out
}

fn definition_id(value: &Kiwi) -> Option<String> {
    match (value.get("sessionID"), value.get("localID")) {
        (Some(Kiwi::UInt(_)), Some(Kiwi::UInt(_))) => guid_key(value),
        _ => None,
    }
}

fn variant_properties(
    node: &Kiwi,
    component: &str,
    set: &str,
    definitions: &HashMap<String, &Kiwi>,
    budget: &mut Budget,
) -> Value {
    let Some(specs) = array(node, "variantPropSpecs") else {
        return json!({"state": if node.get("variantPropSpecs").is_some() { "unsupported" } else { "absent" }, "entries": []});
    };
    let mut entries = Vec::new();
    for spec in specs {
        if entries.len() == MAX_PROPERTIES || !budget.step() {
            budget.stop("property_limit");
            break;
        }
        let id = spec.get("propDefId").and_then(definition_id);
        let def = id.as_ref().and_then(|id| definitions.get(id));
        let mut entry = json!({"definitionId": id, "definitionNodeId": set, "sourceNodeId": component, "source": "variantPropSpecs", "state": "unsupported", "reason": "unresolved_definition_or_value"});
        if let Some(def) = def {
            if let Some(name) = string(def, "name") {
                entry
                    .as_object_mut()
                    .unwrap()
                    .extend(budget.name(name).as_object().unwrap().clone());
            }
            if matches!(def.get("type"), Some(Kiwi::Enum(_, "VARIANT")))
                && string(def, "name").is_some()
                && let Some(value) = string(spec, "value")
                && value.chars().take(MAX_TEXT_CHARS + 1).count() <= MAX_TEXT_CHARS
            {
                entry["value"] = value.into();
                entry["state"] = "recorded".into();
                entry.as_object_mut().unwrap().remove("reason");
            }
        }
        entries.push(entry);
    }
    json!({"state": "recorded", "entries": entries})
}

fn component_set(
    tree: &Tree,
    set: &str,
    current: Option<&str>,
    variant_count: &mut usize,
    budget: &mut Budget,
    triggers: &mut Vec<Value>,
    seen_triggers: &mut HashSet<String>,
) -> Value {
    let raw = tree.nodes[set];
    let mut result = budget.name(string(raw, "name").unwrap_or(""));
    result["nodeId"] = set.into();
    result["relation"] = "component-set-membership".into();
    let mut definitions = HashMap::new();
    for def in array(raw, "componentPropDefs").unwrap_or(&[]) {
        if !budget.step() {
            break;
        }
        if let Some(id) = def.get("id").and_then(definition_id) {
            definitions.insert(id, def);
        }
    }
    let children = tree.children.get(set).map_or(&[][..], Vec::as_slice);
    let mut variants = Vec::new();
    let mut complete = true;
    // Keep the selected component available even when its set exceeds the listing budget.
    let candidates = current.into_iter().chain(
        children
            .iter()
            .map(String::as_str)
            .filter(|id| Some(*id) != current),
    );
    for id in candidates {
        if !budget.step() {
            complete = false;
            break;
        }
        let Some(node) = tree.nodes.get(id).filter(|n| plugin_type(n) == "COMPONENT") else {
            continue;
        };
        if variants.len() == MAX_SET_VARIANTS || *variant_count == MAX_VARIANTS {
            budget.stop("variant_limit");
            complete = false;
            break;
        }
        if !budget.id(id) {
            complete = false;
            continue;
        }
        let mut variant = budget.name(string(node, "name").unwrap_or(""));
        variant["nodeId"] = id.into();
        variant["properties"] = variant_properties(node, id, set, &definitions, budget);
        if !budget.push(&mut variants, variant) {
            complete = false;
            break;
        }
        *variant_count += 1;
        if let Some(evidence) = crate::prototype::stored(tree, id) {
            budget.prototype(triggers, seen_triggers, id, evidence);
        }
    }
    result["variants"] = variants.into();
    result["membershipComplete"] = complete.into();
    result
}

/// `document` is the effective selected subtree; all IDs in this result belong to `file`.
pub fn inspect(tree: &Tree, document: &Value, file: &Path) -> Value {
    let file = file.canonicalize().unwrap_or_else(|_| file.to_owned());
    let source = file.to_string_lossy();
    let mut report = json!({"source": {"file": if source.len() <= MAX_ID_BYTES { Some(source.as_ref()) } else { None }},
        "scopeNodeId": document["id"].as_str().filter(|id| id.len() <= MAX_ID_BYTES),
        "triggerEvidence": {"state": "absent", "scope": "selected-subtree-and-variant-roots", "detailsState":"not-extracted", "businessConditions":"not-inferred", "nodes":[]}});
    let header_bytes = serde_json::to_vec(&report).expect("JSON evidence").len();
    let mut budget = Budget {
        work: 0,
        bytes: MAX_BYTES.saturating_sub(header_bytes + 1024),
        reasons: BTreeSet::new(),
    };
    let mut instances = Vec::new();
    let mut sets = Vec::new();
    let mut seen = HashSet::new();
    let mut variant_count = 0;
    let mut scanned = 0;
    let mut hidden = false;
    let mut triggers = Vec::new();
    let mut seen_triggers = HashSet::new();
    let mut stack = vec![std::slice::from_ref(document).iter()];
    if !budget.id(&source) {
        stack.clear();
    }
    while let Some(iter) = stack.last_mut() {
        let Some(node) = iter.next() else {
            stack.pop();
            continue;
        };
        if !budget.step() {
            break;
        }
        scanned += 1;
        let id = node["id"].as_str().unwrap_or("");
        let kind = node["type"].as_str().unwrap_or("");
        if node["_figlens"]["prototype"].is_object() {
            budget.prototype(
                &mut triggers,
                &mut seen_triggers,
                id,
                node["_figlens"]["prototype"].clone(),
            );
        }
        hidden |= node["visible"] == false && (id.starts_with('I') || kind == "INSTANCE");
        if matches!(kind, "INSTANCE" | "COMPONENT" | "COMPONENT_SET") && budget.id(id) {
            let source = node["_figlens"]["semantics"]["instanceContext"]["sourceNodeId"].as_str();
            // Swapped expanded instances copy the effective main, whose raw node is a COMPONENT.
            let main = source
                .filter(|s| {
                    tree.nodes
                        .get(*s)
                        .is_some_and(|n| plugin_type(n) == "COMPONENT")
                })
                .or_else(|| node["componentId"].as_str())
                .or_else(|| (kind == "COMPONENT").then_some(id));
            let raw = main.and_then(|m| tree.nodes.get(m));
            let set = if kind == "COMPONENT_SET" {
                source
                    .or(Some(id))
                    .filter(|s| {
                        tree.nodes
                            .get(*s)
                            .is_some_and(|n| plugin_type(n) == "COMPONENT_SET")
                    })
                    .map(str::to_owned)
            } else {
                raw.filter(|n| plugin_type(n) == "COMPONENT")
                    .and_then(|n| n.get("parentIndex"))
                    .and_then(|p| p.get("guid"))
                    .and_then(guid_key)
                    .filter(|s| {
                        tree.nodes
                            .get(s)
                            .is_some_and(|n| plugin_type(n) == "COMPONENT_SET")
                    })
            };
            if let Some(set) = set.as_deref().filter(|s| !seen.contains(*s)) {
                if sets.len() == MAX_SETS {
                    budget.stop("set_limit");
                } else if budget.id(set) {
                    seen.insert(set.to_owned());
                    let mut result = component_set(
                        tree,
                        set,
                        main,
                        &mut variant_count,
                        &mut budget,
                        &mut triggers,
                        &mut seen_triggers,
                    );
                    // Variant bytes are charged individually; charge only the set header here.
                    let variants = result.as_object_mut().unwrap().remove("variants").unwrap();
                    if budget.push(&mut sets, result) {
                        sets.last_mut().unwrap()["variants"] = variants;
                    }
                }
            }
            if kind == "INSTANCE" {
                if instances.len() == MAX_INSTANCES {
                    budget.stop("instance_limit");
                } else {
                    let mut item = json!({"nodeId": id, "componentId": main, "componentSetId": set,
                        "state": if raw.is_none() { "missing_component" } else if raw.is_some_and(|n| plugin_type(n) != "COMPONENT") { "unsupported_component" } else if set.is_none() { "no_component_set" } else { "recorded" },
                        "visible": node["visible"] != false});
                    if main == node["componentId"].as_str()
                        && node["componentProperties"].is_object()
                    {
                        item["currentProperties"] = current_properties(node, &mut budget).into();
                        item["currentPropertiesState"] = "effective-instance".into();
                    } else {
                        item["currentProperties"] = json!([]);
                        item["currentPropertiesState"] = "unsupported".into();
                    }
                    budget.push(&mut instances, item);
                }
            }
        }
        if let Some(children) = node["children"].as_array() {
            stack.push(children.iter());
        }
    }
    report["instances"] = instances.into();
    report["sets"] = sets.into();
    report["triggerEvidence"]["state"] = if triggers.iter().any(|n| n["state"] == "recorded") {
        "recorded"
    } else if triggers.is_empty() {
        "absent"
    } else {
        "unsupported"
    }
    .into();
    report["triggerEvidence"]["nodes"] = triggers.into();
    report["coverage"] = json!({"scope": "snapshot-only", "scannedNodes": scanned, "hiddenSubtreesNotExpanded": hidden});
    report["truncated"] = (!budget.reasons.is_empty()).into();
    report["reasons"] = json!(budget.reasons);
    report
}

#[cfg(test)]
mod tests;
