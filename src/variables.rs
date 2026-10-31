//! `design/variables.json` as the plugin writes it: every variable a frame's nodes bind
//! (and the variables those alias), their collections, and the ids the file cannot resolve.

use serde_json::{Value as Json, json};

use crate::Tree;
use crate::rest::{Vars, alias, copies, guid, variables as file_variables};

/// Variables referenced by the REST `document` (or, without one, every local variable).
pub fn variables(tree: &Tree, document: Option<&Json>) -> Json {
    let vars = file_variables(tree);
    let mut ids: Vec<String> = Vec::new();
    match document {
        Some(doc) => aliases(doc, &mut ids),
        None => {
            // Local variables are the ones without a library key.
            let mut local: Vec<&Json> = vars
                .values()
                .flatten()
                .filter(|v| v["type"] == "VARIABLE" && !v["key"].is_string())
                .collect();
            local.sort_by_key(|v| v["sortPosition"].as_str().unwrap_or("").to_owned());
            for v in local {
                push(
                    &mut ids,
                    guid(&v["guid"]).map(|g| format!("VariableID:{g}")),
                );
            }
        }
    }
    let (mut found, mut missing, mut collections) = (Vec::new(), Vec::new(), Vec::<Json>::new());
    // Aliases can point at further aliases, so the list grows while it is read.
    let mut i = 0;
    while i < ids.len() {
        let id = ids[i].clone();
        i += 1;
        let Some(v) = lookup(&vars, &id) else {
            missing.push(json!(id));
            continue;
        };
        let snapshot = snapshot(&id, v);
        aliases(&snapshot["valuesByMode"], &mut ids);
        let set = &v["variableSetID"];
        let set_id = collection_id(set);
        // Without its collection a variable has no modes, so the plugin counts it as unresolved.
        let Some(set_node) = copies(&vars, set).into_iter().next() else {
            missing.push(json!(id));
            continue;
        };
        if !collections
            .iter()
            .any(|c| c["id"].as_str() == set_id.as_deref())
        {
            collections.push(collection(set_id.as_deref().unwrap_or(""), set_node));
        }
        found.push(snapshot);
    }
    json!({ "variables": found, "collections": collections, "missingVariables": missing })
}

/// One line per variable, collection or unresolved id that differs from a plugin `variables.json`.
pub fn diff(ours: &Json, plugin: &Json) -> Vec<String> {
    let mut lines = Vec::new();
    for list in ["variables", "collections"] {
        let by_id = |v: &Json| -> std::collections::BTreeMap<String, Json> {
            v[list]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| Some((x["id"].as_str()?.to_owned(), crate::compare::normalized(x))))
                .collect()
        };
        let (a, b) = (by_id(ours), by_id(plugin));
        for id in a
            .keys()
            .chain(b.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            match (a.get(id), b.get(id)) {
                (Some(_), None) => lines.push(format!("extra {id}")),
                (None, Some(_)) => lines.push(format!("missing {id}")),
                (Some(x), Some(y)) => {
                    for (k, theirs) in y.as_object().into_iter().flatten() {
                        // Local keys are not in the .fig.
                        if x[k] != *theirs && !(k == "key" && x[k].is_null()) {
                            lines.push(format!("{id} {k} ours={} plugin={theirs}", x[k]));
                        }
                    }
                }
                (None, None) => {}
            }
        }
    }
    let set = |v: &Json| -> std::collections::BTreeSet<String> {
        v["missingVariables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect()
    };
    // Older captures leave out missingVariables when it is empty.
    if plugin["missingVariables"].is_array() && set(ours) != set(plugin) {
        lines.push(format!(
            "missingVariables ours={:?} plugin={:?}",
            set(ours),
            set(plugin)
        ));
    }
    lines
}

fn push(ids: &mut Vec<String>, id: Option<String>) {
    if let Some(id) = id.filter(|id| !ids.contains(id)) {
        ids.push(id);
    }
}

/// Every `VARIABLE_ALIAS` id under `v`, once each, appended to `ids` in first-seen order.
pub(crate) fn aliases(v: &Json, ids: &mut Vec<String>) {
    // A set keeps thousands of distinct aliases linear.
    let mut seen: std::collections::HashSet<String> = ids.iter().cloned().collect();
    walk_aliases(v, ids, &mut seen);
}

fn walk_aliases(v: &Json, ids: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
    match v {
        Json::Array(a) => a.iter().for_each(|x| walk_aliases(x, ids, seen)),
        Json::Object(o) => {
            if o.get("type").and_then(Json::as_str) == Some("VARIABLE_ALIAS")
                && let Some(id) = o.get("id").and_then(Json::as_str)
                && seen.insert(id.to_owned())
            {
                ids.push(id.to_owned());
            }
            o.values().for_each(|x| walk_aliases(x, ids, seen));
        }
        _ => {}
    }
}

/// The name of the variable a REST id names, from the file or a library's cached copy.
pub(crate) fn name<'v>(vars: &'v Vars, id: &str) -> Option<&'v str> {
    lookup(vars, id)?["name"].as_str()
}

/// The kiwi variable a REST id names: `VariableID:<guid>` or `VariableID:<key>/<version>`.
fn lookup<'v>(vars: &'v Vars, id: &str) -> Option<&'v Json> {
    let id = id.strip_prefix("VariableID:")?;
    let r = match id.split_once('/') {
        Some((key, version)) => json!({ "assetRef": { "key": key, "version": version } }),
        None => {
            let (s, l) = id.split_once(':')?;
            json!({ "guid": { "sessionID": s.parse::<u64>().ok()?, "localID": l.parse::<u64>().ok()? } })
        }
    };
    copies(vars, &r)
        .into_iter()
        .find(|v| v["type"] == "VARIABLE")
}

