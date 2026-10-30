use crate::test_support::TestDir;
use crate::{call_tool, run, serve};
use serde_json::{Value, json};
use std::path::Path;

#[allow(dead_code)]
#[path = "../../scripts/make-token-fixture.rs"]
mod fixture;

fn export(file: &Path, out: &Path, mode: &str) -> Value {
    json!({"file":file,"format":"dtcg","collection":"VariableCollectionId:0:10",
        "mode":mode,"out":out})
}

#[test]
fn token_modes_and_aliases_reach_cli_and_mcp_without_a_workspace() {
    let dir = TestDir::new();
    let file = dir.join("tokens.fig");
    fixture::write(&file).unwrap();
    let original = std::fs::read(&file).unwrap();
    for (mode, expected_number, expected_red) in [("0:11", 1.5, 0.0), ("0:12", 2.0, 1.0)] {
        let label = mode.replace(':', "-");
        let cli_path = dir.join(&format!("cli-{label}.tokens.json"));
        let mcp_path = dir.join(&format!("mcp-{label}.tokens.json"));
        let mut stdout = String::new();
        run(
            [
                "variables",
                file.to_str().unwrap(),
                "--format",
                "dtcg",
                "--collection",
                "VariableCollectionId:0:10",
                "--mode",
                mode,
                "--out",
                cli_path.to_str().unwrap(),
            ]
            .map(str::to_owned)
            .to_vec(),
            &mut stdout,
        )
        .unwrap();
        let report: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(report["tokenCount"], 4);
        let reply = call_tool(
            &json!({"name":"variables","arguments":export(&file, &mcp_path, mode)}),
            None,
        );
        assert_eq!(reply["isError"], false, "{reply}");
        assert_eq!(reply["structuredContent"]["modeId"], mode);
        let cli: Value = serde_json::from_slice(&std::fs::read(cli_path).unwrap()).unwrap();
        let mcp: Value = serde_json::from_slice(&std::fs::read(mcp_path).unwrap()).unwrap();
        assert_eq!(cli, mcp);
        assert_eq!(cli["ratio"]["$type"], "number");
        assert_eq!(cli["ratio"]["$value"], expected_number);
        assert_eq!(
            cli["palette"]["brand"]["$value"]["components"][0],
            expected_red
        );
        assert_eq!(cli["semantic"]["action"]["$value"], "{semantic.link}");
        assert_eq!(cli["semantic"]["link"]["$value"], "{palette.brand}");
        assert_eq!(
            cli["ratio"]["$extensions"]["org.figlens"]["variableId"],
            "VariableID:0:23"
        );
        assert_eq!(
            cli["$extensions"]["org.figlens"]["source"],
            file.canonicalize().unwrap().to_str().unwrap()
        );
    }
    let rest = call_tool(&json!({"name":"variables","arguments":{"file":file}}), None);
    let rest: Value = serde_json::from_str(rest["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        rest["variables"][0]["valuesByMode"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(std::fs::read(file).unwrap(), original);
}

#[test]
fn invalid_token_export_preserves_outputs_and_source_aliases() {
    let dir = TestDir::new();
    let file = dir.join("tokens.fig");
    fixture::write(&file).unwrap();
    let original = std::fs::read(&file).unwrap();
    let output = dir.join("existing.tokens.json");
    std::fs::write(&output, b"existing").unwrap();
    let reply = call_tool(
        &json!({"name":"variables","arguments":export(&file, &output, "missing")}),
        None,
    );
    assert_eq!(reply["isError"], true, "{reply}");
    let report: Value =
        serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "unknown_mode")
    );
    assert_eq!(std::fs::read(output).unwrap(), b"existing");
    let reply = call_tool(
        &json!({"name":"variables","arguments":export(&file, &file, "0:11")}),
        None,
    );
    assert_eq!(
        reply["_meta"]["figlens"]["code"], "invalid_arguments",
        "{reply}"
    );
    #[cfg(unix)]
    {
        let alias = dir.join("alias.tokens.json");
        std::os::unix::fs::symlink(file.canonicalize().unwrap(), &alias).unwrap();
        let reply = call_tool(
            &json!({"name":"variables","arguments":export(&file, &alias, "0:11")}),
            None,
        );
        assert_eq!(
            reply["_meta"]["figlens"]["code"], "invalid_arguments",
            "{reply}"
        );
    }
    assert_eq!(std::fs::read(file).unwrap(), original);
}

#[test]
fn rooted_token_export_rejects_external_output_before_publication() {
    let dir = TestDir::new();
    let external = TestDir::new();
    let file = dir.join("tokens.fig");
    fixture::write(&file).unwrap();
    let out = external.join("out.tokens.json");
    let request = json!({"id":1,"method":"tools/call","params":{
        "name":"variables","arguments":export(&file, &out, "0:11")}})
    .to_string()
        + "\n";
    let mut output = Vec::new();
    serve(
        request.as_bytes(),
        &mut output,
        &[dir.join("").canonicalize().unwrap()],
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        reply["result"]["_meta"]["figlens"]["code"], "permission_denied",
        "{reply}"
    );
    assert!(!out.exists());
}
