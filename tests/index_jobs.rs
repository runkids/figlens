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
            // Windows roots ignore HOME and use %LOCALAPPDATA%, which would be the runner's own.
            .env("LOCALAPPDATA", &self.home)
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

/// Whether a worker process of `job` is alive, from `ps`: independent of the worker-file lock
/// the product reads, so a lock dropped before the process exits is still caught.
#[cfg(unix)]
fn worker_running(_env: &Env, job: &str) -> bool {
    let out = Command::new("ps")
        .args(["-eo", "stat=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| {
        l.contains("__index-worker") && l.contains(job) && !l.trim_start().starts_with('Z')
    })
}

/// Whether a worker of `job` is alive: `ps` lists no arguments on Windows, so this reads the
/// lock a worker holds on its `claims/<job>.<attempt>.worker` file until it exits.
#[cfg(not(unix))]
fn worker_running(env: &Env, job: &str) -> bool {
    let prefix = format!("{job}.");
    let mut dirs = vec![env.dir.join("")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                dirs.push(path);
            } else if name.starts_with(&prefix)
                && name.ends_with(".worker")
                && std::fs::File::open(&path).is_ok_and(|f| f.try_lock_shared().is_err())
            {
                return true;
            }
        }
    }
    false
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
fn every_root_and_workspace_path_stays_inside_the_test_home() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let prepared = env.json(&["index", "prepare", fig.to_str().unwrap()]);
    let mut paths: Vec<&str> = ["config", "state", "cache", "data"]
        .iter()
        .map(|k| prepared["roots"][k]["path"].as_str().unwrap())
        .collect();
    paths.extend(["workspace", "database", "lock"].map(|k| prepared["paths"][k].as_str().unwrap()));
    let outside: Vec<_> = paths
        .into_iter()
        .filter(|p| !Path::new(p).starts_with(&env.home))
        .collect();
    assert!(
        outside.is_empty(),
        "{outside:?} outside {}",
        env.home.display()
    );
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
        assert!(worker_running(&env, &job));
        let start = Instant::now();
        let reply = env.json(&["jobs", "cancel", &job]);
        assert_eq!(reply["cancel"], "accepted", "{stage}: {reply}");
        assert_eq!(reply["state"], "cancelled");
        // Reported only after the worker exited.
        assert!(
            !worker_running(&env, &job),
            "{stage}: worker outlived the cancel"
        );
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
    wait_for("the orphaned worker to exit", || {
        !worker_running(&env, &job)
    });
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

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.lines.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
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
fn usages_agree_stateless_over_mcp_and_from_a_prepared_scope_and_reject_a_stale_ref() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let stateless = env.json(&["usages", file, "0:1", "--relation", "contains"]);
    assert_eq!(ids(&stateless), ["0:2"]);
    let mut mcp = Mcp::start(&env, &[]);
    let over_mcp = mcp.call(
        "usages",
        json!({"file": file, "node": "0:1", "relation": "contains"}),
    );
    drop(mcp);
    assert_eq!(
        over_mcp["structuredContent"]["results"],
        stateless["results"]
    );
    // Stateless usages, like search, never create a database.
    assert!(!env.home.join(".local/state/figlens").exists());
    let prepared = env.json(&["index", "prepare", file, "--scope", "0:0"]);
    assert_eq!(prepared["job"]["state"], "succeeded");
    // Each call is a new process: the published index is reused, never rebuilt.
    for _ in 0..2 {
        let indexed = env.json(&["usages", "--file", file, "0:1", "--relation", "contains"]);
        let plain = |reply: &Value| {
            let mut results = reply["results"].clone();
            for r in results.as_array_mut().unwrap() {
                r["ref"].as_object_mut().unwrap().remove("sourceId");
            }
            results
        };
        assert_eq!(plain(&indexed), plain(&stateless));
        assert_eq!(indexed["definition"]["state"], "indexed");
    }
    let snapshot = stateless["source"]["snapshotId"].as_str().unwrap();
    let current = env.run(&["usages", "--file", file, "0:1", "--snapshot", snapshot]);
    assert!(current.status.success());
    let stale = env.run(&["usages", "--file", file, "0:1", "--snapshot", "sha256:old"]);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("now serves"));
}

