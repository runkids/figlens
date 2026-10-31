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

/// A diagnostic code and message.
type Failure = (&'static str, String);

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

/// Convert the existing variables snapshot with no alias mode mappings; see [`convert_mapped`].
pub fn convert(snapshot: &Value, collection: &str, mode: &str) -> Result<Value, ConversionError> {
    convert_mapped(snapshot, collection, mode, &BTreeMap::new())
}

/// The DTCG value of a literal of `kind`, or the diagnostic code and message.
fn literal(kind: Option<&str>, value: &Value) -> Result<Value, (&'static str, String)> {
    match kind {
        Some("number") if value.as_f64().is_some_and(f64::is_finite) => Ok(value.clone()),
        Some("number") => Err((
            "invalid_number",
            "FLOAT value must be a finite number".into(),
        )),
        Some("color") => {
            let invalid: Vec<&str> = ["r", "g", "b", "a"]
                .into_iter()
                .filter(|channel| {
                    !value[*channel]
                        .as_f64()
                        .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number))
                })
                .collect();
            if invalid.is_empty() {
                Ok(json!({
                    "colorSpace": "srgb",
                    "components": [value["r"], value["g"], value["b"]],
                    "alpha": value["a"],
                }))
            } else {
                Err((
                    "invalid_color",
                    format!(
                        "COLOR channels {} must each be finite numbers in 0..1",
                        invalid.join(", ")
                    ),
                ))
            }
        }
        _ => Err((
            "unsupported_type",
            "only COLOR and FLOAT resolvedType are supported".into(),
        )),
    }
}

