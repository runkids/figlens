use super::*;
use crate::{TOOLS, contract, mcp_reply};
use serde_json::{Value, json};

fn check(tool: &str, args: Value) -> Result<(Value, bool), contract::Failure> {
    contract::arguments(tool, TOOLS.iter().find(|t| t.0 == tool).unwrap().2, &args)
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
            .0["depth"],
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
fn required_types_null_unknowns_and_legacy_are_explicit() {
    for args in [
        json!({}),
        json!({"file":null}),
        json!({"file":42}),
        json!({"file":""}),
        json!({"file":"a.fig","json":"false"}),
    ] {
        assert!(check("tree", args).is_err());
    }
    for args in [
        json!({"file":"a.fig","depth":"0"}),
        json!({"file":"a.fig","node":false,"depth":null,"json":false}),
        json!({"file":"a.fig","node":"--json"}),
    ] {
        assert!(check("tree", args).unwrap_or_else(|_| panic!()).1);
    }
    assert!(
        check(
            "export",
            json!({"file":"a.fig","node":"0:1","out":"a.png","scale":"2x"})
        )
        .unwrap_or_else(|_| panic!())
        .1
    );
    assert!(
        !check("tree", json!({"file":"a.fig","futureOption":42}))
            .unwrap_or_else(|_| panic!())
            .1
    );
    assert!(check("workflow", json!({"topic":"build","extra":true})).is_err());
    assert!(check("workflow", json!({"topic":null})).is_err());
    assert!(check("workflow", json!({"topic":"missing"})).is_err());
    assert!(check("nodes", json!({"file":"a.fig","node":null})).is_err());
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
        for (key, _, _, required, _) in *params {
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
