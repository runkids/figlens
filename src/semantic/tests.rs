use super::*;
use crate::test_support::TestDir;

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
    let v = vec![0.6f32, -0.8];
    assert_eq!(decode(&encode(&v), 2).unwrap(), v);
    assert!(decode(&encode(&v), 3).is_err());
    assert!(decode(&encode(&[f32::NAN, 0.0]), 2).is_err());
}

#[test]
fn chunks_cover_long_text_to_its_end() {
    let long = "字".repeat(CHUNK_CHARS * 2 + 1);
    let got: Vec<&str> = chunks(&long).collect();
    assert_eq!(
        got.iter().map(|c| c.chars().count()).collect::<Vec<_>>(),
        [CHUNK_CHARS, CHUNK_CHARS, 1]
    );
    assert!(chunks("  ").next().is_none());
}

#[test]
fn pinned_files_refuse_a_changed_file() {
    let dir = TestDir::new();
    std::fs::write(dir.join("w"), "weights").unwrap();
    let digest = crate::search::hex(&Sha256::digest("weights"));
    assert_eq!(
        verify_files(&dir.join(""), &[("w", &digest)], &AtomicBool::new(false))
            .unwrap()
            .len(),
        1
    );
    std::fs::write(dir.join("w"), "other").unwrap();
    assert!(verify_files(&dir.join(""), &[("w", &digest)], &AtomicBool::new(false)).is_err());
}
