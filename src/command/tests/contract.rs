use super::*;
use crate::{TOOLS, contract, mcp_reply};
use serde_json::{Value, json};

fn check(tool: &str, args: Value) -> Result<contract::PreparedCall, contract::Failure> {
    contract::prepare(tool, &args)
}

#[test]
fn schema_and_numeric_preflight_share_boundaries() {
    let list = mcp_reply(r#"{"id":1,"method":"tools/list"}"#).unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    for (tool, key, min, max) in [
        ("inspect", "depth", 0, 10),
        ("find", "limit", 1, 200),
        ("export", "quality", 1, 100),
        ("export", "maxSize", 0, u64::from(u32::MAX)),
        ("tree", "depth", 0, (1 << 53) - 1),
        ("export", "maxKb", 0, (1 << 53) - 1),
    ] {
        let schema = &tools.iter().find(|t| t["name"] == tool).unwrap()["inputSchema"];
        assert_eq!(schema["properties"][key]["minimum"], min);
        assert_eq!(schema["properties"][key]["maximum"], max);
        let mut args = json!({"file":"absent.fig", "node":"0:1", "query":"Frame", "out":"out.png"});
        for value in [min, max] {
            args[key] = json!(value);
            assert!(check(tool, args.clone()).is_ok()); // No expensive rendering at large bounds.
        }
        for value in [
            json!(-1),
            json!(1.5),
            json!(max + 1),
            json!("invalid"),
            json!([]),
        ] {
            args[key] = value;
            let reply = call_tool(&json!({"name":tool,"arguments":args}), None);
            assert_eq!(
                reply["_meta"]["figlens"]["code"], "invalid_arguments",
                "{reply}"
            );
        }
    }
    assert_eq!(
        check("tree", json!({"file":"a.fig","depth":2.0}))
            .unwrap_or_else(|_| panic!())
            .argument("depth"),
        2
    );
    for args in [
        json!({"scale":0}),
        json!({"scale":-1}),
        json!({"scale":"NaN"}),
        json!({"scale":"inf"}),
        json!({"maxKb":u64::MAX}),
    ] {
        let mut all = json!({"file":"absent.fig","node":"0:1","out":"out.png"});
        all.as_object_mut()
            .unwrap()
            .extend(args.as_object().unwrap().clone());
        assert!(check("export", all).is_err());
    }
}

#[test]
fn strict_types_unknowns_and_flag_like_nodes_are_explicit() {
    for args in [
        json!({}),
        json!({"file":null}),
        json!({"file":42}),
        json!({"file":""}),
        json!({"file":"a.fig","json":"false"}),
        json!({"file":"a.fig","depth":"0"}),
        json!({"file":"a.fig","node":false}),
        json!({"file":"a.fig","depth":null}),
    ] {
        assert!(check("tree", args).is_err());
    }
    for args in [
        json!({"file":"a.fig","json":false}),
        json!({"file":"a.fig","node":"--json"}),
        json!({"file":"a.fig","futureOption":42}),
    ] {
        assert!(check("tree", args).is_ok());
    }
    for (key, value) in [
        ("scale", json!("2x")),
        ("maxSize", Value::Null),
        ("maxSize", json!(false)),
    ] {
        let mut args = json!({"file":"a.fig","node":"0:1","out":"a.png"});
        args[key] = value;
        assert!(check("export", args).is_err());
    }
    assert!(check("workflow", json!({"topic":"build","extra":true})).is_err());
    assert!(check("workflow", json!({"topic":null})).is_err());
    assert!(check("workflow", json!({"topic":"missing"})).is_err());
    assert!(check("nodes", json!({"file":"a.fig","node":null})).is_err());
}