#[test]
fn usages_of_a_scope_not_ready_report_its_state_next_step_and_null_fields() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let idle = env.json(&["usages", "--file", file, "0:1"]);
    assert_eq!(idle["state"], "not_prepared");
    assert_eq!(idle["nextSteps"][0]["arguments"]["action"], "refresh");
    let gate = env.gate("parse.hold");
    let mut owner = env.spawn(
        &["index", "prepare", file, "--scope", "0:0"],
        Some(("parse", &gate)),
    );
    let job = active_job(&env, &fig, "parse");
    let mut mcp = Mcp::start(&env, &[]);
    let ws = idle["workspaceId"].as_str().unwrap();
    let reply = mcp.call("usages", json!({"workspaceId": ws, "node": "0:1"}));
    drop(mcp);
    assert_eq!(reply["isError"], false, "{reply}");
    let pending = &reply["structuredContent"];
    assert_eq!(pending["state"], "pending");
    assert_eq!(pending["nextSteps"][0]["arguments"]["jobId"], job.as_str());
    // The fields the output schema requires are present; definition is null until ready.
    for key in ["definition", "results", "coverage", "hasMore", "nextCursor"] {
        assert!(pending.get(key).is_some(), "{key}");
    }
    assert_eq!(pending["definition"], Value::Null);
    std::fs::remove_file(&gate).unwrap();
    assert!(owner.wait().unwrap().success());
    let ready = env.json(&["usages", "--file", file, "0:1"]);
    assert_eq!(ready["definition"]["state"], "indexed");
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
    assert!(!worker_running(&env, &job));
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
    // The hidden directory as it appears inside a JSON reply, with the host's separators.
    let leak = serde_json::to_string(hidden.parent().unwrap()).unwrap();
    let leak = leak.trim_matches('"').to_owned();
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
    assert!(!search.to_string().contains(&leak));
    let coverage = &search["structuredContent"]["coverage"];
    assert_eq!(coverage["complete"], false, "{search}");
    assert!(
        coverage["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!({"code": "authorization_restricted"}))
    );
    let job = mcp.call("jobs", json!({"action": "status", "jobId": hidden_job}));
    assert_eq!(job["isError"], true);
    assert!(!job.to_string().contains(&leak));
    // Usages read only the visible source; naming the hidden one finds nothing to read.
    let usages = mcp.call("usages", json!({"workspaceId": ws, "node": "0:1"}));
    assert_eq!(usages["isError"], false, "{usages}");
    assert!(!usages.to_string().contains(&leak));
    let all = env.json(&["index", "status", "--workspace", dir]);
    // The unrestricted status lists the hidden copy, so the token matches on this host.
    assert!(all.to_string().contains(&leak));
    let hidden_id = all["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| Path::new(s["path"].as_str().unwrap()) == hidden)
        .unwrap()["sourceId"]
        .as_str()
        .unwrap()
        .to_owned();
    let refused = mcp.call(
        "usages",
        json!({"workspaceId": ws, "sourceId": hidden_id, "node": "0:1"}),
    );
    assert_eq!(refused["isError"], true);
    assert!(!refused.to_string().contains(&leak));
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
    assert!(!worker_running(&env, &job));
    assert_eq!(env.json(&["jobs", "status", &job])["state"], "cancelled");
    std::fs::remove_file(&gate).unwrap();
    assert_eq!(
        env.json(&["search", "--file", file, "first"])["state"],
        "ready"
    );
}

#[cfg(unix)]
#[test]
fn mcp_answers_ping_while_control_waits_and_drops_a_queued_call_the_client_cancelled() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let gate = env.gate("parse.hold");
    let mut owner = env.spawn(
        &["index", "rebuild", "--file", file],
        Some(("parse", &gate)),
    );
    let job = active_job(&env, &fig, "parse");
    // A stopped owner cannot act on a cancel, so `jobs cancel` waits its whole CANCEL_WAIT.
    let pid = owner.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-STOP", &pid])
            .status()
            .unwrap()
            .success()
    );
    let mut mcp = Mcp::start(&env, &[]);
    let call = |id: u64, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "jobs", "arguments": arguments}})
    };
    mcp.send(call(1, json!({"action": "cancel", "jobId": job})));
    mcp.send(call(2, json!({"action": "status", "jobId": job})));
    mcp.send(
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 2}}),
    );
    mcp.send(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}));
    mcp.send(call(4, json!({"action": "status", "jobId": job})));
    let ids: Vec<Value> = (0..3).map(|_| mcp.read()["id"].clone()).collect();
    assert_eq!(ids, [json!(3), json!(1), json!(4)]);
    assert!(
        Command::new("kill")
            .args(["-CONT", &pid])
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_file(&gate).unwrap();
    owner.wait().unwrap();
}

