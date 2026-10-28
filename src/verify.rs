//! Checks a design bundle, zipped or extracted, against its own manifest: which listed files
//! match, which are missing or changed, which assets the bundle could not include, and which
//! font faces only the project and its browser can confirm. It reports; it never blocks.

use std::error::Error;
use std::fs::File;
use std::io::{BufReader, ErrorKind, Read};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

/// Largest JSON file read whole. Real bundles stay far below it; a crafted entry is refused.
const JSON_LIMIT: u64 = 256 << 20;
/// Bytes hashed per bundle. Real bundles stay below it (original images are capped at 256 MiB
/// in total); files whose declared size no longer fits are left unhashed and reported.
const HASH_BUDGET: u64 = 1 << 30;

enum Bundle {
    Zip(zip::ZipArchive<BufReader<File>>),
    /// Canonical directory; listed files must resolve inside it.
    Dir(PathBuf),
}

enum Found<T> {
    File(T),
    Missing,
    /// A directory, or a link resolving outside the bundle: never read.
    Other,
}

impl Bundle {
    fn open(path: &Path) -> Result<Self, Box<dyn Error>> {
        if std::fs::metadata(path)?.is_dir() {
            Ok(Self::Dir(std::fs::canonicalize(path)?))
        } else {
            Ok(Self::Zip(crate::open_zip(path)?))
        }
    }

    fn read<T>(
        &mut self,
        path: &str,
        read: impl FnOnce(&mut dyn Read) -> std::io::Result<T>,
    ) -> Result<Found<T>, Box<dyn Error>> {
        match self {
            Self::Zip(zip) => match zip.by_name(path) {
                Ok(mut file) if file.is_file() => Ok(Found::File(read(&mut file)?)),
                Ok(_) => Ok(Found::Other),
                Err(zip::result::ZipError::FileNotFound) => Ok(Found::Missing),
                Err(e) => Err(e.into()),
            },
            Self::Dir(root) => match std::fs::canonicalize(root.join(path)) {
                Ok(real) if real.starts_with(&*root) && real.is_file() => {
                    Ok(Found::File(read(&mut File::open(real)?)?))
                }
                Ok(_) => Ok(Found::Other),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(Found::Missing),
                Err(e) => Err(e.into()),
            },
        }
    }

    fn json(&mut self, path: &str) -> Result<Option<Json>, Box<dyn Error>> {
        let found = self.read(path, |r| {
            let mut bytes = Vec::new();
            r.take(JSON_LIMIT + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        })?;
        match found {
            Found::File(bytes) if bytes.len() as u64 > JSON_LIMIT => {
                Err(format!("{path} is larger than {JSON_LIMIT} bytes").into())
            }
            Found::File(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?,
            )),
            _ => Ok(None),
        }
    }
}

fn digest(r: &mut dyn Read) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    Ok((size, hex))
}

/// Manifest paths are untrusted: only plain relative paths are looked up.
fn relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