fn collection_id(set: &Json) -> Option<String> {
    let lib = &set["assetRef"];
    match (lib["key"].as_str(), lib["version"].as_str()) {
        (Some(key), Some(version)) => Some(format!("VariableCollectionId:{key}/{version}")),
        _ => guid(&set["guid"]).map(|g| format!("VariableCollectionId:{g}")),
    }
}

fn snapshot(id: &str, v: &Json) -> Json {
    let values: serde_json::Map<String, Json> = v["variableDataValues"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((guid(&e["modeID"])?, value(&e["variableData"]))))
        .collect();
    let mut out = json!({
        "id": id,
        "name": v["name"],
        "resolvedType": v["variableResolvedType"],
        "variableCollectionId": collection_id(&v["variableSetID"]),
        "valuesByMode": values,
        "scopes": scopes(v.get("variableScopes")),
        "codeSyntax": code_syntax(&v["codeSyntax"]),
    });
    // A local variable's key is assigned by Figma's servers and is not in the .fig.
    if v["key"].is_string() {
        out["key"] = v["key"].clone();
    }
    out
}

/// Kiwi scope names, renamed where the plugin API differs; kiwi leaves out the default scope.
fn scopes(kiwi: Option<&Json>) -> Json {
    let Some(list) = kiwi.and_then(Json::as_array) else {
        return json!(["ALL_SCOPES"]);
    };
    list.iter()
        .map(|s| match s.as_str() {
            Some("STROKE") => json!("STROKE_COLOR"),
            Some("FONT_STYLE") => json!("FONT_WEIGHT"),
            _ => s.clone(),
        })
        .collect()
}

fn value(d: &Json) -> Json {
    let v = &d["value"];
    match d["dataType"].as_str().unwrap_or("") {
        "COLOR" => v["colorValue"].clone(),
        "FLOAT" => v["floatValue"].clone(),
        "STRING" => v["textValue"].clone(),
        "BOOLEAN" => v["boolValue"].clone(),
        "ALIAS" => alias(d).unwrap_or(Json::Null),
        _ => Json::Null,
    }
}

/// Kiwi `{entries: [{platform, value}]}` → REST `{WEB: ..., ANDROID: ..., iOS: ...}`.
fn code_syntax(c: &Json) -> Json {
    c["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e["platform"].as_str()?.to_owned(), e["value"].clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn collection(id: &str, set: &Json) -> Json {
    let mut modes: Vec<&Json> = set["variableSetModes"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    modes.sort_by_key(|m| m["sortPosition"].as_str().unwrap_or("").to_owned());
    let modes: Vec<Json> = modes
        .iter()
        .map(|m| json!({ "name": m["name"], "modeId": guid(&m["id"]) }))
        .collect();
    let mut out = json!({
        "id": id,
        "name": set["name"],
        "defaultModeId": modes.first().map_or(Json::Null, |m| m["modeId"].clone()),
        "modes": modes,
    });
    if set["key"].is_string() {
        out["key"] = set["key"].clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_each_alias_once() {
        let doc = json!({ "fills": [{ "boundVariables": { "color": { "type": "VARIABLE_ALIAS", "id": "VariableID:6:2" } } }],
                          "children": [{ "boundVariables": { "fills": [
                              { "type": "VARIABLE_ALIAS", "id": "VariableID:k/1:2" },
                              { "type": "VARIABLE_ALIAS", "id": "VariableID:6:2" } ] } }] });
        let mut ids = Vec::new();
        aliases(&doc, &mut ids);
        ids.sort();
        assert_eq!(ids, ["VariableID:6:2", "VariableID:k/1:2"]);
    }

    #[test]
    fn aliases_keep_first_seen_order_after_ids_already_collected() {
        let alias = |id: &str| json!({ "type": "VARIABLE_ALIAS", "id": id });
        let doc = json!([
            alias("VariableID:1:3"),
            alias("VariableID:1:1"),
            alias("VariableID:1:2"),
            alias("VariableID:1:3")
        ]);
        let mut ids = vec!["VariableID:1:2".to_owned()];
        aliases(&doc, &mut ids);
        assert_eq!(ids, ["VariableID:1:2", "VariableID:1:3", "VariableID:1:1"]);
    }

    #[test]
    fn library_variable_snapshot_takes_key_version_ids_and_default_scope() {
        let mode = json!({ "sessionID": 23, "localID": 0 });
        let v = json!({
            "name": "Neutral/1", "key": "18c9", "variableResolvedType": "COLOR",
            "variableSetID": { "assetRef": { "key": "11bb", "version": "348:503" } },
            "variableDataValues": { "entries": [{ "modeID": mode, "variableData": {
                "dataType": "ALIAS", "value": { "alias": { "assetRef": { "key": "aa", "version": "1:2" } } } } }] },
        });
        let s = snapshot("VariableID:18c9/2551:5", &v);
        assert_eq!(
            (
                &s["variableCollectionId"],
                &s["valuesByMode"]["23:0"]["id"],
                &s["scopes"],
                &s["key"]
            ),
            (
                &json!("VariableCollectionId:11bb/348:503"),
                &json!("VariableID:aa/1:2"),
                &json!(["ALL_SCOPES"]),
                &json!("18c9")
            )
        );
    }
}