#[test]
fn a_search_past_the_query_envelope_is_a_work_limit_with_narrower_searches_not_a_bad_index() {
    let env = Env::new();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let dir = std::fs::canonicalize(project).unwrap();
    let dir = dir.to_str().unwrap();
    for n in 0..11 {
        let fig = env.fig(&format!("{n}.fig"));
        env.json(&[
            "index",
            "prepare",
            fig.to_str().unwrap(),
            "--workspace",
            dir,
            "--scope",
            "0:1",
        ]);
    }
    let out = env.run(&[
        "search",
        "--workspace",
        dir,
        "first",
        "--node",
        "0:1",
        "--type",
        "text",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let recovery: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    let actions = recovery["recovery"].as_array().unwrap();
    assert_eq!(actions.len(), 11, "{stderr}");
    assert!(actions.iter().all(|a| a["action"] == "search_one_source"));
    let args = &actions[0]["arguments"];
    assert_eq!(
        (&args["node"], &args["type"]),
        (&json!("0:1"), &json!("TEXT"))
    );
    let one = args["sourceId"].as_str().unwrap();
    let narrowed = env.json(&[
        "search",
        "--workspace",
        dir,
        "first",
        "--source",
        one,
        "--node",
        "0:1",
        "--type",
        "TEXT",
    ]);
    assert_eq!(narrowed["state"], "ready");
    // Nothing was dropped from the registry.
    let status = env.json(&["index", "status", "--workspace", dir]);
    assert_eq!(status["sources"].as_array().unwrap().len(), 11);
}

#[test]
fn rebuild_narrowed_to_one_source_and_scope_builds_only_that_one_even_when_unchanged() {
    let env = Env::new();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let dir = std::fs::canonicalize(project).unwrap();
    let dir = dir.to_str().unwrap();
    let mut sources = Vec::new();
    for name in ["a.fig", "b.fig"] {
        let fig = env.fig(name);
        let done = env.json(&[
            "index",
            "prepare",
            fig.to_str().unwrap(),
            "--workspace",
            dir,
            "--scope",
            "0:0",
        ]);
        sources.push(done["source"]["sourceId"].as_str().unwrap().to_owned());
    }
    let rebuilt = env.json(&[
        "index",
        "rebuild",
        "--workspace",
        dir,
        "--source",
        &sources[0],
        "--scope",
        "0:0",
    ]);
    let jobs = rebuilt["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1, "{rebuilt}");
    assert_eq!(jobs[0]["sourceId"], sources[0].as_str());
    assert_eq!(jobs[0]["kind"], "rebuild");
    assert_eq!(jobs[0]["state"], "succeeded");
    assert_eq!(rebuilt["admission"], "complete");
}

/// Whether `value` has the JSON types `schema` declares, recursively through `properties`,
/// `required` and `items`: what an MCP client validating structuredContent checks.
fn conforms(value: &Value, schema: &Value, at: &str) -> Result<(), String> {
    let kind = |t: &str| match t {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    };
    let ok = match &schema["type"] {
        Value::String(t) => kind(t),
        Value::Array(ts) => ts.iter().any(|t| kind(t.as_str().unwrap_or(""))),
        _ => true,
    };
    if !ok {
        return Err(format!("{at}: {value} is not {}", schema["type"]));
    }
    for name in schema["required"].as_array().into_iter().flatten() {
        if value.get(name.as_str().unwrap()).is_none() {
            return Err(format!("{at}: missing {name}"));
        }
    }
    if let (Some(props), Some(object)) = (schema["properties"].as_object(), value.as_object()) {
        for (name, sub) in props {
            if let Some(v) = object.get(name) {
                conforms(v, sub, &format!("{at}/{name}"))?;
            }
        }
    }
    if let (Some(items), Some(list)) = (schema.get("items"), value.as_array()) {
        for (i, v) in list.iter().enumerate() {
            conforms(v, items, &format!("{at}/{i}"))?;
        }
    }
    Ok(())
}

#[test]
fn every_index_jobs_and_workspace_search_reply_matches_its_declared_output_schema() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let project = env.dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let project = std::fs::canonicalize(project).unwrap();
    let dir = project.to_str().unwrap();
    let other = env.fig("b.fig");
    let mut mcp = Mcp::start(&env, &[]);
    mcp.send(json!({"jsonrpc": "2.0", "id": 0, "method": "tools/list"}));
    let tools = mcp.read()["result"]["tools"].clone();
    let schema = |name: &str| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["outputSchema"]
            .clone()
    };
    let check = |mcp: &mut Mcp, tool: &str, arguments: Value| -> Value {
        let reply = mcp.call(tool, arguments.clone());
        assert_eq!(reply["isError"], false, "{tool} {arguments}: {reply}");
        let content = &reply["structuredContent"];
        conforms(content, &schema(tool), tool).unwrap_or_else(|e| panic!("{arguments}: {e}"));
        content.clone()
    };
    // Until the job's own reply is terminal. Only an active state or a workspace_busy status
    // call is waited out; any other reply fails with it as evidence.
    let wait = |mcp: &mut Mcp, job: &str| {
        let start = Instant::now();
        loop {
            let now = mcp.call("jobs", json!({"action": "status", "jobId": job}));
            let busy =
                now["isError"] == true && now["_meta"]["figlens"]["code"] == "workspace_busy";
            match now["structuredContent"]["state"].as_str() {
                Some("succeeded" | "failed" | "cancelled") if now["isError"] == false => break,
                Some("queued" | "running") if now["isError"] == false => {}
                _ if busy => {}
                _ => panic!("job {job}: unexpected status reply {now}"),
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "job {job} never ended: {now}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    check(&mut mcp, "index", json!({"action": "status", "file": file}));
    check(
        &mut mcp,
        "index",
        json!({"action": "prepare", "file": file}),
    );
    let started = check(
        &mut mcp,
        "index",
        json!({"action": "prepare", "file": file, "scope": "0:0"}),
    );
    let job = started["job"]["jobId"].as_str().unwrap().to_owned();
    let ws = started["workspaceId"].as_str().unwrap().to_owned();
    let source = started["source"]["sourceId"].as_str().unwrap().to_owned();
    wait(&mut mcp, &job);
    let report = check(&mut mcp, "jobs", json!({"action": "status", "jobId": job}));
    assert_eq!(report["workspaceId"], ws.as_str());
    check(&mut mcp, "jobs", json!({"action": "cancel", "jobId": job}));
    check(
        &mut mcp,
        "index",
        json!({"action": "status", "workspaceId": ws}),
    );
    check(
        &mut mcp,
        "search",
        json!({"workspaceId": ws, "query": "first"}),
    );
    check(
        &mut mcp,
        "search",
        json!({"workspaceId": ws, "query": "first", "node": "0:2"}),
    );
    let refreshed = check(
        &mut mcp,
        "index",
        json!({"action": "refresh", "workspaceId": ws}),
    );
    // A rebuild does not join a running refresh, so it starts once the refresh ended.
    for job in refreshed["jobs"].as_array().unwrap() {
        wait(&mut mcp, job["jobId"].as_str().unwrap());
    }
    let rebuilt = check(
        &mut mcp,
        "index",
        json!({"action": "rebuild", "workspaceId": ws, "sourceId": source, "scope": "0:0"}),
    );
    for job in rebuilt["jobs"].as_array().unwrap() {
        wait(&mut mcp, job["jobId"].as_str().unwrap());
    }
    check(
        &mut mcp,
        "index",
        json!({"action": "clear_cache", "workspaceId": ws}),
    );
    check(
        &mut mcp,
        "index",
        json!({"action": "reset", "workspaceId": ws}),
    );
    check(
        &mut mcp,
        "index",
        json!({"action": "status", "workspaceId": ws}),
    );
    // A project workspace: add, relocate and remove.
    let prepared = check(
        &mut mcp,
        "index",
        json!({"action": "prepare", "file": file, "workspace": dir}),
    );
    let pws = prepared["workspaceId"].as_str().unwrap().to_owned();
    let added = check(
        &mut mcp,
        "index",
        json!({"action": "add", "workspaceId": pws, "file": other}),
    );
    let added_id = added["source"]["sourceId"].as_str().unwrap().to_owned();
    let moved = env.dir.join("moved.fig");
    std::fs::rename(&other, &moved).unwrap();
    check(
        &mut mcp,
        "index",
        json!({"action": "relocate", "workspaceId": pws, "sourceId": added_id, "file": moved}),
    );
    check(
        &mut mcp,
        "index",
        json!({"action": "remove", "workspaceId": pws, "sourceId": added_id}),
    );
}

#[test]
fn a_workspace_search_takes_a_query_starting_with_a_dash_after_double_dash() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file, "--scope", "0:0"]);
    let found = env.json(&["search", "--file", file, "--", "-20%"]);
    assert_eq!(found["state"], "ready", "{found}");
}

#[test]
fn an_mcp_opt_in_of_one_frame_is_searched_and_inspected_at_that_scope_only() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let mut mcp = Mcp::start(&env, &[]);
    let started = mcp.call(
        "index",
        json!({"action": "prepare", "file": file, "scope": "0:1"}),
    );
    let job = started["structuredContent"]["job"]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    let ws = started["structuredContent"]["workspaceId"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_for("the job", || {
        mcp.call("jobs", json!({"action": "status", "jobId": job}))["structuredContent"]["state"]
            == "succeeded"
    });
    let found = mcp.call(
        "search",
        json!({"workspaceId": ws, "node": "0:1", "query": "first"}),
    )["structuredContent"]
        .clone();
    assert_eq!(found["state"], "ready", "{found}");
    let node = found["results"][0]["ref"]["nodeId"].as_str().unwrap();
    let inspected = mcp.call("inspect", json!({"file": file, "node": node}));
    assert_eq!(inspected["isError"], false, "{inspected}");
    // The whole file was never opted in: no implicit 0:0 build.
    let whole = mcp.call("search", json!({"workspaceId": ws, "query": "first"}));
    assert_eq!(
        whole["structuredContent"]["sources"][0]["reason"], "scope_not_indexed",
        "{whole}"
    );
}

#[test]
fn a_queued_and_a_cancelled_before_start_job_match_the_jobs_output_schema() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let gate = env.gate("parse.hold");
    let mut child = env
        .command(&["mcp"], Some(("parse", &gate)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stdout.take().unwrap());
    let mut mcp = Mcp {
        child,
        lines,
        next: 0,
    };
    mcp.send(json!({"jsonrpc": "2.0", "id": 0, "method": "tools/list"}));
    let tools = mcp.read()["result"]["tools"].clone();
    let schema = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "jobs")
        .unwrap()["outputSchema"]
        .clone();
    mcp.call(
        "index",
        json!({"action": "prepare", "file": file, "scope": "0:0"}),
    );
    let queued = mcp.call(
        "index",
        json!({"action": "prepare", "file": file, "scope": "0:1"}),
    );
    let job = queued["structuredContent"]["job"]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    let status = mcp.call("jobs", json!({"action": "status", "jobId": job}));
    assert_eq!(status["structuredContent"]["state"], "queued", "{status}");
    let cancelled = mcp.call("jobs", json!({"action": "cancel", "jobId": job}));
    std::fs::remove_file(&gate).unwrap();
    for reply in [status, cancelled] {
        conforms(&reply["structuredContent"], &schema, "jobs").unwrap();
    }
}

