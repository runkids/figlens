//! Token export through the actual CLI error-reporting boundary.
#[path = "../scripts/make-token-fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use std::path::Path;
use std::process::{Command, Output};
use test_support::TestDir;

fn export(source: &Path, mode: &str, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_figlens"))
        .arg("variables")
        .arg(source)
        .args([
            "--format",
            "dtcg",
            "--collection",
            "VariableCollectionId:0:10",
            "--mode",
            mode,
            "--out",
        ])
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn cli_conversion_failure_is_a_json_report_and_preserves_the_output() {
    let dir = TestDir::new();
    let source = dir.join("tokens.fig");
    fixture::write(&source).unwrap();
    let output = dir.join("existing.tokens.json");
    std::fs::write(&output, b"previous export").unwrap();
    let result = export(&source, "missing", &output);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "unknown_mode"
                && d["modeId"] == "missing"
                && d["collectionId"] == "VariableCollectionId:0:10")
    );
    assert_eq!(std::fs::read(output).unwrap(), b"previous export");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_canonical_source_fails_cleanly_without_losing_identity_or_output() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let dir = TestDir::new();
    let source = dir.join("tokens.fig");
    fixture::write(&source).unwrap();
    let original = std::fs::read(&source).unwrap();
    let raw_source = dir
        .join("")
        .join(OsString::from_vec(b"source-\xff.fig".to_vec()));
    std::fs::rename(&source, &raw_source).unwrap();
    symlink(raw_source.canonicalize().unwrap(), &source).unwrap();
    let output = dir.join("existing.tokens.json");
    std::fs::write(&output, b"previous export").unwrap();
    let result = export(&source, "0:11", &output);
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(std::fs::read(output).unwrap(), b"previous export");
    assert_eq!(std::fs::read(raw_source).unwrap(), original);
}
