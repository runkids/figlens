//! Watching across real processes (#92 B2): `watch run` holders, the detached runner they share,
//! and what a save, a holder's exit and a second holder do to it.

#[path = "../scripts/make-corpus.rs"]
#[allow(dead_code)]
mod corpus;
#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use test_support::TestDir;

const BIN: &str = env!("CARGO_BIN_EXE_figlens");

/// A corpus file prepared at scope 0:0 and tracked, in its own HOME.
struct Env {
    dir: TestDir,
    home: PathBuf,
    path: PathBuf,
    file: String,
}

impl Env {
    fn new() -> Self {
        let dir = TestDir::new();
        let home = std::fs::canonicalize(dir.join("")).unwrap().join("home");
        std::fs::create_dir(&home).unwrap();
        let path = dir.join("a.fig");
        corpus::write(path.as_os_str(), false).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let file = path.to_str().unwrap().to_owned();
        let env = Self {
            dir,
            home,
            path,
            file,
        };
        env.json(&["index", "prepare", &env.file, "--scope", "0:0"]);
        env.json(&["index", "track", "--file", &env.file, "--scope", "0:0"]);
        env
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env("HOME", &self.home)
            .env("LOCALAPPDATA", &self.home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("FIGLENS_TEST_HOLD");
        command
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.command(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// A `watch run` holder, once it printed its lease.
    fn holder(&self) -> Holder {
        self.holder_with(
            &mut self.command(&["watch", "run", "--file", &self.file, "--scope", "0:0"]),
        )
    }

    /// A holder whose runner's jobs stop at `FIGLENS_TEST_HOLD=<stage>` while `file` exists.
    fn holding(&self, stage: &str, file: &std::path::Path) -> Holder {
        let mut command = self.command(&["watch", "run", "--file", &self.file, "--scope", "0:0"]);
        command.env("FIGLENS_TEST_HOLD", format!("{stage}:{}", file.display()));
        self.holder_with(&mut command)
    }

    fn holder_with(&self, command: &mut Command) -> Holder {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let lease: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(lease["state"], "active", "{line}");
        Holder {
            child,
            stdin,
            out,
            lease,
        }
    }

    fn status(&self) -> Value {
        self.json(&["watch", "status", "--file", &self.file])
    }

    fn changes(&self) -> usize {
        let reply = self.json(&["changes", "--file", &self.file, "--scope", "0:0"]);
        let revisions = reply["revisions"].as_array().unwrap();
        revisions.iter().filter(|r| r["kind"] == "changes").count()
    }

    fn monitor(&self) -> Value {
        self.json(&["changes", "--file", &self.file, "--scope", "0:0"])["monitor"].clone()
    }

    /// Replaces the file, as Figma's save does.
    fn save(&self, revised: bool) {
        let next = self.dir.join("next.fig");
        corpus::write(next.as_os_str(), revised).unwrap();
        std::fs::rename(&next, &self.path).unwrap();
    }

    fn runner_running(&self) -> bool {
        let reply = self.status();
        reply["runner"]["state"] == "running"
    }
}

struct Holder {
    child: Child,
    stdin: Option<ChildStdin>,
    out: BufReader<std::process::ChildStdout>,
    lease: Value,
}

impl Holder {
    /// Closes its stdin, as a host stops it, and returns its last line.
    fn close(mut self) -> Value {
        drop(self.stdin.take());
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        assert!(self.child.wait().unwrap().success());
        serde_json::from_str(&line).unwrap()
    }
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < limit, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_save_while_watched_records_a_revision_without_a_refresh_call() {
    let env = Env::new();
    let holder = env.holder();
    assert_eq!(holder.lease["runner"]["state"], "running");
    env.save(true);
    wait_for("the save's revision", Duration::from_secs(10), || {
        env.changes() == 1
    });
    holder.close();
}

#[test]
fn a_save_interrupts_a_saved_query_turn_and_an_exit_ends_one() {
    let env = Env::new();
    let hold = env.dir.join("turn");
    std::fs::write(&hold, "").unwrap();
    let mut command = env.command(&["watch", "run", "--file", &env.file, "--scope", "0:0"]);
    command.env(
        "FIGLENS_TEST_WATCH",
        format!("turn_holds:{}", hold.display()),
    );
    let holder = env.holder_with(&mut command);
    wait_for(
        "a saved-query turn in flight",
        Duration::from_secs(10),
        || env.dir.join("turn.reached").exists(),
    );
    env.save(true);
    wait_for(
        "the save's revision past the turn",
        Duration::from_secs(10),
        || env.changes() == 1,
    );
    holder.close();
    wait_for(
        "the runner to exit mid-turn",
        Duration::from_secs(10),
        || !env.runner_running(),
    );
}

impl Env {
    /// Saves a search of the tracked scope as a query subscription; returns a reader of its
    /// stored evaluation count and current evaluation state.
    fn saved_query(&self) -> impl Fn() -> (usize, Value) + '_ {
        let changes = self.json(&["changes", "--file", &self.file, "--scope", "0:0"]);
        let source = changes["tracking"][0]["sourceId"]
            .as_str()
            .unwrap()
            .to_owned();
        let reply = self.json(&[
            "watch",
            "subscribe",
            "--file",
            &self.file,
            "--consumer",
            "bot",
            "--source",
            &source,
            "--scope",
            "0:0",
            "--query",
            "badge",
        ]);
        let id = reply["subscription"]["subscriptionId"]
            .as_str()
            .unwrap()
            .to_owned();
        move || {
            let page = self.json(&["changes", "--file", &self.file, "--subscription", &id]);
            let n = page["evaluations"].as_array().map_or(0, Vec::len);
            (n, page["evaluation"]["state"].clone())
        }
    }
}

#[test]
fn the_runner_evaluates_a_saved_query_after_a_save_without_an_evaluate_call() {
    let env = Env::new();
    let evaluations = env.saved_query();
    let (first, _) = evaluations();
    let holder = env.holder();
    env.save(true);
    wait_for("the runner's evaluation", Duration::from_secs(10), || {
        evaluations().0 > first
    });
    holder.close();
}

#[test]
fn the_runner_evaluates_a_saved_query_after_another_process_publishes() {
    let env = Env::new();
    let evaluations = env.saved_query();
    let holder = env.holder();
    // The same bytes rebuilt: the evaluation goes pending, and only an evaluation makes it
    // current again (no new row, since nothing changed).
    env.json(&["index", "rebuild", "--file", &env.file]);
    wait_for("the runner's evaluation", Duration::from_secs(10), || {
        evaluations().1 == "current"
    });
    holder.close();
}

#[test]
fn changes_reports_the_monitor_apart_from_tracking() {
    let env = Env::new();
    assert_eq!(env.monitor()["state"], "inactive");
    let holder = env.holder();
    assert_eq!(env.monitor()["state"], "watching");
    holder.close();
    assert_eq!(env.monitor()["state"], "inactive");
}

#[test]
fn an_mcp_server_holds_its_lease_until_its_input_ends() {
    let env = Env::new();
    let mut server = env
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = server.stdin.take().unwrap();
    let mut output = BufReader::new(server.stdout.take().unwrap());
    let mut call = |id: u64, method: &str, params: Value| -> Value {
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(input, "{line}").unwrap();
        let mut reply = String::new();
        output.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    };
    call(
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    let index = call(
        2,
        "tools/call",
        serde_json::json!({"name": "index", "arguments": {"action": "status", "file": env.file}}),
    );
    let ws = index["result"]["structuredContent"]["workspaceId"].clone();
    let started = call(
        3,
        "tools/call",
        serde_json::json!({"name": "watch",
        "arguments": {"action": "start", "workspaceId": ws, "scope": "0:0"}}),
    );
    let lease = &started["result"]["structuredContent"];
    assert_eq!(
        (lease["state"].clone(), lease["runner"]["state"].clone()),
        ("active".into(), "running".into()),
        "{started}"
    );
    let status = call(
        4,
        "tools/call",
        serde_json::json!({"name": "watch",
        "arguments": {"action": "status", "workspaceId": ws}}),
    );
    assert_eq!(
        status["result"]["structuredContent"]["leases"][0]["mine"],
        true
    );
    drop(input);
    assert!(server.wait().unwrap().success());
    assert_eq!(env.status()["leases"][0]["reason"], "released");
    wait_for("the runner to exit", Duration::from_secs(10), || {
        !env.runner_running()
    });
}

/// An MCP server holding one lease, with its input still open.
#[cfg(unix)]
fn leased_mcp(env: &Env) -> (Child, ChildStdin) {
    let mut server = env
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = server.stdin.take().unwrap();
    let mut output = BufReader::new(server.stdout.take().unwrap());
    let mut call = |id: u64, method: &str, params: Value| -> Value {
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(input, "{line}").unwrap();
        let mut reply = String::new();
        output.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    };
    call(
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    let index = call(
        2,
        "tools/call",
        serde_json::json!({"name": "index", "arguments": {"action": "status", "file": env.file}}),
    );
    let ws = index["result"]["structuredContent"]["workspaceId"].clone();
    let started = call(
        3,
        "tools/call",
        serde_json::json!({"name": "watch",
        "arguments": {"action": "start", "workspaceId": ws, "scope": "0:0"}}),
    );
    assert_eq!(started["result"]["structuredContent"]["state"], "active");
    (server, input)
}

#[cfg(unix)]
fn terminate(server: &Child) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &server.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(unix)]
fn ended(env: &Env) -> (Value, Value) {
    let status = env.status();
    (
        status["leases"][0]["state"].clone(),
        status["leases"][0]["reason"].clone(),
    )
}

#[cfg(unix)]
#[test]
fn an_mcp_server_signalled_before_its_input_ends_releases_its_lease() {
    let env = Env::new();
    let (mut server, input) = leased_mcp(&env);
    // Input stays open: only the signal stops the server, as when a host signals before EOF lands.
    terminate(&server);
    assert_eq!(server.wait().unwrap().code(), Some(143));
    assert_eq!(ended(&env), ("ended".into(), "released".into()));
    drop(input);
}

#[cfg(unix)]
#[test]
fn an_mcp_server_whose_input_ends_as_it_is_signalled_releases_its_lease_once() {
    let env = Env::new();
    let (mut server, input) = leased_mcp(&env);
    drop(input);
    terminate(&server);
    // Either path may finish first; both end the same lease.
    let code = server.wait().unwrap().code();
    assert!(matches!(code, Some(0 | 143)), "{code:?}");
    assert_eq!(ended(&env), ("ended".into(), "released".into()));
}

#[test]
fn the_last_holder_closing_stdin_releases_its_lease_and_the_runner_exits() {
    let env = Env::new();
    let holder = env.holder();
    let lease = holder.lease["leaseId"].clone();
    let last = holder.close();
    assert_eq!(
        (last["leaseId"].clone(), last["reason"].clone()),
        (lease, "released".into())
    );
    wait_for("the runner to exit", Duration::from_secs(10), || {
        !env.runner_running()
    });
    assert_eq!(env.status()["leases"][0]["state"], "ended");
}

#[test]
fn one_holder_leaving_keeps_the_runner_for_the_other() {
    let env = Env::new();
    let (first, second) = (env.holder(), env.holder());
    first.close();
    // Past the runner's two empty lease reads, were it to count the ended lease.
    std::thread::sleep(Duration::from_secs(3));
    assert!(env.runner_running());
    env.save(true);
    wait_for("the save's revision", Duration::from_secs(10), || {
        env.changes() == 1
    });
    second.close();
}

#[test]
fn a_runner_job_another_host_joined_completes_after_the_last_lease_ends() {
    let env = Env::new();
    let hold = env.dir.join("hold");
    std::fs::write(&hold, "").unwrap();
    let holder = env.holding("start", &hold);
    env.save(true);
    let reached = env.dir.join("hold.reached");
    wait_for("the runner's job to hold", Duration::from_secs(10), || {
        reached.exists()
    });
    // Another host joins the runner's job without a requestId and waits for it.
    let joiner = env
        .command(&["index", "refresh", "--file", &env.file, "--scope", "0:0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    holder.close();
    // Past the runner's two empty lease reads, so it has decided to exit.
    std::thread::sleep(Duration::from_secs(3));
    std::fs::remove_file(&hold).unwrap();
    let out = joiner.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(env.changes(), 1);
}

#[test]
fn the_last_lease_ending_cancels_a_runner_job_nobody_joined_within_500_ms() {
    let env = Env::new();
    let hold = env.dir.join("hold");
    std::fs::write(&hold, "").unwrap();
    let holder = env.holding("start", &hold);
    env.save(true);
    let reached = env.dir.join("hold.reached");
    wait_for("the runner's job to hold", Duration::from_secs(10), || {
        reached.exists()
    });
    holder.close();
    let closed = Instant::now();
    wait_for("the runner to exit", Duration::from_secs(10), || {
        !env.runner_running()
    });
    let took = closed.elapsed();
    eprintln!(
        "runner exit with its own job cancelled: {} ms",
        took.as_millis()
    );
    assert!(took < Duration::from_millis(500), "{took:?}");
    // The hold is still in place: the job ended by its cancel, not by finishing.
    assert!(hold.exists());
    assert_eq!(env.changes(), 0);
}

#[test]
fn a_joined_job_whose_owner_dies_is_retried_not_counted_verified() {
    let env = Env::new();
    let holder = env.holder();
    // Another host's refresh is admitted and holds before running its job.
    let hold = env.dir.join("submit");
    std::fs::write(&hold, "").unwrap();
    let mut other = env
        .command(&["index", "refresh", "--file", &env.file, "--scope", "0:0"])
        .env("FIGLENS_TEST_HOLD", format!("submit:{}", hold.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let reached = env.dir.join("submit.reached");
    wait_for("the other host's job", Duration::from_secs(10), || {
        reached.exists()
    });
    // The runner joins that job for this save, then its owner dies before running it.
    env.save(true);
    std::thread::sleep(Duration::from_secs(2));
    other.kill().unwrap();
    other.wait().unwrap();
    wait_for(
        "the retried refresh's revision",
        Duration::from_secs(15),
        || env.changes() == 1,
    );
    holder.close();
}

/// A `watch run` whose runner start fails through `fault`; it must exit with an error and leave
/// its own lease ended, not renewed.
fn failed_start(env: &Env, fault: &str) {
    let out = env
        .command(&["watch", "run", "--file", &env.file, "--scope", "0:0"])
        .env("FIGLENS_TEST_WATCH", fault)
        .env("FIGLENS_TEST_WATCH_READY_MS", "500")
        .stdin(Stdio::piped())
        .output()
        .unwrap();
    assert!(!out.status.success(), "{fault}");
    let status = env.status();
    assert_eq!(
        (
            status["leases"][0]["state"].clone(),
            status["leases"][0]["reason"].clone()
        ),
        ("ended".into(), "start_failed".into()),
        "{fault}: {status}"
    );
    assert_eq!(status["runner"]["state"], "stopped", "{fault}: {status}");
}

/// Every owner file under the test's home, as `watch status` must leave them.
fn owner_files(env: &Env) -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.parent().is_some_and(|p| p.ends_with("owners")) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&env.dir.join(""), &mut out);
    out.sort();
    out
}

#[test]
fn watch_status_from_another_process_creates_and_removes_no_owner_file() {
    let env = Env::new();
    let holder = env.holder();
    let before = owner_files(&env);
    let status = env.status();
    assert_eq!(status["leases"][0]["mine"], false, "{status}");
    assert_eq!(owner_files(&env), before);
    holder.close();
}

#[test]
fn a_source_whose_refresh_fails_is_retried_after_a_backoff_not_every_poll() {
    let env = Env::new();
    let holder = env.holder();
    let database = env.json(&["index", "status", "--file", &env.file])["paths"]["database"]
        .as_str()
        .unwrap()
        .to_owned();
    let jobs = || -> i64 {
        rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM jobs WHERE state = 'failed'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let next = env.dir.join("next.fig");
    std::fs::write(&next, b"not a fig file").unwrap();
    std::fs::rename(&next, &env.path).unwrap();
    wait_for("the first failed refresh", Duration::from_secs(10), || {
        jobs() >= 1
    });
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(jobs(), 1, "retried within the backoff");
    holder.close();
}

#[test]
fn a_watch_run_whose_lease_line_cannot_be_written_ends_its_lease() {
    let env = Env::new();
    let mut child = env
        .command(&["watch", "run", "--file", &env.file, "--scope", "0:0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert!(!child.wait().unwrap().success());
    let status = env.status();
    assert_eq!(
        (
            status["leases"][0]["state"].clone(),
            status["leases"][0]["reason"].clone()
        ),
        ("ended".into(), "released".into()),
        "{status}"
    );
}

#[test]
fn watch_run_accepts_the_dashed_node_id_form_of_its_scope() {
    let env = Env::new();
    let holder =
        env.holder_with(&mut env.command(&["watch", "run", "--file", &env.file, "--scope", "0-0"]));
    assert_eq!(
        (holder.lease["state"].clone(), holder.lease["scope"].clone()),
        ("active".into(), "0:0".into()),
        "{}",
        holder.lease
    );
    holder.close();
}

#[test]
fn untracking_the_last_watched_scope_ends_watch_run_and_the_runner() {
    let env = Env::new();
    let mut holder = env.holder();
    env.json(&["index", "untrack", "--file", &env.file, "--scope", "0:0"]);
    // Its next renewal, within 10 s, finds the lease untracked.
    wait_for("watch run to end", Duration::from_secs(20), || {
        holder.child.try_wait().unwrap().is_some()
    });
    let mut line = String::new();
    holder.out.read_line(&mut line).unwrap();
    let end: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(end["reason"], "untracked", "{end}");
    assert!(holder.child.wait().unwrap().success());
    wait_for("the runner to exit", Duration::from_secs(5), || {
        env.status()["runner"]["state"] == "stopped"
    });
}

/// Two corpus files in one directory workspace, each prepared and tracked at scope 0:0.
struct Pair {
    env: Env,
    dir: String,
    files: [PathBuf; 2],
    sources: [String; 2],
}

impl Pair {
    fn new() -> Self {
        let env = Env::new();
        let dir = env.dir.join("pair");
        std::fs::create_dir(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let files = ["a.fig", "b.fig"].map(|name| {
            let path = dir.join(name);
            corpus::write(path.as_os_str(), false).unwrap();
            path
        });
        let dir = dir.to_str().unwrap().to_owned();
        let sources = files.clone().map(|file| {
            let file = file.to_str().unwrap();
            let prepared = env.json(&[
                "index",
                "prepare",
                file,
                "--workspace",
                &dir,
                "--scope",
                "0:0",
            ]);
            let source = prepared["source"]["sourceId"].as_str().unwrap().to_owned();
            env.json(&[
                "index",
                "track",
                "--workspace",
                &dir,
                "--source",
                &source,
                "--scope",
                "0:0",
            ]);
            source
        });
        Self {
            env,
            dir,
            files,
            sources,
        }
    }

    fn holder(&self, i: usize, hold: Option<(&str, &std::path::Path)>) -> Holder {
        let mut command = self.env.command(&[
            "watch",
            "run",
            "--workspace",
            &self.dir,
            "--source",
            &self.sources[i],
            "--scope",
            "0:0",
        ]);
        if let Some((stage, file)) = hold {
            command.env("FIGLENS_TEST_HOLD", format!("{stage}:{}", file.display()));
        }
        self.env.holder_with(&mut command)
    }

    fn monitor(&self, source: Option<&str>) -> Value {
        let mut args = vec!["changes", "--workspace", &self.dir, "--scope", "0:0"];
        if let Some(source) = source {
            args.extend(["--source", source]);
        }
        self.env.json(&args)["monitor"].clone()
    }

    fn save(&self, i: usize, revised: bool) {
        let next = self.env.dir.join("next.fig");
        corpus::write(next.as_os_str(), revised).unwrap();
        std::fs::rename(&next, &self.files[i]).unwrap();
    }

    /// The first column of `query` over the workspace database.
    fn column(&self, query: &str) -> Vec<String> {
        let database = self
            .env
            .json(&["index", "status", "--workspace", &self.dir])["paths"]["database"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut stmt = db.prepare(query).unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The sources of the `changes` revisions, in publication order.
    fn published(&self) -> Vec<String> {
        self.column("SELECT source_id FROM revisions WHERE kind = 'changes' ORDER BY seq")
    }
}

#[test]
fn a_monitor_over_a_view_with_an_unwatched_tracked_source_is_partial() {
    let pair = Pair::new();
    let holder = pair.holder(1, None);
    wait_for(
        "the watched source's monitor",
        Duration::from_secs(5),
        || pair.monitor(Some(&pair.sources[1]))["state"] == "watching",
    );
    assert_eq!(pair.monitor(None)["state"], "partial");
    assert_eq!(
        pair.monitor(Some(&pair.sources[0])),
        serde_json::json!({"state": "inactive"})
    );
    let other = pair.holder(0, None);
    assert_eq!(pair.monitor(None)["state"], "watching");
    other.close();
    holder.close();
}

#[test]
fn a_source_waiting_for_the_worker_runs_before_another_follow_up_of_a_busy_one() {
    let pair = Pair::new();
    let hold = pair.env.dir.join("compare");
    let a = pair.holder(0, Some(("compare", &hold)));
    let b = pair.holder(1, None);
    // Each newly watched source is verified once first; those refreshes run unheld.
    wait_for("both first verifications", Duration::from_secs(10), || {
        pair.column("SELECT job_id FROM jobs WHERE kind = 'refresh' AND state = 'succeeded'")
            .len()
            == 2
    });
    // Those passed the stage unheld but still marked it reached.
    std::fs::remove_file(pair.env.dir.join("compare.reached")).unwrap();
    std::fs::write(&hold, "").unwrap();
    pair.save(0, true);
    wait_for("the first refresh to hold", Duration::from_secs(10), || {
        pair.env.dir.join("compare.reached").exists()
    });
    // A changes again while its refresh runs, and B is ready behind it.
    pair.save(0, false);
    pair.save(1, true);
    std::thread::sleep(Duration::from_millis(1500));
    std::fs::remove_file(&hold).unwrap();
    wait_for("three publications", Duration::from_secs(30), || {
        pair.published().len() == 3
    });
    let [first, second] = pair.sources.clone();
    assert_eq!(pair.published(), [first.clone(), second, first]);
    b.close();
    a.close();
}

#[test]
fn a_runner_whose_lease_read_fails_still_cancels_its_own_job_and_exits() {
    let env = Env::new();
    let hold = env.dir.join("start");
    let trigger = env.dir.join("fail");
    std::fs::write(&hold, "").unwrap();
    let mut command = env.command(&["watch", "run", "--file", &env.file, "--scope", "0:0"]);
    command
        .env("FIGLENS_TEST_HOLD", format!("start:{}", hold.display()))
        .env(
            "FIGLENS_TEST_WATCH",
            format!("union_fails:{}", trigger.display()),
        );
    let holder = env.holder_with(&mut command);
    env.save(true);
    wait_for("the refresh to hold", Duration::from_secs(10), || {
        env.dir.join("start.reached").exists()
    });
    std::fs::write(&trigger, "").unwrap();
    let database = env.json(&["index", "status", "--file", &env.file])["paths"]["database"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_for(
        "the runner to cancel its held job",
        Duration::from_secs(10),
        || {
            rusqlite::Connection::open_with_flags(
                &database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap()
            .query_row(
                "SELECT count(*) FROM jobs WHERE state = 'cancelled'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        },
    );
    std::fs::remove_file(&trigger).unwrap();
    std::fs::remove_file(&hold).unwrap();
    holder.close();
}

/// An MCP server confined to the test directory whose calls stop at `stage` while `hold` exists.
struct Mcp {
    child: Child,
    input: ChildStdin,
    output: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Mcp {
    fn new(env: &Env, stage: &str, hold: &std::path::Path) -> Self {
        let root = std::fs::canonicalize(env.dir.join("")).unwrap();
        let mut child = env
            .command(&["mcp", "--root", root.to_str().unwrap()])
            .env("FIGLENS_TEST_HOLD", format!("{stage}:{}", hold.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut mcp = Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            next: 0,
        };
        mcp.send(
            "initialize",
            serde_json::json!({"protocolVersion": "2025-06-18",
            "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        );
        mcp.reply();
        mcp
    }

    fn send(&mut self, method: &str, params: Value) {
        self.next += 1;
        let line = serde_json::json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params});
        writeln!(self.input, "{line}").unwrap();
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.send(
            "tools/call",
            serde_json::json!({"name": name, "arguments": arguments}),
        );
        self.reply()["result"].clone()
    }

    fn close(self) {
        drop(self.input);
        let mut child = self.child;
        assert!(child.wait().unwrap().success());
    }
}

/// The test's own connection to `env`'s workspace database.
fn database(env: &Env) -> rusqlite::Connection {
    let status = env.json(&["index", "status", "--file", &env.file]);
    rusqlite::Connection::open(status["paths"]["database"].as_str().unwrap()).unwrap()
}

#[test]
fn a_relocation_racing_a_start_never_authorizes_the_path_outside_the_roots() {
    let env = Env::new();
    let hold = env.dir.join("admit");
    let mut mcp = Mcp::new(&env, "watch_admit", &hold);
    let ws = mcp.tool(
        "index",
        serde_json::json!({"action": "status", "file": env.file}),
    )["structuredContent"]["workspaceId"]
        .clone();
    std::fs::write(&hold, "").unwrap();
    mcp.send(
        "tools/call",
        serde_json::json!({"name": "watch",
        "arguments": {"action": "start", "workspaceId": ws, "scope": "0:0"}}),
    );
    wait_for(
        "the start to choose its source",
        Duration::from_secs(10),
        || env.dir.join("admit.reached").exists(),
    );
    // The source moves out of the server's roots while the start has chosen it; a start that
    // holds the write lock makes the move wait (here, fail at once) until it commits.
    let db = database(&env);
    db.busy_timeout(Duration::ZERO).unwrap();
    let _ = db.execute("UPDATE sources SET path = '/elsewhere.fig'", []);
    std::fs::remove_file(&hold).unwrap();
    let started = mcp.reply()["result"]["structuredContent"].clone();
    assert_eq!(started["state"], "active", "{started}");
    let leases: (i64, i64) = db
        .query_row(
            "SELECT count(*), count(*) FILTER (WHERE authorized_path = '/elsewhere.fig')
             FROM watch_leases",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(leases, (1, 0));
    mcp.close();
}

#[test]
fn watch_status_never_lists_a_lease_of_a_source_that_moved_out_of_view_meanwhile() {
    let env = Env::new();
    let hold = env.dir.join("status");
    let mut mcp = Mcp::new(&env, "watch_status", &hold);
    let ws = mcp.tool(
        "index",
        serde_json::json!({"action": "status", "file": env.file}),
    )["structuredContent"]["workspaceId"]
        .clone();
    std::fs::write(&hold, "").unwrap();
    mcp.send(
        "tools/call",
        serde_json::json!({"name": "watch",
        "arguments": {"action": "status", "workspaceId": ws}}),
    );
    wait_for(
        "status to pick its visible sources",
        Duration::from_secs(10),
        || env.dir.join("status.reached").exists(),
    );
    // The source moves out of view, and a lease on it starts, after status chose what it sees.
    database(&env)
        .execute_batch(
            "UPDATE sources SET path = '/elsewhere.fig';
             INSERT INTO watch_leases (lease_id, holder, source_id, scope, incarnation, epoch,
             fingerprint, renewed_ms, expires_ms)
             SELECT 'hidden', 'other', source_id, '0:0', '', 1, '', 0, 9e15 FROM sources;",
        )
        .unwrap();
    std::fs::remove_file(&hold).unwrap();
    let status = mcp.reply()["result"]["structuredContent"].clone();
    assert_eq!(status["leases"], serde_json::json!([]), "{status}");
    mcp.close();
}

#[test]
fn a_launcher_that_fails_ends_the_lease_its_start_made() {
    failed_start(&Env::new(), "launch_fails");
}

#[test]
fn a_runner_that_never_says_ready_is_killed_and_its_start_lease_ended() {
    failed_start(&Env::new(), "runner_silent");
}

#[test]
fn a_start_while_the_runner_is_closing_waits_for_it_and_starts_the_next() {
    let env = Env::new();
    let closing = env.dir.join("closing");
    std::fs::write(&closing, "").unwrap();
    let first = env.holding("watch_closing", &closing);
    first.close();
    wait_for(
        "the runner to commit closing",
        Duration::from_secs(10),
        || env.dir.join("closing.reached").exists(),
    );
    // The second start holds where it sees the closing runner, so the runner is released only
    // after that start has committed its lease and observed `closing`.
    let seen = env.dir.join("seen");
    std::fs::write(&seen, "").unwrap();
    let second = std::thread::scope(|s| {
        let start = s.spawn(|| env.holding("watch_start_closing", &seen));
        wait_for(
            "the second start to see closing",
            Duration::from_secs(10),
            || env.dir.join("seen.reached").exists(),
        );
        std::fs::remove_file(&closing).unwrap();
        std::fs::remove_file(&seen).unwrap();
        start.join().unwrap()
    });
    assert_eq!(
        second.lease["runner"]["state"], "running",
        "{}",
        second.lease
    );
    env.save(true);
    wait_for("the save's revision", Duration::from_secs(10), || {
        env.changes() == 1
    });
    second.close();
}

#[test]
fn a_start_as_the_last_lease_ends_gets_a_runner_that_serves_it() {
    let env = Env::new();
    let first = env.holder();
    let started = Instant::now();
    first.close();
    // Within the runner's close window: it may be deciding to exit right now.
    let second = env.holder();
    assert_eq!(
        second.lease["runner"]["state"], "running",
        "{}",
        second.lease
    );
    env.save(true);
    wait_for("the save's revision", Duration::from_secs(10), || {
        env.changes() == 1
    });
    second.close();
    let closed = Instant::now();
    wait_for("the runner to exit", Duration::from_secs(10), || {
        !env.runner_running()
    });
    eprintln!(
        "runner exit after the last lease: {} ms (run {} ms)",
        closed.elapsed().as_millis(),
        started.elapsed().as_millis()
    );
}

#[test]
fn a_reset_ends_the_leases_lets_the_runner_exit_and_ends_watch_run() {
    let env = Env::new();
    let mut holder = env.holder();
    let started = Instant::now();
    env.json(&["index", "reset", "--file", &env.file]);
    // It succeeded under the exclusive lock, so the runner (which holds a shared one) is gone.
    eprintln!("reset with a runner: {} ms", started.elapsed().as_millis());
    // Its next renewal finds no lease: it says so and exits on its own.
    let mut line = String::new();
    holder.out.read_line(&mut line).unwrap();
    let end: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(end["state"], "ended", "{line}");
    assert!(holder.child.wait().unwrap().success());
}

#[test]
fn an_unchanged_save_records_no_revision() {
    let env = Env::new();
    let holder = env.holder();
    env.save(false);
    // The runner refreshes after its stability window; the job finds the same bytes.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(env.changes(), 0);
    holder.close();
}
