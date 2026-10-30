//! Process tests for `figlens::semantic`: this binary is also the fake model child it starts
//! (`__child <mode>`), with deterministic bag-of-characters vectors and no model. Modes: `ok`,
//! `nan`, `dim`, `hang` (never answers), `deaf` (never reads a request) and
//! `grandchild=<file>` (starts a sleeper and writes its pid).

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use figlens::semantic::profile;
use figlens::semantic::{self, Kind, MAX_BATCH, MAX_TEXT_BYTES, Mode, Profile, Runtime, Stop};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DIM: usize = 8;

fn child(mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(file) = mode.strip_prefix("grandchild=") {
        let sleeper = std::process::Command::new("sleep").arg("60").spawn()?;
        std::fs::write(file, sleeper.id().to_string())?;
    }
    if mode == "deaf" {
        println!("{}", json!({"protocol": semantic::PROTOCOL, "dim": DIM}));
        std::thread::sleep(Duration::from_secs(3600));
    }
    let hello = json!({"protocol": semantic::PROTOCOL, "dim": DIM, "model": "fake"});
    semantic::serve(&hello, |_, texts| {
        if mode == "hang" {
            std::thread::sleep(Duration::from_secs(3600));
        }
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; if mode == "dim" { DIM + 1 } else { DIM }];
                for c in t.chars() {
                    v[c as usize % DIM] += 1.0;
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.iter_mut()
                    .for_each(|x| *x = if mode == "nan" { f32::NAN } else { *x / n });
                v
            })
            .collect())
    })
}

fn start(mode: &str) -> Runtime {
    let exe = std::env::current_exe().unwrap();
    Runtime::start(
        &exe,
        &[Path::new("__child"), Path::new(mode)],
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn texts(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

fn after(ms: u64) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        flag.store(true, Ordering::Relaxed);
    });
    cancel
}

fn runtime_embeds_and_keeps_serving_after_a_refusal() {
    let mut rt = start("ok");
    let no = AtomicBool::new(false);
    assert!(matches!(
        rt.embed(Kind::Passage, &texts(&[""]), &no),
        Err(Stop::Refused(_))
    ));
    let v = rt.embed(Kind::Query, &texts(&["a", "ab"]), &no).unwrap();
    assert_eq!((v.len(), v[0].len()), (2, DIM));
    rt.stop();
}

fn runtime_refuses_non_finite_or_wrong_dimension_vectors() {
    for mode in ["nan", "dim"] {
        let mut rt = start(mode);
        let got = rt.embed(Kind::Passage, &texts(&["a"]), &AtomicBool::new(false));
        assert!(
            matches!(got, Err(Stop::Refused(_) | Stop::Failed(_))),
            "{mode}: {got:?}"
        );
    }
}

fn cancel_kills_a_hung_child_and_waits_for_it() {
    let mut rt = start("hang");
    let begun = Instant::now();
    let got = rt.embed(Kind::Query, &texts(&["a"]), &after(200));
    assert!(matches!(got, Err(Stop::Cancelled)), "{got:?}");
    assert!(begun.elapsed() < Duration::from_millis(700));
    assert!(!rt.alive());
}

fn cancel_stops_a_child_that_never_reads_a_full_batch() {
    let mut rt = start("deaf");
    // Far more than a pipe buffer, so the write itself blocks.
    let full = vec!["字".repeat(MAX_TEXT_BYTES / 3); MAX_BATCH];
    let begun = Instant::now();
    let got = rt.embed(Kind::Passage, &full, &after(200));
    assert!(matches!(got, Err(Stop::Cancelled)), "{got:?}");
    assert!(begun.elapsed() < Duration::from_millis(700));
    assert!(!rt.alive());
}

fn stop_ends_the_child_when_stdin_closes() {
    let rt = start("ok");
    let begun = Instant::now();
    rt.stop();
    assert!(begun.elapsed() < Duration::from_secs(1));
}