#[test]
fn advertised_numbers_fit_json_safe_integers() {
    // Model backends parse tool schemas as int64 or JavaScript numbers; one larger value rejects every tool.
    fn walk(value: &Value) {
        match value {
            Value::Number(n) => assert!(n.as_u64().is_none_or(|n| n < 1 << 53), "{n}"),
            Value::Array(items) => items.iter().for_each(walk),
            Value::Object(map) => map.values().for_each(walk),
            _ => {}
        }
    }
    walk(&mcp_reply(r#"{"id":1,"method":"tools/list"}"#).unwrap()["result"]["tools"]);
}

#[test]
fn discovery_and_preparation_share_defaults() {
    for (name, _, params) in TOOLS {
        let args = if *name == "workflow" {
            json!({"topic":"build"})
        } else {
            json!({"file":"a.fig","node":"0:1","query":"Frame","out":"a.png"})
        };
        let call = check(name, args.clone()).unwrap();
        for param in *params {
            let property = contract::property(name, param);
            if let Some(default) = property.get("default") {
                let mut omitted = args.clone();
                omitted.as_object_mut().unwrap().remove(param.0);
                assert_eq!(
                    check(name, omitted).unwrap().argument(param.0),
                    default,
                    "{name}.{}",
                    param.0
                );
            }
        }
        assert_eq!(call.name(), *name);
    }
    let call = check("tree", json!({"file":"a.fig"})).unwrap();
    assert_eq!(call.argument("depth"), usize::MAX);
    assert!(call.string("out").is_none());
    let call = check(
        "export",
        json!({"file":"a.fig","node":"0:1","out":"a.png","maxSize":0}),
    )
    .unwrap();
    assert_eq!(call.argument("maxSize"), 0);
}

#[test]
fn inspect_states_is_an_optional_strict_boolean() {
    let args = json!({"file":"absent.fig", "node":"0:1"});
    assert_eq!(
        check("inspect", args.clone()).unwrap().argument("states"),
        false
    );
    for value in [
        json!(true),
        json!(false),
        json!("true"),
        json!(1),
        Value::Null,
    ] {
        let mut args = args.clone();
        args["states"] = value.clone();
        assert_eq!(check("inspect", args).is_ok(), value.is_boolean());
    }
    let list = mcp_reply(r#"{"id":1,"method":"tools/list"}"#).unwrap();
    let tool = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "inspect")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["states"]["default"],
        false
    );
    assert_eq!(
        tool["inputSchema"]["properties"]["states"]["type"],
        "boolean"
    );
    assert_eq!(
        tool["outputSchema"]["properties"]["states"]["type"],
        "object"
    );
    assert!(
        !tool["outputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("states"))
    );
}

#[test]
fn compare_variant_is_an_optional_nonempty_string() {
    let base = json!({"file":"absent.fig","node":"0:10"});
    assert!(
        check("inspect", base.clone())
            .unwrap()
            .string("compareVariant")
            .is_none()
    );
    for value in [
        json!("0:11"),
        json!(""),
        json!(true),
        json!(42),
        json!([]),
        Value::Null,
    ] {
        let mut args = base.clone();
        args["compareVariant"] = value.clone();
        assert_eq!(
            check("inspect", args).is_ok(),
            value.as_str().is_some_and(|s| !s.is_empty())
        );
    }
    let schema = contract::output_schema("inspect").unwrap();
    assert_eq!(schema["properties"]["variantComparison"]["type"], "object");
}

#[test]
fn bundle_states_is_an_optional_strict_boolean() {
    let base = json!({"file":"absent.fig", "node":"0:1", "out":"out.zip"});
    assert_eq!(
        check("bundle", base.clone()).unwrap().argument("states"),
        false
    );
    for value in [
        json!(true),
        json!(false),
        json!("true"),
        json!(1),
        Value::Null,
    ] {
        let mut args = base.clone();
        args["states"] = value.clone();
        assert_eq!(check("bundle", args).is_ok(), value.is_boolean());
    }
}

#[test]
fn stable_error_codes_preserve_tool_recovery_and_redact_sources() {
    let sensitive = contract::source(
        "gs://example/object?signature=hidden",
        "cloud error signature=hidden".into(),
    );
    assert!(!sensitive.to_string().contains("hidden"));
    let fixture = Fixture::new();
    for (name, arguments, code) in [
        (
            "inspect",
            json!({"file":fixture.file(),"node":"9:9"}),
            "node_not_found",
        ),
        (
            "export",
            json!({"file":fixture.file(),"node":"0:1","out":fixture.0.join("missing/out.png")}),
            "output_write_failed",
        ),
        (
            "bundle",
            json!({"file":fixture.file(),"node":"0:1","out":fixture.0.join("missing/out.zip")}),
            "output_write_failed",
        ),
        (
            "tree",
            json!({"file":"https://invalid.test/design.fig?signature=do-not-echo"}),
            "source_unreadable",
        ),
    ] {
        let reply = call_tool(&json!({"name":name,"arguments":arguments}), None);
        assert_eq!(reply["isError"], true);
        assert_eq!(reply["_meta"]["figlens"]["code"], code, "{reply}");
        assert!(!reply.to_string().contains("do-not-echo"));
        assert!(reply["_meta"]["figlens"]["nextStep"].is_string());
    }
    let reply = call_tool(
        &json!({"name":"inspect","arguments":{"file":fixture.file(),"node":"0:1"}}),
        None,
    );
    assert_eq!(reply["isError"], false);
    assert!(reply.get("_meta").is_none());
}

#[test]
fn protocol_failures_are_not_business_failures() {
    for (request, code) in [
        ("{", -32700),
        ("[]", -32600),
        (r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, -32600),
        (r#"{"id":7,"method":"missing"}"#, -32601),
        (r#"{"id":7,"method":"tools/call","params":{}}"#, -32602),
    ] {
        assert_eq!(mcp_reply(request).unwrap()["error"]["code"], code);
    }
    let reply =
        mcp_reply(r#"{"id":7,"method":"tools/call","params":{"name":"tree","arguments":{}}}"#)
            .unwrap();
    assert_eq!(reply["id"], 7);
    assert!(reply.get("error").is_none());
    assert_eq!(
        reply["result"]["_meta"]["figlens"]["code"],
        "invalid_arguments"
    );
}

#[test]
fn every_tool_rejects_missing_required_fields_and_wrong_types_before_io() {
    for (name, _, params) in TOOLS {
        let valid = if *name == "workflow" {
            json!({"topic":"build"})
        } else {
            json!({"file":"absent.fig","node":"0:1","query":"Frame","out":"out.png"})
        };
        assert!(check(name, valid.clone()).is_ok());
        for (key, _, required, _) in *params {
            let mut args = valid.clone();
            args[*key] = json!([]);
            let reply = call_tool(&json!({"name":name,"arguments":args}), None);
            assert_eq!(
                reply["_meta"]["figlens"]["code"], "invalid_arguments",
                "{name}.{key}: {reply}"
            );
            if *required {
                args.as_object_mut().unwrap().remove(*key);
                assert!(check(name, args).is_err(), "{name}.{key}");
            }
        }
    }
}
