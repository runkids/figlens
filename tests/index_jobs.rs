//! Index jobs across real processes: the CLI and MCP server as users run them, the worker
//! child, cancellation, owner death, single-flight, authorization and publication fences.
//! `FIGLENS_TEST_HOLD=<stage>:<file>` (debug builds only) keeps a worker in a stage.

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use test_support::TestDir;

const BIN: &str = env!("CARGO_BIN_EXE_figlens");

struct Env {
    dir: TestDir,
    home: PathBuf,
}

impl Env {
    fn new() -> Self {
        let dir = TestDir::new();
        let home = std::fs::canonicalize(dir.join("")).unwrap().join("home");
        std::fs::create_dir(&home).unwrap();
        Self { dir, home }
    }

    fn fig(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::copy("examples/first-success/design.fig", &path).unwrap();
        std::fs::canonicalize(path).unwrap()
    }

    fn command(&self, args: &[&str], hold: Option<(&str, &Path)>) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env("HOME", &self.home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("FIGLENS_TEST_HOLD");
        if let Some((stage, file)) = hold {
            command.env("FIGLENS_TEST_HOLD", format!("{stage}:{}", file.display()));
        }
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args, None).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn spawn(&self, args: &[&str], hold: Option<(&str, &Path)>) -> Child {
        self.command(args, hold)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// A file whose existence holds a worker in its stage.
    fn gate(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, "").unwrap();
        path
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The active job of the only workspace, read through the CLI.
fn active_job(env: &Env, file: &Path, stage: &str) -> String {
    let mut found = String::new();
    wait_for("the job to reach its stage", || {
        let status = env.json(&["index", "status", "--file", file.to_str().unwrap()]);
        let ws = status["workspaceId"].as_str().unwrap().to_owned();
        let db = PathBuf::from(status["paths"]["database"].as_str().unwrap());
        let conn = rusqlite::Connection::open_with_flags(
            db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT job_id, stage FROM jobs WHERE state IN ('queued', 'running')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        match row {
            Some((job, Some(at))) if at == stage && job.starts_with(&ws) => {
                found = job;
                true
            }
            _ => false,
        }
    });
    found
}

fn worker_running(job: &str) -> bool {
    let out = Command::new("ps").args(["-eo", "stat=,args="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.contains("__index-worker") && l.contains(job) && !l.trim_start().starts_with('Z'))
}

fn ids(reply: &Value) -> Vec<&str> {
    reply["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"]["nodeId"].as_str().unwrap())
        .collect()
}

#[test]
fn a_prepared_scope_searches_like_the_stateless_search_and_survives_a_rebuild() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let before = env.json(&["search", "--file", file, "first"]);
    assert_eq!(before["state"], "not_prepared");
    let prepared = env.json(&["index", "prepare", file, "--scope", "0:0"]);
    assert_eq!(prepared["job"]["state"], "succeeded");
    let stateless = env.json(&["search", file, "first"]);
    let indexed = env.json(&["search", "--file", file, "first"]);
    assert_eq!(indexed["state"], "ready");
    assert!(!ids(&stateless).is_empty());
    assert_eq!(ids(&indexed), ids(&stateless));
    assert_eq!(indexed["coverage"]["complete"], true);
    let rebuilt = env.json(&["index", "rebuild", "--file", file]);
    assert_eq!(rebuilt["jobs"][0]["state"], "succeeded");
    assert_ne!(rebuilt["jobs"][0]["generation"], prepared["job"]["generation"]);
    assert_eq!(ids(&env.json(&["search", "--file", file, "first"])), ids(&stateless));
}

#[test]
fn a_cursor_over_a_replaced_generation_is_stale() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let page = env.json(&["search", "--file", file, "first", "--limit", "1"]);
    let cursor = page["nextCursor"].as_str().expect("a second page").to_owned();
    env.json(&["index", "rebuild", "--file", file]);
    let out = env.run(&["search", "--file", file, "first", "--limit", "1", "--cursor", &cursor]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("search again without cursor"));
}

#[test]
fn cancel_stops_the_worker_before_reporting_and_keeps_the_previous_index() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let expected = ids(&env.json(&["search", "--file", file, "first"]))
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for stage in ["hash", "parse", "write", "verify"] {
        let gate = env.gate(&format!("{stage}.hold"));
        let mut owner = env.spawn(&["index", "rebuild", "--file", file], Some((stage, &gate)));
        let job = active_job(&env, &fig, stage);
        assert!(worker_running(&job));
        let start = Instant::now();
        let reply = env.json(&["jobs", "cancel", &job]);
        assert_eq!(reply["cancel"], "accepted", "{stage}: {reply}");
        assert_eq!(reply["state"], "cancelled");
        // Reported only after the worker exited.
        assert!(!worker_running(&job), "{stage}: worker outlived the cancel");
        assert!(start.elapsed() < Duration::from_millis(2500));
        assert!(!owner.wait().unwrap().success());
        std::fs::remove_file(&gate).unwrap();
        let search = env.json(&["search", "--file", file, "first"]);
        assert_eq!(search["state"], "ready", "{stage}");
        assert_eq!(ids(&search), expected, "{stage}");
    }
}

