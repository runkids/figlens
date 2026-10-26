//! Additive snapshot semantics. Mappings are not yet compared with same-version Figma exports.
use super::*;

fn status(source: &str, present: bool, supported: bool) -> Json {
    json!({ "source": source, "state": if !present { "absent" } else if supported { "recorded" } else { "unsupported" } })
}

pub(super) fn layout(k: &Obj, parent: Option<&Obj>, out: &mut Obj) -> Json {
    let mut evidence = json!({ "verification": "not-compared-with-figma" });
    let mut constraints = Obj::new();
    for (axis, field, min, max, stretch) in [
        (
            "horizontal",
            "horizontalConstraint",
            "LEFT",
            "RIGHT",
            "LEFT_RIGHT",
        ),
        (
            "vertical",
            "verticalConstraint",
            "TOP",
            "BOTTOM",
            "TOP_BOTTOM",
        ),
    ] {
        let mapped = match at(k, field).as_str() {
            Some("MIN") => Some(min),
            Some("MAX") => Some(max),
            Some("CENTER") => Some("CENTER"),
            Some("SCALE") => Some("SCALE"),
            Some("STRETCH") => Some(stretch),
            _ => None,
        };
        evidence[format!("constraints.{axis}")] =
            status(field, k.contains_key(field), mapped.is_some());
        if let Some(value) = mapped {
            constraints.insert(axis.into(), json!(value));
        }
    }
    if !constraints.is_empty() {
        out.insert("constraints".into(), constraints.into());
    }
    let flow = matches!(
        parent.and_then(|p| at(p, "stackMode").as_str()),
        Some("HORIZONTAL" | "VERTICAL")
    ) && matches!(at(k, "stackPositioning").as_str(), None | Some("AUTO"));
    let grow = at(k, "stackChildPrimaryGrow")
        .as_f64()
        .filter(|v| flow && (*v == 0.0 || *v == 1.0));
    let align = at(k, "stackChildAlignSelf")
        .as_str()
        .filter(|v| flow && matches!(*v, "INHERIT" | "STRETCH" | "MIN" | "CENTER" | "MAX"));
    evidence["layoutGrow"] = status(
        "stackChildPrimaryGrow",
        k.contains_key("stackChildPrimaryGrow"),
        grow.is_some(),
    );
    evidence["layoutAlign"] = status(
        "stackChildAlignSelf",
        k.contains_key("stackChildAlignSelf"),
        align.is_some(),
    );
    if let Some(value) = grow {
        out.insert("layoutGrow".into(), json!(value));
    }
    if let Some(value) = align {
        out.insert("layoutAlign".into(), json!(value));
    }
    evidence
}

fn definition_id(value: &Json) -> Option<String> {
    let id = guid(value)?;
    let session = value["sessionID"].as_u64()?;
    let local = value["localID"].as_u64()?;
    (session <= u32::MAX as u64 && local <= u32::MAX as u64 && id != "4294967295:4294967295")
        .then_some(id)
}

/// Keep the assignment path's existing parent-definition fallback unchanged.
pub(super) fn resolve_definition<'a>(
    own: &'a Json,
    parents: &HashMap<String, &'a Json>,
) -> &'a Json {
    guid(&own["parentPropDefId"])
        .and_then(|id| parents.get(&id).copied())
        .unwrap_or(own)
}

