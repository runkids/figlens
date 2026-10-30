//! Strict DTCG 2025.10 export of one collection and one exact mode.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::{self, Write};

use serde_json::{Value, json};

const MAX_RECORDS: usize = 100_000;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_PATH_DEPTH: usize = 64;
const MAX_PATH_SEGMENTS: usize = 1_000_000;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// A JSON report, separate from the token document. No partial document is returned.
#[derive(Debug)]
pub struct ConversionError {
    pub report: Value,
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.report)
    }
}

impl std::error::Error for ConversionError {}

struct Diagnostics<'a> {
    collection: &'a str,
    mode: &'a str,
    entries: Vec<Value>,
    bytes: usize,
    omitted: usize,
}

impl Diagnostics<'_> {
    fn push(&mut self, code: &str, variable: Option<&Value>, message: impl Into<String>) {
        if self.omitted != 0 {
            self.omitted += 1;
            return;
        }
        let mut entry = json!({
            "code": code,
            "message": message.into(),
            "collectionId": self.collection,
            "modeId": self.mode,
        });
        if let Some(variable) = variable {
            if let Some(id) = variable["id"].as_str() {
                entry["variableId"] = json!(id);
            }
            if let Some(name) = variable["name"].as_str() {
                entry["name"] = json!(name);
            }
        }
        let mut size = OutputBudget(0);
        if serde_json::to_writer(&mut size, &entry).is_err()
            || self.bytes + size.0 + 1 > MAX_OUTPUT_BYTES - 64 * 1024
        {
            self.omitted = 1;
            return;
        }
        self.bytes += size.0 + 1;
        self.entries.push(entry);
    }

    fn error(mut self) -> ConversionError {
        if self.omitted != 0 {
            self.entries.push(json!({
                "code": "resource_limit",
                "message": "diagnostic report exceeds 64 MiB; additional failures were not retained",
                "collectionId": self.collection,
                "modeId": self.mode,
                "omittedDiagnostics": self.omitted,
            }));
        }
        ConversionError {
            report: json!({ "diagnostics": self.entries }),
        }
    }
}