#[cfg(target_os = "linux")]
fn cancel_kills_the_childs_process_group() {
    let dir = test_support::TestDir::new();
    let file = std::path::absolute(dir.join("")).unwrap().join("pid");
    let mut rt = start(&format!("grandchild={}", file.display()));
    let pid: i32 = std::fs::read_to_string(&file).unwrap().parse().unwrap();
    let got = rt.embed(Kind::Query, &texts(&["a"]), &AtomicBool::new(true));
    assert!(matches!(got, Err(Stop::Cancelled)), "{got:?}");
    // Gone, or a zombie waiting for a reaper that is not this test.
    let gone = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.split_whitespace().nth(2) == Some("Z"))
            .unwrap_or(true)
    };
    let end = Instant::now() + Duration::from_secs(2);
    while !gone() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(gone());
}

/// A model directory holding one file, and the pinned file list that matches it.
fn model(dir: &test_support::TestDir, body: &str) -> (PathBuf, Vec<(&'static str, String)>) {
    let model = std::path::absolute(dir.join("")).unwrap().join("model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("w"), body).unwrap();
    let digest = Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (model, vec![("w", digest)])
}

fn probe(dir: &test_support::TestDir, body: &str) -> Profile {
    let (model, files) = model(dir, body);
    let files: Vec<(&str, &str)> = files.iter().map(|(n, d)| (*n, d.as_str())).collect();
    let exe = std::env::current_exe().unwrap();
    Profile::probe(&exe, &model, &files, &AtomicBool::new(false)).unwrap()
}

fn profile_id_changes_with_any_model_file() {
    let dir = test_support::TestDir::new();
    let a = probe(&dir, "one");
    assert_eq!((a.dim, &a.contract["model"]), (DIM, &json!("fake")));
    assert_eq!(probe(&dir, "one").id, a.id);
    assert_ne!(probe(&dir, "two").id, a.id);
}

fn configure_records_a_verified_profile_and_remove_forgets_it() {
    let dir = test_support::TestDir::new();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("")).unwrap().join("config"),
        )
    };
    let storage = figlens::workspace::storage(None).unwrap();
    let (model, files) = model(&dir, "weights");
    let files: &'static [(&str, &str)] = Box::leak(
        files
            .into_iter()
            .map(|(n, d)| (n, &*Box::leak(d.into_boxed_str())))
            .collect(),
    );
    let k = profile::Known {
        name: "fake",
        model: "fake",
        revision: "0",
        license: "MIT",
        files,
    };
    assert!(
        profile::status(&storage).unwrap()["setup"]
            .as_array()
            .is_some_and(|s| !s.is_empty())
    );
    let exe = std::env::current_exe().unwrap();
    let got =
        profile::configure(&storage, &k, &model, Some(&exe), &AtomicBool::new(false)).unwrap();
    let current = profile::active(&storage).unwrap().unwrap();
    assert_eq!(
        got["configured"]["profileId"].as_str(),
        Some(current.id.as_str())
    );
    assert_eq!(
        profile::status(&storage).unwrap()["profiles"][0]["state"],
        "ready"
    );
    assert_eq!(profile::remove(&storage, "fake").unwrap()["removed"], true);
    assert!(profile::active(&storage).unwrap().is_none());
    assert!(model.join("w").is_file());
}

fn configure_refuses_files_that_are_not_the_pinned_model() {
    let dir = test_support::TestDir::new();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("")).unwrap().join("config"),
        )
    };
    let storage = figlens::workspace::storage(None).unwrap();
    let (model, _) = model(&dir, "not the model");
    let exe = std::env::current_exe().unwrap();
    let k = profile::known("bge-m3").unwrap();
    let err =
        profile::configure(&storage, k, &model, Some(&exe), &AtomicBool::new(false)).unwrap_err();
    let kind = err
        .downcast_ref::<figlens::FigError>()
        .map(figlens::FigError::kind);
    assert_eq!(kind, Some(figlens::ErrorKind::InvalidArguments));
    assert!(storage.models().unwrap().is_none());
}

