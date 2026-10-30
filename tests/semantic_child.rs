//! Process tests for `figlens::semantic`: this binary is also the fake model child it starts
//! (`__child <mode>`), with deterministic bag-of-characters vectors and no model. Modes: `ok`,
//! `nan`, `dim`, `hang` (never answers), `deaf` (never reads a request) and
//! `grandchild=<file>` (starts a sleeper and writes its pid).

use figlens::semantic::profile;
use figlens::semantic::{self, Kind, MAX_BATCH, MAX_TEXT_BYTES, Profile, Runtime, Stop};
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

/// A scratch directory under ignored `samples/`, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::path::absolute(format!(
            "samples/semantic-child-{}-{name}",
            std::process::id()
        ))
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    let dir = Dir::new("group");
    let file = dir.0.join("pid");
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
fn model(dir: &Dir, body: &str) -> (PathBuf, Vec<(&'static str, String)>) {
    let model = dir.0.join("model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("w"), body).unwrap();
    let digest = Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (model, vec![("w", digest)])
}

fn probe(dir: &Dir, body: &str) -> Profile {
    let (model, files) = model(dir, body);
    let files: Vec<(&str, &str)> = files.iter().map(|(n, d)| (*n, d.as_str())).collect();
    let exe = std::env::current_exe().unwrap();
    Profile::probe(&exe, &model, &files, &AtomicBool::new(false)).unwrap()
}

fn profile_id_changes_with_any_model_file() {
    let dir = Dir::new("fingerprint");
    let a = probe(&dir, "one");
    assert_eq!((a.dim, &a.contract["model"]), (DIM, &json!("fake")));
    assert_eq!(probe(&dir, "one").id, a.id);
    assert_ne!(probe(&dir, "two").id, a.id);
}

fn configure_records_a_verified_profile_and_remove_forgets_it() {
    let dir = Dir::new("configure");
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.0.join("config")) };
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
    let dir = Dir::new("pinned");
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.0.join("config")) };
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
    let dir = Dir::new("warm");
    let p = probe(&dir, "weights");
    let no = AtomicBool::new(false);
    let pid = |rt: &mut Runtime| Ok(rt.pid());
    let first = profile::with_runtime(&p, &no, pid).unwrap();
    assert_eq!(profile::with_runtime(&p, &no, pid).unwrap(), first);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `__child <mode>` from `start`, or `<model-dir>` alone as a configured profile runs it.
    if args.get(1).map(String::as_str) == Some("__child") {
        child(&args[2]).unwrap();
        return;
    }
    if args.len() == 2 && Path::new(&args[1]).is_dir() {
        child("ok").unwrap();
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