/// Convert the existing variables snapshot. Aliases within the collection stay DTCG
/// references. An alias into another collection is resolved to its value in that
/// collection's mode, which is its only mode or the one `alias_modes` names for it, and
/// the token records the first target in `aliasOf`; several modes and no mapping fail.
///
/// Limits: 100,000 records per input array, 4 KiB per identity/name, 16 MiB of
/// relevant metadata, 64 path segments per name, one million total path segments,
/// and 64 MiB each for compact output and diagnostics. Reports exceeding the
/// latter limit include an explicit omitted-failure count. Output accounting
/// conservatively includes one comma per object member.
/// Alias graph validation is iterative and linear in the selected variable count.
pub fn convert_mapped(
    snapshot: &Value,
    collection: &str,
    mode: &str,
    alias_modes: &BTreeMap<String, String>,
) -> Result<Value, ConversionError> {
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
    // Each mapping names a collection of this snapshot and one of its modes.
    // Indexed once: every cross-collection alias looks its collection up. The first wins, as
    // `find` would.
    let mut modes_by_id: HashMap<&str, &Vec<Value>> = HashMap::new();
    for c in collections {
        if let (Some(id), Some(modes)) = (c["id"].as_str(), c["modes"].as_array()) {
            modes_by_id.entry(id).or_insert(modes);
        }
    }
    let collection_modes = |id: &str| modes_by_id.get(id).copied();
    for (target, target_mode) in alias_modes {
        if !text_fits(target) || !text_fits(target_mode) {
            diagnostics.push(
                "resource_limit",
                None,
                "alias mode metadata exceeds its byte limit",
            );
            return Err(diagnostics.error());
        }
        let known = target != collection
            && collection_modes(target)
                .is_some_and(|modes| modes.iter().any(|m| m["modeId"] == target_mode.as_str()));
        if !known {
            diagnostics.push("unknown_alias_mode", None, format!(
                "alias mode '{target}={target_mode}' does not name another collection of this snapshot and one of its modes"
            ));
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

    let kind_of = |v: &Value| match v["resolvedType"].as_str() {
        Some("COLOR") => Some("color"),
        Some("FLOAT") => Some("number"),
        _ => None,
    };
    let mut types = Vec::with_capacity(selected.len());
    for variable in &selected {
        let token_type = kind_of(variable);
        if token_type.is_none() {
            diagnostics.push(
                "unsupported_type",
                Some(variable),
                "only COLOR and FLOAT resolvedType are supported",
            );
        }
        types.push(token_type);
    }
    let mut values = vec![None; selected.len()];
    let mut sources = vec![None; selected.len()];
    let mut edges = vec![None; selected.len()];
    // The mode a target collection is read in: the selected one for this collection, else
    // the mapped one, else its only mode.
    let mode_of = |owner: &str, id: &str| -> Result<String, (&'static str, String)> {
        if owner == collection {
            return Ok(mode.to_owned());
        }
        if let Some(m) = alias_modes.get(owner) {
            return Ok(m.clone());
        }
        match collection_modes(owner).map(Vec::as_slice) {
            Some([only]) => match only["modeId"].as_str().unwrap_or("") {
                m if m.len() > MAX_TEXT_BYTES => Err((
                    "resource_limit",
                    format!("collection '{owner}' mode id exceeds its byte limit"),
                )),
                m => Ok(m.to_owned()),
            },
            Some(modes) => Err((
                "cross_collection_alias",
                format!(
                    "alias target '{id}' belongs to collection '{owner}' with {} modes; choose one with --alias-mode {owner}=<mode id> (MCP aliasModes)",
                    modes.len()
                ),
            )),
            None => Err((
                "cross_collection_alias",
                format!(
                    "alias target '{id}' belongs to collection '{owner}', which this snapshot does not include"
                ),
            )),
        }
    };
    // Each variable's resolved value is computed once, so all chains together stay linear. The
    // flag says whether a library collection, `<key>/<version>`, was read on the way.
    // A failure is stored once and memoized by index, so a long chain does not copy it per member.
    let mut memo: HashMap<String, Result<(Value, bool), usize>> = HashMap::new();
    let mut failures: Vec<Failure> = Vec::new();
    let mut resolve_foreign = |first: &Value, kind: Option<&str>| {
        let first_id = first["id"].as_str().unwrap_or("");
        let owner = first["variableCollectionId"].as_str().unwrap_or("");
        if kind.is_none() || kind_of(first) != kind {
            return Err((
                "alias_type_mismatch",
                format!("alias target '{first_id}' must have the same supported resolvedType"),
            ));
        }
        mode_of(owner, first_id)?;
        let name = first["name"].as_str().unwrap_or("");
        // The exported collection's names and the ids are checked up front; these are not.
        if name.len() > MAX_TEXT_BYTES {
            return Err((
                "resource_limit",
                format!("alias target '{first_id}' name exceeds its byte limit"),
            ));
        }
        let (mut chain, mut seen) = (Vec::new(), HashSet::new());
        // A memoized failure this walk joined, kept by index rather than stored again.
        let mut joined = None;
        let mut current = first;
        let result = loop {
            let id = current["id"].as_str().unwrap_or("");
            if let Some(known) = memo.get(id) {
                break known.clone().map_err(|i| {
                    joined = Some(i);
                    failures[i].clone()
                });
            }
            if !seen.insert(id) {
                break Err((
                    "alias_cycle",
                    "cross-collection alias participates in a cycle".into(),
                ));
            }
            let owner = current["variableCollectionId"].as_str().unwrap_or("");
            chain.push((id, owner.contains('/')));
            let owner_mode = match mode_of(owner, id) {
                Ok(m) => m,
                Err(e) => break Err(e),
            };
            let Some(value) = current["valuesByMode"]
                .get(&owner_mode)
                .filter(|v| !v.is_null())
            else {
                break Err((
                    "alias_target_missing_mode",
                    format!("alias target '{id}' has no value for mode '{owner_mode}'"),
                ));
            };
            if value["type"] != "VARIABLE_ALIAS" {
                break literal(kind_of(current), value).map(|v| (v, false));
            }
            let next = value["id"].as_str().unwrap_or("");
            // Only the snapshot's own ids were checked up front; an unresolved one was not.
            if next.len() > MAX_TEXT_BYTES {
                break Err((
                    "resource_limit",
                    format!("an alias of '{id}' names a target id over the byte limit"),
                ));
            }
            if duplicate_ids.contains(next) {
                break Err((
                    "ambiguous_alias",
                    format!("alias target '{next}' is not unique"),
                ));
            }
            let Some(next) = all_ids.get(next) else {
                break Err((
                    "missing_alias",
                    format!("alias target '{next}' was not found"),
                ));
            };
            if kind_of(next) != kind_of(current) {
                break Err((
                    "alias_type_mismatch",
                    format!(
                        "alias target '{}' must have the same supported resolvedType",
                        next["id"].as_str().unwrap_or("")
                    ),
                ));
            }
            current = next;
        };
        let mut result = result;
        let failure = match (&result, joined) {
            (Err(_), Some(i)) => i,
            (Err(failure), None) if !chain.is_empty() => {
                failures.push(failure.clone());
                failures.len() - 1
            }
            _ => 0,
        };
        for (id, library) in chain.into_iter().rev() {
            if let Ok((_, from_library)) = &mut result {
                *from_library |= library;
            }
            memo.insert(id.to_owned(), result.as_ref().cloned().map_err(|_| failure));
        }
        result
    };
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
                    match resolve_foreign(target, types[index]) {
                        Ok((value, library)) => {
                            values[index] = Some(value);
                            // Only the target; its `aliasOf` copy is built under the output budget.
                            sources[index] = Some((target, library));
                        }
                        Err((code, message)) => diagnostics.push(code, Some(variable), message),
                    }
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
            if types[index].is_some() {
                match literal(types[index], value) {
                    Ok(value) => values[index] = Some(value),
                    Err((code, message)) => diagnostics.push(code, Some(variable), message),
                }
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
        let mut token = json!({
            "$type": types[index].unwrap(),
            "$value": value,
            "$extensions": { "org.figlens": {
                "variableId": variable["id"],
                "collectionId": collection,
                "modeId": mode,
                "sourceName": variable["name"],
            } },
        });
        if let Some((target, library)) = sources[index].take() {
            let (id, owner) = (
                target["id"].as_str().unwrap_or(""),
                target["variableCollectionId"].as_str().unwrap_or(""),
            );
            let source = &mut token["$extensions"]["org.figlens"]["aliasOf"];
            *source = json!({
                "variableId": id,
                // Checked as a string in resolve_foreign; anything else is not copied.
                "name": target["name"].as_str().unwrap_or(""),
                "collectionId": owner,
                "modeId": mode_of(owner, id).unwrap_or_default(),
            });
            if library {
                source["libraryCopy"] = json!(true);
            }
        }
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
