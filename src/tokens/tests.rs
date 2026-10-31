use super::*;

fn variable(id: &str, name: &str, resolved_type: &str, light: Value, dark: Value) -> Value {
    json!({
        "id": id,
        "name": name,
        "variableCollectionId": "collection",
        "resolvedType": resolved_type,
        "valuesByMode": { "light": light, "dark": dark },
    })
}

fn snapshot(variables: Vec<Value>) -> Value {
    json!({
        "collections": [{
            "id": "collection", "name": "Theme",
            "modes": [
                { "modeId": "light", "name": "Light" },
                { "modeId": "dark", "name": "Dark" },
            ],
        }],
        "variables": variables,
        "missingVariables": [],
    })
}

fn alias(id: &str) -> Value {
    json!({ "type": "VARIABLE_ALIAS", "id": id })
}

fn codes(error: &ConversionError) -> Vec<&str> {
    error.report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["code"].as_str().unwrap())
        .collect()
}

fn fails(snapshot: &Value, code: &str) -> ConversionError {
    let error = convert(snapshot, "collection", "light").unwrap_err();
    assert!(codes(&error).contains(&code), "{error}");
    for diagnostic in error.report["diagnostics"].as_array().unwrap() {
        assert_eq!(diagnostic["collectionId"], "collection");
        assert_eq!(diagnostic["modeId"], "light");
        assert!(diagnostic["message"].is_string());
    }
    error
}

#[test]
fn exports_exact_modes_srgb_alpha_unitless_numbers_and_source_identities() {
    let input = snapshot(vec![
        variable(
            "color",
            "palette/brand",
            "COLOR",
            json!({ "r": 0.1, "g": 0.2, "b": 0.3, "a": 0.4 }),
            json!({ "r": 0.8, "g": 0.7, "b": 0.6, "a": 1 }),
        ),
        variable("number", "spacing/base", "FLOAT", json!(8), json!(12.5)),
    ]);
    let light = convert(&input, "collection", "light").unwrap();
    let dark = convert(&input, "collection", "dark").unwrap();
    assert_eq!(
        light["palette"]["brand"]["$value"],
        json!({
            "colorSpace": "srgb", "components": [0.1, 0.2, 0.3], "alpha": 0.4,
        })
    );
    assert_eq!(
        dark["palette"]["brand"]["$value"],
        json!({
            "colorSpace": "srgb", "components": [0.8, 0.7, 0.6], "alpha": 1,
        })
    );
    assert_eq!(light["spacing"]["base"]["$type"], "number");
    assert_eq!(light["spacing"]["base"]["$value"], 8);
    assert_eq!(dark["spacing"]["base"]["$value"], 12.5);
    assert_eq!(
        light["$extensions"]["org.figlens"],
        json!({
            "converterVersion": "1", "collectionId": "collection", "collectionName": "Theme",
            "modeId": "light", "modeName": "Light",
        })
    );
    assert_eq!(
        light["palette"]["brand"]["$extensions"]["org.figlens"],
        json!({
            "variableId": "color", "collectionId": "collection", "modeId": "light", "sourceName": "palette/brand",
        })
    );
    assert!(light.get("diagnostics").is_none());
    assert_eq!(light["palette"]["brand"]["$type"], "color");
    assert_eq!(dark["$extensions"]["org.figlens"]["modeName"], "Dark");
}

#[test]
fn preserves_multihop_alias_references_instead_of_inlining_values() {
    let input = snapshot(vec![
        variable("a", "semantic/size", "FLOAT", alias("b"), alias("b")),
        variable("b", "base/size", "FLOAT", alias("c"), alias("c")),
        variable("c", "raw/size", "FLOAT", json!(4), json!(6)),
    ]);
    for mode in ["light", "dark"] {
        let output = convert(&input, "collection", mode).unwrap();
        assert_eq!(output["semantic"]["size"]["$value"], "{base.size}");
        assert_eq!(output["base"]["size"]["$value"], "{raw.size}");
        assert_eq!(
            output["raw"]["size"]["$value"],
            if mode == "light" { 4 } else { 6 }
        );
    }
}

