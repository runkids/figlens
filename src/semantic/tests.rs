use super::*;
use crate::test_support::TestDir;

/// The `fake-embed` example `cargo test` builds next to this test binary.
pub(super) fn fake() -> PathBuf {
    let deps = std::env::current_exe().unwrap();
    let dir = deps.parent().unwrap().parent().unwrap().join("examples");
    dir.join(format!("fake-embed{}", std::env::consts::EXE_SUFFIX))
}

fn start(mode: &str) -> Runtime {
    let mode = format!("mode={mode}");
    Runtime::start(&fake(), &[Path::new(&mode)], &AtomicBool::new(false)).unwrap()
}

fn texts(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

#[test]
fn fuse_sums_reciprocal_ranks_and_breaks_ties_by_key() {
    let fused = fuse(&[&[3, 1][..], &[1, 2][..]]);
    let keys: Vec<i32> = fused.iter().map(|f| f.0).collect();
    assert_eq!(keys, [1, 3, 2]);
    assert!((fused[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
}

#[test]
fn nearest_orders_by_similarity_then_key() {
    let vs = [
        (2u32, vec![1.0, 0.0]),
        (1, vec![1.0, 0.0]),
        (0, vec![0.0, 1.0]),
    ];
    let got = nearest(&[1.0, 0.0], vs.iter().map(|(k, v)| (*k, v.as_slice())), 2).unwrap();
    assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn stored_vectors_round_trip_and_refuse_bad_bytes() {
    let v = vec![0.5f32, -0.25];
    assert_eq!(decode(&encode(&v), 2).unwrap(), v);
    assert!(decode(&encode(&v), 3).is_err());
    assert!(decode(&encode(&[f32::NAN, 0.0]), 2).is_err());
}

#[test]
fn chunks_cover_long_text_to_its_end() {
    let long = "字".repeat(CHUNK_CHARS * 2 + 1);
    let got = chunks("text", &long);
    assert_eq!(
        got.iter()
            .map(|c| c.text.chars().count())
            .collect::<Vec<_>>(),
        [CHUNK_CHARS, CHUNK_CHARS, 1]
    );
    assert!(chunks("name", "  ").is_empty());
}

#[test]
fn pinned_files_refuse_a_changed_file() {
    let dir = TestDir::new();
    std::fs::write(dir.join("w"), "weights").unwrap();
    let digest = crate::search::hex(&Sha256::digest("weights"));
    assert_eq!(
        verify_files(&dir.join(""), &[("w", &digest)])
            .unwrap()
            .len(),
        1
    );
    std::fs::write(dir.join("w"), "other").unwrap();
    assert!(verify_files(&dir.join(""), &[("w", &digest)]).is_err());
}

#[test]
fn runtime_embeds_and_keeps_serving_after_a_refusal() {
    let mut rt = start("");
    let no = AtomicBool::new(false);
    assert!(matches!(
        rt.embed(Kind::Passage, &texts(&[""]), &no),
        Err(Stop::Refused(_))
    ));
    let v = rt.embed(Kind::Query, &texts(&["a", "ab"]), &no).unwrap();
    assert_eq!((v.len(), v[0].len()), (2, 8));
    rt.stop();
}

#[test]
fn runtime_refuses_non_finite_or_wrong_dimension_vectors() {
    for mode in ["nan", "dim"] {
        let mut rt = start(mode);
        let got = rt.embed(Kind::Passage, &texts(&["a"]), &AtomicBool::new(false));
        assert!(
            matches!(got, Err(Stop::Refused(_)) | Err(Stop::Failed(_))),
            "{mode}"
        );
    }
}

#[test]
fn cancel_kills_a_hung_child_and_waits_for_it() {
    let mut rt = start("hang");
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        flag.store(true, Ordering::Relaxed);
    });
    let begun = Instant::now();
    assert!(matches!(
        rt.embed(Kind::Query, &texts(&["a"]), &cancel),
        Err(Stop::Cancelled)
    ));
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert!(!rt.alive());
}

#[test]
fn stop_ends_the_child_when_stdin_closes() {
    let rt = start("");
    let begun = Instant::now();
    rt.stop();
    assert!(begun.elapsed() < EXIT);
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_kills_the_childs_process_group() {
    let dir = TestDir::new();
    let file = dir.join("pid");
    let mut rt = start(&format!("grandchild={}", file.display()));
    let pid: i32 = std::fs::read_to_string(&file).unwrap().parse().unwrap();
    assert!(matches!(
        rt.embed(Kind::Query, &texts(&["a"]), &AtomicBool::new(true)),
        Err(Stop::Cancelled)
    ));
    let gone = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.split_whitespace().nth(2) == Some("Z"))
            .unwrap_or(true)
    };
    let end = Instant::now() + Duration::from_secs(2);
    while !gone() && Instant::now() < end {
        std::thread::sleep(POLL);
    }
    assert!(gone());
}

#[test]
fn profile_id_changes_with_any_model_file() {
    let dir = TestDir::new();
    let write = |body: &str| {
        std::fs::write(dir.join("w"), body).unwrap();
        let digest = crate::search::hex(&Sha256::digest(body));
        Profile::probe(
            &fake(),
            &dir.join(""),
            &[("w", &digest)],
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let a = write("one");
    assert_eq!((a.dim, &a.contract["model"]), (8, &json!("fake")));
    assert_eq!(write("one").id, a.id);
    assert_ne!(write("two").id, a.id);
}
