//! The live viewer's semantic search routes (#94), through a real `figlens preview` process with
//! every XDG root inside a test directory: text stays workspace-free, Enable index builds only
//! the chosen scope, and a save stops the build the viewer owns.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

const FIG: &str = "examples/first-success/design.fig";

struct Viewer {
    child: Child,
    authority: String,
    token: String,
    root: PathBuf,
    design: PathBuf,
}

impl Viewer {
    fn start(root: &Path, env: &[(&str, &Path)]) -> Self {
        let design = root.join("design.fig");
        std::fs::copy(FIG, &design).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_figlens"));
        command
            .args(["preview", design.to_str().unwrap(), "--port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .env_remove("FIGLENS_TEST_HOLD");
        for (var, sub) in [
            ("HOME", "home"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
        ] {
            command.env(var, root.join(sub));
        }
        for (var, value) in env {
            command.env(var, value);
        }
        let mut child = command.spawn().unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        // Keep draining so the viewer never blocks on a full pipe.
        std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
        let url = line.trim().strip_prefix("Preview: http://").unwrap();
        let (authority, token) = url.trim_end_matches('/').split_once('/').unwrap();
        Viewer {
            child,
            authority: authority.to_owned(),
            token: token.to_owned(),
            root: root.to_owned(),
            design,
        }
    }

    fn get(&self, route: &str) -> Value {
        let mut stream = TcpStream::connect(&self.authority).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        write!(
            stream,
            "GET /{}/{route} HTTP/1.1\r\nHost: {}\r\n\r\n",
            self.token, self.authority
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned()))
    }

    /// The page's Enable index: a same-origin JSON POST.
    fn enable(&self, revision: u64, body: &str) -> Value {
        let mut stream = TcpStream::connect(&self.authority).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        write!(
            stream,
            "POST /{}/enable/{revision} HTTP/1.1\r\nHost: {a}\r\nOrigin: http://{a}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.token,
            body.len(),
            a = self.authority
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned()))
    }

    /// Polls `semantic/<revision>?scope=` until the viewer's build has ended.
    fn settled(&self, revision: u64, scope: &str) -> Value {
        let start = Instant::now();
        loop {
            let state = self.get(&format!("semantic/{revision}?scope={scope}"));
            if state["job"]["running"] == false {
                return state;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "{state}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn workspaces(&self) -> bool {
        self.root.join("state/figlens/workspaces").exists()
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn text_search_and_semantic_status_create_no_workspace() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let viewer = Viewer::start(&root, &[]);
    let found = viewer.get("search/1?q=First+success");
    assert_eq!(found["results"][0]["ref"]["nodeId"], "0:1", "{found}");
    let state = viewer.get("semantic/1?scope=0:0");
    assert!(state["error"].is_null(), "{state}");
    assert!(
        state["models"]["setup"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "{state}"
    );
    assert!(state["job"].is_null(), "{state}");
    assert!(!viewer.workspaces(), "a read created a workspace");
    // Semantic search of a file nobody enabled reads no workspace and creates none.
    let semantic = viewer.get("search/1?q=First+success&mode=semantic");
    assert!(!semantic["error"].is_null(), "{semantic}");
    assert!(!viewer.workspaces(), "semantic search created a workspace");
}

#[test]
fn enable_indexes_only_the_chosen_scope_and_modes_follow_the_profile() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let viewer = Viewer::start(&root, &[]);
    // A GET never changes state; a POST names exactly one stored scope of this revision.
    assert!(viewer.get("enable/1?scope=0:1").is_string());
    for (revision, body) in [
        (1, "{}"),
        (1, r#"{"scope":"9:9"}"#),
        (1, r#"{"scope":"0:1","source":"x"}"#),
        (2, r#"{"scope":"0:1"}"#),
    ] {
        assert!(!viewer.enable(revision, body)["error"].is_null(), "{body}");
    }
    assert!(!viewer.workspaces(), "a refused enable created a workspace");
    assert_eq!(viewer.enable(1, r#"{"scope":"0:1"}"#)["started"], true);
    let state = viewer.settled(1, "0:1");
    assert_eq!(state["job"]["report"]["state"], "succeeded", "{state}");
    assert_eq!(state["job"]["report"]["scope"], "0:1", "{state}");
    assert!(viewer.workspaces());
    // No profile is configured: semantic says so, hybrid answers with text only.
    let semantic = viewer.get("search/1?q=First+success&mode=semantic&scope=0:1");
    assert_eq!(semantic["error"]["kind"], "ModeUnavailable", "{semantic}");
    let hybrid = viewer.get("search/1?q=First+success&mode=hybrid&scope=0:1");
    assert_eq!(
        hybrid["query"]["usedModes"],
        serde_json::json!(["text"]),
        "{hybrid}"
    );
    assert_eq!(
        hybrid["source"],
        viewer.get("revision")["source"],
        "{hybrid}"
    );
    assert!(
        hybrid["coverage"]["layers"]["semantic"]["reasons"]
            .to_string()
            .contains("no_profile"),
        "{hybrid}"
    );
    assert!(!viewer.get("search/1?q=x&mode=vector")["error"].is_null());
}

#[cfg(debug_assertions)]
#[test]
fn a_save_cancels_the_build_the_viewer_owns_before_the_new_revision_serves() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let hold = root.join("hold");
    std::fs::write(&hold, "").unwrap();
    let held = PathBuf::from(format!("parse:{}", hold.display()));
    let viewer = Viewer::start(&root, &[("FIGLENS_TEST_HOLD", &held)]);
    assert_eq!(viewer.enable(1, r#"{"scope":"0:0"}"#)["started"], true);
    let reached = PathBuf::from(format!("{}.reached", hold.display()));
    let start = Instant::now();
    while !reached.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the build never reached parse"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let running = viewer.get("semantic/1?scope=0:0");
    assert_eq!(running["job"]["running"], true, "{running}");
    // A save: the next request reloads, which first stops and awaits the held build.
    let bytes = std::fs::read(&viewer.design).unwrap();
    std::fs::write(&viewer.design, &bytes).unwrap();
    let file = std::fs::File::options()
        .write(true)
        .open(&viewer.design)
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(viewer.get("revision")["generation"], 2);
    // The old revision's build ended and the new revision owns none.
    assert!(viewer.get("semantic/2?scope=0:0")["job"].is_null());
    let status = Command::new(env!("CARGO_BIN_EXE_figlens"))
        .args(["index", "status", "--file", viewer.design.to_str().unwrap()])
        .envs([
            ("HOME", root.join("home")),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_CONFIG_HOME", root.join("config")),
        ])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    let jobs = status.to_string();
    assert!(
        !jobs.contains("\"running\"") && !jobs.contains("\"queued\""),
        "{status}"
    );
    std::fs::remove_file(&hold).unwrap();
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn a_signal_cancels_the_build_the_viewer_owns_before_it_exits() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let hold = root.join("hold");
    std::fs::write(&hold, "").unwrap();
    let held = PathBuf::from(format!("parse:{}", hold.display()));
    let mut viewer = Viewer::start(&root, &[("FIGLENS_TEST_HOLD", &held)]);
    assert_eq!(viewer.enable(1, r#"{"scope":"0:0"}"#)["started"], true);
    let reached = PathBuf::from(format!("{}.reached", hold.display()));
    let start = Instant::now();
    while !reached.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the build never reached parse"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let begun = Instant::now();
    let sent = Command::new("kill")
        .args(["-TERM", &viewer.child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let status = viewer.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    let report = Command::new(env!("CARGO_BIN_EXE_figlens"))
        .args(["index", "status", "--file", viewer.design.to_str().unwrap()])
        .envs([
            ("HOME", root.join("home")),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_CONFIG_HOME", root.join("config")),
        ])
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&report.stdout);
    assert!(
        !report.contains("\"running\"") && !report.contains("\"queued\""),
        "{report}"
    );
    std::fs::remove_file(&hold).unwrap();
}

#[test]
fn a_relocated_workspace_serves_the_viewer_under_its_own_source_id() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let cli = |args: &[&str]| -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_figlens"))
            .args(args)
            .envs([
                ("HOME", root.join("home")),
                ("XDG_STATE_HOME", root.join("state")),
                ("XDG_CONFIG_HOME", root.join("config")),
            ])
            .env_remove("FIGLENS_TEST_HOLD")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    // Indexed where it was, then moved to where the viewer opens it.
    let old = root.join("old.fig");
    std::fs::copy(FIG, &old).unwrap();
    let prepared = cli(&["index", "prepare", old.to_str().unwrap(), "--scope", "0:0"]);
    let (ws, source) = (
        prepared["workspaceId"].as_str().unwrap(),
        prepared["job"]["sourceId"].as_str().unwrap().to_owned(),
    );
    let viewer = Viewer::start(&root, &[]);
    std::fs::remove_file(&old).unwrap();
    cli(&[
        "index",
        "relocate",
        &source,
        viewer.design.to_str().unwrap(),
        "--id",
        ws,
    ]);
    assert_eq!(viewer.enable(1, r#"{"scope":"0:0"}"#)["started"], true);
    assert_eq!(
        viewer.settled(1, "0:0")["job"]["report"]["state"],
        "succeeded"
    );
    let hybrid = viewer.get("search/1?q=First+success&mode=hybrid&scope=0:0");
    let mine = viewer.get("revision")["source"]["sourceId"].clone();
    assert_ne!(
        mine, source,
        "the viewer derives its own id from the new path"
    );
    assert_eq!(hybrid["source"]["sourceId"], mine, "{hybrid}");
    // The ref keeps the registered identity, so workspace consumers resolve it.
    let ref_source = hybrid["results"][0]["ref"]["sourceId"].as_str().unwrap();
    assert_eq!(ref_source, source, "{hybrid}");
    let found = cli(&[
        "search",
        "--id",
        ws,
        "--source",
        ref_source,
        "First success",
    ]);
    assert_eq!(
        found["results"][0]["ref"], hybrid["results"][0]["ref"],
        "{found}"
    );
    // Other bytes at the same path: the next revision refuses the index of the old ones.
    let mut bytes = std::fs::read(&viewer.design).unwrap();
    bytes.push(0);
    std::fs::write(&viewer.design, bytes).unwrap();
    let file = std::fs::File::options()
        .write(true)
        .open(&viewer.design)
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(viewer.get("revision")["generation"], 2);
    let stale = viewer.get("search/2?q=First+success&mode=hybrid&scope=0:0");
    // Core reports the index stale (or the viewer, not ready): either way nothing is served.
    assert!(
        !stale["error"].is_null() && stale["results"].is_null(),
        "{stale}"
    );
}

#[cfg(debug_assertions)]
#[test]
fn enable_awaits_a_build_another_process_runs_before_it_ends() {
    let owned = test_support::TestDir::new();
    let root = std::path::absolute(owned.join("")).unwrap();
    let viewer = Viewer::start(&root, &[]);
    // Another process builds the same scope and is held in parse.
    let hold = root.join("hold");
    std::fs::write(&hold, "").unwrap();
    let mut other = Command::new(env!("CARGO_BIN_EXE_figlens"))
        .args([
            "index",
            "prepare",
            viewer.design.to_str().unwrap(),
            "--scope",
            "0:0",
        ])
        .envs([
            ("HOME", root.join("home")),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_CONFIG_HOME", root.join("config")),
        ])
        .env("FIGLENS_TEST_HOLD", format!("parse:{}", hold.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let reached = PathBuf::from(format!("{}.reached", hold.display()));
    let start = Instant::now();
    while !reached.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the other build never reached parse"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(viewer.enable(1, r#"{"scope":"0:0"}"#)["started"], true);
    std::thread::sleep(Duration::from_millis(600));
    let joined = viewer.get("semantic/1?scope=0:0");
    assert_eq!(joined["job"]["running"], true, "{joined}");
    std::fs::remove_file(&hold).unwrap();
    let state = viewer.settled(1, "0:0");
    assert_eq!(state["job"]["report"]["state"], "succeeded", "{state}");
    assert!(other.wait().unwrap().success());
}