#[test]
fn selectors_use_ids_only_and_modes_must_belong_to_selected_collection() {
    let mut input = snapshot(vec![]);
    assert_eq!(
        codes(&convert(&input, "Theme", "light").unwrap_err()),
        ["unknown_collection"]
    );
    assert_eq!(
        codes(&convert(&input, "collection", "Light").unwrap_err()),
        ["unknown_mode"]
    );
    input["collections"].as_array_mut().unwrap().push(json!({
        "id": "other", "name": "Other", "modes": [{ "modeId": "foreign", "name": "Foreign" }],
    }));
    assert_eq!(
        codes(&convert(&input, "collection", "foreign").unwrap_err()),
        ["unknown_mode"]
    );
}

#[test]
fn rejects_every_invalid_segment_without_renaming() {
    for name in ["", "/a", "a/", "a//b", "$a", "a/$b", "a.b", "a/{b", "a/b}"] {
        let input = snapshot(vec![variable("v", name, "FLOAT", json!(1), json!(1))]);
        let error = fails(&input, "invalid_name");
        assert_eq!(error.report["diagnostics"][0]["variableId"], "v");
        assert_eq!(error.report["diagnostics"][0]["name"], name);
    }
}

#[test]
fn rejects_duplicate_paths_and_token_group_collisions_in_either_input_order() {
    for names in [
        ["a", "a"],
        ["a", "a/b"],
        ["a/b", "a"],
        ["a", "a/b/c"],
        ["a/b", "a/b/c"],
    ] {
        let input = snapshot(vec![
            variable("1", names[0], "FLOAT", json!(1), json!(1)),
            variable("2", names[1], "FLOAT", json!(1), json!(1)),
        ]);
        fails(
            &input,
            if names[0] == names[1] {
                "duplicate_path"
            } else {
                "path_collision"
            },
        );
    }
}

#[test]
fn reports_independent_failures_deterministically_with_no_token_document() {
    let mut input = snapshot(vec![
        variable("z", "bad.name", "STRING", Value::Null, Value::Null),
        variable("a", "number", "FLOAT", json!("no"), json!(1)),
    ]);
    let first = fails(&input, "invalid_name");
    assert_eq!(
        codes(&first),
        [
            "invalid_name",
            "unsupported_type",
            "invalid_number",
            "null_value"
        ]
    );
    input["variables"].as_array_mut().unwrap().reverse();
    let second = convert(&input, "collection", "light").unwrap_err();
    assert_eq!(first.report, second.report);
    assert_eq!(
        serde_json::from_str::<Value>(&first.to_string()).unwrap(),
        first.report
    );
    assert!(first.report.get("$extensions").is_none());
}

#[test]
fn rejects_missing_mode_values_without_using_defaults_or_another_mode() {
    let mut input = snapshot(vec![variable("v", "number", "FLOAT", json!(1), json!(2))]);
    input["variables"][0]["valuesByMode"]
        .as_object_mut()
        .unwrap()
        .remove("light");
    fails(&input, "missing_mode_value");
    assert!(convert(&input, "collection", "dark").is_ok());
}

#[test]
fn rejects_missing_cross_collection_cross_mode_and_malformed_aliases() {
    let mut input = snapshot(vec![variable("a", "a", "FLOAT", alias("b"), json!(2))]);
    fails(&input, "missing_alias");
    input["variables"][0]["valuesByMode"]["light"] = json!({ "type": "VARIABLE_ALIAS" });
    fails(&input, "invalid_alias");
    input["variables"][0]["valuesByMode"]["light"] = alias("b");
    let mut target = variable("b", "b", "FLOAT", json!(1), json!(2));
    target["variableCollectionId"] = json!("other");
    input["variables"].as_array_mut().unwrap().push(target);
    fails(&input, "cross_collection_alias");
    input["variables"][1]["variableCollectionId"] = json!("collection");
    input["variables"][1]["valuesByMode"]
        .as_object_mut()
        .unwrap()
        .remove("light");
    let error = fails(&input, "alias_target_missing_mode");
    assert!(codes(&error).contains(&"missing_mode_value"));
}