pub(super) fn definitions(tree: &Tree, id: &str, k: &Obj, out: &mut Obj, evidence: &mut Json) {
    let parent_id = parent_id(tree, id);
    let parent = parent_id.as_ref().map(|id| to_json(tree.nodes[id]));
    let parents = parent
        .iter()
        .flat_map(|p| p["componentPropDefs"].as_array().into_iter().flatten())
        .filter_map(|d| Some((guid(&d["id"])?, d)))
        .collect::<HashMap<_, _>>();
    let list = at(k, "componentPropDefs").as_array();
    let mut declarations = Obj::new();
    let mut entries = Vec::new();
    for (index, own) in list.into_iter().flatten().enumerate() {
        let def = resolve_definition(own, &parents);
        let inherited = own.get("parentPropDefId").is_some();
        let resolved = !inherited
            || definition_id(&own["parentPropDefId"]).is_some_and(|id| parents.contains_key(&id));
        let def_id = definition_id(&def["id"]);
        let own_id = definition_id(&own["id"]);
        let name = def["name"].as_str();
        let kind = match def["type"].as_str() {
            Some("BOOL" | "BOOLEAN") => Some("BOOLEAN"),
            Some("TEXT") => Some("TEXT"),
            Some("INSTANCE_SWAP") => Some("INSTANCE_SWAP"),
            Some("VARIANT") => Some("VARIANT"),
            _ => None,
        };
        let supported =
            resolved && def_id.is_some() && own_id.is_some() && name.is_some() && kind.is_some();
        let mut entry = status(
            "componentPropDefs",
            true,
            supported && kind != Some("VARIANT"),
        );
        entry["index"] = json!(index);
        entry["id"] = json!(own_id);
        entry["definitionId"] = json!(def_id);
        entry["name"] = json!(name);
        entry["type"] = json!(def["type"].as_str());
        entry["sourceNodeId"] = json!(if inherited && resolved {
            parent_id.as_deref().unwrap_or(id)
        } else {
            id
        });
        if supported {
            let kind = kind.unwrap();
            let mut declaration = json!({"type":kind});
            let value = prop_value_source(def);
            let usable = value.as_ref().is_some_and(|(v, source)| {
                let raw = match *source {
                    "varValue.value" => &def["varValue"]["value"],
                    source => &def[source],
                };
                match kind {
                    "BOOLEAN" => v.is_boolean(),
                    "TEXT" => {
                        v.is_string()
                            && (raw["textDataValue"]["characters"].is_string()
                                || raw["textValue"].is_string()
                                || raw["textValue"]["characters"].is_string())
                    }
                    "INSTANCE_SWAP" => definition_id(&raw["symbolIdValue"]["guid"])
                        .or_else(|| definition_id(&raw["guidValue"]))
                        .is_some_and(|id| v.as_str() == Some(id.as_str())),
                    _ => false,
                }
            });
            let source = value
                .as_ref()
                .map_or("varValue.value|value|initialValue", |(_, source)| *source);
            entry["defaultValue"] = status(
                source,
                ["varValue", "value", "initialValue"]
                    .iter()
                    .any(|f| def.get(*f).is_some()),
                usable,
            );
            if usable {
                declaration["defaultValue"] = value.unwrap().0;
            }
            if kind == "INSTANCE_SWAP" {
                let preferred = def["preferredValues"]["instanceSwapValues"].as_array();
                let usable = preferred.is_some_and(|a| {
                    a.iter().all(|v| {
                        matches!(v["type"].as_str(), Some("COMPONENT" | "COMPONENT_SET"))
                            && v["key"].as_str().is_some_and(|k| !k.is_empty())
                    })
                });
                entry["preferredValues"] = status(
                    "preferredValues.instanceSwapValues",
                    def["preferredValues"].get("instanceSwapValues").is_some(),
                    usable,
                );
                if usable {
                    declaration["preferredValues"] = preferred
                        .unwrap()
                        .iter()
                        .map(|v| json!({"type":v["type"],"key":v["key"]}))
                        .collect();
                }
            }
            if kind == "VARIANT" {
                // shortcut: stored variant defaults/options are not established; add them after independent capture validation.
                entry["defaultValue"]["state"] = json!("unsupported");
                entry["variantOptions"] = json!({"state":"unsupported", "source":"componentPropDefs", "reason":"complete variant options are not established"});
            }
            let key = if kind == "VARIANT" {
                name.unwrap().to_owned()
            } else {
                format!("{}#{}", name.unwrap(), def_id.unwrap())
            };
            declarations.insert(key, declaration);
        }
        entries.push(entry);
    }
    let mut state = status(
        "componentPropDefs",
        k.contains_key("componentPropDefs"),
        list.is_some(),
    );
    if !entries.is_empty() {
        state["entries"] = entries.into();
    }
    evidence["componentPropertyDefinitions"] = state;
    if !declarations.is_empty() {
        out.insert("componentPropertyDefinitions".into(), declarations.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwi_schema::Value as K;

    fn kiwi<'a>(v: &'a Json, field: &str) -> K<'a> {
        match v {
            Json::Object(o) => K::Object(
                "Test",
                o.iter().map(|(k, v)| (k.as_str(), kiwi(v, k))).collect(),
            ),
            Json::Array(a) => K::Array(a.iter().map(|v| kiwi(v, field)).collect()),
            Json::Bool(b) => K::Bool(*b),
            Json::Number(n) if matches!(field, "sessionID" | "localID" | "commandsBlob") => {
                K::UInt(n.as_u64().unwrap() as u32)
            }
            Json::Number(n) => K::Float(n.as_f64().unwrap() as f32),
            Json::String(s)
                if matches!(
                    field,
                    "type"
                        | "stackMode"
                        | "horizontalConstraint"
                        | "verticalConstraint"
                        | "stackChildAlignSelf"
                        | "stackPositioning"
                ) =>
            {
                K::Enum("TestEnum", s)
            }
            Json::String(s) => K::String(s.clone()),
            _ => panic!("unsupported test value"),
        }
    }

    #[test]
    fn layout_records_only_present_supported_values_in_flow() {
        for (raw, h, v) in [
            ("MIN", "LEFT", "TOP"),
            ("MAX", "RIGHT", "BOTTOM"),
            ("CENTER", "CENTER", "CENTER"),
            ("SCALE", "SCALE", "SCALE"),
            ("STRETCH", "LEFT_RIGHT", "TOP_BOTTOM"),
        ] {
            let mut out = Obj::new();
            let k = object(json!({"horizontalConstraint":raw,"verticalConstraint":raw}));
            let evidence = layout(&k, None, &mut out);
            assert_eq!(out["constraints"], json!({"horizontal":h,"vertical":v}));
            assert_eq!(evidence["constraints.horizontal"]["state"], "recorded");
        }
        for mode in ["HORIZONTAL", "VERTICAL", "GRID", "NONE"] {
            for positioning in [None, Some("AUTO"), Some("ABSOLUTE")] {
                for grow in [0, 1, 2] {
                    let mut k = object(
                        json!({"horizontalConstraint":"NEW", "stackChildPrimaryGrow":grow,"stackChildAlignSelf":"STRETCH"}),
                    );
                    if let Some(p) = positioning {
                        k.insert("stackPositioning".into(), json!(p));
                    }
                    let parent = object(json!({"stackMode":mode}));
                    let mut out = Obj::new();
                    let evidence = layout(&k, Some(&parent), &mut out);
                    let flow = matches!(mode, "HORIZONTAL" | "VERTICAL")
                        && positioning != Some("ABSOLUTE");
                    assert_eq!(out.contains_key("layoutGrow"), flow && grow < 2);
                    assert_eq!(out.contains_key("layoutAlign"), flow);
                    assert!(!out.contains_key("constraints"));
                    assert_eq!(evidence["constraints.horizontal"]["state"], "unsupported");
                    assert_eq!(evidence["constraints.vertical"]["state"], "absent");
                }
            }
        }
        let mut out = Obj::new();
        let evidence = layout(&Obj::new(), None, &mut out);
        assert!(out.is_empty());
        assert_eq!(evidence["layoutGrow"]["state"], "absent");
        for align in ["INHERIT", "MIN", "CENTER", "MAX", "STRETCH", "NEW"] {
            let mut out = Obj::new();
            layout(
                &object(json!({"stackChildAlignSelf":align})),
                Some(&object(json!({"stackMode":"HORIZONTAL"}))),
                &mut out,
            );
            assert_eq!(out.contains_key("layoutAlign"), align != "NEW");
        }
    }

    #[test]
    fn definitions_preserve_typed_defaults_and_explicit_unknowns() {
        let tree = Tree {
            nodes: HashMap::new(),
            children: HashMap::new(),
            blobs: Vec::new(),
        };
        let k = object(json!({"componentPropDefs":[
            {"id":{"sessionID":1,"localID":1},"name":"Show","type":"BOOL","varValue":{"value":{"boolValue":false}}},
            {"id":{"sessionID":1,"localID":2},"name":"Label","type":"TEXT","initialValue":{"textDataValue":{"characters":""}}},
            {"id":{"sessionID":1,"localID":3},"name":"Icon","type":"INSTANCE_SWAP","value":{"symbolIdValue":{"guid":{"sessionID":2,"localID":3}}},"preferredValues":{"instanceSwapValues":[{"type":"COMPONENT","key":"icon"},{"type":"COMPONENT_SET","key":"icons"}]}},
            {"id":{"sessionID":1,"localID":4},"name":"Bad swap","type":"INSTANCE_SWAP","value":{"textValue":"2:3"},"preferredValues":{"instanceSwapValues":[{"type":"NEW","key":"unknown"}]}},
            {"id":{"sessionID":1,"localID":5},"name":"State","type":"VARIANT","value":{"textValue":""}},
            {"id":{"sessionID":1,"localID":6},"name":"Slot","type":"SLOT"},
            {"id":{"sessionID":1,"localID":7},"name":"Future","type":"NEW"},
            {"id":{"sessionID":1,"localID":8},"name":"Missing","type":"TEXT","parentPropDefId":{"sessionID":9,"localID":9}},
            {"id":{"sessionID":1,"localID":9},"name":"Unset","type":"INSTANCE_SWAP","value":{"guidValue":{"sessionID":4294967295u64,"localID":4294967295u64}}},
            {"id":{"sessionID":1,"localID":10},"name":"Wrong text","type":"TEXT","value":{"guidValue":{"sessionID":2,"localID":3}}}
        ]}));
        let mut out = Obj::new();
        let mut evidence = json!({});
        definitions(&tree, "1:0", &k, &mut out, &mut evidence);
        let defs = &out["componentPropertyDefinitions"];
        assert_eq!(defs["Show#1:1"]["defaultValue"], false);
        assert_eq!(defs["Label#1:2"]["defaultValue"], "");
        assert_eq!(defs["Icon#1:3"]["defaultValue"], "2:3");
        assert_eq!(
            defs["Icon#1:3"]["preferredValues"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(defs["Bad swap#1:4"], json!({"type":"INSTANCE_SWAP"}));
        assert_eq!(defs["State"], json!({"type":"VARIANT"}));
        assert_eq!(defs["Unset#1:9"], json!({"type":"INSTANCE_SWAP"}));
        assert_eq!(defs["Wrong text#1:10"], json!({"type":"TEXT"}));
        assert_eq!(defs.as_object().unwrap().len(), 7);
        let entries = &evidence["componentPropertyDefinitions"]["entries"];
        for i in [4, 5, 6, 7] {
            assert_eq!(entries[i]["state"], "unsupported");
        }
        assert_eq!(entries[0]["defaultValue"]["source"], "varValue.value");
        assert_eq!(entries[1]["defaultValue"]["source"], "initialValue");
        assert_eq!(entries[3]["preferredValues"]["state"], "unsupported");
    }

    #[test]
    fn definitions_resolve_parent_and_referenced_metadata() {
        let raw = json!([
            {"type":"FRAME","isStateGroup":true,"componentPropDefs":[{"id":{"sessionID":4,"localID":0},"name":"Label","type":"TEXT","value":{"textValue":"parent"}}]},
            {"type":"SYMBOL","parentIndex":{"guid":{"sessionID":1,"localID":0}},"componentPropDefs":[{"id":{"sessionID":4,"localID":1},"parentPropDefId":{"sessionID":4,"localID":0}}]},
            {"type":"INSTANCE","symbolData":{"symbolID":{"sessionID":1,"localID":1}}}
        ]);
        let values: Vec<_> = raw
            .as_array()
            .unwrap()
            .iter()
            .map(|v| kiwi(v, ""))
            .collect();
        let tree = Tree {
            nodes: values
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("1:{i}"), v))
                .collect(),
            children: HashMap::new(),
            blobs: Vec::new(),
        };
        let component = nodes(&tree, "1:1");
        assert_eq!(
            component["document"]["componentPropertyDefinitions"]["Label#4:0"]["defaultValue"],
            "parent"
        );
        let entry = &component["document"]["_figlens"]["semantics"]["componentPropertyDefinitions"]
            ["entries"][0];
        assert_eq!(entry["id"], "4:1");
        assert_eq!(entry["definitionId"], "4:0");
        assert_eq!(entry["sourceNodeId"], "1:0");
        let instance = nodes(&tree, "1:2");
        assert_eq!(
            instance["components"]["1:1"]["componentPropertyDefinitions"],
            component["document"]["componentPropertyDefinitions"]
        );
        assert_eq!(
            instance["componentSets"]["1:0"]["componentPropertyDefinitions"],
            component["document"]["componentPropertyDefinitions"]
        );
        assert_eq!(
            instance["document"]["componentProperties"]["Label#4:0"]["value"],
            "parent"
        );
    }

    #[test]
    fn expanded_layout_uses_effective_override_and_swap_values() {
        let raw = json!([
            {"type":"INSTANCE","stackMode":"HORIZONTAL","size":{"x":100,"y":100},"symbolData":{"symbolID":{"sessionID":1,"localID":1},"symbolOverrides":[{"guidPath":{"guids":[{"sessionID":1,"localID":2}]},"horizontalConstraint":"MAX","stackChildPrimaryGrow":1,"stackChildAlignSelf":"STRETCH","overriddenSymbolID":{"sessionID":1,"localID":3}}]}},
            {"type":"SYMBOL","stackMode":"HORIZONTAL","size":{"x":100,"y":100}},
            {"type":"INSTANCE","size":{"x":20,"y":20},"horizontalConstraint":"MIN","stackChildPrimaryGrow":0,"stackChildAlignSelf":"INHERIT","symbolData":{"symbolID":{"sessionID":1,"localID":4}}},
            {"type":"SYMBOL","size":{"x":20,"y":20},"verticalConstraint":"CENTER"},
            {"type":"SYMBOL","size":{"x":20,"y":20}}
        ]);
        let values: Vec<_> = raw
            .as_array()
            .unwrap()
            .iter()
            .map(|v| kiwi(v, ""))
            .collect();
        let tree = Tree {
            nodes: values
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("1:{i}"), v))
                .collect(),
            children: HashMap::from([("1:1".into(), vec!["1:2".into()])]),
            blobs: Vec::new(),
        };
        let doc = nodes(&tree, "1:0");
        let child = &doc["document"]["children"][0];
        assert_eq!(child["constraints"], json!({"horizontal":"RIGHT"}));
        assert_eq!(child["layoutGrow"], 1.0);
        assert_eq!(child["layoutAlign"], "STRETCH");
        assert_eq!(child["layoutSizingHorizontal"], "FILL");
        assert_eq!(
            child["_figlens"]["semantics"]["instanceContext"],
            json!({"instanceId":"1:0","sourceNodeId":"1:3","state":"effective"})
        );
        let selected = resolve_nodes(&tree, child["id"].as_str().unwrap()).unwrap();
        assert_eq!(selected["document"], *child);
    }
    #[test]
    fn swapped_layout_does_not_inherit_target_placement() {
        let raw = json!([
            {"type":"INSTANCE","stackMode":"HORIZONTAL","size":{"x":100,"y":100},"symbolData":{"symbolID":{"sessionID":1,"localID":1},"symbolOverrides":[{"guidPath":{"guids":[{"sessionID":1,"localID":2}]},"overriddenSymbolID":{"sessionID":1,"localID":3}}]}},
            {"type":"SYMBOL","stackMode":"HORIZONTAL","size":{"x":100,"y":100}},
            {"type":"INSTANCE","size":{"x":20,"y":20},"symbolData":{"symbolID":{"sessionID":1,"localID":4}}},
            {"type":"SYMBOL","size":{"x":20,"y":20},"verticalConstraint":"CENTER","stackChildPrimaryGrow":1,"stackChildAlignSelf":"STRETCH"},
            {"type":"SYMBOL","size":{"x":20,"y":20}}
        ]);
        let values: Vec<_> = raw
            .as_array()
            .unwrap()
            .iter()
            .map(|v| kiwi(v, ""))
            .collect();
        let tree = Tree {
            nodes: values
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("1:{i}"), v))
                .collect(),
            children: HashMap::from([("1:1".into(), vec!["1:2".into()])]),
            blobs: Vec::new(),
        };
        let doc = nodes(&tree, "1:0");
        let child = &doc["document"]["children"][0];
        assert!(child.get("constraints").is_none());
        assert!(child.get("layoutGrow").is_none());
        assert!(child.get("layoutAlign").is_none());
        for field in ["layoutGrow", "layoutAlign"] {
            assert_eq!(child["_figlens"]["semantics"][field]["state"], "absent");
        }
        // The existing sizing path is deliberately unchanged by additive semantics.
        assert_eq!(child["layoutSizingHorizontal"], "FILL");
        assert_eq!(
            child["_figlens"]["semantics"]["instanceContext"],
            json!({"instanceId":"1:0","sourceNodeId":"1:3","state":"effective"})
        );
        let selected = resolve_nodes(&tree, child["id"].as_str().unwrap()).unwrap();
        assert_eq!(selected["document"], *child);
    }
    #[test]
    fn swapped_constraints_preserve_placement_and_absence() {
        for constraint in [Some("STRETCH"), None] {
            let mut raw = json!([
                {"type":"INSTANCE","size":{"x":200,"y":100},"symbolData":{"symbolID":{"sessionID":3,"localID":1},"symbolOverrides":[{"guidPath":{"guids":[{"sessionID":3,"localID":2}]},"overriddenSymbolID":{"sessionID":3,"localID":3}}]}},
                {"type":"SYMBOL","size":{"x":100,"y":100}},
                {"type":"INSTANCE","size":{"x":20,"y":20},"symbolData":{"symbolID":{"sessionID":3,"localID":4}}},
                {"type":"SYMBOL","size":{"x":20,"y":20},"horizontalConstraint":"MIN","verticalConstraint":"MAX"},
                {"type":"SYMBOL","size":{"x":20,"y":20}}
            ]);
            if let Some(value) = constraint {
                raw[2]["horizontalConstraint"] = json!(value);
            }
            let values: Vec<_> = raw
                .as_array()
                .unwrap()
                .iter()
                .map(|v| kiwi(v, ""))
                .collect();
            let tree = Tree {
                nodes: values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("3:{i}"), v))
                    .collect(),
                children: HashMap::from([("3:1".into(), vec!["3:2".into()])]),
                blobs: Vec::new(),
            };
            let doc = nodes(&tree, "3:0");
            let child = &doc["document"]["children"][0];
            assert_eq!(
                child["absoluteBoundingBox"]["width"],
                if constraint.is_some() { 120.0 } else { 20.0 }
            );
            assert_eq!(
                child["constraints"]["horizontal"],
                constraint
                    .map(|_| json!("LEFT_RIGHT"))
                    .unwrap_or(Json::Null)
            );
            assert!(child["constraints"].get("vertical").is_none());
            assert_eq!(
                child["_figlens"]["semantics"]["constraints.vertical"]["state"],
                "absent"
            );
        }
    }

    #[test]
    fn render_diagnostics_include_drawn_strokes_and_masks() {
        let gradient = json!({"type":"GRADIENT_ANGULAR","transform":{"m00":1,"m01":0,"m02":0,"m10":0,"m11":1,"m12":0},"stops":[{"position":0,"color":{"r":1,"g":1,"b":1,"a":0}},{"position":1,"color":{"r":1,"g":1,"b":1,"a":1}}]});
        let mut bytes = Vec::new();
        for (command, x, y) in [(1u8, 0f32, 0f32), (2, 16., 0.), (2, 16., 4.), (2, 0., 4.)] {
            bytes.push(command);
            bytes.extend(x.to_le_bytes());
            bytes.extend(y.to_le_bytes());
        }
        bytes.push(0);
        let blob = K::Object("Blob", HashMap::from([("bytes", K::Bytes(bytes))]));
        for (weight, geometry, visible, expected) in [
            (4, true, true, true),
            (0, true, true, false),
            (4, false, true, false),
            (4, true, false, false),
        ] {
            let mut raw = json!({"type":"RECTANGLE","visible":visible,"size":{"x":16,"y":16},"strokeWeight":weight,"strokePaints":[gradient.clone()]});
            if geometry {
                raw["strokeGeometry"] = json!([{"commandsBlob":0}]);
            }
            let node = kiwi(&raw, "");
            let tree = Tree {
                nodes: HashMap::from([("1:1".into(), &node)]),
                children: HashMap::new(),
                blobs: vec![&blob],
            };
            let doc = render_nodes(&tree, "1:1");
            let n = &doc["document"];
            assert_eq!(n["_figlens"]["renderDiagnostics"].is_array(), expected);
            if expected {
                assert_eq!(
                    n["_figlens"]["renderDiagnostics"][0]["field"],
                    "strokePaints[0].type"
                );
                let svg = render::Svg::new(&tree, &|_| None).document(n, [0., 0., 16., 16.], 1.);
                assert!(svg.contains("radialGradient"));
            }
        }
        for visible in [true, false] {
            let raw = json!([
                {"type":"FRAME","size":{"x":16,"y":16}},
                {"type":"RECTANGLE","size":{"x":16,"y":16},"mask":true,"visible":visible,"fillPaints":[gradient.clone()]},
                {"type":"RECTANGLE","size":{"x":16,"y":16},"fillPaints":[{"type":"SOLID","color":{"r":1,"g":0,"b":0,"a":1}}]}
            ]);
            let values: Vec<_> = raw
                .as_array()
                .unwrap()
                .iter()
                .map(|v| kiwi(v, ""))
                .collect();
            let tree = Tree {
                nodes: values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("2:{i}"), v))
                    .collect(),
                children: HashMap::from([("2:0".into(), vec!["2:1".into(), "2:2".into()])]),
                blobs: Vec::new(),
            };
            let doc = render_nodes(&tree, "2:0");
            let mask = &doc["document"]["children"][0];
            assert_eq!(mask["_figlens"]["renderDiagnostics"].is_array(), visible);
            if visible {
                assert_eq!(
                    mask["_figlens"]["renderDiagnostics"][0]["field"],
                    "fillPaints[0].type"
                );
                let svg = render::Svg::new(&tree, &|_| None).document(
                    &doc["document"],
                    [0., 0., 16., 16.],
                    1.,
                );
                assert!(svg.contains("mask-type:alpha") && svg.contains("radialGradient"));
            }
        }
    }
}
