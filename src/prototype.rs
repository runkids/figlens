//! Literal, bounded prototype fields and their snapshot provenance; no behavior inference.
use kiwi_schema::Value as K;
use serde_json::{Value, json};

use crate::{Tree, expand::Expanded, guid_key};

const MAX_INTERACTIONS: usize = 32;
const MAX_ACTIONS: usize = 16;
const MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub node_id: String,
    pub override_index: Option<usize>,
}

impl Source {
    fn field(&self) -> String {
        self.override_index.map_or_else(
            || "prototypeInteractions".into(),
            |index| format!("symbolData.symbolOverrides[{index}].prototypeInteractions"),
        )
    }

    fn record<'a>(&self, tree: &'a Tree) -> Option<&'a K<'a>> {
        let node = *tree.nodes.get(&self.node_id)?;
        if let Some(index) = self.override_index {
            let K::Array(list) = node.get("symbolData")?.get("symbolOverrides")? else {
                return None;
            };
            list.get(index)
        } else {
            Some(node)
        }
    }
}

fn scalar(value: Option<&K>, field: String) -> Value {
    let mut result = json!({"sourceField":field,"state":"absent"});
    if let Some(value) = value {
        result["state"] = "unsupported".into();
        let text = match value {
            K::Enum(_, name) => Some(*name),
            K::String(text) => Some(text.as_str()),
            _ => None,
        };
        if let Some(text) = text.filter(|s| !s.is_empty() && s.chars().take(129).count() <= 128) {
            result["state"] = "recorded".into();
            result["value"] = text.into();
        }
    }
    result
}

fn destination(tree: &Tree, value: Option<&K>, field: String) -> Value {
    let mut result = json!({"sourceField":field,"state":"absent"});
    let Some(value) = value else { return result };
    let valid = matches!(
        (value.get("sessionID"), value.get("localID")),
        (Some(K::UInt(_)), Some(K::UInt(_)))
    );
    if valid && guid_key(value).is_none() {
        result["reason"] = "unset_node_reference".into();
        return result;
    }
    let Some(id) = valid.then(|| guid_key(value)).flatten() else {
        result["state"] = "unsupported".into();
        return result;
    };
    result["nodeId"] = id.clone().into();
    result["state"] = if tree.nodes.contains_key(&id) {
        "resolved"
    } else {
        "unresolved"
    }
    .into();
    if let Some(K::String(name)) = tree.nodes.get(&id).and_then(|n| n.get("name")) {
        result["name"] = name.chars().take(256).collect::<String>().into();
        result["nameTruncated"] = (name.chars().take(257).count() > 256).into();
    }
    result
}

fn read(tree: &Tree, source: &Source, basis: &str) -> Value {
    let field = source.field();
    let record = source.record(tree);
    let value = record.and_then(|n| n.get("prototypeInteractions"));
    let mut result = json!({"sourceNodeId":source.node_id,"sourceField":field,"basis":basis,
        "verification":"not-compared-with-figma","interpretation":"stored-fields-only",
        "state": if !matches!(record,Some(K::Object(..))) { "unsupported" } else if value.is_none() { "absent" } else { "unsupported" },
        "interactions":[],"truncated":false});
    if basis == "component-definition" {
        result["inheritanceState"] = "unsupported".into();
        result["reason"] = "instance_inheritance_not_verified".into();
    }
    let Some(K::Array(list)) = value else {
        return result;
    };
    result["state"] = "recorded".into();
    let mut interactions = Vec::new();
    let mut remaining = MAX_BYTES - 1024;
    let mut truncated = list.len() > MAX_INTERACTIONS;
    for (index, interaction) in list.iter().take(MAX_INTERACTIONS).enumerate() {
        if matches!(interaction.get("isDeleted"), Some(K::Bool(true))) {
            continue;
        }
        let base = format!("{field}[{index}]");
        let mut entry = json!({"index":index,"sourceField":base,"state":"recorded","detailsState":"not-extracted"});
        let malformed = !matches!(interaction, K::Object(..))
            || interaction
                .get("isDeleted")
                .is_some_and(|v| !matches!(v, K::Bool(_)));
        let event = interaction.get("event");
        let mut trigger = scalar(
            event.and_then(|e| e.get("interactionType")),
            format!("{base}.event.interactionType"),
        );
        if malformed || event.is_some_and(|v| !matches!(v, K::Object(..))) {
            trigger["state"] = "unsupported".into();
            trigger.as_object_mut().unwrap().remove("value");
            entry["state"] = "unsupported".into();
        }
        entry["trigger"] = trigger;
        let actions = interaction.get("actions");
        entry["actionsState"] = if malformed {
            "unsupported"
        } else if actions.is_none() {
            "absent"
        } else if matches!(actions, Some(K::Array(_))) {
            "recorded"
        } else {
            "unsupported"
        }
        .into();
        entry["actionsTruncated"] = false.into();
        let header_size = serde_json::to_vec(&entry).expect("JSON evidence").len() + 16;
        if header_size > remaining {
            truncated = true;
            break;
        }
        remaining -= header_size;
        let mut projected = Vec::new();
        let mut actions_truncated = false;
        if !malformed && let Some(K::Array(actions)) = actions {
            actions_truncated = actions.len() > MAX_ACTIONS;
            for (action_index, action) in actions.iter().take(MAX_ACTIONS).enumerate() {
                let action_field = format!("{base}.actions[{action_index}]");
                let mut item = json!({"index":action_index,"sourceField":action_field,"state":"recorded","detailsState":"not-extracted",
                    "navigationType":scalar(action.get("navigationType"),format!("{action_field}.navigationType")),
                    "connectionType":scalar(action.get("connectionType"),format!("{action_field}.connectionType")),
                    "destination":destination(tree,action.get("transitionNodeID"),format!("{action_field}.transitionNodeID"))});
                if !matches!(action, K::Object(..)) {
                    item["state"] = "unsupported".into();
                    for key in ["navigationType", "connectionType", "destination"] {
                        item[key]["state"] = "unsupported".into();
                    }
                }
                let bytes = serde_json::to_vec(&item).expect("JSON evidence").len() + 1;
                if bytes > remaining {
                    actions_truncated = true;
                    break;
                }
                remaining -= bytes;
                projected.push(item);
            }
        }
        entry["actionsTruncated"] = actions_truncated.into();
        truncated |= actions_truncated;
        entry["actions"] = projected.into();
        interactions.push(entry);
    }
    result["interactions"] = interactions.into();
    result["truncated"] = truncated.into();
    result
}