#[test]
fn a_cancel_after_the_owner_died_is_pending_from_its_worker_s_start_to_its_exit() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let gate = env.gate("start.hold");
    let mut owner = env
        .command(
            &["index", "prepare", file, "--scope", "0:0"],
            Some(("start", &gate)),
        )
        .env("FIGLENS_TEST_ORPHAN", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let reached = env.dir.join("start.hold.reached");
    wait_for("the worker to start", || reached.exists());
    let job = active_job(&env, &fig, "stage");
    owner.kill().unwrap();
    owner.wait().unwrap();
    // Not yet locked: a starting worker is not proof of exit.
    let starting = env.json(&["jobs", "cancel", &job]);
    assert_eq!(starting["cancel"], "pending", "{starting:#}");
    std::fs::remove_file(&gate).unwrap();
    wait_for("the orphaned worker to exit", || {
        env.json(&["jobs", "cancel", &job])["cancel"] == "accepted"
    });
}

#[test]
fn mcp_whose_reply_stream_closes_stops_its_build_and_exits_though_stdin_stays_open() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let gate = env.gate("parse.hold");
    let mut child = env
        .command(&["mcp"], Some(("parse", &gate)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "index",
        "arguments": {"action": "prepare", "file": file, "scope": "0:0"}}}),
    );
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    let started: Value = serde_json::from_str(&line).unwrap();
    let job = started["result"]["structuredContent"]["job"]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    active_job(&env, &fig, "parse");
    // The reply stream and stderr close; stdin stays open, and then a reply has to be written.
    drop(lines);
    drop(child.stderr.take());
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
    wait_for("the server to exit", || child.try_wait().unwrap().is_some());
    let state = env.json(&["jobs", "status", &job])["state"].clone();
    std::fs::remove_file(&gate).unwrap();
    assert_eq!(state, "cancelled");
}

