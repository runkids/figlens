//! Stdio request admission and borrowed local Tree reuse.

use std::error::Error;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use figlens::{Document, ErrorKind, Tree};
use serde_json::{Value, json};

use crate::{command, contract, workflow};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

struct ResponseSize(usize);

impl std::io::Write for ResponseSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MAX_RESPONSE_BYTES {
            return Err(std::io::Error::other("response budget exceeded"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bounded_response(value: Value) -> Value {
    // Count the complete escaped wire message, including its newline, without another large buffer.
    if serde_json::to_writer(&mut ResponseSize(1), &value).is_ok() {
        return value;
    }
    let message = "response exceeds 8388608 bytes; use inspect, a scoped bundle, or CLI nodes instead of a larger MCP reply";
    if value["result"].get("isError").is_some() {
        let mut error = tool_reply(
            "",
            Err(contract::Failure::new(ErrorKind::ResponseBudget, message)),
        );
        if let Some(path) = value["result"]["structuredContent"].get("path") {
            error["_meta"]["figlens"]["outputPath"] = path.clone();
        }
        response(&value["id"], error)
    } else {
        protocol(value["id"].clone(), -32603, message)
    }
}

fn next_request(input: &mut impl BufRead, roots: &[PathBuf]) -> std::io::Result<Option<Admission>> {
    loop {
        let mut line = Vec::new();
        if input
            .by_ref()
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut line)?
            == 0
        {
            return Ok(None);
        }
        if line.len() as u64 > MAX_REQUEST_BYTES {
            if line.last() != Some(&b'\n') {
                input.skip_until(b'\n')?;
            }
            return Ok(Some(Admission::Reply(protocol(
                Value::Null,
                -32600,
                "request line exceeds 1048576 bytes; send a smaller request",
            ))));
        }
        match std::str::from_utf8(&line) {
            Ok(line) if line.trim().is_empty() => continue,
            Ok(line) => return Ok(Some(admit(line, roots))),
            Err(_) => {
                return Ok(Some(Admission::Reply(protocol(
                    Value::Null,
                    -32700,
                    "parse error",
                ))));
            }
        }
    }
}

// Only admission can construct these values; execution consumes the checked paths.
struct AdmittedCall {
    id: Value,
    call: contract::PreparedCall,
    source: Option<Source>,
    output: Option<PathBuf>,
    /// A bundle to verify: read, never loaded as a design.
    input: Option<PathBuf>,
}

enum Source {
    Local(PathBuf),
    Remote(String),
}

enum Admission {
    Ignore,
    Reply(Value),
    Call(AdmittedCall),
}

fn envelope(req: &Value) -> bool {
    req.is_object()
        && req["method"].is_string()
        && req.get("jsonrpc").is_none_or(|v| v == "2.0")
        && req
            .get("id")
            .is_none_or(|v| v.is_null() || v.is_string() || v.is_number())
}

fn protocol(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

fn response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}

/// Preserve ordinary unrooted I/O failures instead of inventing new path restrictions.
/// Restricted new outputs require an existing canonical parent; dangling links are refused.
fn checked_path(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, contract::Failure> {
    #[cfg(test)]
    PATH_CHECKS.with(|n| n.set(n.get() + 1));
    let resolved = std::fs::canonicalize(path).or_else(|error| {
        if std::fs::symlink_metadata(path).is_ok() {
            return if roots.is_empty() {
                std::path::absolute(path)
            } else {
                Err(error)
            };
        }
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => {
                let parent = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
                std::fs::canonicalize(parent)
                    .map(|parent| parent.join(name))
                    .or_else(|e| {
                        if roots.is_empty() {
                            std::path::absolute(path)
                        } else {
                            Err(e)
                        }
                    })
            }
            _ if roots.is_empty() => std::path::absolute(path),
            _ => Err(error),
        }
    });
    if let Ok(path) = resolved
        && (roots.is_empty() || roots.iter().any(|root| path.starts_with(root)))
    {
        return Ok(path);
    }
    let folders: Vec<_> = roots.iter().map(|r| r.display().to_string()).collect();
    Err(contract::Failure::new(
        ErrorKind::PermissionDenied,
        format!(
            "source or output is outside the folders this server may use ({})",
            folders.join(", ")
        ),
    ))
}

fn admit(line: &str, roots: &[PathBuf]) -> Admission {
    let req: Value = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(_) => return Admission::Reply(protocol(Value::Null, -32700, "parse error")),
    };
    if !envelope(&req) {
        return Admission::Reply(protocol(Value::Null, -32600, "invalid request"));
    }
    let Some(id) = req.get("id") else {
        return Admission::Ignore;
    };
    let result = match req["method"].as_str().unwrap() {
        "initialize" => {
            let mut instructions = workflow::INSTRUCTIONS.to_owned();
            if !roots.is_empty() {
                let folders: Vec<_> = roots.iter().map(|r| r.display().to_string()).collect();
                instructions +=
                    &format!(" Files and outputs must be under: {}.", folders.join(", "));
            }
            json!({
                "protocolVersion":req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities":{"tools":{}},
                "serverInfo":{"name":"figlens","version":env!("CARGO_PKG_VERSION")},
                "instructions":instructions,
            })
        }
        "ping" => json!({}),
        "tools/list" => contract::tools(),
        "tools/call" => {
            let Some(name) = req["params"]["name"].as_str() else {
                return Admission::Reply(protocol(
                    id.clone(),
                    -32602,
                    "tools/call requires a tool name",
                ));
            };
            #[cfg(test)]
            PREPARATIONS.with(|n| n.set(n.get() + 1));
            let prepared = contract::prepare(name, &req["params"]["arguments"]);
            let admitted = prepared.and_then(|call| {
                let source = match call.string("file") {
                    Some(file) if file.starts_with("gs://") => {
                        if !roots.is_empty() {
                            return Err(contract::Failure::new(ErrorKind::PermissionDenied,
                                "GCS sources are not supported with --root; use an authorized local copy inside the configured roots"));
                        }
                        Some(Source::Remote(file.to_owned()))
                    }
                    Some(file) => Some(Source::Local(checked_path(Path::new(file), roots)?)),
                    None => None,
                };
                let output = call.string("out").map(|out| checked_path(Path::new(out), roots)).transpose()?;
                let input = call.string("bundle").map(|path| checked_path(Path::new(path), roots)).transpose()?;
                Ok(AdmittedCall { id:id.clone(), call, source, output, input })
            });
            return match admitted {
                Ok(call) => Admission::Call(call),
                Err(error) => Admission::Reply(response(id, tool_reply(name, Err(error)))),
            };
        }
        method => {
            return Admission::Reply(protocol(
                id.clone(),
                -32601,
                &format!("unknown method {method}"),
            ));
        }
    };
    Admission::Reply(response(id, result))
}

impl AdmittedCall {
    fn local(&self) -> Option<&Path> {
        match &self.source {
            Some(Source::Local(path)) => Some(path),
            _ => None,
        }
    }