fn warm_runtime_is_reused_across_requests() {
    let dir = test_support::TestDir::new();
    let p = probe(&dir, "weights");
    let no = AtomicBool::new(false);
    let pid = |rt: &mut Runtime| Ok(rt.pid());
    let first = profile::with_runtime(&p, &no, pid).unwrap();
    assert_eq!(profile::with_runtime(&p, &no, pid).unwrap(), first);
}

/// Configures a fake profile in a fresh config root and returns the storage.
fn configured(dir: &test_support::TestDir) -> figlens::workspace::Storage {
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("")).unwrap().join("config"),
        )
    };
    let storage = figlens::workspace::storage(None).unwrap();
    let (model, files) = model(dir, "weights");
    let files: &'static [(&str, &str)] = Box::leak(
        files
            .into_iter()
            .map(|(n, d)| (n, &*Box::leak(d.into_boxed_str())))
            .collect(),
    );
    let k = profile::Known {
        name: "fake",
        model: "fake",
        revision: "0",
        license: "MIT",
        files,
    };
    let exe = std::env::current_exe().unwrap();
    profile::configure(&storage, &k, &model, Some(&exe), &AtomicBool::new(false)).unwrap();
    storage
}

fn request(query: &str, limit: usize, cursor: Option<String>) -> figlens::search::Request {
    figlens::search::Request {
        query: query.into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit,
        cursor,
    }
}

const FIG: &str = "examples/first-success/design.fig";

fn semantic_search_pages_by_meaning_without_text_spans() {
    let dir = test_support::TestDir::new();
    let storage = configured(&dir);
    let no = AtomicBool::new(false);
    let search = |req| {
        figlens::search::semantic::search(Path::new(FIG), &req, Mode::Semantic, &storage, &no)
            .unwrap()
    };
    let first = search(request("task", 1, None));
    assert_eq!(first["query"]["usedModes"], json!(["semantic"]));
    assert_eq!(first["coverage"]["layers"]["semantic"]["state"], "ready");
    let m = &first["results"][0]["matches"][0];
    assert_eq!(
        (m["basis"].as_str(), m.get("spans"), m.get("snippet")),
        (Some("semantic"), None, None)
    );
    let next = search(request(
        "task",
        1,
        first["nextCursor"].as_str().map(str::to_owned),
    ));
    assert_eq!(next["results"][0]["rank"], 2);
    assert_ne!(next["results"][0]["ref"], first["results"][0]["ref"]);
}

fn without_a_profile_hybrid_says_why_and_semantic_refuses() {
    let dir = test_support::TestDir::new();
    let storage = configured(&dir);
    profile::remove(&storage, "fake").unwrap();
    let no = AtomicBool::new(false);
    let path = Path::new(FIG);
    let hybrid = figlens::search::semantic::search(
        path,
        &request("task", 5, None),
        Mode::Hybrid,
        &storage,
        &no,
    )
    .unwrap();
    assert_eq!(hybrid["query"]["usedModes"], json!(["text"]));
    assert_eq!(
        hybrid["coverage"]["layers"]["semantic"]["reasons"][0]["code"],
        "no_profile"
    );
    let err = figlens::search::semantic::search(
        path,
        &request("task", 5, None),
        Mode::Semantic,
        &storage,
        &no,
    )
    .unwrap_err();
    let kind = err
        .downcast_ref::<figlens::FigError>()
        .map(figlens::FigError::kind);
    assert_eq!(kind, Some(figlens::ErrorKind::ModeUnavailable));
}

fn a_changed_model_file_is_refused_before_the_child_starts() {
    let dir = test_support::TestDir::new();
    // Its own weights, so no warm child of an earlier test shares its profile id.
    let p = probe(&dir, "weights to change");
    std::fs::write(
        std::path::absolute(dir.join(""))
            .unwrap()
            .join("model")
            .join("w"),
        "other weights",
    )
    .unwrap();
    let got = profile::with_runtime(&p, &AtomicBool::new(false), |rt| Ok(rt.pid()));
    assert!(
        matches!(&got, Err(Stop::Failed(m)) if m.contains("changed since models configure")),
        "{got:?}"
    );
}