/// A missing instance-local list does not establish inheritance from the main component.
pub(crate) fn node(
    tree: &Tree,
    id: &str,
    expanded: Option<&Expanded>,
    kind: &str,
) -> Option<Value> {
    if let Some(source) = expanded.and_then(|e| e.prototype_source.as_ref()) {
        let mut evidence = read(
            tree,
            source,
            if source.override_index.is_some() {
                "instance-override"
            } else {
                "instance-local"
            },
        );
        if source.override_index.is_none()
            && let Some(expanded) = expanded.filter(|e| e.source != source.node_id)
        {
            evidence["swappedDefinitionId"] = expanded.source.clone().into();
            evidence["precedenceState"] = "unsupported".into();
            evidence["reason"] = "swapped_instance_prototype_precedence_not_verified".into();
        }
        return Some(evidence);
    }
    let source_id = expanded.map_or(id, |e| e.source.as_str());
    let raw = tree.nodes.get(source_id)?;
    let source = Source {
        node_id: source_id.into(),
        override_index: None,
    };
    if raw.get("prototypeInteractions").is_some() {
        return Some(read(
            tree,
            &source,
            if kind == "INSTANCE" && crate::expand::plugin_type(raw) == "COMPONENT" {
                "component-definition"
            } else if crate::expand::plugin_type(raw) == "INSTANCE" {
                "instance-local"
            } else if expanded.is_some() {
                "expanded-source"
            } else {
                "stored-node"
            },
        ));
    }
    if kind != "INSTANCE" {
        return None;
    }
    // A swapped expanded instance already points at the main copied by the expander.
    if crate::expand::plugin_type(raw) == "COMPONENT" {
        return None;
    }
    let main = raw
        .get("overriddenSymbolID")
        .and_then(guid_key)
        .or_else(|| raw.get("symbolData")?.get("symbolID").and_then(guid_key));
    let Some(component) = main
        .as_ref()
        .and_then(|id| tree.nodes.get(id))
        .filter(|n| crate::expand::plugin_type(n) == "COMPONENT")
    else {
        let mut evidence = read(tree, &source, "stored-node");
        evidence["state"] = "unsupported".into();
        evidence["reason"] = "component_source_unavailable".into();
        evidence["componentId"] = json!(main);
        return Some(evidence);
    };
    if component.get("prototypeInteractions").is_some() {
        return Some(read(
            tree,
            &Source {
                node_id: main.unwrap(),
                override_index: None,
            },
            "component-definition",
        ));
    }
    None
}

pub(crate) fn stored(tree: &Tree, id: &str) -> Option<Value> {
    node(tree, id, None, "COMPONENT")
}

#[cfg(test)]
mod tests;