/// Convert the existing variables snapshot, without resolving aliases to literals.
///
/// Limits: 100,000 records per input array, 4 KiB per identity/name, 16 MiB of
/// relevant metadata, 64 path segments per name, one million total path segments,
/// and 64 MiB each for compact output and diagnostics. Reports exceeding the
/// latter limit include an explicit omitted-failure count. Output accounting
/// conservatively includes one comma per object member.
/// Alias graph validation is iterative and linear in the selected variable count.
pub fn convert(snapshot: &Value, collection: &str, mode: &str) -> Result<Value, ConversionError> {
    if collection.len() > MAX_TEXT_BYTES || mode.len() > MAX_TEXT_BYTES {
        return Err(ConversionError {
            report: json!({ "diagnostics": [{
            "code": "resource_limit",
            "message": "selector exceeds 4096 bytes",
            "collectionId": (collection.len() <= MAX_TEXT_BYTES).then_some(collection),
            "modeId": (mode.len() <= MAX_TEXT_BYTES).then_some(mode),
        }] }),
        });
    }
    let mut diagnostics = Diagnostics {
        collection,
        mode,
        entries: Vec::new(),
        bytes: 0,
        omitted: 0,
    };
    let (Some(collections), Some(variables)) = (
        snapshot["collections"].as_array(),
        snapshot["variables"].as_array(),
    ) else {
        diagnostics.push(
            "invalid_snapshot",
            None,
            "collections and variables must be arrays",
        );
        return Err(diagnostics.error());
    };
    if collections.len() > MAX_RECORDS || variables.len() > MAX_RECORDS {
        diagnostics.push("resource_limit", None, "input array exceeds 100000 records");
        return Err(diagnostics.error());
    }
    let mut metadata_bytes = 0;
    let mut text_fits = |text: &str| {
        metadata_bytes += text.len();
        text.len() <= MAX_TEXT_BYTES && metadata_bytes <= MAX_METADATA_BYTES
    };
    if !text_fits(collection) || !text_fits(mode) {
        diagnostics.push(
            "resource_limit",
            None,
            "selector metadata exceeds its byte limit",
        );
        return Err(diagnostics.error());
    }
    // Check lengths before copying identities into diagnostics or output.
    for record in collections.iter().chain(variables) {
        for field in ["id", "variableCollectionId"] {
            if let Some(text) = record[field].as_str()
                && !text_fits(text)
            {
                diagnostics.push(
                    "resource_limit",
                    None,
                    "identity metadata exceeds its byte limit",
                );
                return Err(diagnostics.error());
            }
        }
    }
    let mut matching = collections.iter().filter(|entry| entry["id"] == collection);
    let Some(selected_collection) = matching.next() else {
        diagnostics.push("unknown_collection", None, "collection ID was not found");
        return Err(diagnostics.error());
    };
    if matching.next().is_some() {
        diagnostics.push(
            "duplicate_collection_id",
            None,
            "collection ID is not unique",
        );
    }
    let collection_name = selected_collection["name"].as_str();
    if collection_name.is_none() {
        diagnostics.push(
            "invalid_collection",
            None,
            "collection name must be a string",
        );
    }
    let Some(modes) = selected_collection["modes"].as_array() else {
        diagnostics.push(
            "invalid_collection",
            None,
            "collection modes must be an array",
        );
        return Err(diagnostics.error());
    };
    if modes.len() > MAX_RECORDS {
        diagnostics.push("resource_limit", None, "modes array exceeds 100000 records");
        return Err(diagnostics.error());
    }
    let mut matching = modes.iter().filter(|entry| entry["modeId"] == mode);
    let selected_mode = matching.next();
    if selected_mode.is_none() {
        diagnostics.push(
            "unknown_mode",
            None,
            "mode ID was not found in the selected collection",
        );
    }
    if matching.next().is_some() {
        diagnostics.push(
            "duplicate_mode_id",
            None,
            "mode ID is not unique in the selected collection",
        );
    }
    let mode_name = selected_mode.and_then(|entry| entry["name"].as_str());
    if selected_mode.is_some() && mode_name.is_none() {
        diagnostics.push("invalid_mode", None, "mode name must be a string");
    }
    for text in [collection_name, mode_name].into_iter().flatten() {
        if !text_fits(text) {
            diagnostics.push(
                "resource_limit",
                None,
                "collection/mode name exceeds its byte limit",
            );
            return Err(diagnostics.error());
        }
    }
    let mut selected: Vec<&Value> = variables
        .iter()
        .filter(|entry| entry["variableCollectionId"] == collection)
        .collect();
    selected.sort_by(|a, b| {
        a["id"]
            .as_str()
            .cmp(&b["id"].as_str())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let mut total_segments = 0;
    for variable in &selected {
        for text in [
            variable["name"].as_str(),
            variable["resolvedType"].as_str(),
            variable["valuesByMode"][mode]["id"].as_str(),
        ]
        .into_iter()
        .flatten()
        {
            if !text_fits(text) {
                diagnostics.push(
                    "resource_limit",
                    None,
                    "variable metadata exceeds its byte limit",
                );
                return Err(diagnostics.error());
            }
        }
        total_segments += variable["name"]
            .as_str()
            .map_or(0, |name| name.split('/').count());
        if total_segments > MAX_PATH_SEGMENTS {
            diagnostics.push(
                "resource_limit",
                None,
                "names exceed 1000000 total path segments",
            );
            return Err(diagnostics.error());
        }
    }

    let mut all_ids = HashMap::with_capacity(variables.len());
    let mut duplicate_ids = HashSet::new();
    for variable in variables {
        if let Some(id) = variable["id"].as_str()
            && all_ids.insert(id, variable).is_some()
        {
            duplicate_ids.insert(id);
        }
    }
    let mut selected_ids = HashMap::with_capacity(selected.len());
    let mut paths = BTreeMap::new();
    for (index, variable) in selected.iter().enumerate() {
        match variable["id"].as_str() {
            Some(id) => {
                selected_ids.insert(id, index);
                if duplicate_ids.contains(id) {
                    diagnostics.push(
                        "duplicate_variable_id",
                        Some(variable),
                        "variable ID is not unique",
                    );
                }
            }
            None => diagnostics.push(
                "invalid_variable",
                Some(variable),
                "variable ID must be a string",
            ),
        }
        let Some(name) = variable["name"].as_str() else {
            diagnostics.push("invalid_name", Some(variable), "name must be a string");
            continue;
        };
        let segments = name.split('/');
        let depth = segments.clone().count();
        if depth > MAX_PATH_DEPTH {
            diagnostics.push(
                "path_depth",
                Some(variable),
                "name exceeds 64 path segments",
            );
        }
        let invalid_segments: Vec<usize> = segments
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.is_empty()
                    || segment.starts_with('$')
                    || segment.contains(['.', '{', '}']))
                .then_some(index + 1)
            })
            .collect();
        let valid = depth <= MAX_PATH_DEPTH && invalid_segments.is_empty();
        if !invalid_segments.is_empty() {
            diagnostics.push("invalid_name", Some(variable), format!(
                "segments {invalid_segments:?} are empty, begin with '$', or contain '.', '{{' or '}}'"
            ));
        }
        if valid && let Some(previous) = paths.insert(name, index) {
            diagnostics.push(
                "duplicate_path",
                Some(variable),
                format!("token path is also used by {}", selected[previous]["id"]),
            );
        }
    }
    for (name, index) in &paths {
        for (offset, _) in name.match_indices('/') {
            if let Some(parent) = paths.get(&name[..offset]) {
                diagnostics.push(
                    "path_collision",
                    Some(selected[*index]),
                    format!(
                        "group prefix '{}' is the token {}",
                        &name[..offset],
                        selected[*parent]["id"]
                    ),
                );
            }
        }
    }

    let mut types = Vec::with_capacity(selected.len());
    for variable in &selected {
        let token_type = match variable["resolvedType"].as_str() {
            Some("COLOR") => Some("color"),
            Some("FLOAT") => Some("number"),
            _ => {
                diagnostics.push(
                    "unsupported_type",
                    Some(variable),
                    "only COLOR and FLOAT resolvedType are supported",
                );
                None
            }
        };
        types.push(token_type);
    }
    let mut values = vec![None; selected.len()];
    let mut edges = vec![None; selected.len()];
    if selected_mode.is_some() {
        for (index, variable) in selected.iter().enumerate() {
            let Some(value) = variable["valuesByMode"]
                .as_object()
                .and_then(|values| values.get(mode))
            else {
                diagnostics.push(
                    "missing_mode_value",
                    Some(variable),
                    "variable has no value for the selected mode",
                );
                continue;
            };
            if value.is_null() {
                diagnostics.push("null_value", Some(variable), "selected-mode value is null");
                continue;
            }
            if value["type"] == "VARIABLE_ALIAS" {
                let Some(target_id) = value["id"].as_str() else {
                    diagnostics.push("invalid_alias", Some(variable), "alias ID must be a string");
                    continue;
                };
                let Some(target) = all_ids.get(target_id) else {
                    diagnostics.push(
                        "missing_alias",
                        Some(variable),
                        format!("alias target '{target_id}' was not found"),
                    );
                    continue;
                };
                if duplicate_ids.contains(target_id) {
                    diagnostics.push(
                        "ambiguous_alias",
                        Some(variable),
                        format!("alias target '{target_id}' is not unique"),
                    );
                    continue;
                }
                if target["variableCollectionId"] != collection {
                    diagnostics.push("cross_collection_alias", Some(variable), format!(
                        "alias target '{target_id}' belongs to a different collection; no mode mapping is inferred"
                    ));
                    continue;
                }
                let target_index = selected_ids[target_id];
                if !target["valuesByMode"]
                    .as_object()
                    .is_some_and(|values| values.contains_key(mode))
                {
                    diagnostics.push(
                        "alias_target_missing_mode",
                        Some(variable),
                        format!(
                            "alias target '{target_id}' has no value for selected mode '{mode}'"
                        ),
                    );
                }
                if types[index].is_none() || types[index] != types[target_index] {
                    diagnostics.push(
                        "alias_type_mismatch",
                        Some(variable),
                        format!(
                            "alias target '{target_id}' must have the same supported resolvedType"
                        ),
                    );
                }
                edges[index] = Some(target_index);
                continue;
            }
            match types[index] {
                Some("number") => {
                    if value.as_f64().is_some_and(f64::is_finite) {
                        values[index] = Some(value.clone());
                    } else {
                        diagnostics.push(
                            "invalid_number",
                            Some(variable),
                            "FLOAT value must be a finite number",
                        );
                    }
                }
                Some("color") => {
                    let invalid: Vec<&str> = ["r", "g", "b", "a"]
                        .into_iter()
                        .filter(|channel| {
                            !value[*channel].as_f64().is_some_and(|number| {
                                number.is_finite() && (0.0..=1.0).contains(&number)
                            })
                        })
                        .collect();
                    if invalid.is_empty() {
                        values[index] = Some(json!({
                            "colorSpace": "srgb",
                            "components": [value["r"], value["g"], value["b"]],
                            "alpha": value["a"],
                        }));
                    } else {
                        diagnostics.push(
                            "invalid_color",
                            Some(variable),
                            format!(
                                "COLOR channels {} must each be finite numbers in 0..1",
                                invalid.join(", ")
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
    }
    // Each node enters the stack once; long alias chains never consume call stack.
    let mut states = vec![0_u8; selected.len()];
    let mut positions = vec![0; selected.len()];
    let mut stack = Vec::new();
    for start in 0..selected.len() {
        if states[start] != 0 {
            continue;
        }
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            if states[index] != 0 {
                if states[index] == 1 {
                    for member in &stack[positions[index]..] {
                        diagnostics.push(
                            "alias_cycle",
                            Some(selected[*member]),
                            "selected-mode alias participates in a cycle",
                        );
                    }
                }
                break;
            }
            states[index] = 1;
            positions[index] = stack.len();
            stack.push(index);
            cursor = edges[index];
        }
        for index in stack.drain(..) {
            states[index] = 2;
        }
    }
    if !diagnostics.entries.is_empty() || diagnostics.omitted != 0 {
        return Err(diagnostics.error());
    }

    let mut output = json!({ "$extensions": { "org.figlens": {
        "converterVersion": "1",
        "collectionId": collection,
        "collectionName": collection_name.unwrap(),
        "modeId": mode,
        "modeName": mode_name.unwrap(),
    } } });
    let mut budget = OutputBudget(0);
    if serde_json::to_writer(&mut budget, &output).is_err() {
        diagnostics.push("resource_limit", None, "output exceeds 64 MiB");
        return Err(diagnostics.error());
    }
    for (index, variable) in selected.iter().enumerate() {
        let value = match edges[index] {
            Some(target) => json!(format!(
                "{{{}}}",
                selected[target]["name"].as_str().unwrap().replace('/', ".")
            )),
            None => values[index].take().unwrap(),
        };
        let token = json!({
            "$type": types[index].unwrap(),
            "$value": value,
            "$extensions": { "org.figlens": {
                "variableId": variable["id"],
                "collectionId": collection,
                "modeId": mode,
                "sourceName": variable["name"],
            } },
        });
        let name = variable["name"].as_str().unwrap();
        let mut segments = name.split('/').peekable();
        let mut group = output.as_object_mut().unwrap();
        while let Some(segment) = segments.next() {
            let leaf = segments.peek().is_none();
            if !group.contains_key(segment) {
                // Charge keys, colon/comma and the new value before retaining them.
                let charged = serde_json::to_writer(&mut budget, segment).is_ok()
                    && budget.write_all(b":,").is_ok()
                    && if leaf {
                        serde_json::to_writer(&mut budget, &token).is_ok()
                    } else {
                        budget.write_all(b"{}").is_ok()
                    };
                if !charged {
                    diagnostics.push("resource_limit", None, "output exceeds 64 MiB");
                    return Err(diagnostics.error());
                }
                if !leaf {
                    group.insert(segment.to_owned(), json!({}));
                }
            }
            if leaf {
                group.insert(segment.to_owned(), token);
                break;
            }
            group = group.get_mut(segment).unwrap().as_object_mut().unwrap();
        }
    }
    Ok(output)
}

struct OutputBudget(usize);

impl Write for OutputBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES - self.0 {
            return Err(io::Error::other("token output budget exceeded"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
