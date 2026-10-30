//! Process tests for `figlens::semantic`: this binary is also the fake model child it starts
//! (`__child <mode>`), with deterministic bag-of-characters vectors and no model. Modes: `ok`,
//! `nan`, `dim`, `hang` (never answers), `deaf` (never reads a request) and
//! `grandchild=<file>` (starts a sleeper and writes its pid).

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use figlens::search::visual::QueryImage;
use figlens::semantic::profile;
use figlens::semantic::{
    self, Kind, MAX_BATCH, MAX_IMAGES, MAX_TEXT_BYTES, Mode, Profile, Runtime, Stop,
};
use resvg::tiny_skia::Pixmap;
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
    // A vision child: one vector per image; `image-hang` never answers, `image-fail` reports a
    // model error, and `image-nan` and `image-dim` answer like `nan` and `dim`.
    if mode.starts_with("image") {
        return semantic::serve_images(&image_hello(), |images| {
            if mode == "image-hang" {
                std::thread::sleep(Duration::from_secs(3600));
            }
            if mode == "image-fail" {
                return Err("the model failed".into());
            }
            Ok(images
                .iter()
                .map(|_| bag("a", mode.trim_start_matches("image-")))
                .collect())
        });
    }
    semantic::serve(&hello(), |_, texts| {
        if mode == "hang" {
            std::thread::sleep(Duration::from_secs(3600));
        }
        Ok(texts.iter().map(|t| bag(t, mode)).collect())
    })
}

fn hello() -> serde_json::Value {
    json!({"protocol": semantic::PROTOCOL, "dim": DIM, "model": "fake"})
}

fn image_hello() -> serde_json::Value {
    json!({"protocol": semantic::PROTOCOL, "dim": DIM, "model": "fake", "modality": "image"})
}

/// The deterministic bag-of-characters vector of `t`.
fn bag(t: &str, mode: &str) -> Vec<f32> {
    let mut v = vec![0f32; if mode == "dim" { DIM + 1 } else { DIM }];
    for c in t.chars() {
        v[c as usize % DIM] += 1.0;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut()
        .for_each(|x| *x = if mode == "nan" { f32::NAN } else { *x / n });
    v
}

/// `__model-worker <dir> <jobId>`, as a configured profile's runtime runs an embed job: the
/// library worker with the fake embedder. `SEMANTIC_EMBED_LOG` gets one line per model load and
/// per batch; `SEMANTIC_CHILD_PIDS` gets this process's pid.
fn model_worker(dir: &str, job: &str) -> bool {
    if let Ok(pids) = std::env::var("SEMANTIC_CHILD_PIDS") {
        std::fs::write(Path::new(&pids).join(std::process::id().to_string()), "").unwrap();
    }
    let log = |line: String| {
        if let Ok(file) = std::env::var("SEMANTIC_EMBED_LOG") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
                .unwrap();
            writeln!(f, "{line}").unwrap();
        }
    };
    let report = figlens::workspace::model::run_embed_worker(Path::new(dir), job, |_| {
        log("load".into());
        let embed: figlens::workspace::model::Embed = Box::new(move |texts: &[String]| {
            log(format!("batch {}", texts.len()));
            if std::env::var_os("SEMANTIC_EMBED_FAIL").is_some() {
                return Err("transient runtime failure".into());
            }
            Ok(texts.iter().map(|t| bag(t, "ok")).collect())
        });
        Ok((hello(), embed))
    });
    println!("{report}");
    !report["error"].is_object()
}

/// `__visual-worker <dir> <jobId>`, as a configured vision profile's runtime runs a visual
/// job: the library worker with a fake image embedder, logging and recording pids like
/// `model_worker`.
fn visual_worker(dir: &str, job: &str) -> bool {
    if let Ok(pids) = std::env::var("SEMANTIC_CHILD_PIDS") {
        std::fs::write(Path::new(&pids).join(std::process::id().to_string()), "").unwrap();
    }
    let log = |line: String| {
        if let Ok(file) = std::env::var("SEMANTIC_EMBED_LOG") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
                .unwrap();
            writeln!(f, "{line}").unwrap();
        }
    };
    let report = figlens::workspace::visual_job::run_visual_worker(Path::new(dir), job, |_| {
        log("load".into());
        let embed: figlens::workspace::visual_job::EmbedImages = Box::new(move |images| {
            log(format!("batch {}", images.len()));
            Ok(images.iter().map(|_| bag("a", "ok")).collect())
        });
        Ok((image_hello(), embed))
    });
    println!("{report}");
    !report["error"].is_object()
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

fn image(shade: u8) -> QueryImage {
    let mut p = Pixmap::new(16, 16).unwrap();
    p.fill(resvg::tiny_skia::Color::from_rgba8(
        shade,
        0,
        255 - shade,
        255,
    ));
    QueryImage::from_pixmap(&p, None).unwrap()
}

fn runtime_embeds_images_and_refuses_bad_batches() {
    let mut rt = start("image");
    let no = AtomicBool::new(false);
    for batch in [vec![], (0..=MAX_IMAGES as u8).map(image).collect()] {
        assert!(matches!(
            rt.embed_images(&batch, &no),
            Err(Stop::Refused(_))
        ));
    }
    // A text request is refused and the vision child keeps serving.
    assert!(matches!(
        rt.embed(Kind::Query, &texts(&["a"]), &no),
        Err(Stop::Refused(_))
    ));
    let v = rt.embed_images(&[image(0), image(200)], &no).unwrap();
    assert_eq!((v.len(), v[0].len()), (2, DIM));
    rt.stop();
}

fn runtime_refuses_non_finite_or_wrong_dimension_image_vectors() {
    for mode in ["image-nan", "image-dim"] {
        let mut rt = start(mode);
        let got = rt.embed_images(&[image(3)], &AtomicBool::new(false));
        assert!(
            matches!(got, Err(Stop::Refused(_) | Stop::Failed(_))),
            "{mode}: {got:?}"
        );
    }
}

/// A model error is transient, not a refusal of the input: the child keeps serving.
fn an_image_model_failure_is_not_a_refusal() {
    let mut rt = start("image-fail");
    let got = rt.embed_images(&[image(4)], &AtomicBool::new(false));
    assert!(matches!(got, Err(Stop::Failed(_))), "{got:?}");
    assert!(rt.alive());
    rt.stop();
}

fn a_text_child_refuses_images_and_keeps_serving() {
    let mut rt = start("ok");
    let no = AtomicBool::new(false);
    assert!(matches!(
        rt.embed_images(&[image(9)], &no),
        Err(Stop::Refused(_))
    ));
    assert_eq!(rt.embed(Kind::Query, &texts(&["a"]), &no).unwrap().len(), 1);
    rt.stop();
}

fn cancel_kills_a_hung_vision_child() {
    let mut rt = start("image-hang");
    let begun = Instant::now();
    let got = rt.embed_images(&[image(1)], &after(200));
    assert!(matches!(got, Err(Stop::Cancelled)), "{got:?}");
    assert!(begun.elapsed() < Duration::from_millis(700));
    assert!(!rt.alive());
}

