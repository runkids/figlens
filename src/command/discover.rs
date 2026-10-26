//! Bounded discovery output; full design data stays in `nodes` and `bundle`.

use std::error::Error;
use std::path::Path;

use figlens::expand::{plugin_type, subtree};
use figlens::{Tree, guid_key};
use serde_json::{Value, json};

const MAX_NODES: usize = 200;
const TEXT_CHARS: usize = 200;

fn text_preview(summary: &mut Value, text: &str) {
    summary["characters"] = text.chars().take(TEXT_CHARS).collect::<String>().into();
    if text.chars().count() > TEXT_CHARS {
        summary["textTruncated"] = true.into();
    }
}

pub fn find(
    tree: &Tree,
    query: &str,
    root: &str,
    kind: Option<&str>,
    limit: usize,
) -> Result<Value, Box<dyn Error>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Err("find needs a non-empty query".into());
    }
    if !(1..=MAX_NODES).contains(&limit) {
        return Err("--limit takes 1-200".into());
    }
    let mut results = Vec::new();
    let mut truncated = false;
    for id in subtree(tree, root) {
        let node = tree.nodes[&id];
        let node_type = plugin_type(node);
        if kind.is_some_and(|kind| kind != node_type) {
            continue;
        }
        let name = node.get("name").map(|v| v.as_string()).unwrap_or("");
        let text = node
            .get("textData")
            .and_then(|v| v.get("characters"))
            .map(|v| v.as_string())
            .unwrap_or("");
        let name_match = name.to_lowercase().contains(&query);
        let text_match = text.to_lowercase().contains(&query);
        if !name_match && !text_match {
            continue;
        }
        if results.len() == limit {
            truncated = true;
            break;
        }
        let mut path = Vec::new();
        let mut ancestor = Some(id.clone());
        while let Some(id) = ancestor {
            let Some(node) = tree.nodes.get(&id) else {
                break;
            };
            path.push(
                json!({"id": id, "name": node.get("name").map(|v| v.as_string()).unwrap_or("")}),
            );
            ancestor = node
                .get("parentIndex")
                .and_then(|p| p.get("guid"))
                .and_then(guid_key);
        }
        path.reverse();
        let mut summary = json!({
            "id": id, "type": node_type, "name": name, "path": path,
            "childCount": tree.children.get(&id).map_or(0, Vec::len),
            "matchedOn": {"name": name_match, "text": text_match},
        });
        if let Some(size) = node.get("size") {
            summary["size"] = json!({
                "width": size.get("x").map_or(0.0, |v| v.as_float()),
                "height": size.get("y").map_or(0.0, |v| v.as_float()),
            });
        }
        if node_type == "TEXT" {
            text_preview(&mut summary, text);
        }
        results.push(summary);
    }
    Ok(json!({"root": root, "results": results, "truncated": truncated}))
}

pub fn inspect(tree: &Tree, id: &str, depth: usize) -> Result<Value, Box<dyn Error>> {
    if depth > 10 {
        return Err("--depth takes 0-10 for inspect".into());
    }
    let doc = figlens::rest::resolve_nodes(tree, id)?;
    Ok(inspect_document(&doc["document"], depth))
}

pub fn inspect_images(
    tree: &Tree,
    id: &str,
    depth: usize,
    fig: &Path,
) -> Result<Value, Box<dyn Error>> {
    if depth > 10 {
        return Err("--depth takes 0-10 for inspect".into());
    }
    let doc = figlens::rest::resolve_render_nodes(tree, id)?;
    let mut result = inspect_document(&doc["document"], depth);
    result["images"] = figlens::images::inspect(fig, &doc["document"])?;
    Ok(result)
}

fn inspect_document(document: &Value, depth: usize) -> Value {
    let mut remaining = MAX_NODES;
    let mut truncated = false;
    let node = summarize(document, depth, &mut remaining, &mut truncated);
    json!({"node": node, "truncated": truncated})
}