#[test]
fn rejects_self_cycles_and_all_members_of_longer_cycles() {
    let input = snapshot(vec![variable("a", "a", "FLOAT", alias("a"), json!(1))]);
    assert_eq!(codes(&fails(&input, "alias_cycle")), ["alias_cycle"]);
    let input = snapshot(vec![
        variable("a", "a", "FLOAT", alias("b"), json!(1)),
        variable("b", "b", "FLOAT", alias("c"), json!(1)),
        variable("c", "c", "FLOAT", alias("b"), json!(1)),
    ]);
    let error = fails(&input, "alias_cycle");
    let ids: Vec<&str> = error.report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["variableId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["b", "c"]);
    assert!(convert(&input, "collection", "dark").is_ok());
}

#[test]
fn rejects_alias_type_mismatch_and_invalid_terminal_values() {
    let mut input = snapshot(vec![
        variable("a", "a", "FLOAT", alias("b"), json!(1)),
        variable(
            "b",
            "b",
            "COLOR",
            json!({ "r": 0, "g": 0, "b": 0, "a": 1 }),
            json!(1),
        ),
    ]);
    fails(&input, "alias_type_mismatch");
    input["variables"][1]["resolvedType"] = json!("FLOAT");
    input["variables"][1]["valuesByMode"]["light"] = Value::Null;
    fails(&input, "null_value");
    input["variables"][1]["resolvedType"] = json!("BOOLEAN");
    fails(&input, "unsupported_type");
    fails(&input, "alias_type_mismatch");
}

#[test]
fn rejects_non_numeric_numbers_and_each_invalid_srgb_channel() {
    for value in [json!(true), json!("1"), json!({}), json!([])] {
        fails(
            &snapshot(vec![variable("n", "n", "FLOAT", value, json!(1))]),
            "invalid_number",
        );
    }
    for channel in ["r", "g", "b", "a"] {
        for value in [json!(-0.1), json!(1.1), json!("0.5"), Value::Null] {
            let mut color = json!({ "r": 0, "g": 0.5, "b": 1, "a": 1 });
            color[channel] = value;
            fails(
                &snapshot(vec![variable("c", "c", "COLOR", color, Value::Null)]),
                "invalid_color",
            );
        }
    }
}

#[test]
fn does_not_validate_unselected_modes_or_unrelated_collection_types() {
    let mut foreign = variable("foreign", "bad.name", "STRING", Value::Null, Value::Null);
    foreign["variableCollectionId"] = json!("other");
    let input = snapshot(vec![
        variable("v", "v", "FLOAT", json!(1), Value::Null),
        foreign,
    ]);
    assert!(convert(&input, "collection", "light").is_ok());
    assert_eq!(
        codes(&convert(&input, "collection", "dark").unwrap_err()),
        ["null_value"]
    );
}

#[test]
fn bounds_names_records_output_and_diagnostic_memory() {
    let name = vec!["x"; MAX_PATH_DEPTH].join("/");
    let input = snapshot(vec![variable("v", &name, "FLOAT", json!(1), json!(1))]);
    assert!(convert(&input, "collection", "light").is_ok());
    fails(
        &snapshot(vec![variable(
            "v",
            &(name + "/x"),
            "FLOAT",
            json!(1),
            json!(1),
        )]),
        "path_depth",
    );
    fails(
        &snapshot(vec![variable(
            "v",
            &"x".repeat(MAX_TEXT_BYTES + 1),
            "FLOAT",
            json!(1),
            json!(1),
        )]),
        "resource_limit",
    );
    fails(
        &snapshot(vec![Value::Null; MAX_RECORDS + 1]),
        "resource_limit",
    );
    let mut budget = OutputBudget(MAX_OUTPUT_BYTES - 1);
    assert!(budget.write_all(b"xx").is_err());
    assert_eq!(budget.0, MAX_OUTPUT_BYTES - 1);
    let mut diagnostics = Diagnostics {
        collection: "collection",
        mode: "light",
        entries: Vec::new(),
        bytes: MAX_OUTPUT_BYTES,
        omitted: 0,
    };
    diagnostics.push("invalid_name", None, "a");
    diagnostics.push("invalid_number", None, "b");
    let error = diagnostics.error();
    assert_eq!(codes(&error), ["resource_limit"]);
    assert_eq!(error.report["diagnostics"][0]["omittedDiagnostics"], 2);
}

#[test]
fn validates_long_alias_chains_without_recursive_stack_or_repeated_walks() {
    let variables = (0..10_000)
        .map(|index| {
            let id = format!("v{index:05}");
            let value = if index == 9999 {
                json!(1)
            } else {
                alias(&format!("v{:05}", index + 1))
            };
            variable(&id, &id, "FLOAT", value, json!(1))
        })
        .collect();
    let output = convert(&snapshot(variables), "collection", "light").unwrap();
    assert_eq!(output["v00000"]["$value"], "{v00001}");
    assert_eq!(output["v09999"]["$value"], 1);
}

#[test]
fn rejects_ambiguous_ids_and_malformed_snapshot_metadata() {
    fails(&json!({}), "invalid_snapshot");
    let mut input = snapshot(vec![variable("v", "v", "FLOAT", json!(1), json!(1))]);
    input["variables"][0]["id"] = Value::Null;
    fails(&input, "invalid_variable");
    input["variables"][0]["id"] = json!("v");
    input["variables"].as_array_mut().unwrap().push(variable(
        "v",
        "other",
        "FLOAT",
        json!(1),
        json!(1),
    ));
    input["variables"].as_array_mut().unwrap().push(variable(
        "alias",
        "alias",
        "FLOAT",
        alias("v"),
        json!(1),
    ));
    fails(&input, "duplicate_variable_id");
    fails(&input, "ambiguous_alias");
    let collection = input["collections"][0].clone();
    input["collections"]
        .as_array_mut()
        .unwrap()
        .push(collection);
    fails(&input, "duplicate_collection_id");
    input["collections"][0]["modes"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "modeId": "light", "name": "Again" }));
    fails(&input, "duplicate_mode_id");
}