/// Semantic search takes the first text profile and visual search the first image one,
/// whatever their order; a record from before visual profiles is text.
fn each_search_selects_only_its_own_modality() {
    let dir = test_support::TestDir::new();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("")).unwrap().join("config"),
        )
    };
    let storage = figlens::workspace::storage(None).unwrap();
    let record = |name: &str, modality: Option<&str>| {
        let mut r =
            json!({"name": name, "profileId": name, "model": "m", "runtime": "r", "dim": DIM});
        if let Some(m) = modality {
            r["modality"] = json!(m);
        }
        r
    };
    let both = json!({"version": 1, "profiles": [record("siglip-base", Some("image")), record("bge-m3", None), record("gone", None)]});
    storage.update_models(|_| Ok(both)).unwrap();
    let id = |p: Option<Profile>| p.map(|p| p.id);
    // Rewritten through a profile change, image records move to their own key: a figlens that
    // predates them reads "profiles" only, and finds just the text one.
    profile::remove(&storage, "gone").unwrap();
    storage
        .update_models(|record| {
            let record = record.unwrap();
            assert_eq!(record["profiles"].as_array().map(Vec::len), Some(1));
            assert_eq!(record["visualProfiles"][0]["name"], "siglip-base");
            Ok(record)
        })
        .unwrap();
    assert_eq!(
        id(profile::active(&storage).unwrap()),
        Some("bge-m3".into())
    );
    assert_eq!(
        id(profile::active_visual(&storage).unwrap()),
        Some("siglip-base".into())
    );
    storage
        .update_models(|_| {
            Ok(json!({"version": 1, "profiles": [record("siglip-base", Some("image"))]}))
        })
        .unwrap();
    assert_eq!(id(profile::active(&storage).unwrap()), None);
}

/// The real pinned SigLIP child, only when `FIGLENS_SIGLIP_DIR` names a local copy and
/// `FIGLENS_EMBED_BIN` the built `figlens-embed`: its vectors through the wire match the model
/// run in this process on the same rasters. Not a retrieval-quality check.
#[cfg(feature = "embed")]
fn a_pinned_siglip_child_matches_the_in_process_model() {
    let (Some(dir), Some(exe)) = (
        std::env::var_os("FIGLENS_SIGLIP_DIR"),
        std::env::var_os("FIGLENS_EMBED_BIN"),
    ) else {
        println!("skipped: set FIGLENS_SIGLIP_DIR and FIGLENS_EMBED_BIN");
        return;
    };
    let (dir, exe, no) = (
        PathBuf::from(dir),
        PathBuf::from(exe),
        AtomicBool::new(false),
    );
    let p = Profile::probe(
        &exe,
        &dir,
        profile::known("siglip-base").unwrap().files,
        &no,
    )
    .unwrap();
    assert_eq!(
        (p.dim, p.contract["modality"].as_str()),
        (768, Some("image"))
    );
    let images = [image(0), image(200)];
    // A cold start (re-hash the pinned files, start the child, load the model) is what each
    // switch between a text and an image profile costs in the one warm slot.
    let begun = Instant::now();
    let child = profile::with_runtime(&p, &no, |rt| rt.embed_images(&images, &no)).unwrap();
    let cold = begun.elapsed();
    let begun = Instant::now();
    profile::with_runtime(&p, &no, |rt| rt.embed_images(&images, &no)).unwrap();
    println!(
        "siglip cold start + 2 images {cold:?}; warm 2 images {:?}",
        begun.elapsed()
    );
    let direct = figlens::search::visual::model::Model::load(&dir)
        .unwrap()
        .embed(&images)
        .unwrap();
    let diff = child
        .iter()
        .flatten()
        .zip(direct.iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    println!("siglip child vs in-process: max |diff| {diff:e}");
    assert!(diff < 1e-5, "{diff}");
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
        modality: "text",
        files,
    };
    let empty = profile::status(&storage).unwrap();
    assert!(empty["setup"].as_array().is_some_and(|s| !s.is_empty()));
    let shipped = profile::default_runtime().unwrap();
    assert_eq!(
        (&empty["configRoot"], &empty["defaultRuntime"]),
        (
            &json!(storage.config_root()),
            &json!({"path": shipped, "available": shipped.is_file()})
        )
    );
    assert!(
        storage
            .config_root()
            .starts_with(std::path::absolute(dir.join("")).unwrap().join("config"))
    );
    assert!(
        !storage.config_root().join("models.json").exists(),
        "status wrote the record"
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
    configured_with(dir, "weights")
}

/// `configured` with model file contents `body`: another body is another profile.
fn configured_with(dir: &test_support::TestDir, body: &str) -> figlens::workspace::Storage {
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("")).unwrap().join("config"),
        )
    };
    let storage = figlens::workspace::storage(None).unwrap();
    let (model, files) = model(dir, body);
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
        modality: "text",
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

/// A workspace test home: every XDG root inside `dir`, a fake profile configured (unless
/// `profile` is false) and `FIG` prepared with scope `0:0` by the CLI. Returns the prepare
/// report and the copied file.
fn indexed(dir: &test_support::TestDir, profile: bool) -> (serde_json::Value, PathBuf) {
    indexed_from(dir, profile, Path::new(FIG))
}

/// `indexed` of the design at `source`, copied first.
fn indexed_from(
    dir: &test_support::TestDir,
    profile: bool,
    source: &Path,
) -> (serde_json::Value, PathBuf) {
    let root = std::path::absolute(dir.join("")).unwrap();
    for (var, sub) in [
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
    ] {
        // SAFETY: this harness runs one test at a time on the main thread.
        unsafe { std::env::set_var(var, root.join(sub)) };
    }
    if profile {
        configured(dir);
    }
    let fig = root.join("design.fig");
    std::fs::copy(source, &fig).unwrap();
    let report = cli(&["index", "prepare", fig.to_str().unwrap(), "--scope", "0:0"]);
    (report, fig)
}

fn figlens(args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_figlens"));
    cmd.args(args).env_remove("FIGLENS_TEST_HOLD");
    cmd
}