fn summarize(node: &Value, depth: usize, remaining: &mut usize, truncated: &mut bool) -> Value {
    *remaining -= 1;
    let mut summary = json!({});
    for key in [
        "id",
        "type",
        "name",
        "visible",
        "absoluteBoundingBox",
        "rotation",
        "componentId",
        "layoutMode",
        "layoutWrap",
        "layoutPositioning",
        "constraints",
        "layoutGrow",
        "layoutAlign",
        "layoutSizingHorizontal",
        "layoutSizingVertical",
        "itemSpacing",
        "counterAxisSpacing",
        "paddingLeft",
        "paddingRight",
        "paddingTop",
        "paddingBottom",
        "primaryAxisAlignItems",
        "counterAxisAlignItems",
    ] {
        if let Some(value) = node.get(key) {
            summary[key] = value.clone();
        }
    }
    if let Some(semantics) = node["_figlens"]["semantics"].as_object() {
        let mut evidence = serde_json::Map::new();
        for (key, value) in semantics {
            if key != "componentPropertyDefinitions" {
                evidence.insert(key.clone(), value.clone());
            }
        }
        if let Some(definitions) = semantics.get("componentPropertyDefinitions") {
            let mut compact = json!({"source":definitions["source"], "state":definitions["state"]});
            if let Some(entries) = definitions["entries"].as_array() {
                compact["entries"] = entries.iter().take(20).cloned().collect();
                compact["count"] = json!(entries.len());
                compact["truncated"] = json!(entries.len() > 20);
                *truncated |= entries.len() > 20;
            }
            evidence.insert("componentPropertyDefinitions".into(), compact);
        }
        summary["_figlens"] = json!({"semantics":evidence});
    }
    if let Some(definitions) = node["componentPropertyDefinitions"].as_object() {
        let compact: serde_json::Map<String, Value> = definitions
            .iter()
            .take(20)
            .map(|(name, definition)| (name.clone(), json!({"type":definition["type"]})))
            .collect();
        summary["componentPropertyDefinitions"] = compact.into();
        summary["componentPropertyDefinitionCount"] = json!(definitions.len());
        *truncated |= definitions.len() > 20;
    }
    if let Some(items) = node["_figlens"]["renderDiagnostics"].as_array() {
        if summary.get("_figlens").is_none() {
            summary["_figlens"] = json!({});
        }
        summary["_figlens"]["renderDiagnostics"] = json!(items.iter().take(20).collect::<Vec<_>>());
        summary["_figlens"]["renderDiagnosticsTruncated"] = json!(items.len() > 20);
        *truncated |= items.len() > 20;
    }
    if let Some(text) = node["characters"].as_str() {
        text_preview(&mut summary, text);
    }
    if let Some(style) = node["style"].as_object() {
        for key in [
            "fontFamily",
            "fontWeight",
            "fontSize",
            "lineHeightPx",
            "letterSpacing",
            "textAlignHorizontal",
            "textAlignVertical",
        ] {
            if let Some(value) = style.get(key) {
                if summary.get("style").is_none() {
                    summary["style"] = json!({});
                }
                summary["style"][key] = value.clone();
            }
        }
    }
    let children = node["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    summary["childCount"] = children.len().into();
    if depth > 0 && !children.is_empty() {
        let mut included = Vec::new();
        for child in children {
            if *remaining == 0 {
                break;
            }
            included.push(summarize(child, depth - 1, remaining, truncated));
        }
        if included.len() < children.len() {
            *truncated = true;
        }
        summary["children"] = included.into();
    } else if !children.is_empty() {
        *truncated = true;
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwi_schema::Value as Kiwi;

    #[test]
    fn inspect_bounds_definition_summaries_and_keeps_diagnostics() {
        let definitions: serde_json::Map<String, Value> = (0..21)
            .map(|i| {
                (
                    format!("Label#{i}"),
                    json!({"type":"TEXT","defaultValue":"full default"}),
                )
            })
            .collect();
        let node = json!({"id":"1:1","componentPropertyDefinitions":definitions,"_figlens":{"semantics":{"verification":"not-compared-with-figma","componentPropertyDefinitions":{"source":"componentPropDefs","state":"recorded","entries":vec![json!({"state":"recorded"});21]}},"renderDiagnostics":[{"state":"skipped"}]}});
        let result = inspect_document(&node, 0);
        let summary = &result["node"];
        assert_eq!(result["truncated"], true);
        assert_eq!(summary["componentPropertyDefinitionCount"], 21);
        assert_eq!(
            summary["componentPropertyDefinitions"]
                .as_object()
                .unwrap()
                .len(),
            20
        );
        assert!(
            summary["componentPropertyDefinitions"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.get("defaultValue").is_none())
        );
        let evidence = &summary["_figlens"]["semantics"]["componentPropertyDefinitions"];
        assert_eq!(evidence["count"], 21);
        assert_eq!(evidence["entries"].as_array().unwrap().len(), 20);
        assert_eq!(
            summary["_figlens"]["renderDiagnostics"][0]["state"],
            "skipped"
        );
    }

    #[test]
    fn inspect_bounds_render_diagnostics_and_marks_truncation() {
        let node = json!({"id":"1:1", "_figlens":{"renderDiagnostics":vec![json!({"state":"skipped"});21]}});
        let mut truncated = false;
        let summary = summarize(&node, 1, &mut 200, &mut truncated);
        assert_eq!(
            summary["_figlens"]["renderDiagnostics"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert_eq!(summary["_figlens"]["renderDiagnosticsTruncated"], true);
        assert!(truncated);
    }

    fn obj(fields: Vec<(&'static str, Kiwi<'static>)>) -> Kiwi<'static> {
        Kiwi::Object("Test", fields.into_iter().collect())
    }

    fn guid(id: u32) -> Kiwi<'static> {
        obj(vec![
            ("sessionID", Kiwi::UInt(0)),
            ("localID", Kiwi::UInt(id)),
        ])
    }

    fn message() -> Kiwi<'static> {
        let node = |id, parent, kind, name: &str, extra: Vec<_>| {
            let mut fields = vec![
                ("guid", guid(id)),
                ("type", Kiwi::Enum("NodeType", kind)),
                ("name", Kiwi::String(name.into())),
            ];
            if let Some(parent) = parent {
                fields.push((
                    "parentIndex",
                    obj(vec![
                        ("guid", guid(parent)),
                        ("position", Kiwi::String(format!("{id:03}"))),
                    ]),
                ));
            }
            fields.extend(extra);
            obj(fields)
        };
        let text = |s: String| vec![("textData", obj(vec![("characters", Kiwi::String(s))]))];
        obj(vec![(
            "nodeChanges",
            Kiwi::Array(vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some(0), "CANVAS", "Page", vec![]),
                node(2, Some(1), "FRAME", "Login", vec![]),
                node(
                    3,
                    Some(2),
                    "TEXT",
                    "Label",
                    text(format!("Welcome ÉCOLE {}", "貓".repeat(210))),
                ),
                node(4, Some(1), "FRAME", "Login", vec![]),
                node(
                    5,
                    Some(4),
                    "TEXT",
                    "Login text",
                    text("Other page text".into()),
                ),
                node(
                    6,
                    Some(1),
                    "FRAME",
                    "Login group",
                    vec![("resizeToFit", Kiwi::Bool(true))],
                ),
                node(7, Some(1), "SYMBOL", "Button", vec![]),
                node(8, Some(7), "TEXT", "Button label", text("Click".into())),
                node(
                    9,
                    Some(2),
                    "INSTANCE",
                    "Button instance",
                    vec![("symbolData", obj(vec![("symbolID", guid(7))]))],
                ),
            ]),
        )])
    }

    #[test]
    fn search_matches_text_and_names_with_scope_types_and_real_truncation() {
        let message = message();
        let tree = Tree::new(&message);
        let found = find(&tree, "école", "0:2", Some("TEXT"), 20).unwrap();
        assert_eq!(found["results"][0]["id"], "0:3");
        assert_eq!(
            found["results"][0]["matchedOn"],
            json!({"name": false, "text": true})
        );
        assert_eq!(
            found["results"][0]["path"][2],
            json!({"id": "0:2", "name": "Login"})
        );
        assert_eq!(
            found["results"][0]["characters"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            TEXT_CHARS
        );
        assert_eq!(found["results"][0]["textTruncated"], true);
        assert_eq!(found["truncated"], false);
        let first = find(&tree, "LOGIN", "0:0", Some("FRAME"), 1).unwrap();
        assert_eq!(first["results"][0]["id"], "0:2");
        assert_eq!(first["truncated"], true);
        let all = find(&tree, "LOGIN", "0:0", Some("FRAME"), 2).unwrap();
        assert_eq!(all["results"][1]["id"], "0:4");
        assert_eq!(all["truncated"], false);
        assert_eq!(
            find(&tree, "LOGIN", "0:0", Some("GROUP"), 20).unwrap()["results"][0]["id"],
            "0:6"
        );
        assert!(
            find(&tree, "other page", "0:2", None, 20).unwrap()["results"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // Stored-layer search does not pretend to search expanded component copies.
        assert!(
            find(&tree, "click", "0:2", None, 20).unwrap()["results"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn inspect_projects_rest_fields_and_expands_instances_with_depth_limits() {
        let message = message();
        let tree = Tree::new(&message);
        let full = figlens::rest::nodes(&tree, "0:2");
        let shallow = inspect(&tree, "0:2", 0).unwrap();
        assert_eq!(shallow["node"]["childCount"], 2);
        assert!(shallow["node"].get("children").is_none());
        assert_eq!(shallow["truncated"], true);
        let summary = inspect(&tree, "0:2", 2).unwrap();
        assert_eq!(summary["truncated"], false);
        assert_eq!(
            summary["node"]["absoluteBoundingBox"],
            full["document"]["absoluteBoundingBox"]
        );
        let child = &summary["node"]["children"][1]["children"][0];
        assert_eq!(child["id"], "I0:9;0:8");
        assert_eq!(child["characters"], "Click");
        assert!(child.get("fills").is_none());
    }

    #[test]
    fn summary_budget_is_shared_across_siblings() {
        let doc = json!({"id": "root", "children": (0..250).map(|i| json!({"id": i})).collect::<Vec<_>>()});
        let mut remaining = MAX_NODES;
        let mut truncated = false;
        let summary = summarize(&doc, 1, &mut remaining, &mut truncated);
        assert_eq!(summary["childCount"], 250);
        assert_eq!(summary["children"].as_array().unwrap().len(), 199);
        assert_eq!(remaining, 0);
        assert!(truncated);
    }
}