pub fn verify(path: &Path) -> Result<Json, Box<dyn Error>> {
    let mut bundle = Bundle::open(path)?;
    let manifest = bundle
        .json("manifest.json")?
        .filter(|m| m["format"] == "figlens-bundle" && m["version"] == 1)
        .ok_or(
            "not a figlens bundle: manifest.json is missing or not format figlens-bundle version 1",
        )?;
    let (mut verified, mut missing, mut changed, mut invalid) = (0, vec![], vec![], vec![]);
    let (mut budget, mut unhashed) = (HASH_BUDGET, 0);
    for file in manifest["files"].as_array().into_iter().flatten() {
        let listed = file["path"].as_str().unwrap_or("");
        if !relative(listed) {
            invalid.push(file["path"].clone());
            continue;
        }
        // Read one byte past the declared size at most: a larger entry has already changed.
        // Declared sizes are untrusted, so each is reserved from the budget before reading.
        let declared = file["size"].as_u64().unwrap_or(0);
        if declared >= budget {
            unhashed += 1;
            continue;
        }
        budget -= declared;
        let limit = declared + 1;
        let found = match bundle.read(listed, |r| digest(&mut r.take(limit))) {
            Ok(found) => found,
            // A damaged entry is a changed file; the rest of the bundle is still checked.
            Err(e) => {
                changed.push(json!({
                    "path": listed, "expected": {"size": file["size"], "sha256": file["sha256"]},
                    "actual": null, "error": e.to_string(),
                }));
                continue;
            }
        };
        match found {
            Found::File((size, sha256))
                if file["size"] == size && file["sha256"].as_str() == Some(&sha256) =>
            {
                verified += 1
            }
            Found::File((size, sha256)) if size < limit => changed.push(json!({
                "path": listed,
                "expected": {"size": file["size"], "sha256": file["sha256"]},
                "actual": {"size": size, "sha256": sha256},
            })),
            Found::Missing => missing.push(json!(listed)),
            Found::File(_) | Found::Other => changed.push(json!({
                "path": listed, "expected": {"size": file["size"], "sha256": file["sha256"]},
                "actual": null,
            })),
        }
    }
    let mut unverified = vec![];
    if unhashed > 0 {
        unverified.push(format!("{unhashed} listed files were not hashed: their declared sizes exceed the {HASH_BUDGET}-byte verification budget."));
    }
    // A damaged metadata file is already listed as changed; keep the rest of the report.
    let mut metadata = |path: &str, lost: &str| match bundle.json(path) {
        Ok(value) => value,
        Err(e) => {
            unverified.push(format!("{e}; {lost}."));
            None
        }
    };
    let missing_assets =
        metadata("missing-assets.json", "missingAssets is incomplete").unwrap_or(json!([]));
    let mut fonts = match metadata("design/nodes.json", "font faces were not listed") {
        Some(nodes) => crate::bundle::fonts(&nodes["document"])["faces"].take(),
        None => json!([]),
    };
    for face in fonts.as_array_mut().into_iter().flatten() {
        face["status"] = json!("unverified");
    }
    unverified.extend([
        "Font faces: the bundle has no font files and figlens cannot see the project's fonts. Load each face in the running project with document.fonts.load and the design text and check the returned faces; document.fonts.check alone can pass on a fallback.".to_owned(),
        "Browser delivery: check the deployed asset URLs, decoding and rendered dimensions in the running project.".to_owned(),
    ]);
    Ok(json!({
        "path": path,
        "files": {"verified": verified, "missing": missing, "changed": changed, "invalid": invalid},
        "missingAssets": missing_assets,
        "fonts": fonts,
        "unverified": unverified,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::io::Write as _;

    fn fixture_bundle(dir: &TestDir) -> PathBuf {
        let fig = Path::new("examples/first-success/design.fig");
        let mut doc = crate::Document::open(fig).unwrap();
        let message = doc.take_message().unwrap();
        let out = dir.join("bundle.zip");
        crate::bundle::write(&crate::Tree::new(&message), fig, None, None, "0:1", &out).unwrap();
        out
    }

    #[test]
    fn a_fresh_bundle_verifies_with_its_fonts_unverified() {
        let dir = TestDir::new();
        let report = verify(&fixture_bundle(&dir)).unwrap();
        let fonts: Vec<_> = report["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["family"].clone(), f["status"].clone()))
            .collect();
        assert_eq!(
            (
                &report["files"]["missing"],
                &report["files"]["changed"],
                fonts
            ),
            (
                &json!([]),
                &json!([]),
                vec![(json!("Figlens Blocks"), json!("unverified"))]
            )
        );
    }

    #[test]
    fn an_extracted_bundle_reports_changed_and_missing_files() {
        let dir = TestDir::new();
        let extracted = dir.join("extracted");
        crate::open_zip(&fixture_bundle(&dir))
            .unwrap()
            .extract(&extracted)
            .unwrap();
        std::fs::write(extracted.join("design/variables.json"), "{}").unwrap();
        std::fs::write(extracted.join("context.md"), vec![b'x'; 1 << 20]).unwrap();
        std::fs::remove_file(extracted.join("references/screen.png")).unwrap();
        let report = verify(&extracted).unwrap();
        let files = &report["files"];
        let changed: Vec<_> = files["changed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["path"].clone(), c["actual"].is_null()))
            .collect();
        assert_eq!(
            (&files["missing"], changed),
            (
                &json!(["references/screen.png"]),
                vec![
                    (json!("context.md"), true),
                    (json!("design/variables.json"), false)
                ]
            )
        );
    }

    #[test]
    fn manifest_paths_and_links_never_reach_outside_the_bundle() {
        let dir = TestDir::new();
        let bundle = dir.join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(dir.join("outside"), "outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            std::fs::canonicalize(dir.join("outside")).unwrap(),
            bundle.join("link"),
        )
        .unwrap();
        let entry = |path: &str| json!({"path": path, "size": 7, "sha256": "unused"});
        let manifest = json!({"format": "figlens-bundle", "version": 1,
            "files": [entry("../outside"), entry("/outside"), entry("link")]});
        std::fs::write(bundle.join("manifest.json"), manifest.to_string()).unwrap();
        let report = verify(&bundle).unwrap();
        let files = &report["files"];
        assert_eq!(
            (&files["invalid"], &files["verified"]),
            (&json!(["../outside", "/outside"]), &json!(0))
        );
        #[cfg(unix)]
        assert_eq!(files["changed"][0]["actual"], Json::Null);
    }

    #[test]
    fn missing_assets_are_reported_from_the_bundle() {
        let dir = TestDir::new();
        let entry = json!({"key": "img", "nodeId": "1:2", "error": "image not in the .fig"});
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": []});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        std::fs::write(dir.join("missing-assets.json"), json!([entry]).to_string()).unwrap();
        assert_eq!(
            verify(&dir.join("")).unwrap()["missingAssets"],
            json!([entry])
        );
    }

    #[test]
    fn damaged_metadata_keeps_the_integrity_report() {
        let dir = TestDir::new();
        let extracted = dir.join("extracted");
        crate::open_zip(&fixture_bundle(&dir))
            .unwrap()
            .extract(&extracted)
            .unwrap();
        for damaged in ["design/nodes.json", "missing-assets.json"] {
            std::fs::write(extracted.join(damaged), "{").unwrap();
        }
        let report = verify(&extracted).unwrap();
        let changed: Vec<_> = report["files"]["changed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["path"].as_str().unwrap())
            .collect();
        let notes = report["unverified"].to_string();
        assert_eq!(
            (
                changed,
                notes.contains("font faces were not listed"),
                notes.contains("missingAssets is incomplete")
            ),
            (vec!["design/nodes.json", "missing-assets.json"], true, true)
        );
    }

    #[test]
    fn declared_sizes_beyond_the_budget_are_reported_not_hashed() {
        let dir = TestDir::new();
        // Each half fits alone; the second no longer fits once the first is reserved.
        let half = |path| json!({"path": path, "size": HASH_BUDGET / 2, "sha256": "0"});
        let files = json!([half("a.bin"), half("b.bin")]);
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": files});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        std::fs::write(dir.join("a.bin"), "small").unwrap();
        std::fs::write(dir.join("b.bin"), "small").unwrap();
        let report = verify(&dir.join("")).unwrap();
        assert_eq!(
            (
                &report["files"]["changed"][0]["path"],
                &report["files"]["changed"].as_array().unwrap().len(),
                &report["unverified"][0]
            ),
            (
                &json!("a.bin"),
                &1,
                &json!(format!(
                    "1 listed files were not hashed: their declared sizes exceed the {HASH_BUDGET}-byte verification budget."
                ))
            )
        );
    }

    #[test]
    fn an_undecodable_zip_entry_is_reported_and_checking_continues() {
        let dir = TestDir::new();
        let (data, path) = (b"figlens stored entry".as_slice(), dir.join("damaged.zip"));
        let entry = |name| json!({"path": name, "size": data.len(), "sha256": digest(&mut &data[..]).unwrap().1});
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": [entry("a.txt"), entry("b.txt")]});
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("manifest.json", stored).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        for name in ["a.txt", "b.txt"] {
            zip.start_file(name, stored).unwrap();
            zip.write_all(data).unwrap();
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        // Damage a.txt's stored bytes so its CRC check fails while the directory stays readable.
        let at = bytes.windows(data.len()).position(|w| w == data).unwrap();
        bytes[at] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        let report = verify(&path).unwrap();
        let changed = &report["files"]["changed"][0];
        assert_eq!(
            (
                &changed["path"],
                &changed["actual"],
                changed["error"].is_string(),
                &report["files"]["verified"]
            ),
            (&json!("a.txt"), &Json::Null, true, &json!(1))
        );
    }

    #[test]
    fn a_directory_without_a_bundle_manifest_is_an_error() {
        let dir = TestDir::new();
        assert!(verify(&dir.join("")).is_err());
    }
}