/// A successful CLI call's JSON report.
fn cli(args: &[&str]) -> serde_json::Value {
    let out = figlens(args).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn workspace_semantic_search_says_the_layer_is_pending_and_names_the_embed_step() {
    let dir = test_support::TestDir::new();
    let (_, fig) = indexed(&dir, true);
    let fig = fig.to_str().unwrap();
    let semantic = cli(&["search", "--file", fig, "task", "--mode", "semantic"]);
    let layer = &semantic["coverage"]["layers"]["semantic"];
    assert_eq!(
        (layer["state"].as_str(), semantic["nextCursor"].is_null()),
        (Some("pending"), true)
    );
    let steps = semantic["nextSteps"].to_string();
    assert!(steps.contains("\"embed\""), "{steps}");
    // "card" is also a layer name, so the text channel answers while the layer is pending.
    let hybrid = cli(&["search", "--file", fig, "card", "--mode", "hybrid"]);
    // No vector exists yet, so only the text channel answered.
    assert_eq!(hybrid["query"]["usedModes"], json!(["text"]));
    assert!(hybrid["results"].as_array().is_some_and(|r| !r.is_empty()));
    assert_eq!(hybrid["coverage"]["complete"], false);
}

fn db(prepared: &serde_json::Value) -> rusqlite::Connection {
    let path = prepared["paths"]["database"].as_str().unwrap();
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `index embed` of scope `0:0` started in the background, held once its layer reads
/// `building`; returns the CLI process and the hold file to remove.
fn held_embed(dir: &test_support::TestDir, ws: &str) -> (std::process::Child, PathBuf) {
    let hold = std::path::absolute(dir.join("hold")).unwrap();
    std::fs::write(&hold, "").unwrap();
    let child = figlens(&["index", "embed", "--id", ws, "--scope", "0:0"])
        .env("FIGLENS_TEST_HOLD", format!("embed:{}", hold.display()))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let reached = hold.with_extension("reached");
    let end = Instant::now() + Duration::from_secs(60);
    while !reached.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(reached.exists(), "the embed worker never reached its hold");
    (child, hold)
}

fn a_transient_model_error_fails_the_embed_without_refusing_texts() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_EMBED_FAIL", "1") };
    let out = figlens(&["index", "embed", "--id", ws, "--scope", "0:0"])
        .output()
        .unwrap();
    unsafe { std::env::remove_var("SEMANTIC_EMBED_FAIL") };
    assert!(String::from_utf8_lossy(&out.stdout).contains("failed") || !out.status.success());
    let conn = db(&prepared);
    assert_eq!(count(&conn, "SELECT count(*) FROM model_vectors"), 0);
    let again = cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    assert_eq!(again["jobs"][0]["state"], "succeeded");
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state = 'ready' AND refused = 0"
        ),
        1
    );
}

fn workspace_semantic_search_pages_a_built_layer() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    let fig = fig.to_str().unwrap();
    let ask = |cursor: Option<&str>| {
        let mut args = vec![
            "search", "--file", fig, "task", "--mode", "semantic", "--limit", "1",
        ];
        args.extend(cursor.map(|c| ["--cursor", c]).into_iter().flatten());
        cli(&args)
    };
    let first = ask(None);
    assert_eq!(first["coverage"]["layers"]["semantic"]["state"], "ready");
    assert_eq!(first["results"][0]["matches"][0]["basis"], "semantic");
    let next = ask(first["nextCursor"].as_str());
    assert_eq!(next["results"][0]["rank"], 2);
    assert_ne!(next["results"][0]["ref"], first["results"][0]["ref"]);
}

fn embed_job_publishes_a_layer_and_later_ones_reuse_its_vectors() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let log = std::path::absolute(dir.join("embed.log")).unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_EMBED_LOG", &log) };
    let first = cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    let job = &first["jobs"][0];
    assert_eq!(
        (job["kind"].as_str(), job["state"].as_str()),
        (Some("embed"), Some("succeeded"))
    );
    let conn = db(&prepared);
    let texts = count(
        &conn,
        "SELECT texts FROM model_layers WHERE state = 'ready'",
    );
    assert!(texts > 0);
    assert_eq!(
        [
            count(&conn, "SELECT embedded FROM model_layers"),
            count(&conn, "SELECT count(*) FROM model_refs"),
            count(&conn, "SELECT count(*) FROM model_vectors"),
        ],
        [texts; 3]
    );
    let loaded = std::fs::read_to_string(&log).unwrap();
    // A rebuild publishes a new generation of the same texts: its layer references the stored
    // vectors and never loads the model.
    cli(&["index", "rebuild", "--id", ws]);
    let second = cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    assert_eq!(second["jobs"][0]["state"], "succeeded");
    assert_ne!(second["jobs"][0]["generation"], job["generation"]);
    assert_eq!(std::fs::read_to_string(&log).unwrap(), loaded);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state = 'ready'"
        ),
        2
    );
    // The next rebuild drops the first generation; the second still references every vector.
    cli(&["index", "rebuild", "--id", ws]);
    assert_eq!(count(&conn, "SELECT count(*) FROM model_vectors"), texts);
    // The one after drops the second; no generation references a vector any more.
    cli(&["index", "rebuild", "--id", ws]);
    assert_eq!(count(&conn, "SELECT count(*) FROM model_vectors"), 0);
    // SAFETY: as above.
    unsafe { std::env::remove_var("SEMANTIC_EMBED_LOG") };
}

fn a_cancelled_configure_reports_cancelled_and_records_nothing() {
    let dir = test_support::TestDir::new();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("config")).unwrap(),
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
        modality: "text",
        model: "fake",
        revision: "0",
        license: "MIT",
        files,
    };
    let exe = std::env::current_exe().unwrap();
    let err =
        profile::configure(&storage, &k, &model, Some(&exe), &AtomicBool::new(true)).unwrap_err();
    let kind = err
        .downcast_ref::<figlens::FigError>()
        .map(figlens::FigError::kind);
    assert_eq!(kind, Some(figlens::ErrorKind::Cancelled));
    assert!(storage.models().unwrap().is_none());
}

fn index_track_does_not_wait_for_a_running_embed() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let (mut owner, hold) = held_embed(&dir, ws);
    let tracked = figlens(&[
        "index",
        "track",
        "--file",
        fig.to_str().unwrap(),
        "--scope",
        "0:0",
    ])
    .output()
    .unwrap();
    std::fs::remove_file(&hold).unwrap();
    owner.wait().unwrap();
    assert!(
        tracked.status.success(),
        "{}",
        String::from_utf8_lossy(&tracked.stderr)
    );
}

fn cancel_of_a_running_embed_kills_its_worker_and_leaves_the_layer_not_ready() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let pids = std::path::absolute(dir.join("pids")).unwrap();
    std::fs::create_dir_all(&pids).unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_CHILD_PIDS", &pids) };
    let (mut owner, hold) = held_embed(&dir, ws);
    unsafe { std::env::remove_var("SEMANTIC_CHILD_PIDS") };
    let conn = db(&prepared);
    let job: String = conn
        .query_row("SELECT job_id FROM jobs WHERE kind = 'embed'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let cancelled = cli(&["jobs", "cancel", &job]);
    assert_eq!(
        (cancelled["cancel"].as_str(), cancelled["state"].as_str()),
        (Some("accepted"), Some("cancelled"))
    );
    #[cfg(target_os = "linux")]
    assert!(all_gone(&children(&pids, 1)));
    // Released after the cancel: a worker still running would now write its layer.
    std::fs::remove_file(&hold).unwrap();
    assert!(!owner.wait().unwrap().success());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state <> 'building'"
        ),
        0
    );
}

