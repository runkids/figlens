//! Stdio request admission, lanes and borrowed local Tree reuse.
//!
//! A reader thread only frames and parses requests and answers protocol methods, so a jobs
//! status or cancel is read while a search or build runs. Tool calls go to bounded lanes:
//! control (jobs, index status), interactive (design tools and search, keeping the warm Tree),
//! index (workspace changes) and build (this server's index jobs, one at a time). One writer on
//! the calling thread applies the reply budget. Path authorization runs in the executing lane.

use std::error::Error;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

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

/// The writer's queue. Each reply is bounded before it waits there, so a stalled stdout holds
/// at most `WRITER_QUEUE` replies within the response budget, never complete oversized ones.
#[derive(Clone)]
struct Writer(mpsc::SyncSender<Value>);

impl Writer {
    fn send(&self, value: Value) {
        let _ = self.0.send(bounded_response(value));
    }
}

/// The next request line, parsed: no path, file or storage is touched here.
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
            Ok(line) => return Ok(Some(frame(line, roots))),
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
    /// An export's folder of SVGs to compare against.
    match_dir: Option<PathBuf>,
    /// The project directory of an index call.
    project: Option<PathBuf>,
    /// The earlier revision a diff reads beside `source`.
    old: Option<PathBuf>,
}

enum Source {
    Local(PathBuf),
    Remote(String),
}

enum Admission {
    Ignore,
    /// `notifications/cancelled` for this request id.
    Cancel(Value),
    Reply(Value),
    Call(AdmittedCall),
    /// A parsed tools/call, admitted later by the lane that runs it.
    Request(Value),
}

/// Parses one line: protocol methods are answered here, tool calls are left for their lane.
fn frame(line: &str, roots: &[PathBuf]) -> Admission {
    let req: Value = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(_) => return Admission::Reply(protocol(Value::Null, -32700, "parse error")),
    };
    if envelope(&req) && req.get("id").is_some() && req["method"] == "tools/call" {
        return Admission::Request(req);
    }
    if envelope(&req) && req.get("id").is_none() && req["method"] == "notifications/cancelled" {
        return Admission::Cancel(req["params"]["requestId"].clone());
    }
    admit_value(req, roots)
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
    PATH_CHECKS.with(Counter::bump);
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

#[cfg(test)]
fn admit(line: &str, roots: &[PathBuf]) -> Admission {
    match serde_json::from_str(line) {
        Ok(req) => admit_value(req, roots),
        Err(_) => Admission::Reply(protocol(Value::Null, -32700, "parse error")),
    }
}