    fn recheck(&self, roots: &[PathBuf]) -> Result<(), contract::Failure> {
        for path in self
            .local()
            .into_iter()
            .chain(self.output.as_deref())
            .chain(self.input.as_deref())
        {
            // Recheck current filesystem state, not raw arguments or a permanent authorization lease.
            checked_path(path, roots)?;
        }
        Ok(())
    }

    fn execute(&self, loaded: Option<(&Path, &Tree)>, roots: &[PathBuf]) -> Value {
        let name = self.call.name();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.recheck(roots)?;
            if name == "workflow" {
                return workflow::read(self.call.string("topic").unwrap())
                    .map(command::Output::Text)
                    .map_err(contract::Failure::argument);
            }
            if let Some(bundle) = &self.input {
                let mut value =
                    figlens::verify::verify(bundle).map_err(contract::Failure::from_error)?;
                // Reply with the caller's label, like output paths.
                value["path"] = self.call.string("bundle").unwrap().into();
                return Ok(command::Output::Json {
                    value,
                    pretty: false,
                });
            }
            let run = |tree: &Tree, path: &Path| {
                command::Command::from_tool(&self.call, tree, self.output.as_deref())?
                    .execute(tree, path)
            };
            let result: Result<command::Output, Box<dyn Error>> = (|| {
                if let Some((path, tree)) = loaded {
                    // A metadata match alone does not prove that a warm source remains readable.
                    std::fs::File::open(path).map_err(|e| contract::source("", e.into()))?;
                    return run(tree, path);
                }
                let path = match self.source.as_ref().unwrap() {
                    Source::Local(path) => path.clone(),
                    Source::Remote(file) => {
                        figlens::remote::local(file).map_err(|e| contract::source(file, e))?
                    }
                };
                #[cfg(test)]
                LOADS.with(|n| n.set(n.get() + 1));
                let mut doc = Document::open(&path).map_err(|e| contract::source("", e))?;
                let message = doc.take_message()?;
                let tree = Tree::new(&message);
                run(&tree, &path)
            })();
            result.map_err(contract::Failure::from_error)
        }))
        .unwrap_or_else(|_| {
            Err(contract::Failure::new(
                ErrorKind::OperationFailed,
                "figlens crashed on this file",
            ))
        });
        // Keep the caller's output label in replies; it is never used for filesystem work.
        let result = result.map(|mut output| {
            if let Some(label) = self.call.string("out") {
                match &mut output {
                    command::Output::Json { value, .. } | command::Output::Image { value, .. } => {
                        value["path"] = label.into()
                    }
                    _ => {}
                }
            }
            output
        });
        response(&self.id, tool_reply(name, result))
    }
}

fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Keep Document/message/Tree lexical ownership; carry admitted calls, never raw lines.
pub fn serve(
    mut input: impl std::io::BufRead,
    mut output: impl std::io::Write,
    roots: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    let mut next = || next_request(&mut input, roots);
    let mut reply = |value: Value| -> Result<(), Box<dyn Error>> {
        let value = bounded_response(value);
        writeln!(output, "{value}")?;
        output.flush()?;
        Ok(())
    };
    let mut carried = None;
    loop {
        let call = match if let Some(call) = carried.take() {
            Some(Admission::Call(call))
        } else {
            next()?
        } {
            None => return Ok(()),
            Some(Admission::Ignore) => continue,
            Some(Admission::Reply(value)) => {
                reply(value)?;
                continue;
            }
            Some(Admission::Call(call)) => call,
        };
        let Some(path) = call.local() else {
            reply(call.execute(None, roots))?;
            continue;
        };
        let Some(stamp_at_open) = stamp(path) else {
            reply(call.execute(None, roots))?;
            continue;
        };
        if let Err(error) = call.recheck(roots) {
            reply(response(&call.id, tool_reply(call.call.name(), Err(error))))?;
            continue;
        }
        #[cfg(test)]
        LOADS.with(|n| n.set(n.get() + 1));
        let Ok(Ok(mut doc)) = std::panic::catch_unwind(|| Document::open(path)) else {
            reply(call.execute(None, roots))?;
            continue;
        };
        let document = &mut doc;
        let Ok(Ok(message)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let document = document;
            document.take_message()
        })) else {
            reply(call.execute(None, roots))?;
            continue;
        };
        let Ok(tree) = std::panic::catch_unwind(|| Tree::new(&message)) else {
            reply(call.execute(None, roots))?;
            continue;
        };
        reply(call.execute(Some((path, &tree)), roots))?;
        loop {
            match next()? {
                None => return Ok(()),
                Some(Admission::Ignore) => {}
                Some(Admission::Reply(value)) => reply(value)?,
                Some(Admission::Call(next_call)) => match next_call.local() {
                    None => reply(next_call.execute(None, roots))?,
                    Some(next_path) if next_path == path && stamp(path) == Some(stamp_at_open) => {
                        reply(next_call.execute(Some((path, &tree)), roots))?
                    }
                    Some(_) => {
                        carried = Some(next_call);
                        break;
                    }
                },
            }
        }
    }
}