#[cfg(unix)]
fn a_fifo_model_file_is_refused_without_blocking() {
    let dir = test_support::TestDir::new();
    let model = dir.join("model");
    std::fs::create_dir_all(&model).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(model.join("w"))
        .status()
        .unwrap();
    assert!(made.success());
    let exe = std::env::current_exe().unwrap();
    let digest = "0".repeat(64);
    let begun = Instant::now();
    let got = Profile::probe(&exe, &model, &[("w", &digest)], &AtomicBool::new(false));
    assert!(got.is_err_and(|e| e.to_string().contains("not a regular file")));
    assert!(begun.elapsed() < Duration::from_secs(2));
}

/// `figlens mcp` with a profile whose child hangs embedding, one semantic search in flight,
/// then `stop` (a cancel notification, or the end of input) and the reply that comes back.
#[cfg(target_os = "linux")]
fn mcp_semantic_search_stopped_by(cancel: bool) {
    use std::io::{BufRead, BufReader, Write};
    let dir = test_support::TestDir::new();
    configured(&dir);
    let pid_file = std::path::absolute(dir.join("child.pid")).unwrap();
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_figlens"))
        .arg("mcp")
        .env("SEMANTIC_CHILD_MODE", "hang")
        .env("SEMANTIC_CHILD_PID", &pid_file)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    let call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "search",
        "arguments": {"file": std::path::absolute(FIG).unwrap(), "query": "task", "mode": "semantic"}}});
    writeln!(stdin, "{call}").unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while !pid_file.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(25));
    }
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let begun = Instant::now();
    if cancel {
        let note = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}});
        writeln!(stdin, "{note}").unwrap();
        let mut line = String::new();
        BufReader::new(mcp.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.contains("cancelled"), "{line}");
        drop(stdin);
    } else {
        drop(stdin);
    }
    let status = mcp.wait().unwrap();
    assert!(status.success() || !cancel, "{status:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "{:?}",
        begun.elapsed()
    );
    let gone = || {
        std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
            .map(|s| s.split_whitespace().nth(2) == Some("Z"))
            .unwrap_or(true)
    };
    assert!(gone(), "model child {pid} still running");
}

/// `figlens mcp` with a profile whose child hangs embedding and writes its pid to `pids/`.
#[cfg(target_os = "linux")]
fn hanging_mcp(dir: &test_support::TestDir) -> (std::process::Child, PathBuf) {
    configured(dir);
    let pids = std::path::absolute(dir.join("pids")).unwrap();
    std::fs::create_dir_all(&pids).unwrap();
    let mcp = std::process::Command::new(env!("CARGO_BIN_EXE_figlens"))
        .arg("mcp")
        .env("SEMANTIC_CHILD_MODE", "hang")
        .env("SEMANTIC_CHILD_PIDS", &pids)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    (mcp, pids)
}

#[cfg(target_os = "linux")]
fn semantic_call(id: u64) -> serde_json::Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "search",
        "arguments": {"file": std::path::absolute(FIG).unwrap(), "query": "task", "mode": "semantic"}}})
}

/// Waits until `n` model children have written their pids, then returns them.
#[cfg(target_os = "linux")]
fn children(pids: &Path, n: usize) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let found: Vec<String> = std::fs::read_dir(pids)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        if found.len() >= n || Instant::now() > end {
            return found;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "linux")]
fn all_gone(pids: &[String]) -> bool {
    pids.iter().all(|pid| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.split_whitespace().nth(2) == Some("Z"))
            .unwrap_or(true)
    })
}