#[test]
fn mcp_whose_reply_stream_closes_mid_submit_cancels_the_job_it_was_admitting() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let gate = env.gate("submit.hold");
    let mut child = env
        .command(&["mcp"], Some(("submit", &gate)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let lines = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "index",
        "arguments": {"action": "prepare", "file": fig.to_str().unwrap(), "scope": "0:0"}}}),
    );
    let reached = gate.with_extension("hold.reached");
    wait_for("the submit to admit its job", || reached.exists());
    // The job row exists but its build is not queued yet when the reply stream dies.
    drop(lines);
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
    std::thread::sleep(Duration::from_millis(200));
    let early = child.try_wait().unwrap();
    std::fs::remove_file(&gate).unwrap();
    wait_for("the server to exit", || child.try_wait().unwrap().is_some());
    let status = env.json(&["index", "status", "--file", fig.to_str().unwrap()]);
    let db = PathBuf::from(status["paths"]["database"].as_str().unwrap());
    let states: Vec<String> = rusqlite::Connection::open(db)
        .unwrap()
        .prepare("SELECT state FROM jobs")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!((early, states), (None, vec!["cancelled".to_owned()]));
}

#[test]
fn a_cancel_completes_when_the_owner_dies_while_its_worker_is_starting() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    env.json(&["index", "prepare", file]);
    let gate = env.gate("start.hold");
    // No FIGLENS_TEST_ORPHAN: the worker's stdin watchdog fires once the owner dies.
    let mut owner = env
        .command(
            &["index", "prepare", file, "--scope", "0:0"],
            Some(("start", &gate)),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the worker to start", || {
        env.dir.join("start.hold.reached").exists()
    });
    let job = active_job(&env, &fig, "stage");
    owner.kill().unwrap();
    owner.wait().unwrap();
    let starting = env.json(&["jobs", "cancel", &job]);
    assert_eq!(starting["cancel"], "pending", "{starting:#}");
    std::fs::remove_file(&gate).unwrap();
    wait_for("the cancel to complete", || {
        env.json(&["jobs", "cancel", &job])["cancel"] == "accepted"
    });
}