fn admit_value(req: Value, roots: &[PathBuf]) -> Admission {
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
            PREPARATIONS.with(Counter::bump);
            let prepared = contract::prepare(name, &req["params"]["arguments"]);
            let admitted = prepared.and_then(|call| {
                let source = match call.string("file") {
                    Some(file) if file.starts_with("gs://") && call.name() == "diff" => {
                        return Err(contract::Failure::argument(
                            "diff reads two local .fig files; download the gs:// object first",
                        ));
                    }
                    Some(file) if file.starts_with("gs://") && call.name() == "index" => {
                        return Err(contract::Failure::argument(
                            "index registers local .fig files; download the gs:// object first",
                        ));
                    }
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
                let match_dir = call.string("matchDir").map(|path| checked_path(Path::new(path), roots)).transpose()?;
                let project = call.string("workspace").map(|path| checked_path(Path::new(path), roots)).transpose()?;
                let old = match call.string("old") {
                    Some(file) if file.starts_with("gs://") => {
                        return Err(contract::Failure::argument(
                            "diff reads two local .fig files; download the gs:// object first",
                        ));
                    }
                    file => file.map(|path| checked_path(Path::new(path), roots)).transpose()?,
                };
                Ok(AdmittedCall { id:id.clone(), call, source, output, input, match_dir, project, old })
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

    /// The local source a warm tree serves; search reads and hashes its own snapshot instead.
    fn tree_source(&self) -> Option<&Path> {
        self.local().filter(|_| {
            !matches!(
                self.call.name(),
                "search" | "usages" | "diff" | "index" | "jobs"
            )
        })
    }

    /// Whether this call reads or changes a workspace rather than a design file.
    fn workspace(&self) -> bool {
        matches!(self.call.name(), "index" | "jobs")
            || (matches!(self.call.name(), "search" | "usages")
                && self.call.string("workspaceId").is_some())
    }

    /// Runs a workspace call; the jobs an index call submitted for this process come back
    /// for the build lane.
    fn run_workspace(
        &self,
        roots: &[PathBuf],
        storage_roots: &[PathBuf],
    ) -> (Value, Option<command::Runs>) {
        let mut runs = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.recheck(roots)?;
            match self.call.name() {
                "jobs" => command::jobs_tool(&self.call, roots, storage_roots),
                "search" => command::workspace_search_tool(&self.call, roots, storage_roots),
                "usages" => command::workspace_usages_tool(&self.call, roots, storage_roots),
                _ => command::index_tool(
                    &self.call,
                    self.local(),
                    self.project.as_deref(),
                    roots,
                    storage_roots,
                )
                .map(|(output, submitted)| {
                    runs = submitted;
                    output
                }),
            }
            .map_err(contract::Failure::from_error)
        }))
        .unwrap_or_else(|_| {
            Err(contract::Failure::new(
                ErrorKind::OperationFailed,
                "figlens crashed on this workspace call",
            ))
        });
        (
            response(&self.id, tool_reply(self.call.name(), result)),
            runs,
        )
    }

    fn recheck(&self, roots: &[PathBuf]) -> Result<(), contract::Failure> {
        for path in self
            .local()
            .into_iter()
            .chain(self.output.as_deref())
            .chain(self.input.as_deref())
            .chain(self.match_dir.as_deref())
            .chain(self.project.as_deref())
            .chain(self.old.as_deref())
        {
            // Recheck current filesystem state, not raw arguments or a permanent authorization lease.
            checked_path(path, roots)?;
        }
        Ok(())
    }

    fn execute(
        &self,
        loaded: Option<(&Path, &Tree)>,
        roots: &[PathBuf],
        storage_roots: &[PathBuf],
    ) -> Value {
        if self.workspace() {
            return self.run_workspace(roots, storage_roots).0;
        }
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
            if name == "diff" {
                let (old, new) = (self.old.as_deref().unwrap(), self.local().unwrap());
                return command::diff_tool(&self.call)
                    .and_then(|request| figlens::search::diff::diff(old, new, &request))
                    .map(|value| command::Output::Json {
                        value,
                        pretty: false,
                    })
                    .map_err(contract::Failure::from_error);
            }
            if matches!(name, "search" | "usages") {
                let path = match self.source.as_ref().unwrap() {
                    Source::Local(path) => path.clone(),
                    Source::Remote(file) => figlens::remote::local(file)
                        .map_err(|e| contract::Failure::from_error(contract::source(file, e)))?,
                };
                if name == "usages" {
                    return command::usages_tool(&self.call)
                        .and_then(|request| command::usages(&path, &request, ""))
                        .map_err(contract::Failure::from_error);
                }
                return command::search_tool(&self.call)
                    .and_then(|request| command::search(&path, &request, ""))
                    .map_err(contract::Failure::from_error);
            }
            let run = |tree: &Tree, path: &Path| {
                command::Command::from_tool(
                    &self.call,
                    tree,
                    self.output.as_deref(),
                    self.match_dir.as_deref(),
                )?
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
                LOADS.with(Counter::bump);
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

/// Requests a lane may queue before the reader answers busy instead.
const CONTROL_QUEUE: usize = 64;
const INTERACTIVE_QUEUE: usize = 64;
const INDEX_QUEUE: usize = 8;
/// Replies waiting for stdout before every sender, the reader included, waits.
const WRITER_QUEUE: usize = 64;
/// Build submissions this server may hold before a build-submitting call is answered busy.
const BUILD_QUEUE: usize = 8;

/// Tool calls queued in a lane, by request id; `true` once the client cancelled one before it
/// ran. Bounded by the lane queues, since only queued ids are kept.
type Pending = std::sync::Mutex<std::collections::HashMap<String, bool>>;

/// Takes a call off the pending list as its lane starts it; false when it was cancelled.
fn live(pending: &Pending, req: &Value) -> bool {
    pending.lock().unwrap().remove(&req["id"].to_string()) != Some(true)
}

/// Whether an index call submits jobs to the build lane.
fn submits(req: &Value) -> bool {
    matches!(
        req["params"]["arguments"]["action"].as_str(),
        Some("prepare" | "refresh" | "rebuild")
    )
}

enum Lane {
    Control,
    Interactive,
    Index,
}

/// The lane that runs a tool call, chosen from its name and action alone.
fn lane(req: &Value) -> Lane {
    match (
        req["params"]["name"].as_str(),
        req["params"]["arguments"]["action"].as_str(),
    ) {
        (Some("jobs"), _) | (Some("index"), Some("status")) => Lane::Control,
        (Some("index"), _) => Lane::Index,
        _ => Lane::Interactive,
    }
}

fn busy(id: &Value) -> Value {
    response(
        id,
        tool_reply(
            "",
            Err(contract::Failure::new(
                ErrorKind::WorkspaceBusy,
                "figlens is busy with earlier calls of this kind; retry after a reply arrives",
            )),
        ),
    )
}

/// Replies listed in the order of `ids`: protocol replies answer at once, so they can overtake
/// earlier tool calls; JSON-RPC pairs them by id.
#[cfg(test)]
pub(crate) fn in_request_order(replies: &[Value], ids: &[Value]) -> Vec<Value> {
    ids.iter()
        .map(|id| replies.iter().find(|r| &r["id"] == id).cloned().unwrap())
        .collect()
}

/// Serves MCP on `input`/`output` with design roots `roots` and no custom storage roots.
#[cfg(test)]
pub fn serve(
    input: impl BufRead + Send,
    output: impl std::io::Write,
    roots: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    serve_with(input, output, roots, &[])
}

/// Serves MCP until `input` ends. `storage_roots` allow custom workspace storage besides the
/// default roots. At the end of input, this server's running build is cancelled; when a reply
/// cannot be written, its builds are cancelled and the process exits 1.
pub fn serve_with(
    mut input: impl BufRead + Send,
    mut output: impl std::io::Write,
    roots: &[PathBuf],
    storage_roots: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    // Bounded: when stdout is slow, replies wait here and then the reader stops reading input,
    // so memory stays at this many capped replies.
    let (sender, replies) = mpsc::sync_channel::<Value>(WRITER_QUEUE);
    let to_writer = Writer(sender);
    let stop = AtomicBool::new(false);
    // Set when a reply cannot be written; no lane admits a build after that.
    let dead = AtomicBool::new(false);
    let pending = Pending::default();
    let builds = std::sync::atomic::AtomicUsize::new(0);
    #[cfg(test)]
    let counters = Counter::share();
    std::thread::scope(|scope| -> Result<(), Box<dyn Error>> {
        let (control, control_rx) = mpsc::sync_channel::<Value>(CONTROL_QUEUE);
        let (interactive, interactive_rx) = mpsc::sync_channel::<Admission>(INTERACTIVE_QUEUE);
        let (index, index_rx) = mpsc::sync_channel::<Value>(INDEX_QUEUE);
        let (to_build, build_rx) = mpsc::channel::<command::Runs>();
        let (stop, dead, pending, builds) = (&stop, &dead, &pending, &builds);
        // Control and index lanes: each call admitted and run in arrival order.
        for (rx, build) in [(control_rx, None), (index_rx, Some(to_build))] {
            let out = to_writer.clone();
            #[cfg(test)]
            let counters = counters.clone();
            scope.spawn(move || {
                #[cfg(test)]
                Counter::adopt(counters);
                for req in rx {
                    if !live(pending, &req) {
                        continue;
                    }
                    // Counted before the call creates its job, so a writer that fails meanwhile
                    // waits for it instead of exiting with the job unowned.
                    // With `dead` checked after the count (both SeqCst), either the writer sees
                    // this build or this lane sees the writer died.
                    let counted = build.is_some() && submits(&req);
                    if counted {
                        let full = builds.fetch_add(1, Ordering::SeqCst) >= BUILD_QUEUE;
                        if full || dead.load(Ordering::SeqCst) {
                            builds.fetch_sub(1, Ordering::SeqCst);
                            out.send(busy(&req["id"]));
                            continue;
                        }
                    }
                    let mut queued = false;
                    match admit_value(req, roots) {
                        Admission::Call(call) => {
                            let (reply, runs) = call.run_workspace(roots, storage_roots);
                            if let (Some(runs), Some(build)) = (runs, &build) {
                                if !counted {
                                    builds.fetch_add(1, Ordering::SeqCst);
                                }
                                queued = build.send(runs).is_ok();
                            }
                            out.send(reply);
                        }
                        Admission::Reply(reply) => {
                            out.send(reply);
                        }
                        _ => {}
                    }
                    if counted && !queued {
                        builds.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            });
        }
        // Build lane: this server's jobs, one at a time; they stop when input ends or a reply
        // cannot be written.
        scope.spawn(move || {
            for runs in build_rx {
                for job in &runs.jobs {
                    let _ = figlens::workspace::jobs::run(
                        &runs.storage,
                        &runs.workspace_id,
                        job,
                        stop,
                        &mut |_| {},
                    );
                }
                builds.fetch_sub(1, Ordering::SeqCst);
            }
        });
        // Interactive lane: the warm Tree stays on this thread.
        let out = to_writer.clone();
        #[cfg(test)]
        let lane_counters = counters.clone();
        scope.spawn(move || {
            #[cfg(test)]
            Counter::adopt(lane_counters);
            let next = || {
                interactive_rx.recv().ok().map(|a| match a {
                    Admission::Request(req) if !live(pending, &req) => Admission::Ignore,
                    Admission::Request(req) => admit_value(req, roots),
                    other => other,
                })
            };
            interactive_loop(
                next,
                |value| {
                    out.send(value);
                },
                roots,
                storage_roots,
            );
        });
        // Reader: framing and protocol only.
        let reader = to_writer.clone();
        #[cfg(test)]
        let reader_counters = counters.clone();
        scope.spawn(move || {
            #[cfg(test)]
            Counter::adopt(reader_counters);
            let lanes = [control, index];
            loop {
                let request = match next_request(&mut input, roots) {
                    Ok(Some(request)) => request,
                    Ok(None) => break,
                    Err(e) => {
                        reader.send(protocol(Value::Null, -32700, &format!("input failed: {e}")));
                        break;
                    }
                };
                match request {
                    // Protocol replies never wait behind a lane, so ping stays responsive.
                    Admission::Reply(reply) => {
                        reader.send(reply);
                    }
                    Admission::Cancel(id) => {
                        if let Some(cancelled) = pending.lock().unwrap().get_mut(&id.to_string()) {
                            *cancelled = true;
                        }
                    }
                    Admission::Request(req) => {
                        let id = req["id"].clone();
                        pending.lock().unwrap().insert(id.to_string(), false);
                        let full = match lane(&req) {
                            Lane::Interactive => interactive
                                .try_send(Admission::Request(req))
                                .is_err_and(|e| matches!(e, mpsc::TrySendError::Full(_))),
                            Lane::Control => lanes[0]
                                .try_send(req)
                                .is_err_and(|e| matches!(e, mpsc::TrySendError::Full(_))),
                            Lane::Index => lanes[1]
                                .try_send(req)
                                .is_err_and(|e| matches!(e, mpsc::TrySendError::Full(_))),
                        };
                        if full {
                            pending.lock().unwrap().remove(&id.to_string());
                            reader.send(busy(&id));
                        }
                    }
                    _ => {}
                }
            }
            stop.store(true, Ordering::Relaxed);
        });
        drop(to_writer);
        // Writer: every reply, already bounded, in completion order.
        for value in replies.iter() {
            if let Err(e) = writeln!(output, "{value}").and_then(|_| output.flush()) {
                // The client stopped reading, and the reader may wait on an open stdin forever:
                // stop this server's builds, wait until they cancelled their workers, then end.
                dead.store(true, Ordering::SeqCst);
                stop.store(true, Ordering::Relaxed);
                // Replies still sent meanwhile are dropped, so no lane waits on this queue.
                let tick = std::time::Duration::from_millis(20);
                while builds.load(Ordering::SeqCst) > 0 {
                    if let Err(mpsc::RecvTimeoutError::Disconnected) = replies.recv_timeout(tick) {
                        std::thread::sleep(tick);
                    }
                }
                // Not eprintln!: a closed stderr would panic and leave the reader's join hanging.
                use std::io::Write as _;
                let _ = writeln!(
                    std::io::stderr(),
                    "figlens mcp: cannot write a reply ({e}); stopped this server's builds"
                );
                std::process::exit(1);
            }
        }
        Ok(())
    })
}

/// Keep Document/message/Tree lexical ownership; carry admitted calls, never raw lines.
fn interactive_loop(
    mut next: impl FnMut() -> Option<Admission>,
    mut reply: impl FnMut(Value),
    roots: &[PathBuf],
    storage_roots: &[PathBuf],
) {
    let mut carried = None;
    loop {
        let call = match if let Some(call) = carried.take() {
            Some(Admission::Call(call))
        } else {
            next()
        } {
            None => return,
            Some(Admission::Call(call)) => call,
            Some(Admission::Reply(value)) => {
                reply(value);
                continue;
            }
            Some(_) => continue,
        };
        let Some(path) = call.tree_source() else {
            reply(call.execute(None, roots, storage_roots));
            continue;
        };
        let Some(stamp_at_open) = stamp(path) else {
            reply(call.execute(None, roots, storage_roots));
            continue;
        };
        if let Err(error) = call.recheck(roots) {
            reply(response(&call.id, tool_reply(call.call.name(), Err(error))));
            continue;
        }
        #[cfg(test)]
        LOADS.with(Counter::bump);
        let Ok(Ok(mut doc)) = std::panic::catch_unwind(|| Document::open(path)) else {
            reply(call.execute(None, roots, storage_roots));
            continue;
        };
        let document = &mut doc;
        let Ok(Ok(message)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let document = document;
            document.take_message()
        })) else {
            reply(call.execute(None, roots, storage_roots));
            continue;
        };
        let Ok(tree) = std::panic::catch_unwind(|| Tree::new(&message)) else {
            reply(call.execute(None, roots, storage_roots));
            continue;
        };
        reply(call.execute(Some((path, &tree)), roots, storage_roots));
        loop {
            match next() {
                None => return,
                Some(Admission::Reply(value)) => reply(value),
                Some(Admission::Call(next_call)) => match next_call.tree_source() {
                    None => reply(next_call.execute(None, roots, storage_roots)),
                    Some(next_path) if next_path == path && stamp(path) == Some(stamp_at_open) => {
                        reply(next_call.execute(Some((path, &tree)), roots, storage_roots))
                    }
                    Some(_) => {
                        carried = Some(next_call);
                        break;
                    }
                },
                Some(_) => {}
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
            Admission::Call(call) => call.execute(None, roots, &[]),
            _ => panic!("expected reply"),
        }
    }

    #[test]
    fn an_oversized_reply_is_bounded_before_it_waits_for_the_writer() {
        let (sender, replies) = mpsc::sync_channel(1);
        let text = "x".repeat(9 << 20);
        Writer(sender).send(serde_json::json!({"jsonrpc": "2.0", "id": 1,
            "result": {"content": [{"type": "text", "text": text}], "isError": false}}));
        assert!(replies.recv().unwrap().to_string().len() < 1 << 20);
    }

    #[test]
    fn index_and_jobs_declare_that_they_change_state() {
        let tools = contract::tools();
        let hints = |name: &str| {
            tools["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap()["annotations"]
                .clone()
        };
        assert_eq!(
            hints("index"),
            serde_json::json!({"readOnlyHint": false, "destructiveHint": true,
                "idempotentHint": false, "openWorldHint": false})
        );
        assert_eq!(
            hints("jobs"),
            serde_json::json!({"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false})
        );
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
                "search",
                "usages",
                "diff",
                "inspect",
                "nodes",
                "variables",
                "bundle",
                "verify",
                "export",
                "index",
                "jobs"
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
            [
                "find", "search", "usages", "diff", "inspect", "bundle", "verify", "export",
                "index", "jobs"
            ]
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
    fn tree_prints_the_rest_types_nodes_uses() {
        let message = test_support::component_set();
        let tree = Tree::new(&message);
        let run = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            command::Command::from_cli("tree", &args, &tree)
                .unwrap()
                .execute(&tree, Path::new("unused.fig"))
                .unwrap()
        };
        assert_eq!(
            run(&["--depth", "2", "--json"]).json().unwrap(),
            &serde_json::json!({ "id": "0:0", "type": "CANVAS", "name": "n0",
                "children": [{ "id": "0:1", "type": "COMPONENT_SET", "name": "n1",
                    "layoutMode": "HORIZONTAL",
                    "children": [{ "id": "0:2", "type": "COMPONENT", "name": "n2", "visible": false,
                        "size": [24.0, 16.0] }] }] })
        );
        let text = run(&["--depth", "2"]).text().unwrap();
        assert!(text.contains("0:1 COMPONENT_SET \"n1\"") && text.contains("0:2 COMPONENT \"n2\""));
    }

    #[test]
    fn tree_json_gives_top_level_nodes_their_first_visible_text() {
        use kiwi_schema::Value;
        use std::collections::HashMap;
        let g = |l| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(l))]),
            )
        };
        let node = |l: u32, parent: u32, kind, text: Option<&str>, visible: bool| {
            let mut fields = HashMap::from([
                ("guid", g(l)),
                ("type", Value::Enum("NodeType", kind)),
                ("name", Value::String(format!("n{l}"))),
                ("visible", Value::Bool(visible)),
                (
                    "parentIndex",
                    Value::Object(
                        "ParentIndex",
                        HashMap::from([
                            ("guid", g(parent)),
                            ("position", Value::String(format!("{l:02}"))),
                        ]),
                    ),
                ),
            ]);
            if let Some(text) = text {
                let chars = HashMap::from([("characters", Value::String(text.into()))]);
                fields.insert("textData", Value::Object("TextData", chars));
            }
            Value::Object("NodeChange", fields)
        };
        let message = Value::Object(
            "Message",
            HashMap::from([(
                "nodeChanges",
                Value::Array(vec![
                    node(1, 0, "CANVAS", None, true),
                    // A canvas label: a box around one line of text.
                    node(2, 1, "FRAME", None, true),
                    node(3, 2, "TEXT", Some("Onboarding"), true),
                    // A screen whose hidden text is skipped for the next visible one, deeper down.
                    node(4, 1, "FRAME", None, true),
                    node(5, 4, "TEXT", Some("hidden"), false),
                    node(6, 4, "FRAME", None, true),
                    node(7, 6, "TEXT", Some(&"x".repeat(250)), true),
                    node(8, 1, "FRAME", None, true),
                    // Inside a section, blank text is passed over for the next visible text.
                    node(9, 1, "SECTION", None, true),
                    node(10, 9, "FRAME", None, true),
                    node(11, 10, "TEXT", Some(" \n"), true),
                    node(12, 10, "TEXT", Some("Checkout"), true),
                ]),
            )]),
        );
        let tree = Tree::new(&message);
        let run = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            command::Command::from_cli("tree", &args, &tree)
                .unwrap()
                .execute(&tree, Path::new("unused.fig"))
                .unwrap()
                .json()
                .unwrap()
                .clone()
        };
        let page = run(&["0:1", "--depth", "2", "--json"]);
        let first = |v: &serde_json::Value| v.get("firstText").cloned();
        assert_eq!(first(&page["children"][0]), Some("Onboarding".into()));
        assert_eq!(first(&page["children"][1]), Some("x".repeat(200).into()));
        // Only children of a page or section carry it; a node without text has none.
        assert_eq!(first(&page["children"][0]["children"][0]), None);
        assert_eq!(first(&page["children"][2]), None);
        let section = run(&["0:9", "--depth", "1", "--json"]);
        assert_eq!(first(&section["children"][0]), Some("Checkout".into()));
        // Scoped to the frame itself, it keeps the text it has in the page listing.
        assert_eq!(first(&run(&["0:2", "--json"])), Some("Onboarding".into()));
        assert_eq!(first(&run(&["0:3", "--json"])), None);
        let frames = run(&["0:1", "--top-frames", "--json"]);
        assert_eq!(first(&frames[0]), Some("Onboarding".into()));
    }

    #[test]
    fn tree_top_frames_lists_screens_in_pages_and_sections_for_frame_lists() {
        use kiwi_schema::Value;
        use std::collections::HashMap;
        let g = |l| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(l))]),
            )
        };
        let node = |l: u32, parent: Option<u32>, kind, extra: Option<&'static str>| {
            let mut fields = HashMap::from([
                ("guid", g(l)),
                ("type", Value::Enum("NodeType", kind)),
                ("name", Value::String(format!("n{l}"))),
            ]);
            if let Some(p) = parent {
                let position = Value::String(format!("{l:02}"));
                fields.insert(
                    "parentIndex",
                    Value::Object(
                        "ParentIndex",
                        HashMap::from([("guid", g(p)), ("position", position)]),
                    ),
                );
            }
            match extra {
                Some("newline") => {
                    fields.insert("name", Value::String("a\n0:9 b".into()));
                }
                Some(flag) => {
                    fields.insert(flag, Value::Bool(true));
                }
                None => {}
            }
            Value::Object("NodeChange", fields)
        };
        let message = Value::Object(
            "Message",
            HashMap::from([(
                "nodeChanges",
                Value::Array(vec![
                    node(0, None, "DOCUMENT", None),
                    node(1, Some(0), "CANVAS", None),
                    node(2, Some(1), "FRAME", None),
                    node(3, Some(1), "SECTION", None),
                    node(4, Some(3), "SYMBOL", None),
                    node(5, Some(3), "SECTION", None),
                    node(6, Some(5), "INSTANCE", None),
                    node(7, Some(1), "FRAME", Some("resizeToFit")),
                    node(8, Some(7), "FRAME", None),
                    node(9, Some(1), "TEXT", None),
                    node(10, Some(2), "FRAME", None),
                    node(11, Some(0), "CANVAS", Some("internalOnly")),
                    node(12, Some(11), "FRAME", None),
                    node(13, Some(3), "FRAME", Some("newline")),
                    node(14, Some(99), "SECTION", None),
                    node(15, Some(14), "FRAME", None),
                ]),
            )]),
        );
        let tree = Tree::new(&message);
        let run = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            command::Command::from_cli("tree", &args, &tree)
                .unwrap()
                .execute(&tree, Path::new("unused.fig"))
                .unwrap()
        };
        let text = run(&["--top-frames", "--depth", "0"]).text().unwrap();
        assert_eq!(
            text,
            "0:2 FRAME \"n2\"\n0:4 COMPONENT \"n4\"\n0:6 INSTANCE \"n6\"\n0:13 FRAME \"a\\n0:9 b\"\n"
        );
        // An escaped newline in a name does not add an id line.
        assert_eq!(crate::frame_ids(&text), ["0:2", "0:4", "0:6", "0:13"]);
        assert_eq!(
            run(&["--top-frames", "--json"]).json().unwrap(),
            &serde_json::json!([
                { "id": "0:2", "type": "FRAME", "name": "n2", "page": "0:1" },
                { "id": "0:4", "type": "COMPONENT", "name": "n4", "page": "0:1", "section": "0:3" },
                { "id": "0:6", "type": "INSTANCE", "name": "n6", "page": "0:1", "section": "0:5" },
                { "id": "0:13", "type": "FRAME", "name": "a\n0:9 b", "page": "0:1", "section": "0:3" },
            ])
        );
        // A section under a parent the file lacks lists its screens without a page.
        assert_eq!(
            run(&["0:14", "--top-frames", "--json"]).json().unwrap(),
            &serde_json::json!([{ "id": "0:15", "type": "FRAME", "name": "n15", "section": "0:14" }])
        );
        // A section root keeps its page; a frame root is not a page or section and lists nothing.
        let section = run(&["0:3", "--top-frames", "--json"])
            .json()
            .unwrap()
            .clone();
        assert_eq!(
            (&section[0]["page"], &section[0]["section"]),
            (&"0:1".into(), &"0:3".into())
        );
        assert_eq!(run(&["0:2", "--top-frames"]).text().unwrap(), "");
        // An internal page lists its screens when it is the root.
        assert_eq!(
            run(&["0:11", "--top-frames"]).text().unwrap(),
            "0:12 FRAME \"n12\"\n"
        );
        let file = std::fs::canonicalize("examples/first-success/design.fig").unwrap();
        let reply = call_tool(
            &serde_json::json!({"name":"tree","arguments":{"file":file,"topFrames":true,"json":true}}),
            None,
        );
        let frames: serde_json::Value =
            serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(frames[0]["id"], "0:1", "{reply}");
    }

    #[test]
    fn mcp_diff_checks_both_paths_and_pages_the_cli_reply() {
        let inside = test_support::TestDir::new();
        let outside = test_support::TestDir::new();
        let roots = [std::fs::canonicalize(inside.join("")).unwrap()];
        for dir in [&inside, &outside] {
            std::fs::copy("examples/first-success/design.fig", dir.join("a.fig")).unwrap();
        }
        let call = |old: &Path, new: &Path| {
            let line = json!({"id":1,"method":"tools/call","params":{"name":"diff","arguments":{"old":old,"file":new}}});
            rooted_reply(&line.to_string(), &roots)["result"].clone()
        };
        let (a, out) = (inside.join("a.fig"), outside.join("a.fig"));
        let same = call(&a, &a);
        let value = &same["structuredContent"];
        assert_eq!(
            (&value["frames"], &value["coverage"]["complete"]),
            (&json!([]), &json!(true)),
            "{same}"
        );
        assert_eq!(
            value["matching"],
            json!({"basis": "node_id+kind", "lineage": "unverified"})
        );
        // The old file is admitted like `file`: outside the roots it is refused before any read.
        for (old, new) in [(&out, &a), (&a, &out)] {
            assert_eq!(
                call(old, new)["_meta"]["figlens"]["code"],
                "permission_denied"
            );
        }
        let gs = json!({"id":1,"method":"tools/call","params":{"name":"diff","arguments":{"old":"gs://b/o.fig","file":a}}});
        assert_eq!(
            rooted_reply(&gs.to_string(), &[])["result"]["_meta"]["figlens"]["code"],
            "invalid_arguments"
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
        let calls: Vec<_> = replies.iter().filter(|r| r["id"] == 1).collect();
        assert!(calls.len() == 2 && calls.iter().all(|r| r["result"]["isError"] == true));
        assert!(
            replies
                .iter()
                .any(|r| r["id"] == 2 && r["result"] == serde_json::json!({}))
        );
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

/// A test counter shared by a serve call's lanes and the thread that started it.
#[cfg(test)]
#[derive(Default)]
struct Counter(std::cell::RefCell<std::sync::Arc<std::sync::atomic::AtomicUsize>>);

#[cfg(test)]
type Shared = [std::sync::Arc<std::sync::atomic::AtomicUsize>; 3];

#[cfg(test)]
impl Counter {
    fn get(&self) -> usize {
        self.0.borrow().load(Ordering::SeqCst)
    }
    fn set(&self, n: usize) {
        self.0.borrow().store(n, Ordering::SeqCst)
    }
    fn bump(&self) {
        self.0.borrow().fetch_add(1, Ordering::SeqCst);
    }
    fn share() -> Shared {
        [&PREPARATIONS, &PATH_CHECKS, &LOADS].map(|c| c.with(|c| c.0.borrow().clone()))
    }
    fn adopt(shared: Shared) {
        for (c, s) in [&PREPARATIONS, &PATH_CHECKS, &LOADS]
            .into_iter()
            .zip(shared)
        {
            c.with(|c| *c.0.borrow_mut() = s);
        }
    }
}

#[cfg(test)]
thread_local! {
    static PREPARATIONS: Counter = Counter::default();
    static PATH_CHECKS: Counter = Counter::default();
    static LOADS: Counter = Counter::default();
}

#[cfg(test)]
pub fn mcp_reply(line: &str) -> Option<Value> {
    match admit(line, &[]) {
        Admission::Reply(reply) => Some(reply),
        Admission::Call(call) => Some(call.execute(None, &[], &[])),
        _ => None,
    }
}

#[cfg(test)]
pub fn call_tool(params: &Value, loaded: Option<(&Path, &Tree)>) -> Value {
    let request = json!({"id":1,"method":"tools/call","params":params}).to_string();
    match admit(&request, &[]) {
        Admission::Reply(reply) => reply["result"].clone(),
        Admission::Call(call) => call.execute(loaded, &[], &[])["result"].clone(),
        _ => unreachable!(),
    }
}