fn tool_reply(name: &str, result: Result<command::Output, contract::Failure>) -> serde_json::Value {
    let mut structured = None;
    let mut image = None;
    let result = result.and_then(|output| {
        let text = output
            .text()
            .map_err(|e| contract::Failure::from_error(Box::new(e)))?;
        if let command::Output::Image { bytes, mime, .. } = &output {
            image = Some(serde_json::json!({
                "type": "image", "data": figlens::render::base64(bytes), "mimeType": mime,
            }));
        }
        if contract::output_schema(name).is_some() {
            structured = output.json().cloned();
        }
        Ok(text)
    });
    let (text, diagnostic) = match result {
        Ok(text) => (text, None),
        Err(error) => (error.text.clone(), Some(error.metadata())),
    };
    let is_error = diagnostic.is_some();
    let mut reply = serde_json::json!({
        "content": [{ "type": "text", "text": text }], "isError": is_error
    });
    if let Some(diagnostic) = diagnostic {
        reply["_meta"] = serde_json::json!({"figlens": diagnostic});
    }
    if let Some(value) = structured {
        reply["structuredContent"] = value;
    }
    if let Some(image) = image {
        reply["content"].as_array_mut().unwrap().push(image);
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn allowed(path: &Path, roots: &[PathBuf]) -> bool {
        checked_path(path, roots).is_ok()
    }
    fn rooted_reply(line: &str, roots: &[PathBuf]) -> Value {
        match admit(line, roots) {
            Admission::Reply(reply) => reply,
            Admission::Call(call) => call.execute(None, roots),
            Admission::Ignore => panic!("expected reply"),
        }
    }

    #[test]
    fn mcp_lists_every_tool() {
        let reply = mcp_reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        let names: Vec<_> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "workflow",
                "tree",
                "find",
                "inspect",
                "nodes",
                "variables",
                "bundle",
                "verify",
                "export"
            ]
        );
    }

    #[test]
    fn mcp_declares_output_schemas_for_json_results() {
        let reply = mcp_reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        let with_schema: Vec<_> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["outputSchema"].is_object())
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            with_schema,
            ["find", "inspect", "bundle", "verify", "export"]
        );
    }

    #[test]
    fn mcp_workflows_work_without_design_files_under_restricted_roots() {
        let dir = test_support::TestDir::new();
        let roots = [dir.join("").canonicalize().unwrap()];
        let mut requests = vec![
            serde_json::json!({"id": 1, "method": "initialize", "params": {}}),
            serde_json::json!({"id": 2, "method": "tools/list"}),
        ];
        for topic in workflow::TOPICS {
            requests.push(
                serde_json::json!({"id": topic, "method": "tools/call", "params": {
                    "name": "workflow", "arguments": {"topic": topic}
                }}),
            );
        }
        let input = requests
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = Vec::new();
        serve(std::io::Cursor::new(input), &mut out, &roots).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("workflow(topic=build)"));
        assert!(instructions.contains("workflow(topic=audit)"));
        assert!(instructions.contains("Simple questions and exports"));
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        let tool = tools.iter().find(|t| t["name"] == "workflow").unwrap();
        assert_eq!(
            tool["inputSchema"]["required"],
            serde_json::json!(["topic"])
        );
        assert_eq!(
            tool["inputSchema"]["properties"]["topic"]["enum"],
            serde_json::json!(workflow::TOPICS)
        );
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .contains("audit for comparing existing UI")
        );
        assert!(workflow::TOPICS.contains(&"audit"));
        for reply in &replies[2..] {
            assert_eq!(reply["result"]["isError"], false, "{reply}");
            let text = reply["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("host's own tools"));
            assert!(!text.contains("](references/") && !text.contains("](planning"));
            assert!(!text.contains("](../SKILL.md"));
            if reply["id"] == "audit" {
                assert!(text.contains("# Audit an existing UI"));
                assert!(text.contains("Static review is not visual acceptance"));
            }
        }
        assert_eq!(std::fs::read_dir(dir.join("")).unwrap().count(), 0);
    }

    #[test]
    fn mcp_workflow_rejects_invalid_arguments_without_cli_fallback() {
        for arguments in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"topic": 1}),
            serde_json::json!({"topic": "../SKILL.md"}),
            serde_json::json!({"topic": "build", "file": "/private/design.fig"}),
        ] {
            let request = serde_json::json!({"id": 1, "method": "tools/call", "params": {
                "name": "workflow", "arguments": arguments
            }});
            assert!(matches!(
                admit(&request.to_string(), &[]),
                Admission::Reply(_)
            ));
            let result = call_tool(&request["params"], None);
            assert_eq!(result["isError"], true, "{result}");
            assert!(
                !result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("usage:")
            );
        }
    }

    #[test]
    fn tree_json_nests_children_down_to_the_depth() {
        use kiwi_schema::Value;
        use std::collections::HashMap;
        let g = |l| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(l))]),
            )
        };
        let node = |l: u32, parent: Option<u32>| {
            let mut fields = HashMap::from([
                ("guid", g(l)),
                ("type", Value::Enum("NodeType", "FRAME")),
                ("name", Value::String(format!("n{l}"))),
            ]);
            if let Some(p) = parent {
                fields.insert(
                    "parentIndex",
                    Value::Object(
                        "ParentIndex",
                        HashMap::from([("guid", g(p)), ("position", Value::String("a".into()))]),
                    ),
                );
            }
            Value::Object("NodeChange", fields)
        };
        let message = Value::Object(
            "Message",
            HashMap::from([(
                "nodeChanges",
                Value::Array(vec![node(0, None), node(1, Some(0)), node(2, Some(1))]),
            )]),
        );
        let tree = Tree::new(&message);
        assert_eq!(
            command::Command::from_cli(
                "tree",
                &["--depth".into(), "1".into(), "--json".into()],
                &tree
            )
            .unwrap()
            .execute(&tree, Path::new("unused.fig"))
            .unwrap()
            .json()
            .unwrap(),
            &serde_json::json!({ "id": "0:0", "type": "FRAME", "name": "n0",
                "children": [{ "id": "0:1", "type": "FRAME", "name": "n1" }] })
        );
    }

    #[test]
    fn mcp_ignores_notifications() {
        assert!(mcp_reply(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn roots_confine_files_and_outputs() {
        let inside = test_support::TestDir::new();
        let outside = test_support::TestDir::new();
        let roots = [std::fs::canonicalize(inside.join("")).unwrap()];
        assert!(allowed(&inside.join("a.fig"), &roots));
        assert!(!allowed(&outside.join("a.fig"), &roots));
        assert!(!allowed(&inside.join("../escape.fig"), &roots));
        assert!(!allowed(Path::new("gs://bucket/a.fig"), &roots));
        assert!(allowed(&outside.join("a.fig"), &[]));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("new.png"), inside.join("link.png")).unwrap();
            assert!(!allowed(&inside.join("link.png"), &roots));
        }
        // Argument failures precede path admission even with roots.
        let malformed = r#"{"id":3,"method":"tools/call","params":{"name":"tree","arguments":{"node":"outside.fig"}}}"#;
        assert!(matches!(admit(malformed, &roots), Admission::Reply(_)));
        assert_eq!(
            mcp_reply(malformed).unwrap()["result"]["_meta"]["figlens"]["code"],
            "invalid_arguments"
        );
        let legacy = serde_json::json!({"id":3,"method":"tools/call","params":{"name":"tree",
            "arguments":{"file":inside.join("a.fig"),"depth":"0"}}})
        .to_string();
        assert_eq!(
            rooted_reply(&legacy, &roots)["result"]["_meta"]["figlens"]["code"],
            "invalid_arguments"
        );

        let call = |file: &Path| {
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"tree","arguments":{{"file":{}}}}}}}"#,
                serde_json::json!(file)
            )
        };
        let input = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{{}}}}\n{}\n{}\n",
            call(&outside.join("a.fig")),
            call(&inside.join("a.fig"))
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &roots).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(&roots[0].display().to_string()),
            "{instructions}"
        );
        let text = replies[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(replies[1]["result"]["isError"], true);
        assert!(text.contains("outside the folders"), "{text}");
        // Inside the root the call runs and fails on the missing file instead.
        let text = replies[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("outside the folders"), "{text}");
    }

    #[test]
    fn serve_answers_every_call_when_the_file_cannot_be_opened() {
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tree","arguments":{"file":"/missing.fig"}}}"#;
        let input =
            format!("{call}\n{call}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n");
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &[]).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(replies[1]["result"]["isError"], true);
        assert_eq!(replies[2]["result"], serde_json::json!({}));
    }

    #[test]
    fn mcp_reports_a_failed_call_as_a_tool_error() {
        let reply = mcp_reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"tree","arguments":{"file":"/missing.fig"}}}"#,
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }
}

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
thread_local! {
    static PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PATH_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub fn mcp_reply(line: &str) -> Option<Value> {
    match admit(line, &[]) {
        Admission::Ignore => None,
        Admission::Reply(reply) => Some(reply),
        Admission::Call(call) => Some(call.execute(None, &[])),
    }
}

#[cfg(test)]
pub fn call_tool(params: &Value, loaded: Option<(&Path, &Tree)>) -> Value {
    let request = json!({"id":1,"method":"tools/call","params":params}).to_string();
    match admit(&request, &[]) {
        Admission::Reply(reply) => reply["result"].clone(),
        Admission::Call(call) => call.execute(loaded, &[])["result"].clone(),
        Admission::Ignore => unreachable!(),
    }
}