fn a_newer_publication_fences_out_a_running_embed() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let (owner, hold) = held_embed(&dir, ws);
    // The text build runs and publishes while the model job still holds its layer.
    let rebuilt = cli(&["index", "rebuild", "--id", ws]);
    assert_eq!(rebuilt["jobs"][0]["state"], "succeeded");
    std::fs::remove_file(&hold).unwrap();
    let out = owner.wait_with_output().unwrap();
    assert!(!out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let job = &report["jobs"][0];
    assert_eq!(job["state"], "failed");
    assert!(
        job["error"]["message"]
            .as_str()
            .unwrap()
            .contains("superseded")
    );
    let conn = db(&prepared);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state <> 'building'"
        ),
        0
    );
}

fn an_mcp_text_build_queues_and_runs_its_embed_on_the_model_lane() {
    use std::io::Write;
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let mut mcp = figlens(&["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "index",
        "arguments": {"action": "refresh", "workspaceId": prepared["workspaceId"]}}});
    let mut stdin = mcp.stdin.take().unwrap();
    writeln!(stdin, "{call}").unwrap();
    let conn = db(&prepared);
    let ready = || {
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state = 'ready'",
        )
    };
    let end = Instant::now() + Duration::from_secs(60);
    while ready() == 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(stdin);
    mcp.wait().unwrap();
    assert_eq!(ready(), 1);
}

/// Configures the fake child as the vision profile.
fn configured_vision(dir: &test_support::TestDir) {
    let storage = figlens::workspace::storage(None).unwrap();
    let (model, files) = model(dir, "vision weights");
    let files: &'static [(&str, &str)] = Box::leak(
        files
            .into_iter()
            .map(|(n, d)| (n, &*Box::leak(d.into_boxed_str())))
            .collect(),
    );
    let k = profile::Known {
        name: "fake-vision",
        modality: "image",
        model: "fake",
        revision: "0",
        license: "MIT",
        files,
    };
    let exe = std::env::current_exe().unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_CHILD_MODE", "image") };
    profile::configure(&storage, &k, &model, Some(&exe), &AtomicBool::new(false)).unwrap();
    unsafe { std::env::remove_var("SEMANTIC_CHILD_MODE") };
}

const VISUAL_POLICY: &str = figlens::workspace::visual::POLICY;

/// `index visual` of scope `0:0` started in the background, held at `stage`; returns the CLI
/// process and the hold file to remove.
fn held_visual(
    dir: &test_support::TestDir,
    ws: &str,
    stage: &str,
) -> (std::process::Child, PathBuf) {
    let hold = std::path::absolute(dir.join("hold")).unwrap();
    std::fs::write(&hold, "").unwrap();
    let child = figlens(&["index", "visual", "--id", ws, "--scope", "0:0"])
        .env("FIGLENS_TEST_HOLD", format!("{stage}:{}", hold.display()))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let reached = hold.with_extension("reached");
    let end = Instant::now() + Duration::from_secs(60);
    while !reached.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(reached.exists(), "the visual worker never reached {stage}");
    (child, hold)
}

fn a_hash_only_visual_job_publishes_a_ready_hash_layer() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let done = cli(&["index", "visual", "--id", ws, "--scope", "0:0"]);
    let job = &done["jobs"][0];
    assert_eq!(
        (job["kind"].as_str(), job["state"].as_str()),
        (Some("visual"), Some("succeeded"))
    );
    let conn = db(&prepared);
    let units = count(
        &conn,
        "SELECT count(*) FROM visual_units WHERE raster_key IS NOT NULL",
    );
    assert!(units > 0);
    let layer: (String, i64, i64) = conn
        .query_row(
            "SELECT state, chunks, embedded FROM model_layers WHERE profile_id = 'visual-hash-1' AND policy = ?1",
            [VISUAL_POLICY],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(layer, ("ready".to_owned(), units, units));
    assert!(count(&conn, "SELECT count(*) FROM visual_rasters") > 0);
}

fn a_vision_visual_job_writes_refs_and_vectors_and_reuses_them_after_a_rebuild() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    configured_vision(&dir);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let log = std::path::absolute(dir.join("visual.log")).unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_EMBED_LOG", &log) };
    let first = cli(&["index", "visual", "--id", ws, "--scope", "0:0"]);
    assert_eq!(first["jobs"][0]["state"], "succeeded");
    let conn = db(&prepared);
    let rasters = count(
        &conn,
        "SELECT count(DISTINCT raster_key) FROM visual_units WHERE raster_key IS NOT NULL",
    );
    assert!(rasters > 0);
    let refs = format!("SELECT count(*) FROM model_refs WHERE policy = '{VISUAL_POLICY}'");
    assert_eq!(
        [
            count(&conn, &refs),
            count(&conn, "SELECT count(*) FROM model_vectors"),
            count(
                &conn,
                &format!(
                    "SELECT count(*) FROM model_layers WHERE policy = '{VISUAL_POLICY}' AND state = 'ready'"
                )
            ),
        ],
        [rasters, rasters, 2]
    );
    let loaded = std::fs::read_to_string(&log).unwrap();
    assert!(loaded.starts_with("load"), "{loaded}");
    // A rebuild publishes a new generation of the same pixels: its layer references the
    // stored vectors and never loads the model.
    let rebuilt = cli(&["index", "rebuild", "--id", ws]);
    let steps = rebuilt["jobs"][0]["nextSteps"].to_string();
    assert!(steps.contains("\"visual\""), "{steps}");
    let second = cli(&["index", "visual", "--id", ws, "--scope", "0:0"]);
    assert_eq!(second["jobs"][0]["state"], "succeeded");
    assert_ne!(
        second["jobs"][0]["generation"],
        first["jobs"][0]["generation"]
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), loaded);
    assert_eq!(count(&conn, &refs), 2 * rasters);
    // SAFETY: as above.
    unsafe { std::env::remove_var("SEMANTIC_EMBED_LOG") };
}

fn cancel_of_a_running_visual_job_kills_its_worker_and_leaves_the_layer_not_ready() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    configured_vision(&dir);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let pids = std::path::absolute(dir.join("pids")).unwrap();
    std::fs::create_dir_all(&pids).unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_CHILD_PIDS", &pids) };
    let (mut owner, hold) = held_visual(&dir, ws, "embed");
    unsafe { std::env::remove_var("SEMANTIC_CHILD_PIDS") };
    let conn = db(&prepared);
    let job: String = conn
        .query_row("SELECT job_id FROM jobs WHERE kind = 'visual'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let cancelled = cli(&["jobs", "cancel", &job]);
    assert_eq!(
        (cancelled["cancel"].as_str(), cancelled["state"].as_str()),
        (Some("accepted"), Some("cancelled"))
    );
    #[cfg(target_os = "linux")]
    assert!(all_gone(&children(&pids, 1)));
    // Released after the cancel: a worker still running would now write its layers.
    std::fs::remove_file(&hold).unwrap();
    assert!(!owner.wait().unwrap().success());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state <> 'building'"
        ),
        0
    );
}