#[test]
fn a_dead_owner_leaves_an_interrupted_job_whose_worker_exits_and_a_refresh_retries_it() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let gate = env.gate("parse.hold");
    let mut owner = env.spawn(&["index", "prepare", file, "--scope", "0:0"], Some(("parse", &gate)));
    let job = active_job(&env, &fig, "parse");
    owner.kill().unwrap();
    owner.wait().unwrap();
    // The worker sees its stdin close and exits although its stage is still held.
    wait_for("the orphaned worker to exit", || !worker_running(&job));
    assert_eq!(env.json(&["jobs", "status", &job])["state"], "interrupted");
    std::fs::remove_file(&gate).unwrap();
    let retried = env.json(&["index", "refresh", "--file", file, "--scope", "0:0"]);
    assert_eq!(retried["jobs"][0]["state"], "succeeded");
    assert_eq!(env.json(&["search", "--file", file, "first"])["state"], "ready");
}

#[test]
fn concurrent_prepares_of_one_scope_share_one_job() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let gate = env.gate("parse.hold");
    let first = env.spawn(&["index", "prepare", file, "--scope", "0:0"], Some(("parse", &gate)));
    let job = active_job(&env, &fig, "parse");
    let second = env.spawn(&["index", "prepare", file, "--scope", "0:0"], None);
    std::thread::sleep(Duration::from_millis(300));
    std::fs::remove_file(&gate).unwrap();
    let replies: Vec<Value> = [first, second]
        .into_iter()
        .map(|c| {
            let out = c.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            serde_json::from_slice(&out.stdout).unwrap()
        })
        .collect();
    for reply in &replies {
        assert_eq!(reply["job"]["jobId"], job.as_str());
        assert_eq!(reply["job"]["state"], "succeeded");
    }
}

#[test]
fn a_source_removed_and_added_during_a_build_is_not_published_by_that_build() {
    let env = Env::new();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let project = std::fs::canonicalize(project).unwrap();
    let fig = env.fig("a.fig");
    let (file, dir) = (fig.to_str().unwrap(), project.to_str().unwrap());
    let prepared = env.json(&["index", "prepare", file, "--workspace", dir]);
    let source = prepared["source"]["sourceId"].as_str().unwrap().to_owned();
    let gate = env.gate("write.hold");
    let owner = env.spawn(
        &["index", "prepare", file, "--workspace", dir, "--scope", "0:0"],
        Some(("write", &gate)),
    );
    let mut job = String::new();
    wait_for("the build to reach write", || {
        let status = env.json(&["index", "status", "--workspace", dir]);
        let db = status["paths"]["database"].as_str().unwrap().to_owned();
        let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        conn.query_row(
            "SELECT job_id FROM jobs WHERE stage = 'write' AND state = 'running'",
            [],
            |r| r.get::<_, String>(0),
        )
        .map(|j| job = j)
        .is_ok()
    });
    env.json(&["index", "remove", &source, "--workspace", dir]);
    let added = env.json(&["index", "add", file, "--workspace", dir]);
    assert_eq!(added["source"]["sourceId"], source.as_str());
    std::fs::remove_file(&gate).unwrap();
    let _ = owner.wait_with_output().unwrap();
    assert_ne!(env.json(&["jobs", "status", &job])["state"], "succeeded");
    let search = env.json(&["search", "--workspace", dir, "first"]);
    assert_eq!(search["state"], "not_prepared", "{search}");
}