#[cfg(target_os = "linux")]
fn mcp_cancel_reaches_a_running_call_after_another_lane_answers_its_id() {
    use std::io::{BufRead, BufReader, Write};
    let dir = test_support::TestDir::new();
    let (mut mcp, pids) = hanging_mcp(&dir);
    let mut stdin = mcp.stdin.take().unwrap();
    let mut out = BufReader::new(mcp.stdout.take().unwrap());
    writeln!(stdin, "{}", semantic_call(1)).unwrap();
    let running = children(&pids, 1);
    // A control-lane call reusing id 1 is answered while the search still runs.
    let jobs = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "jobs",
        "arguments": {"action": "status", "jobId": "job_none"}}});
    writeln!(stdin, "{jobs}").unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    assert!(
        line.contains("job_none") || line.contains("isError"),
        "{line}"
    );
    let note =
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}});
    writeln!(stdin, "{note}").unwrap();
    line.clear();
    let begun = Instant::now();
    out.read_line(&mut line).unwrap();
    assert!(line.contains("cancelled"), "{line}");
    assert!(begun.elapsed() < Duration::from_secs(3));
    drop(stdin);
    mcp.wait().unwrap();
    assert!(all_gone(&running));
}

#[cfg(target_os = "linux")]
fn mcp_end_of_input_ends_queued_model_calls_too() {
    use std::io::Write;
    let dir = test_support::TestDir::new();
    let (mut mcp, pids) = hanging_mcp(&dir);
    let mut stdin = mcp.stdin.take().unwrap();
    writeln!(stdin, "{}", semantic_call(1)).unwrap();
    writeln!(stdin, "{}", semantic_call(2)).unwrap();
    children(&pids, 1);
    let begun = Instant::now();
    drop(stdin);
    mcp.wait().unwrap();
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    let started = children(&pids, 0);
    assert_eq!(
        started.len(),
        1,
        "the queued call started a model child after shutdown"
    );
    assert!(all_gone(&started));
}

#[cfg(target_os = "linux")]
fn mcp_cancel_stops_an_in_flight_semantic_search_and_its_child() {
    mcp_semantic_search_stopped_by(true);
}

#[cfg(target_os = "linux")]
fn mcp_end_of_input_stops_an_in_flight_semantic_search_and_its_child() {
    mcp_semantic_search_stopped_by(false);
}