fn a_newer_publication_fences_out_a_running_visual_job() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let (owner, hold) = held_visual(&dir, ws, "render");
    // The text build runs and publishes while the visual job holds its render.
    let rebuilt = cli(&["index", "rebuild", "--id", ws]);
    assert_eq!(rebuilt["jobs"][0]["state"], "succeeded");
    std::fs::remove_file(&hold).unwrap();
    let out = owner.wait_with_output().unwrap();
    assert!(!out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let job = &report["jobs"][0];
    assert_eq!(job["state"], "failed");
    assert!(
        job["error"]["message"]
            .as_str()
            .unwrap()
            .contains("superseded")
    );
    let conn = db(&prepared);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state <> 'building'"
        ),
        0
    );
}

fn a_render_error_is_unit_coverage_and_the_visual_job_succeeds_partial() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let out = figlens(&["index", "visual", "--id", ws, "--scope", "0:0"])
        .env("FIGLENS_TEST_RENDER_FAIL", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["jobs"][0]["state"], "succeeded");
    let conn = db(&prepared);
    let layer: (String, i64) = conn
        .query_row(
            "SELECT state, refused FROM model_layers WHERE profile_id = 'visual-hash-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(layer, ("partial".to_owned(), 1));
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM visual_units WHERE error IS NOT NULL"
        ),
        1
    );
}

fn index_visual_without_a_published_scope_names_the_refresh_step() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let storage = figlens::workspace::storage(None).unwrap();
    let select = figlens::workspace::Select::Id(prepared["workspaceId"].as_str().unwrap().into());
    let err = figlens::workspace::jobs::visual(&storage, &select, None, Some("0:1"), None)
        .err()
        .unwrap();
    let err = err.downcast_ref::<figlens::FigError>().unwrap();
    let step = &err.recovery().unwrap()[0];
    assert_eq!(
        (
            step["arguments"]["action"].as_str(),
            step["arguments"]["scope"].as_str()
        ),
        (Some("refresh"), Some("0:1"))
    );
}

fn an_interrupted_visual_job_is_retried_as_visual() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let (mut owner, hold) = held_visual(&dir, ws, "render");
    // A second caller joins the live job, then its owner dies.
    let joiner = figlens(&["index", "visual", "--id", ws, "--scope", "0:0"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let conn = db(&prepared);
    let end = Instant::now() + Duration::from_secs(60);
    while count(
        &conn,
        "SELECT count(*) FROM jobs WHERE kind = 'visual' AND shared = 1",
    ) == 0
        && Instant::now() < end
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    owner.kill().unwrap();
    owner.wait().unwrap();
    std::fs::remove_file(&hold).unwrap();
    let out = joiner.wait_with_output().unwrap();
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let job = &report["jobs"][0];
    assert_eq!(
        (
            job["kind"].as_str(),
            job["state"].as_str(),
            job["attempt"].as_i64()
        ),
        (Some("visual"), Some("succeeded"), Some(2))
    );
}

fn index_embed_without_a_profile_is_mode_unavailable() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, false);
    let storage = figlens::workspace::storage(None).unwrap();
    let select = figlens::workspace::Select::Id(prepared["workspaceId"].as_str().unwrap().into());
    let err = figlens::workspace::jobs::embed(&storage, &select, None, Some("0:0"), None)
        .err()
        .unwrap();
    let kind = err
        .downcast_ref::<figlens::FigError>()
        .map(figlens::FigError::kind);
    assert_eq!(kind, Some(figlens::ErrorKind::ModeUnavailable));
}

fn a_cli_build_with_a_profile_lists_the_embed_instead_of_queueing_it() {
    let dir = test_support::TestDir::new();
    let (prepared, _) = indexed(&dir, true);
    let steps = prepared["job"]["nextSteps"].as_array().unwrap();
    assert!(
        steps.iter().any(|s| s["arguments"]["action"] == "embed"),
        "{steps:?}"
    );
    assert_eq!(
        count(
            &db(&prepared),
            "SELECT count(*) FROM jobs WHERE kind = 'embed'"
        ),
        0
    );
}

#[path = "../scripts/make-corpus.rs"]
#[allow(dead_code)]
mod corpus;

fn semantic_request(query: &str) -> figlens::search::Request {
    figlens::search::Request {
        query: query.into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit: 5,
        cursor: None,
    }
}

fn refs(reply: &serde_json::Value) -> Vec<serde_json::Value> {
    reply["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"]["nodeId"].clone())
        .collect()
}

fn one_process_keeps_two_workspaces_layers_apart() {
    let dir = test_support::TestDir::new();
    let (first, _) = indexed(&dir, true);
    let other = std::path::absolute(dir.join("corpus.fig")).unwrap();
    corpus::write(other.as_os_str(), false).unwrap();
    let second = cli(&[
        "index",
        "prepare",
        other.to_str().unwrap(),
        "--scope",
        "0:0",
    ]);
    for ws in [&first, &second] {
        cli(&[
            "index",
            "embed",
            "--id",
            ws["workspaceId"].as_str().unwrap(),
            "--scope",
            "0:0",
        ]);
    }
    let storage = figlens::workspace::storage(None).unwrap();
    let no = AtomicBool::new(false);
    let search = |ws: &serde_json::Value| {
        let select = figlens::workspace::Select::Id(ws["workspaceId"].as_str().unwrap().into());
        figlens::workspace::query::search_mode(
            &storage,
            &select,
            &[],
            &semantic_request("task"),
            Mode::Semantic,
            &no,
        )
        .unwrap()
    };
    // The first workspace's layer is cached first; the second must rank its own records.
    search(&first);
    let got = search(&second);
    let alone = figlens::search::semantic::search(
        &other,
        &semantic_request("task"),
        Mode::Semantic,
        &storage,
        &no,
    )
    .unwrap();
    assert_eq!(refs(&got), refs(&alone));
}

fn a_workspace_hybrid_without_a_model_pages_without_a_cursor() {
    let dir = test_support::TestDir::new();
    indexed(&dir, false);
    let other = std::path::absolute(dir.join("corpus.fig")).unwrap();
    corpus::write(other.as_os_str(), false).unwrap();
    let other = other.to_str().unwrap();
    cli(&["index", "prepare", other, "--scope", "0:0"]);
    // Three layers say "chip": a text page of one would have more.
    let got = cli(&[
        "search", "--file", other, "chip", "--mode", "hybrid", "--limit", "1",
    ]);
    assert_eq!(got["query"]["usedModes"], json!(["text"]));
    assert_eq!(
        (got["hasMore"].as_bool(), got["nextCursor"].is_null()),
        (Some(true), true)
    );
    assert_eq!(got["coverage"]["complete"], false);
}