#[test]
fn a_text_only_search_cache_upgrades_once_for_usages_and_keeps_search_cursors() {
    let env = Env::new();
    let fig = env.fig("a.fig");
    let file = fig.to_str().unwrap();
    let mut mcp = Mcp::start(&env, &[]);
    let search = |mcp: &mut Mcp, cursor: Option<&str>| {
        let mut args = json!({"file": file, "query": "first", "limit": 1});
        if let Some(c) = cursor {
            args["cursor"] = json!(c);
        }
        mcp.call("search", args)["structuredContent"].clone()
    };
    let cold = search(&mut mcp, None);
    let kinds = &cold["coverage"]["layers"]["relations"]["kinds"];
    // Ordinary search builds text and structure only; field relations are not built, not empty.
    assert_eq!(
        (
            &cold["coverage"]["complete"],
            &kinds["instance_of"],
            &kinds["binds_variable"]
        ),
        (&json!(true), &json!("ready"), &json!("not_built"))
    );
    assert_eq!(cold["source"]["freshness"]["reusedIndex"], false);
    let usages = |mcp: &mut Mcp| {
        mcp.call(
            "usages",
            json!({"file": file, "node": "0:1", "relation": "contains"}),
        )["structuredContent"]
            .clone()
    };
    // The first usages call rebuilds with every relation, once; the next one reuses it.
    let first = usages(&mut mcp);
    assert_eq!(
        (ids(&first), &first["source"]["freshness"]["reusedIndex"]),
        (vec!["0:2"], &json!(false))
    );
    assert_eq!(usages(&mut mcp)["source"]["freshness"]["reusedIndex"], true);
    // The full index serves search too, with the same results.
    let warm = search(&mut mcp, None);
    assert_eq!(
        (
            &warm["results"],
            &warm["source"]["freshness"]["reusedIndex"]
        ),
        (&cold["results"], &json!(true))
    );
    assert_eq!(
        warm["coverage"]["layers"]["relations"]["kinds"]["binds_variable"],
        "ready"
    );
    // A new revision is read again: same content, another snapshot, fresh text-only build.
    let bytes = std::fs::read(&fig).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::fs::File::create(&fig).unwrap());
    for i in 0..archive.len() {
        out.raw_copy_file(archive.by_index(i).unwrap()).unwrap();
    }
    out.start_file("revision.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    out.write_all(b"2").unwrap();
    out.finish().unwrap();
    let changed = search(&mut mcp, None);
    assert_ne!(
        changed["source"]["snapshotId"],
        cold["source"]["snapshotId"]
    );
    assert_eq!(
        (
            ids(&changed),
            &changed["coverage"]["layers"]["relations"]["kinds"]["binds_variable"]
        ),
        (ids(&cold), &json!("not_built"))
    );
    let after = usages(&mut mcp);
    assert_eq!(
        (ids(&after), &after["source"]["snapshotId"]),
        (vec!["0:2"], &changed["source"]["snapshotId"])
    );
    drop(mcp);
}