/// Runs the figlens CLI with every XDG root under `dir`, as configured() set config.
fn figlens(dir: &test_support::TestDir, args: &[&str]) -> serde_json::Value {
    let root = std::path::absolute(dir.join("")).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_figlens"))
        .args(args)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn workspace_semantic_search_says_the_layer_is_pending_and_names_the_embed_step() {
    let dir = test_support::TestDir::new();
    configured(&dir);
    let fig = std::path::absolute(dir.join("a.fig")).unwrap();
    std::fs::copy(FIG, &fig).unwrap();
    let fig = fig.to_str().unwrap();
    figlens(&dir, &["index", "prepare", fig, "--scope", "0:0"]);
    let semantic = figlens(
        &dir,
        &["search", "--file", fig, "task", "--mode", "semantic"],
    );
    let layer = &semantic["coverage"]["layers"]["semantic"];
    assert_eq!(
        (layer["state"].as_str(), semantic["nextCursor"].is_null()),
        (Some("pending"), true)
    );
    let steps = semantic["nextSteps"].to_string();
    assert!(steps.contains("\"embed\""), "{steps}");
    // "card" is also a layer name, so the text channel answers while the layer is pending.
    let hybrid = figlens(&dir, &["search", "--file", fig, "card", "--mode", "hybrid"]);
    assert_eq!(hybrid["query"]["usedModes"], json!(["text", "semantic"]));
    assert!(hybrid["results"].as_array().is_some_and(|r| !r.is_empty()));
    assert_eq!(hybrid["coverage"]["complete"], false);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `__child <mode>` from `start`, or `<model-dir>` alone as a configured profile runs it.
    if args.get(1).map(String::as_str) == Some("__child") {
        child(&args[2]).unwrap();
        return;
    }
    if args.len() == 2 && Path::new(&args[1]).is_dir() {
        // Started by figlens as a configured profile: the test chooses its mode and learns its pid.
        if let Ok(file) = std::env::var("SEMANTIC_CHILD_PID") {
            std::fs::write(file, std::process::id().to_string()).unwrap();
        }
        if let Ok(dir) = std::env::var("SEMANTIC_CHILD_PIDS") {
            std::fs::write(Path::new(&dir).join(std::process::id().to_string()), "").unwrap();
        }
        child(&std::env::var("SEMANTIC_CHILD_MODE").unwrap_or("ok".into())).unwrap();
        return;
    }
    let tests: Vec<(&str, fn())> = vec![
        (
            "runtime_embeds_and_keeps_serving_after_a_refusal",
            runtime_embeds_and_keeps_serving_after_a_refusal,
        ),
        (
            "runtime_refuses_non_finite_or_wrong_dimension_vectors",
            runtime_refuses_non_finite_or_wrong_dimension_vectors,
        ),
        (
            "cancel_kills_a_hung_child_and_waits_for_it",
            cancel_kills_a_hung_child_and_waits_for_it,
        ),
        (
            "cancel_stops_a_child_that_never_reads_a_full_batch",
            cancel_stops_a_child_that_never_reads_a_full_batch,
        ),
        (
            "stop_ends_the_child_when_stdin_closes",
            stop_ends_the_child_when_stdin_closes,
        ),
        #[cfg(target_os = "linux")]
        (
            "cancel_kills_the_childs_process_group",
            cancel_kills_the_childs_process_group,
        ),
        (
            "profile_id_changes_with_any_model_file",
            profile_id_changes_with_any_model_file,
        ),
        (
            "configure_records_a_verified_profile_and_remove_forgets_it",
            configure_records_a_verified_profile_and_remove_forgets_it,
        ),
        (
            "configure_refuses_files_that_are_not_the_pinned_model",
            configure_refuses_files_that_are_not_the_pinned_model,
        ),
        (
            "warm_runtime_is_reused_across_requests",
            warm_runtime_is_reused_across_requests,
        ),
        (
            "semantic_search_pages_by_meaning_without_text_spans",
            semantic_search_pages_by_meaning_without_text_spans,
        ),
        (
            "without_a_profile_hybrid_says_why_and_semantic_refuses",
            without_a_profile_hybrid_says_why_and_semantic_refuses,
        ),
        (
            "a_changed_model_file_is_refused_before_the_child_starts",
            a_changed_model_file_is_refused_before_the_child_starts,
        ),
        (
            "workspace_semantic_search_says_the_layer_is_pending_and_names_the_embed_step",
            workspace_semantic_search_says_the_layer_is_pending_and_names_the_embed_step,
        ),
        #[cfg(unix)]
        (
            "a_fifo_model_file_is_refused_without_blocking",
            a_fifo_model_file_is_refused_without_blocking,
        ),
        #[cfg(target_os = "linux")]
        (
            "mcp_cancel_stops_an_in_flight_semantic_search_and_its_child",
            mcp_cancel_stops_an_in_flight_semantic_search_and_its_child,
        ),
        #[cfg(target_os = "linux")]
        (
            "mcp_end_of_input_stops_an_in_flight_semantic_search_and_its_child",
            mcp_end_of_input_stops_an_in_flight_semantic_search_and_its_child,
        ),
        #[cfg(target_os = "linux")]
        (
            "mcp_cancel_reaches_a_running_call_after_another_lane_answers_its_id",
            mcp_cancel_reaches_a_running_call_after_another_lane_answers_its_id,
        ),
        #[cfg(target_os = "linux")]
        (
            "mcp_end_of_input_ends_queued_model_calls_too",
            mcp_end_of_input_ends_queued_model_calls_too,
        ),
    ];
    let filter = args.iter().skip(1).find(|a| !a.starts_with('-'));
    let mut failed = 0;
    for (name, test) in tests {
        if filter.is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        let ok = std::panic::catch_unwind(test).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        failed += usize::from(!ok);
    }
    if failed > 0 {
        eprintln!("{failed} failed");
        std::process::exit(1);
    }
}