fn the_watch_runner_embeds_a_scope_it_refreshed() {
    use std::io::BufRead;
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    cli(&[
        "index",
        "track",
        "--file",
        fig.to_str().unwrap(),
        "--scope",
        "0:0",
    ]);
    let mut holder = figlens(&[
        "watch",
        "run",
        "--file",
        fig.to_str().unwrap(),
        "--scope",
        "0:0",
    ])
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(holder.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("active"), "{line}");
    // A save the runner refreshes; its publication then needs a model layer.
    let next = std::path::absolute(dir.join("next.fig")).unwrap();
    corpus::write(next.as_os_str(), false).unwrap();
    std::fs::rename(&next, &fig).unwrap();
    let conn = db(&prepared);
    let end = Instant::now() + Duration::from_secs(60);
    let ready = || {
        count(
            &conn,
            "SELECT count(*) FROM model_layers WHERE state = 'ready'",
        ) > 0
    };
    while !ready() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(holder.stdin.take());
    holder.wait().unwrap();
    assert!(
        ready(),
        "the runner left the refreshed scope without a model layer"
    );
}

fn a_schema_8_workspace_reads_its_model_layer_as_pending() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let path = prepared["paths"]["database"].as_str().unwrap();
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch("DROP TABLE model_vectors; DROP TABLE model_refs; DROP TABLE model_layers; PRAGMA user_version = 8;")
        .unwrap();
    let got = cli(&[
        "search",
        "--file",
        fig.to_str().unwrap(),
        "task",
        "--mode",
        "semantic",
    ]);
    assert_eq!(got["coverage"]["layers"]["semantic"]["state"], "pending");
}

fn status_names_a_changed_or_missing_runtime_and_how_to_configure_again() {
    let dir = test_support::TestDir::new();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::path::absolute(dir.join("config")).unwrap(),
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
        modality: "text",
        files,
    };
    let runtime =
        std::path::absolute(dir.join(&format!("runtime{}", std::env::consts::EXE_SUFFIX))).unwrap();
    std::fs::copy(std::env::current_exe().unwrap(), &runtime).unwrap();
    profile::configure(
        &storage,
        &k,
        &model,
        Some(&runtime),
        &AtomicBool::new(false),
    )
    .unwrap();
    let code = || profile::status(&storage).unwrap()["profiles"][0].clone();
    assert_eq!(code()["state"], "ready");
    // An upgrade replaces the runtime.
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&runtime)
        .unwrap()
        .write_all(b"\0")
        .unwrap();
    let changed = code();
    assert_eq!(changed["code"], "runtime_changed");
    assert_eq!(changed["recovery"][0]["arguments"]["action"], "configure");
    std::fs::remove_file(&runtime).unwrap();
    assert_eq!(code()["code"], "runtime_missing");
}

/// The fixture's text `READY` and its siblings share the scope `Examples / First success / Card`;
/// no name or text holds it.
const SCOPE: &str = "Examples / First success / Card";

fn scope_context_alone_finds_a_layer_and_names_the_scope_field() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let storage = figlens::workspace::storage(None).unwrap();
    let no = AtomicBool::new(false);
    let stateless = figlens::search::semantic::search(
        Path::new(FIG),
        &request(SCOPE, 1, None),
        Mode::Semantic,
        &storage,
        &no,
    )
    .unwrap();
    let ws = prepared["workspaceId"].as_str().unwrap();
    cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    let stored = cli(&[
        "search",
        "--file",
        fig.to_str().unwrap(),
        "--mode",
        "semantic",
        "--limit",
        "1",
        "--",
        SCOPE,
    ]);
    for got in [&stateless, &stored] {
        let first = &got["results"][0];
        assert_eq!(
            (&first["ref"]["nodeId"], &first["matches"]),
            (
                &json!("0:3"),
                &json!([{"field": "scope", "chunk": 0, "basis": "semantic"}])
            ),
            "{got}"
        );
        let layer = &got["coverage"]["layers"]["semantic"];
        assert_eq!(
            (&layer["state"], &layer["inputs"]),
            (
                &json!("ready"),
                &json!(["name", "text", "properties", "scope"])
            )
        );
    }
}

fn an_old_policy_layer_is_pending_and_embedding_again_reuses_unchanged_vectors() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let log = std::path::absolute(dir.join("embed.log")).unwrap();
    // SAFETY: this harness runs one test at a time on the main thread.
    unsafe { std::env::set_var("SEMANTIC_EMBED_LOG", &log) };
    cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    // As a name-and-text layer left it: its policy, and no vector of a scope-only text.
    let scope_only = ["Examples / First success", SCOPE];
    let path = prepared["paths"]["database"].as_str().unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "UPDATE model_layers SET policy = 'chunks-400-name-text-1';
         UPDATE model_refs SET policy = 'chunks-400-name-text-1';",
    )
    .unwrap();
    for text in scope_only {
        let hash = Sha256::digest(text).to_vec();
        conn.execute("DELETE FROM model_vectors WHERE text_hash = ?1", [&hash])
            .unwrap();
        conn.execute("DELETE FROM model_refs WHERE text_hash = ?1", [&hash])
            .unwrap();
    }
    let fig = fig.to_str().unwrap();
    let pending = cli(&["search", "--file", fig, "task", "--mode", "semantic"]);
    assert_eq!(
        pending["coverage"]["layers"]["semantic"]["state"],
        "pending"
    );
    assert!(
        pending["nextSteps"].to_string().contains("\"embed\""),
        "{pending}"
    );
    let before = std::fs::read_to_string(&log).unwrap();
    let again = cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    assert_eq!(again["jobs"][0]["state"], "succeeded");
    // Only the two scope texts reach the model; every other chunk keeps its stored vector.
    let after = std::fs::read_to_string(&log).unwrap();
    assert_eq!(&after[before.len()..], "load\nbatch 2\n");
    let ready = cli(&["search", "--file", fig, "task", "--mode", "semantic"]);
    assert_eq!(ready["coverage"]["layers"]["semantic"]["state"], "ready");
    // SAFETY: as above.
    unsafe { std::env::remove_var("SEMANTIC_EMBED_LOG") };
}

fn changes_show_the_model_layer_and_a_profile_change_records_no_revision() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed(&dir, true);
    let ws = prepared["workspaceId"].as_str().unwrap();
    let fig = fig.to_str().unwrap();
    cli(&["index", "track", "--file", fig, "--scope", "0:0"]);
    let conn = db(&prepared);
    let generation = count(&conn, "SELECT gen FROM published WHERE scope = '0:0'");
    let model =
        || cli(&["changes", "--file", fig, "--scope", "0:0"])["tracking"][0]["model"].clone();
    let first = figlens::semantic::profile::active(&figlens::workspace::storage(None).unwrap())
        .unwrap()
        .unwrap()
        .id;
    let policy = figlens::workspace::model::POLICY;
    let layer = |state: &str, profile: &serde_json::Value| json!({"state": state, "profileId": profile, "policy": policy, "generation": generation});
    assert_eq!(model(), layer("pending", &json!(first)));
    cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    assert_eq!(model(), layer("ready", &json!(first)));
    let revisions = count(&conn, "SELECT count(*) FROM revisions");
    // Another profile: its layer is not embedded yet, which is coverage, not a design change.
    let storage = configured_with(&dir, "other weights");
    let second = figlens::semantic::profile::active(&storage)
        .unwrap()
        .unwrap()
        .id;
    assert_ne!(second, first);
    assert_eq!(model(), layer("pending", &json!(second)));
    figlens::semantic::profile::remove(&storage, "fake").unwrap();
    let mut gone = layer("unavailable", &serde_json::Value::Null);
    gone["reason"] = json!("no_profile");
    assert_eq!(model(), gone);
    assert_eq!(count(&conn, "SELECT count(*) FROM revisions"), revisions);
}