#[test]
fn rejects_malformed_collection_mode_and_variable_metadata() {
    let mut input = snapshot(vec![variable("v", "v", "FLOAT", json!(1), json!(1))]);
    input["collections"][0]["name"] = Value::Null;
    fails(&input, "invalid_collection");
    input["collections"][0]["name"] = json!("Theme");
    input["collections"][0]["modes"][0]["name"] = Value::Null;
    fails(&input, "invalid_mode");
    input["collections"][0]["modes"] = Value::Null;
    fails(&input, "invalid_collection");
    let mut input = snapshot(vec![variable("v", "v", "FLOAT", json!(1), json!(1))]);
    input["variables"][0]["name"] = Value::Null;
    fails(&input, "invalid_name");
    input["variables"][0]["name"] = json!("v");
    input["variables"][0]["valuesByMode"] = json!([]);
    fails(&input, "missing_mode_value");
}

#[test]
fn bounds_aggregate_path_work_metadata_and_selectors() {
    let name = vec!["x"; MAX_PATH_DEPTH].join("/");
    let entry = variable("v", &name, "FLOAT", json!(1), json!(1));
    fails(
        &snapshot(vec![entry; MAX_PATH_SEGMENTS / MAX_PATH_DEPTH + 1]),
        "resource_limit",
    );
    let entry = variable(
        "v",
        &"x".repeat(MAX_TEXT_BYTES),
        "FLOAT",
        json!(1),
        json!(1),
    );
    fails(
        &snapshot(vec![entry; MAX_METADATA_BYTES / MAX_TEXT_BYTES + 1]),
        "resource_limit",
    );
    let error = convert(&snapshot(vec![]), &"x".repeat(MAX_TEXT_BYTES + 1), "light").unwrap_err();
    assert_eq!(codes(&error), ["resource_limit"]);
    assert!(error.report["diagnostics"][0]["collectionId"].is_null());
}

/// A snapshot whose Theme aliases into `primitives`, a collection with the given modes:
/// accent -> blue -> base, base's blue channel 1 and red 0.25 per later mode.
fn two_collections(primitives: &str, modes: &[&str]) -> Value {
    let mut input = snapshot(vec![
        variable("accent", "accent", "COLOR", alias("blue"), alias("blue")),
        variable("link", "link", "COLOR", alias("accent"), alias("accent")),
    ]);
    let per_mode = |value: &dyn Fn(usize) -> Value| -> Value {
        modes
            .iter()
            .enumerate()
            .map(|(i, m)| ((*m).to_owned(), value(i)))
            .collect()
    };
    input["collections"].as_array_mut().unwrap().push(json!({
        "id": primitives, "name": "Primitives",
        "modes": modes.iter().map(|m| json!({ "modeId": m, "name": m })).collect::<Vec<_>>(),
    }));
    let blue = per_mode(&|_| alias("base"));
    let base = per_mode(&|i| json!({ "r": i as f64 / 4.0, "g": 0, "b": 1, "a": 1 }));
    input["variables"].as_array_mut().unwrap().extend([
        json!({ "id": "blue", "name": "blue", "variableCollectionId": primitives,
            "resolvedType": "COLOR", "valuesByMode": blue }),
        json!({ "id": "base", "name": "base", "variableCollectionId": primitives,
            "resolvedType": "COLOR", "valuesByMode": base }),
    ]);
    input
}

