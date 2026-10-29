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
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out: {what}"
        );
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
        let conn =
            rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
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
    let out = Command::new("ps")
        .args(["-eo", "stat=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| {
        l.contains("__index-worker") && l.contains(job) && !l.trim_start().starts_with('Z')
    })
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
    let before = env.run(&["search", "--file", file, "first"]);
    assert!(String::from_utf8_lossy(&before.stderr).contains("is not prepared"));
    let prepared = env.json(&["index", "prepare", file, "--scope", "0:0"]);
    assert_eq!(prepared["job"]["state"], "succeeded");
    let stateless = env.json(&["search", file, "first"]);
    let indexed = env.json(&["search", "--file", file, "first"]);
    assert_eq!(indexed["state"], "ready");
    assert!(!ids(&stateless).is_empty());
    assert_eq!(ids(&indexed), ids(&stateless));
    // Whole results agree; only the source identity is the workspace's own.
    let plain = |reply: &Value| {
        let mut results = reply["results"].clone();
        for r in results.as_array_mut().unwrap() {
            r["ref"].as_object_mut().unwrap().remove("sourceId");
        }
        results
    };
    assert_eq!(plain(&indexed), plain(&stateless));
    assert_eq!(
        indexed["coverage"]["scannedRecords"],
        stateless["coverage"]["scannedRecords"]
    );
    assert_eq!(indexed["coverage"]["complete"], true);
    let rebuilt = env.json(&["index", "rebuild", "--file", file]);
    assert_eq!(rebuilt["jobs"][0]["state"], "succeeded");
    assert_ne!(
        rebuilt["jobs"][0]["generation"],
        prepared["job"]["generation"]
    );
    assert_eq!(
        ids(&env.json(&["search", "--file", file, "first"])),
        ids(&stateless)
    );
}