/// One MCP server session; requests are answered by id.
struct Mcp {
    child: Child,
    lines: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(env: &Env, args: &[&str]) -> Self {
        let mut child = env
            .command(&[&["mcp"], args].concat(), None)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap());
        Self { child, lines, next: 0 }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.next += 1;
        let request = json!({"jsonrpc": "2.0", "id": self.next, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            self.lines.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == self.next {
                return reply["result"].clone();
            }
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_builds_in_the_background_and_answers_status_and_search_meanwhile() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let mut mcp = Mcp::start(&env, &[]);
    let gate = env.gate("parse.hold");
    // The server's worker inherits its environment, so the hold applies to its build.
    drop(mcp);
    let mut child = env
        .command(&["mcp"], Some(("parse", &gate)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stdout.take().unwrap());
    mcp = Mcp { child, lines, next: 0 };
    let started = mcp.call("index", json!({"action": "prepare", "file": file, "scope": "0:0"}));
    assert_eq!(started["isError"], false, "{started}");
    let job = started["structuredContent"]["job"]["jobId"].as_str().unwrap().to_owned();
    let ws = started["structuredContent"]["workspaceId"].as_str().unwrap().to_owned();
    wait_for("the build to start", || {
        mcp.call("jobs", json!({"action": "status", "jobId": job}))["structuredContent"]["stage"]
            == "parse"
    });
    let pending = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    assert_eq!(pending["structuredContent"]["state"], "pending");
    let start = Instant::now();
    let cancelled = mcp.call("jobs", json!({"action": "cancel", "jobId": job}));
    assert!(start.elapsed() < Duration::from_millis(2500));
    assert_eq!(cancelled["structuredContent"]["state"], "cancelled", "{cancelled}");
    assert!(!worker_running(&job));
    std::fs::remove_file(&gate).unwrap();
    let again = mcp.call("index", json!({"action": "refresh", "workspaceId": ws, "scope": "0:0"}));
    let job = again["structuredContent"]["jobs"][0]["jobId"].as_str().unwrap().to_owned();
    wait_for("the refresh to finish", || {
        mcp.call("jobs", json!({"action": "status", "jobId": job}))["structuredContent"]["state"]
            == "succeeded"
    });
    let found = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    let stateless = mcp.call("search", json!({"file": file, "query": "first"}));
    assert_eq!(
        ids(&found["structuredContent"]),
        ids(&stateless["structuredContent"])
    );
}

#[test]
fn mcp_roots_hide_sources_jobs_and_counts_outside_them() {
    let env = Env::new();
    let (inside, outside) = (env.dir.join("in"), env.dir.join("out"));
    std::fs::create_dir(&inside).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let project = std::fs::canonicalize(env.dir.join("")).unwrap();
    let copy = |dir: &Path| {
        let path = dir.join("design.fig");
        std::fs::copy("examples/first-success/design.fig", &path).unwrap();
        std::fs::canonicalize(path).unwrap()
    };
    let (seen, hidden) = (copy(&inside), copy(&outside));
    let dir = project.to_str().unwrap();
    env.json(&["index", "prepare", seen.to_str().unwrap(), "--workspace", dir, "--scope", "0:0"]);
    let other = env.json(&["index", "prepare", hidden.to_str().unwrap(), "--workspace", dir, "--scope", "0:0"]);
    let hidden_job = other["job"]["jobId"].as_str().unwrap().to_owned();
    let ws = other["workspaceId"].as_str().unwrap().to_owned();
    let root = std::fs::canonicalize(&inside).unwrap();
    let mut mcp = Mcp::start(&env, &["--root", root.to_str().unwrap()]);
    let status = mcp.call("index", json!({"action": "status", "workspaceId": ws}));
    let text = status.to_string();
    assert!(!text.contains(outside.file_name().unwrap().to_str().unwrap()), "{text}");
    assert_eq!(status["structuredContent"]["sources"].as_array().unwrap().len(), 1);
    let search = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    assert_eq!(search["structuredContent"]["sources"].as_array().unwrap().len(), 1);
    assert!(!search.to_string().contains("/out/"));
    let job = mcp.call("jobs", json!({"action": "status", "jobId": hidden_job}));
    assert_eq!(job["isError"], true);
    assert!(!job.to_string().contains("/out/"));
}

#[test]
fn listing_tools_and_stateless_calls_create_no_database() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let mut mcp = Mcp::start(&env, &[]);
    mcp.call("find", json!({"file": fig, "query": "first"}));
    mcp.call("search", json!({"file": fig, "query": "first"}));
    mcp.call("search", json!({"workspaceId": "ws_0000000000000000", "query": "first"}));
    drop(mcp);
    assert!(!env.home.join(".local/state/figlens").exists());
}