#[path = "../scripts/make-corpus.rs"]
#[allow(dead_code)]
mod corpus;

#[test]
fn diff_text_lists_a_page_points_at_the_next_and_names_the_missing_side() {
    let env = Env::new();
    let [old, new] = [("old.fig", false), ("new.fig", true)].map(|(name, revised)| {
        let path = env.dir.join(name);
        corpus::write(path.as_os_str(), revised).unwrap();
        path.to_str().unwrap().to_owned()
    });
    let page = env.run(&["diff", &old, &new, "--limit", "1"]);
    assert!(page.status.success());
    let text = String::from_utf8_lossy(&page.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].contains(" changed: ")
            || lines[0].contains(" added: ")
            || lines[0].contains(" removed: "),
        "{text}"
    );
    assert!(lines[1].starts_with("more: --cursor "), "{text}");
    let same = env.run(&["diff", &old, &old]);
    assert_eq!(String::from_utf8_lossy(&same.stdout), "no changes\n");
    let missing = env.dir.join("missing.fig");
    let gone = env.run(&["diff", &old, missing.to_str().unwrap()]);
    assert!(!gone.status.success());
    let stderr = String::from_utf8_lossy(&gone.stderr);
    assert!(
        stderr.contains("missing.fig") && !stderr.contains("old.fig"),
        "{stderr}"
    );
}

#[test]
fn diff_of_two_revisions_pages_the_same_changes_over_the_cli_and_mcp() {
    let env = Env::new();
    let [old, new] = [("old.fig", false), ("new.fig", true)].map(|(name, revised)| {
        let path = env.dir.join(name);
        corpus::write(path.as_os_str(), revised).unwrap();
        std::fs::canonicalize(path).unwrap()
    });
    let (old, new) = (old.to_str().unwrap(), new.to_str().unwrap());
    let cli = env.json(&["diff", old, new, "--json"]);
    let changes = |reply: &Value| {
        let mut all: Vec<(String, String)> = reply["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|f| f["nodes"].as_array().unwrap())
            .map(|n| {
                (
                    n["id"].as_str().unwrap().into(),
                    n["change"].as_str().unwrap().into(),
                )
            })
            .collect();
        all.sort();
        all
    };
    assert_eq!(
        changes(&cli),
        [
            ("0:1", "changed"),
            ("0:11", "changed"),
            ("0:13", "removed"),
            ("0:14", "added")
        ]
        .map(|(id, c)| (id.to_owned(), c.to_owned()))
    );
    assert_eq!(cli["coverage"]["complete"], true);
    // One change per MCP page: the cached pair pages through the same list.
    let mut mcp = Mcp::start(&env, &[]);
    let (mut paged, mut cursor) = (Vec::new(), Value::Null);
    loop {
        let mut args = json!({"old": old, "file": new, "limit": 1});
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let reply = mcp.call("diff", args)["structuredContent"].clone();
        paged.extend(changes(&reply));
        cursor = reply["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    paged.sort();
    assert_eq!(paged, changes(&cli));
    // Stateless, like search: no database.
    assert!(!env.home.join(".local/state/figlens").exists());
}