#[test]
fn two_sources_rank_as_one_corpus_and_a_cursor_over_a_replaced_generation_is_stale() {
    let env = Env::new();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let dir = std::fs::canonicalize(project).unwrap();
    let dir = dir.to_str().unwrap();
    for name in ["a.fig", "b.fig"] {
        let fig = env.fig(name);
        env.json(&[
            "index",
            "prepare",
            fig.to_str().unwrap(),
            "--workspace",
            dir,
            "--scope",
            "0:0",
        ]);
    }
    let all = env.json(&["search", "--workspace", dir, "first"]);
    let sources: Vec<&Value> = all["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["ref"]["sourceId"])
        .collect();
    assert_eq!(sources.len(), 2);
    assert_ne!(sources[0], sources[1]);
    let page = env.json(&["search", "--workspace", dir, "first", "--limit", "1"]);
    assert_eq!(page["results"][0]["ref"], all["results"][0]["ref"]);
    let cursor = page["nextCursor"]
        .as_str()
        .expect("a second page")
        .to_owned();
    let next = env.json(&[
        "search",
        "--workspace",
        dir,
        "first",
        "--limit",
        "1",
        "--cursor",
        &cursor,
    ]);
    assert_eq!(next["results"][0]["ref"], all["results"][1]["ref"]);
    env.json(&["index", "rebuild", "--workspace", dir]);
    let out = env.run(&[
        "search",
        "--workspace",
        dir,
        "first",
        "--limit",
        "1",
        "--cursor",
        &cursor,
    ]);
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
    let mut owner = env.spawn(
        &["index", "prepare", file, "--scope", "0:0"],
        Some(("parse", &gate)),
    );
    let job = active_job(&env, &fig, "parse");
    owner.kill().unwrap();
    owner.wait().unwrap();
    // The worker sees its stdin close and exits although its stage is still held.
    wait_for("the orphaned worker to exit", || !worker_running(&job));
    assert_eq!(env.json(&["jobs", "status", &job])["state"], "interrupted");
    std::fs::remove_file(&gate).unwrap();
    let retried = env.json(&["index", "refresh", "--file", file, "--scope", "0:0"]);
    assert_eq!(retried["jobs"][0]["state"], "succeeded");
    assert_eq!(
        env.json(&["search", "--file", file, "first"])["state"],
        "ready"
    );
}

#[test]
fn concurrent_prepares_of_one_scope_share_one_job() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let gate = env.gate("parse.hold");
    let first = env.spawn(
        &["index", "prepare", file, "--scope", "0:0"],
        Some(("parse", &gate)),
    );
    let job = active_job(&env, &fig, "parse");
    let second = env.spawn(&["index", "prepare", file, "--scope", "0:0"], None);
    std::thread::sleep(Duration::from_millis(300));
    std::fs::remove_file(&gate).unwrap();
    let replies: Vec<Value> = [first, second]
        .into_iter()
        .map(|c| {
            let out = c.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
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
        &[
            "index",
            "prepare",
            file,
            "--workspace",
            dir,
            "--scope",
            "0:0",
        ],
        Some(("write", &gate)),
    );
    let mut job = String::new();
    wait_for("the build to reach write", || {
        let status = env.json(&["index", "status", "--workspace", dir]);
        let db = status["paths"]["database"].as_str().unwrap().to_owned();
        let conn =
            rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
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
        Self {
            child,
            lines,
            next: 0,
        }
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
    mcp = Mcp {
        child,
        lines,
        next: 0,
    };
    let started = mcp.call(
        "index",
        json!({"action": "prepare", "file": file, "scope": "0:0"}),
    );
    assert_eq!(started["isError"], false, "{started}");
    let job = started["structuredContent"]["job"]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    let ws = started["structuredContent"]["workspaceId"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_for("the build to start", || {
        mcp.call("jobs", json!({"action": "status", "jobId": job}))["structuredContent"]["stage"]
            == "parse"
    });
    let pending = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    assert_eq!(pending["structuredContent"]["state"], "pending");
    let start = Instant::now();
    let cancelled = mcp.call("jobs", json!({"action": "cancel", "jobId": job}));
    assert!(start.elapsed() < Duration::from_millis(2500));
    assert_eq!(
        cancelled["structuredContent"]["state"], "cancelled",
        "{cancelled}"
    );
    assert!(!worker_running(&job));
    std::fs::remove_file(&gate).unwrap();
    let again = mcp.call(
        "index",
        json!({"action": "refresh", "workspaceId": ws, "scope": "0:0"}),
    );
    let job = again["structuredContent"]["jobs"][0]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
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
    env.json(&[
        "index",
        "prepare",
        seen.to_str().unwrap(),
        "--workspace",
        dir,
        "--scope",
        "0:0",
    ]);
    let other = env.json(&[
        "index",
        "prepare",
        hidden.to_str().unwrap(),
        "--workspace",
        dir,
        "--scope",
        "0:0",
    ]);
    let hidden_job = other["job"]["jobId"].as_str().unwrap().to_owned();
    let ws = other["workspaceId"].as_str().unwrap().to_owned();
    let root = std::fs::canonicalize(&inside).unwrap();
    let mut mcp = Mcp::start(&env, &["--root", root.to_str().unwrap()]);
    let status = mcp.call("index", json!({"action": "status", "workspaceId": ws}));
    let text = status.to_string();
    assert!(
        !text.contains(outside.file_name().unwrap().to_str().unwrap()),
        "{text}"
    );
    assert_eq!(
        status["structuredContent"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let search = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    assert_eq!(
        search["structuredContent"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
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
    mcp.call(
        "search",
        json!({"workspaceId": "ws_0000000000000000", "query": "first"}),
    );
    drop(mcp);
    assert!(!env.home.join(".local/state/figlens").exists());
}

#[test]
fn a_relocated_source_scoped_file_keeps_its_workspace_and_source_under_the_new_path() {
    let env = Env::new();
    let old = env.fig("a.fig");
    let prepared = env.json(&["index", "prepare", old.to_str().unwrap(), "--scope", "0:0"]);
    let (ws, source) = (&prepared["workspaceId"], &prepared["source"]["sourceId"]);
    let new = env.dir.join("moved.fig");
    std::fs::rename(&old, &new).unwrap();
    let new = std::fs::canonicalize(new).unwrap();
    let (old, new) = (old.to_str().unwrap(), new.to_str().unwrap());
    let source_id = source.as_str().unwrap();
    env.json(&["index", "relocate", source_id, new, "--file", old]);
    let status = env.json(&["index", "status", "--file", new]);
    assert_eq!(&status["workspaceId"], ws);
    assert_eq!(&status["sources"][0]["sourceId"], source);
    // The new membership stops serving the old path's index until a refresh.
    let stale = env.run(&["search", "--file", new, "first"]);
    assert!(String::from_utf8_lossy(&stale.stderr).contains("refresh"));
    env.json(&["index", "refresh", "--file", new, "--scope", "0:0"]);
    let found = env.json(&["search", "--file", new, "first"]);
    assert_eq!(&found["workspaceId"], ws);
    assert_eq!(&found["results"][0]["ref"]["sourceId"], source);
    let again = env.json(&["index", "prepare", new]);
    assert_eq!(&again["workspaceId"], ws);
    assert_eq!(again["source"]["added"], false);
}

#[test]
fn mcp_without_flags_reuses_a_workspace_the_cli_placed_in_custom_storage() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let store = env.dir.join("store");
    std::fs::create_dir(&store).unwrap();
    let store = std::fs::canonicalize(store).unwrap();
    let prepared = env.json(&[
        "index",
        "prepare",
        file,
        "--storage",
        store.to_str().unwrap(),
        "--scope",
        "0:0",
    ]);
    let mut mcp = Mcp::start(&env, &[]);
    let status = mcp.call("index", json!({"action": "status", "file": file}));
    assert_eq!(
        status["structuredContent"]["incarnation"], prepared["incarnation"],
        "{status}"
    );
    assert_eq!(status["structuredContent"]["paths"], prepared["paths"]);
    let found = mcp.call(
        "search",
        json!({"workspaceId": prepared["workspaceId"], "query": "first"}),
    );
    assert_eq!(found["structuredContent"]["state"], "ready", "{found}");
    drop(mcp);
    assert!(!env.home.join(".local/state/figlens/workspaces").exists());
}

#[test]
fn mcp_searches_a_workspace_kept_under_a_storage_root() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let store = env.dir.join("store");
    std::fs::create_dir(&store).unwrap();
    let store = std::fs::canonicalize(store).unwrap();
    let store = store.to_str().unwrap();
    let prepared = env.json(&[
        "index",
        "prepare",
        file,
        "--storage",
        store,
        "--scope",
        "0:0",
    ]);
    let mut mcp = Mcp::start(&env, &["--storage-root", store]);
    let found = mcp.call(
        "search",
        json!({"workspaceId": prepared["workspaceId"], "storage": store, "query": "first"}),
    );
    assert_eq!(found["structuredContent"]["state"], "ready", "{found}");
}

#[test]
fn a_stored_parent_cycle_is_refused_as_an_unreadable_index_instead_of_spinning() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let prepared = env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let conn = rusqlite::Connection::open(prepared["paths"]["database"].as_str().unwrap()).unwrap();
    conn.execute("UPDATE records SET parent = rid WHERE node_id = '0:1'", [])
        .unwrap();
    drop(conn);
    let out = env.run(&["search", "--file", file, "first"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rebuild"));
}

#[cfg(unix)]
#[test]
fn mcp_roots_do_not_serve_a_source_later_replaced_by_a_link_out_of_them() {
    let env = Env::new();
    let (inside, outside) = (env.dir.join("in"), env.dir.join("out"));
    std::fs::create_dir(&inside).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let (inside, outside) = (
        std::fs::canonicalize(inside).unwrap(),
        std::fs::canonicalize(outside).unwrap(),
    );
    let fig = inside.join("design.fig");
    std::fs::copy("examples/first-success/design.fig", &fig).unwrap();
    let prepared = env.json(&["index", "prepare", fig.to_str().unwrap(), "--scope", "0:0"]);
    let secret = outside.join("secret.fig");
    std::fs::rename(&fig, &secret).unwrap();
    std::os::unix::fs::symlink(&secret, &fig).unwrap();
    let mut mcp = Mcp::start(&env, &["--root", inside.to_str().unwrap()]);
    let found = mcp.call(
        "search",
        json!({"workspaceId": prepared["workspaceId"], "query": "first"}),
    );
    let text = found.to_string();
    assert!(!text.contains("secret"), "{text}");
    assert!(
        found["structuredContent"]["results"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{text}"
    );
}

#[test]
fn an_unusable_workspace_lists_stateless_search_of_the_named_file() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let store = env.dir.join("store");
    std::fs::create_dir(&store).unwrap();
    let store = std::fs::canonicalize(store).unwrap();
    env.json(&[
        "index",
        "prepare",
        file,
        "--storage",
        store.to_str().unwrap(),
        "--scope",
        "0:0",
    ]);
    std::fs::rename(&store, env.dir.join("offline")).unwrap();
    let out = env.run(&["search", "--file", file, "first"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"stateless_search\""), "{stderr}");
}

#[cfg(unix)]
#[test]
fn a_source_replaced_by_a_fifo_is_reported_unreadable_without_blocking() {
    let env = Env::new();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let dir = std::fs::canonicalize(project).unwrap();
    let dir = dir.to_str().unwrap();
    let fig = env.fig("a.fig");
    env.json(&[
        "index",
        "prepare",
        fig.to_str().unwrap(),
        "--workspace",
        dir,
        "--scope",
        "0:0",
    ]);
    std::fs::remove_file(&fig).unwrap();
    assert!(Command::new("mkfifo").arg(&fig).status().unwrap().success());
    let mut search = env.spawn(&["search", "--workspace", dir, "first"], None);
    let start = Instant::now();
    while search.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = search.kill();
            panic!("search blocked on a FIFO source");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = search.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("no searched source can be read"));
}

#[test]
fn the_old_path_of_a_relocated_source_does_not_serve_it_under_that_name() {
    let env = Env::new();
    let old = env.fig("a.fig");
    let prepared = env.json(&["index", "prepare", old.to_str().unwrap(), "--scope", "0:0"]);
    let new = env.dir.join("moved.fig");
    std::fs::rename(&old, &new).unwrap();
    let new = std::fs::canonicalize(new).unwrap();
    let source = prepared["source"]["sourceId"].as_str().unwrap();
    let (old_s, new_s) = (old.to_str().unwrap(), new.to_str().unwrap());
    env.json(&["index", "relocate", source, new_s, "--file", old_s]);
    // A different design now sits at the old path.
    std::fs::copy("examples/first-success/design.fig", &old).unwrap();
    let searched = env.run(&["search", "--file", old_s, "first"]);
    assert!(!searched.status.success());
    let stderr = String::from_utf8_lossy(&searched.stderr);
    assert!(
        stderr.contains("relocated") && stderr.contains(new_s),
        "{stderr}"
    );
    let prepared_again = env.run(&["index", "prepare", old_s]);
    assert!(!prepared_again.status.success());
    assert!(String::from_utf8_lossy(&prepared_again.stderr).contains("relocated"));
    // The new path still selects the workspace.
    assert_eq!(
        env.json(&["index", "status", "--file", new_s])["workspaceId"],
        prepared["workspaceId"]
    );
}

#[cfg(unix)]
#[test]
fn ctrl_c_cancels_the_cli_owned_build_and_waits_for_its_worker() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let gate = env.gate("parse.hold");
    let owner = env.spawn(
        &["index", "rebuild", "--file", file],
        Some(("parse", &gate)),
    );
    let job = active_job(&env, &fig, "parse");
    assert!(
        Command::new("kill")
            .args(["-INT", &owner.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let start = Instant::now();
    let out = owner.wait_with_output().unwrap();
    assert!(start.elapsed() < Duration::from_millis(2500));
    assert!(!out.status.success());
    assert!(!worker_running(&job));
    assert_eq!(env.json(&["jobs", "status", &job])["state"], "cancelled");
    std::fs::remove_file(&gate).unwrap();
    assert_eq!(
        env.json(&["search", "--file", file, "first"])["state"],
        "ready"
    );
}