#[test]
fn aliases_into_a_single_mode_collection_export_resolved_values_with_their_source() {
    let input = two_collections("VariableCollectionId:pk/1:2", &["base"]);
    for mode in ["light", "dark"] {
        let output = convert(&input, "collection", mode).unwrap();
        let accent = &output["accent"];
        assert_eq!(accent["$value"]["components"], json!([0.0, 0, 1]));
        assert_eq!(
            accent["$extensions"]["org.figlens"]["aliasOf"],
            json!({ "variableId": "blue", "name": "blue", "modeId": "base",
                "collectionId": "VariableCollectionId:pk/1:2", "libraryCopy": true })
        );
        // An alias within the collection stays a reference, even to a resolved token.
        assert_eq!(output["link"]["$value"], "{accent}");
        assert!(output.get("blue").is_none());
    }
}

#[test]
fn several_target_modes_need_an_explicit_alias_mode() {
    let input = two_collections("prims", &["one", "two"]);
    let error = fails(&input, "cross_collection_alias");
    let message = error.report["diagnostics"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("--alias-mode prims=<mode id>"),
        "{message}"
    );
    let mapping = BTreeMap::from([("prims".to_owned(), "two".to_owned())]);
    let output = convert_mapped(&input, "collection", "light", &mapping).unwrap();
    assert_eq!(
        output["accent"]["$value"]["components"],
        json!([0.25, 0, 1])
    );
    assert_eq!(
        output["accent"]["$extensions"]["org.figlens"]["aliasOf"]["modeId"],
        "two"
    );
    assert!(
        output["accent"]["$extensions"]["org.figlens"]["aliasOf"]
            .get("libraryCopy")
            .is_none()
    );
    for (target, mode) in [
        ("prims", "three"),
        ("missing", "one"),
        ("collection", "light"),
    ] {
        let mapping = BTreeMap::from([(target.to_owned(), mode.to_owned())]);
        let error = convert_mapped(&input, "collection", "light", &mapping).unwrap_err();
        assert!(
            codes(&error).contains(&"unknown_alias_mode"),
            "{target}={mode}"
        );
    }
}

#[test]
fn a_library_collection_anywhere_on_the_chain_marks_the_source_and_names_are_bounded() {
    // Local Theme -> local Brand -> library Primitives: the value is a cached library copy.
    let mut input = two_collections("VariableCollectionId:pk/1:2", &["base"]);
    input["collections"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "brand", "name": "Brand", "modes": [{ "modeId": "b", "name": "B" }] }));
    input["variables"].as_array_mut().unwrap().push(json!({
        "id": "middle", "name": "middle", "variableCollectionId": "brand",
        "resolvedType": "COLOR", "valuesByMode": { "b": alias("blue") },
    }));
    input["variables"][0]["valuesByMode"] =
        json!({ "light": alias("middle"), "dark": alias("middle") });
    let output = convert(&input, "collection", "light").unwrap();
    let source = &output["accent"]["$extensions"]["org.figlens"]["aliasOf"];
    assert_eq!(source["collectionId"], "brand");
    assert_eq!(source["libraryCopy"], true);
    input["variables"][4]["name"] = json!("x".repeat(MAX_TEXT_BYTES + 1));
    assert!(
        codes(&convert(&input, "collection", "light").unwrap_err()).contains(&"resource_limit")
    );
}

#[test]
fn an_oversized_unresolved_target_id_fails_without_being_copied() {
    let mut input = two_collections("prims", &["base"]);
    let huge = "x".repeat(MAX_TEXT_BYTES + 1);
    input["variables"][3]["valuesByMode"]["base"] = alias(&huge);
    let error = convert(&input, "collection", "light").unwrap_err();
    assert!(codes(&error).contains(&"resource_limit"));
    assert!(!error.report.to_string().contains(&huge));
}

#[test]
fn branches_joining_a_failed_alias_share_its_failure() {
    // accent -> blue -> base fails; a second branch, other -> blue, joins it.
    let mut input = two_collections("prims", &["base"]);
    input["variables"][3]["valuesByMode"]["base"] = alias("missing");
    input["variables"].as_array_mut().unwrap().push(json!({
        "id": "other", "name": "other", "variableCollectionId": "prims",
        "resolvedType": "COLOR", "valuesByMode": { "base": alias("blue") },
    }));
    input["variables"].as_array_mut().unwrap().push(variable(
        "second",
        "second",
        "COLOR",
        alias("other"),
        alias("other"),
    ));
    let error = convert(&input, "collection", "light").unwrap_err();
    assert_eq!(codes(&error), ["missing_alias", "missing_alias"]);
}