/// The mapping fixture with `--properties`, as `cargo test` builds the example: `First button`
/// (0:2) overrides `Label` and `Show icon`, `Second button` (0:3) keeps their defaults.
fn properties_fig(dir: &test_support::TestDir) -> PathBuf {
    let example = Path::new(env!("CARGO_BIN_EXE_figlens"))
        .parent()
        .unwrap()
        .join("examples")
        .join(format!("mapping-fixture{}", std::env::consts::EXE_SUFFIX));
    let path = std::path::absolute(dir.join("properties.fig")).unwrap();
    let made = std::process::Command::new(example)
        .arg("--properties")
        .arg(&path)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    path
}

/// The stored `records.props` of node `id` in the published generation of scope `0:0`.
fn stored_props(conn: &rusqlite::Connection, id: &str) -> Option<String> {
    conn.query_row(
        "SELECT r.props FROM records r JOIN published p ON p.gen = r.gen AND p.scope = '0:0'
         WHERE r.node_id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn an_instance_property_override_is_stored_and_found_with_its_field() {
    let dir = test_support::TestDir::new();
    let (prepared, fig) = indexed_from(&dir, true, &properties_fig(&dir));
    let conn = db(&prepared);
    assert_eq!(
        stored_props(&conn, "0:2").as_deref(),
        Some(r#"[["Label","Pay now"],["Show icon","false"]]"#)
    );
    assert_eq!(
        stored_props(&conn, "0:3").as_deref(),
        Some(r#"[["Label","Button"],["Show icon","true"]]"#)
    );
    // A layer that is no instance is indexed as having none, not as unknown.
    assert_eq!(stored_props(&conn, "0:1").as_deref(), Some("[]"));
    let ws = prepared["workspaceId"].as_str().unwrap();
    cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
    // The fake model's vectors are equal only for equal character bags: the override's line.
    let query = "Label: Pay now; Show icon: false";
    let storage = figlens::workspace::storage(None).unwrap();
    let no = AtomicBool::new(false);
    let stateless = figlens::search::semantic::search(
        &fig,
        &request(query, 1, None),
        Mode::Semantic,
        &storage,
        &no,
    )
    .unwrap();
    let stored = cli(&[
        "search",
        "--file",
        fig.to_str().unwrap(),
        "--mode",
        "semantic",
        "--limit",
        "1",
        "--",
        query,
    ]);
    for got in [&stateless, &stored] {
        let first = &got["results"][0];
        assert_eq!(
            (&first["ref"]["nodeId"], &first["matches"]),
            (
                &json!("0:2"),
                &json!([{"field": "properties", "chunk": 0, "basis": "semantic"}])
            ),
            "{got}"
        );
    }
}

/// A rebuild step for scope `0:0` among `reply`'s next steps, and no embed step.
fn names_rebuild_not_embed(reply: &serde_json::Value) {
    let actions: Vec<&serde_json::Value> = reply["nextSteps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| &s["arguments"]["action"])
        .collect();
    assert!(
        actions.contains(&&json!("rebuild")) && !actions.contains(&&json!("embed")),
        "{reply}"
    );
}

fn older_schema_workspaces_read_without_migrating_and_upgrade_keeping_their_data() {
    for schema in [9, 10] {
        let dir = test_support::TestDir::new();
        let (prepared, fig) = indexed_from(&dir, true, &properties_fig(&dir));
        let ws = prepared["workspaceId"].as_str().unwrap();
        cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
        let path = prepared["paths"]["database"].as_str().unwrap();
        let conn = rusqlite::Connection::open(path).unwrap();
        // As that schema left it: no property column, and before 10 no visual tables.
        conn.execute_batch(
            "ALTER TABLE records DROP COLUMN props;
             INSERT INTO visual_rasters VALUES (x'01', 'p', x'02', x'03');",
        )
        .unwrap();
        if schema == 9 {
            conn.execute_batch(
                "DROP TABLE visual_units; DROP TABLE visual_rasters; DROP INDEX jobs_visual;",
            )
            .unwrap();
        }
        conn.execute_batch(&format!("PRAGMA user_version = {schema};"))
            .unwrap();
        let one = |sql: &str| -> serde_json::Value {
            conn.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0))
                .map(|v| match v {
                    rusqlite::types::Value::Integer(i) => json!(i),
                    rusqlite::types::Value::Text(t) => json!(t),
                    other => json!(format!("{other:?}")),
                })
                .unwrap()
        };
        let kept = |conn_one: &dyn Fn(&str) -> serde_json::Value| {
            [
                "SELECT count(*) FROM sources",
                "SELECT value FROM meta WHERE key = 'incarnation'",
                "SELECT group_concat(name || '|' || ifnull(text, ''), ',') FROM records",
                "SELECT count(*) FROM model_vectors",
                "SELECT count(*) FROM model_layers",
            ]
            .map(conn_one)
        };
        let before = kept(&one);
        let rasters = (schema == 10).then(|| one("SELECT count(*) FROM visual_rasters"));
        let fig = fig.to_str().unwrap();
        // Read-only: text and semantic search answer without the column and migrate nothing.
        let text = cli(&["search", "--file", fig, "button"]);
        assert!(
            text["results"].as_array().is_some_and(|r| !r.is_empty()),
            "{text}"
        );
        let semantic = cli(&["search", "--file", fig, "button", "--mode", "semantic"]);
        let layer = &semantic["coverage"]["layers"]["semantic"]["sources"][0];
        assert_ne!(layer["state"], "ready", "{semantic}");
        assert_eq!(
            layer["reasons"][0]["code"], "properties_not_indexed",
            "{semantic}"
        );
        names_rebuild_not_embed(&semantic);
        assert_eq!(one("PRAGMA user_version"), json!(schema));
        // A write upgrades in place, keeping every row; embedding an index stored before the
        // column still ends partial and names the rebuild, never another embed.
        cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
        assert_eq!(one("PRAGMA user_version"), json!(11));
        assert_eq!(kept(&one), before);
        if let Some(rasters) = rasters {
            assert_eq!(one("SELECT count(*) FROM visual_rasters"), rasters);
        }
        assert_eq!(
            one("SELECT count(*) FROM records WHERE props IS NOT NULL"),
            json!(0)
        );
        let again = cli(&["search", "--file", fig, "button", "--mode", "semantic"]);
        assert_eq!(
            again["coverage"]["layers"]["semantic"]["sources"][0]["state"],
            "partial"
        );
        names_rebuild_not_embed(&again);
        // A refresh that keeps the unchanged generation proposes no embed it cannot finish.
        let refreshed = cli(&["index", "refresh", "--id", ws, "--scope", "0:0"]);
        let proposes_embed = refreshed["nextSteps"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["arguments"]["action"] == "embed");
        assert!(!proposes_embed, "{refreshed}");
        // The rebuild indexes the properties; one embed then completes the layer.
        cli(&["index", "rebuild", "--id", ws, "--scope", "0:0"]);
        cli(&["index", "embed", "--id", ws, "--scope", "0:0"]);
        let ready = cli(&["search", "--file", fig, "button", "--mode", "semantic"]);
        assert_eq!(
            ready["coverage"]["layers"]["semantic"]["state"], "ready",
            "{ready}"
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, worker, dir, job] = args.as_slice()
        && worker == "__model-worker"
    {
        std::process::exit(if model_worker(dir, job) { 0 } else { 1 });
    }
    if let [_, worker, dir, job] = args.as_slice()
        && worker == "__visual-worker"
    {
        std::process::exit(if visual_worker(dir, job) { 0 } else { 1 });
    }
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
    #[allow(unused_mut)]
    let mut tests: Vec<(&str, fn())> = vec![
        (
            "runtime_embeds_images_and_refuses_bad_batches",
            runtime_embeds_images_and_refuses_bad_batches,
        ),
        (
            "a_text_child_refuses_images_and_keeps_serving",
            a_text_child_refuses_images_and_keeps_serving,
        ),
        (
            "cancel_kills_a_hung_vision_child",
            cancel_kills_a_hung_vision_child,
        ),
        (
            "an_image_model_failure_is_not_a_refusal",
            an_image_model_failure_is_not_a_refusal,
        ),
        (
            "runtime_refuses_non_finite_or_wrong_dimension_image_vectors",
            runtime_refuses_non_finite_or_wrong_dimension_image_vectors,
        ),
        (
            "each_search_selects_only_its_own_modality",
            each_search_selects_only_its_own_modality,
        ),
        (
            "an_instance_property_override_is_stored_and_found_with_its_field",
            an_instance_property_override_is_stored_and_found_with_its_field,
        ),
        (
            "older_schema_workspaces_read_without_migrating_and_upgrade_keeping_their_data",
            older_schema_workspaces_read_without_migrating_and_upgrade_keeping_their_data,
        ),
        (
            "scope_context_alone_finds_a_layer_and_names_the_scope_field",
            scope_context_alone_finds_a_layer_and_names_the_scope_field,
        ),
        (
            "an_old_policy_layer_is_pending_and_embedding_again_reuses_unchanged_vectors",
            an_old_policy_layer_is_pending_and_embedding_again_reuses_unchanged_vectors,
        ),
        (
            "changes_show_the_model_layer_and_a_profile_change_records_no_revision",
            changes_show_the_model_layer_and_a_profile_change_records_no_revision,
        ),
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
        (
            "workspace_semantic_search_pages_a_built_layer",
            workspace_semantic_search_pages_a_built_layer,
        ),
        (
            "a_transient_model_error_fails_the_embed_without_refusing_texts",
            a_transient_model_error_fails_the_embed_without_refusing_texts,
        ),
        (
            "one_process_keeps_two_workspaces_layers_apart",
            one_process_keeps_two_workspaces_layers_apart,
        ),
        (
            "a_schema_8_workspace_reads_its_model_layer_as_pending",
            a_schema_8_workspace_reads_its_model_layer_as_pending,
        ),
        (
            "a_workspace_hybrid_without_a_model_pages_without_a_cursor",
            a_workspace_hybrid_without_a_model_pages_without_a_cursor,
        ),
        (
            "the_watch_runner_embeds_a_scope_it_refreshed",
            the_watch_runner_embeds_a_scope_it_refreshed,
        ),
        (
            "index_track_does_not_wait_for_a_running_embed",
            index_track_does_not_wait_for_a_running_embed,
        ),
        (
            "a_cancelled_configure_reports_cancelled_and_records_nothing",
            a_cancelled_configure_reports_cancelled_and_records_nothing,
        ),
        (
            "status_names_a_changed_or_missing_runtime_and_how_to_configure_again",
            status_names_a_changed_or_missing_runtime_and_how_to_configure_again,
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
        (
            "embed_job_publishes_a_layer_and_later_ones_reuse_its_vectors",
            embed_job_publishes_a_layer_and_later_ones_reuse_its_vectors,
        ),
        (
            "cancel_of_a_running_embed_kills_its_worker_and_leaves_the_layer_not_ready",
            cancel_of_a_running_embed_kills_its_worker_and_leaves_the_layer_not_ready,
        ),
        (
            "a_newer_publication_fences_out_a_running_embed",
            a_newer_publication_fences_out_a_running_embed,
        ),
        (
            "an_mcp_text_build_queues_and_runs_its_embed_on_the_model_lane",
            an_mcp_text_build_queues_and_runs_its_embed_on_the_model_lane,
        ),
        (
            "index_embed_without_a_profile_is_mode_unavailable",
            index_embed_without_a_profile_is_mode_unavailable,
        ),
        (
            "a_cli_build_with_a_profile_lists_the_embed_instead_of_queueing_it",
            a_cli_build_with_a_profile_lists_the_embed_instead_of_queueing_it,
        ),
        (
            "a_hash_only_visual_job_publishes_a_ready_hash_layer",
            a_hash_only_visual_job_publishes_a_ready_hash_layer,
        ),
        (
            "a_vision_visual_job_writes_refs_and_vectors_and_reuses_them_after_a_rebuild",
            a_vision_visual_job_writes_refs_and_vectors_and_reuses_them_after_a_rebuild,
        ),
        (
            "cancel_of_a_running_visual_job_kills_its_worker_and_leaves_the_layer_not_ready",
            cancel_of_a_running_visual_job_kills_its_worker_and_leaves_the_layer_not_ready,
        ),
        (
            "a_newer_publication_fences_out_a_running_visual_job",
            a_newer_publication_fences_out_a_running_visual_job,
        ),
        (
            "a_render_error_is_unit_coverage_and_the_visual_job_succeeds_partial",
            a_render_error_is_unit_coverage_and_the_visual_job_succeeds_partial,
        ),
        (
            "index_visual_without_a_published_scope_names_the_refresh_step",
            index_visual_without_a_published_scope_names_the_refresh_step,
        ),
        (
            "an_interrupted_visual_job_is_retried_as_visual",
            an_interrupted_visual_job_is_retried_as_visual,
        ),
    ];
    #[cfg(feature = "embed")]
    tests.push((
        "a_pinned_siglip_child_matches_the_in_process_model",
        a_pinned_siglip_child_matches_the_in_process_model,
    ));
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
