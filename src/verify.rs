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
    let missing_assets = metadata("missing-assets.json", "missingAssets is incomplete");
    let nodes = metadata(
        "design/nodes.json",
        "font faces were not listed and render, semantic and variable findings are incomplete",
    );
    let variables = metadata("design/variables.json", "variable findings are incomplete");
    // Optional: absent when the bundle was made without states.
    let states = metadata(
        "design/states.json",
        "state inventory coverage was not checked",
    );
    let mut fonts = match &nodes {
        Some(nodes) => crate::bundle::fonts(&nodes["document"])["faces"].take(),
        None => json!([]),
    };
    for face in fonts.as_array_mut().into_iter().flatten() {
        face["status"] = json!("unverified");
    }
    let mut files =
        json!({"verified": verified, "missing": missing, "changed": changed, "invalid": invalid});
    let unexamined: Vec<_> = [
        ("missing-assets.json", missing_assets.is_none()),
        ("design/nodes.json", nodes.is_none()),
        ("design/variables.json", variables.is_none()),
    ]
    .into_iter()
    .filter_map(|(path, absent)| absent.then_some(path))
    .collect();
    let mut missing_assets = missing_assets.unwrap_or(json!([]));
    let mut all = findings(&Inputs {
        files: &files,
        unhashed,
        missing_assets: &missing_assets,
        nodes: nodes.as_ref(),
        variables: variables.as_ref(),
        states: states.as_ref(),
        fonts: &fonts,
    });
    unverified.extend([
        "Font faces: the bundle has no font files and figlens cannot see the project's fonts. Load each face in the running project with document.fonts.load and the design text and check the returned faces; document.fonts.check alone can pass on a fallback.".to_owned(),
        "Browser delivery: check the deployed asset URLs, decoding and rendered dimensions in the running project.".to_owned(),
    ]);
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for f in &all {
        *counts
            .entry(format!(
                "{}/{}",
                f["category"].as_str().unwrap(),
                f["status"].as_str().unwrap()
            ))
            .or_default() += 1;
    }
    let total = all.len();
    fit(&mut all, MAX_FINDINGS, FINDINGS_BYTES);
    // Every other list also comes from the bundle; each keeps its total when cut.
    let mut truncated = serde_json::Map::new();
    let mut cap = |name: &str, list: &mut Json| {
        if let Some(total) = list
            .as_array_mut()
            .and_then(|a| fit(a, usize::MAX, LIST_BYTES))
        {
            truncated.insert(name.into(), total.into());
        }
    };
    for key in ["missing", "changed", "invalid"] {
        cap(&format!("files.{key}"), &mut files[key]);
    }
    cap("missingAssets", &mut missing_assets);
    cap("fonts", &mut fonts);
    let short = |v: &Json| (v.to_string().len() <= 1024).then(|| v.clone());
    let summary = json!({
        "scope": {"nodeId": short(&manifest["root"]["id"]), "name": short(&manifest["root"]["name"])},
        "total": total,
        "returned": all.len(),
        "truncated": all.len() < total,
        "counts": counts,
        "unexamined": unexamined,
        // Once here, not per semantic finding: an untrusted coverage value would be copied per item.
        "semanticsCoverage": nodes.as_ref().and_then(|n| short(&n["_figlens"]["handoffDiagnostics"]["coverage"])),
        "coverage": "Only checks recorded in this bundle: manifest integrity, missing-assets.json, design/variables.json, design/states.json truncation, node _figlens.renderDiagnostics, handoffDiagnostics and font faces. Few or no findings do not establish complete semantics, font readiness, browser delivery or pixel fidelity.",
    });
    Ok(json!({
            "path": path,
            "files": files,
            "missingAssets": missing_assets,
            "fonts": fonts,
            "unverified": unverified,
            "findings": all,
            "findingsSummary": summary,
            "truncated": truncated,
    }))
}

/// Findings returned at most; `findingsSummary` keeps the complete counts.
const MAX_FINDINGS: usize = 500;
/// Serialized bytes returned at most for findings and for each other list. MCP sends the report
/// as structuredContent and again as escaped text, up to three times these bytes, so together
/// (2.25 MiB) they stay inside the 8 MiB reply.
const FINDINGS_BYTES: usize = 1 << 20;
const LIST_BYTES: usize = 256 << 10;

/// Keeps the leading entries of `list` within `max` entries and `budget` serialized bytes;
/// returns the original length when any were cut.
fn fit(list: &mut Vec<Json>, max: usize, budget: usize) -> Option<usize> {
    let (mut kept, mut bytes) = (0, 0usize);
    for v in list.iter().take(max) {
        bytes = bytes.saturating_add(serde_json::to_vec(v).map_or(usize::MAX, |v| v.len()));
        if bytes > budget {
            break;
        }
        kept += 1;
    }
    let total = list.len();
    list.truncate(kept);
    (kept < total).then_some(total)
}
/// Node ids kept per finding; `nodeCount` keeps the total.
const MAX_NODE_IDS: usize = 20;

struct Inputs<'a> {
    files: &'a Json,
    unhashed: usize,
    missing_assets: &'a Json,
    nodes: Option<&'a Json>,
    variables: Option<&'a Json>,
    states: Option<&'a Json>,
    fonts: &'a Json,
}

fn items(v: &Json) -> impl Iterator<Item = &Json> {
    v.as_array().into_iter().flatten()
}

/// One handoff finding. Design strings stay in `evidence`; `impact` and `action` are fixed text.
#[allow(clippy::too_many_arguments)]
fn finding(
    category: &str,
    status: &str,
    source: &str,
    node_ids: &[Json],
    node_count: usize,
    evidence: Json,
    impact: &str,
    action: &str,
) -> Json {
    json!({
        "category": category, "status": status, "source": source,
        "nodeIds": &node_ids[..node_ids.len().min(MAX_NODE_IDS)], "nodeCount": node_count,
        "evidence": evidence, "impact": impact, "action": action,
    })
}

/// Render diagnostics in document order and the nodes binding each id already in `bound`.
fn walk(
    node: &Json,
    render: &mut Vec<Json>,
    bound: &mut std::collections::BTreeMap<String, Vec<Json>>,
) {
    render.extend(items(&node["_figlens"]["renderDiagnostics"]).cloned());
    if !bound.is_empty() {
        for (key, value) in node.as_object().into_iter().flatten() {
            if key != "children" {
                bind(value, &node["id"], bound);
            }
        }
    }
    for child in items(&node["children"]) {
        walk(child, render, bound);
    }
}

/// Records `node` once under each id in `bound` that a `VARIABLE_ALIAS` under `v` names.
/// Map lookups only: alias lists in an untrusted bundle can be huge.
fn bind(v: &Json, node: &Json, bound: &mut std::collections::BTreeMap<String, Vec<Json>>) {
    match v {
        Json::Array(a) => a.iter().for_each(|x| bind(x, node, bound)),
        Json::Object(o) => {
            if o.get("type").and_then(Json::as_str) == Some("VARIABLE_ALIAS")
                && let Some(nodes) = o
                    .get("id")
                    .and_then(Json::as_str)
                    .and_then(|id| bound.get_mut(id))
                && nodes.last() != Some(node)
            {
                nodes.push(node.clone());
            }
            o.values().for_each(|x| bind(x, node, bound));
        }
        _ => {}
    }
}

/// Missing inputs first, then known limitations, then checks only the project can make.
fn findings(i: &Inputs) -> Vec<Json> {
    let mut out = vec![];
    let rebuild = "Recreate the bundle from the source .fig with figlens bundle; do not repair bundle files by hand.";
    for path in items(&i.files["missing"]) {
        out.push(finding(
            "integrity",
            "missing",
            "manifest.json",
            &[],
            0,
            json!({"path": path}),
            "A listed bundle file is absent; anything read from it is unavailable.",
            rebuild,
        ));
    }
    for entry in items(&i.files["changed"]) {
        out.push(finding("integrity", "missing", "manifest.json", &[], 0, entry.clone(),
            "A listed bundle file no longer matches its recorded size and sha256; its original content is unavailable.", rebuild));
    }
    for path in items(&i.files["invalid"]) {
        out.push(finding(
            "integrity",
            "unverified",
            "manifest.json",
            &[],
            0,
            json!({"path": path}),
            "A manifest path is not a plain relative path, so its file was not checked.",
            rebuild,
        ));
    }
    if i.unhashed > 0 {
        out.push(finding(
            "integrity",
            "unverified",
            "manifest.json",
            &[],
            0,
            json!({"unhashedFiles": i.unhashed}),
            "Some listed files exceed the verification budget and were not hashed.",
            rebuild,
        ));
    }
    for entry in items(i.missing_assets) {
        out.push(finding("asset", "missing", "missing-assets.json", &[entry["nodeId"].clone()], 1, entry.clone(),
            "The bundle has no file for this node's image, vector or composite render; its visual acceptance stays pending.",
            "Ask the designer for the original image (for an image not in the .fig, re-save it with File > Save local copy once the image has loaded) and rebuild the bundle. Until then use a clearly labelled placeholder at the node's dimensions and report it; do not substitute another image."));
    }
    let missing_ids: Vec<_> = i
        .variables
        .into_iter()
        .flat_map(|v| items(&v["missingVariables"]))
        .collect();
    let mut bound: std::collections::BTreeMap<String, Vec<Json>> = missing_ids
        .iter()
        .filter_map(|id| Some((id.as_str()?.to_owned(), vec![])))
        .collect();
    let mut render = vec![];
    if let Some(nodes) = i.nodes {
        walk(&nodes["document"], &mut render, &mut bound);
    }
    let resolved = i.variables.map_or(0, |v| items(&v["variables"]).count());
    for id in missing_ids {
        let nodes = id
            .as_str()
            .and_then(|id| bound.get(id))
            .map_or(&[][..], Vec::as_slice);
        out.push(finding("variable", "missing", "design/variables.json", nodes, nodes.len(), json!({"id": id}),
            "Bound properties keep their saved resolved values in design/nodes.json, but the token identity is lost (a deleted or unpublished library variable).",
            "Implement with the resolved values from design/nodes.json and report the missing token; do not invent a token name. Ask the designer to restore or publish the variable for an exact mapping."));
    }
    if let Some(states) = i.states.filter(|s| s["truncated"] == true) {
        out.push(finding("scope", "unverified", "design/states.json", &[], 0,
            json!({"reasons": states["reasons"], "coverage": states["coverage"]}),
            "The state inventory stopped early; related variants beyond it were not examined.",
            "Treat unlisted states as unknown, not absent. Inspect specific component ids with figlens inspect or confirm the missing states with the designer."));
    }
    for d in &render {
        out.push(finding("render", "unsupported", "design/nodes.json", &[d["nodeId"].clone()], 1, d.clone(),
            "references/screen.png skips or approximates this feature at the node, so pixel comparison there is uncertain.",
            "Build from the node's fields, exclude the area from pixel matching, and confirm its appearance against Figma's own export or with the designer."));
    }
    if let Some(nodes) = i.nodes {
        for item in items(&nodes["_figlens"]["handoffDiagnostics"]["items"]) {
            let examples: Vec<_> = items(&item["exampleNodeIds"]).cloned().collect();
            let count = item["nodeCount"].as_u64().unwrap_or(examples.len() as u64) as usize;
            out.push(finding("semantics", "unsupported", "design/nodes.json", &examples, count, item.clone(),
                "The .fig stores data for this feature that design/nodes.json does not emit, so these nodes are under-described.",
                "Confirm the feature's behaviour in Figma or with the designer before implementing it; do not infer it from the emitted fields."));
        }
    }
    for face in items(i.fonts) {
        let nodes: Vec<_> = items(&face["nodeIds"]).cloned().collect();
        out.push(finding("font", "unverified", "design/nodes.json", &nodes, nodes.len(),
            json!({"family": face["family"], "style": face["style"], "weight": face["weight"], "postScriptName": face["postScriptName"]}),
            "Text metrics and appearance depend on this face, and the bundle has no font files.",
            "Locate the licensed face in the project or system. In the running page, call document.fonts.load with the face's weight, style and family and the design text, then check the returned faces; document.fonts.check alone can pass on a fallback. Use a documented fallback until it loads."));
    }
    if resolved > 0 {
        out.push(finding("variable", "unverified", "design/variables.json", &[], 0, json!({"variableCount": resolved}),
            "Resolved design tokens are not yet matched to the project's existing token system.",
            "Match each variable in design/variables.json to an existing project token by name and value; report unmatched ones instead of adding new tokens."));
    }
    // Stable: categories keep their order within each status.
    out.sort_by_key(|f| match f["status"].as_str() {
        Some("missing") => 0,
        Some("unsupported") => 1,
        _ => 2,
    });
    out
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

    /// An extracted bundle holding only the given metadata files.
    fn metadata_bundle(dir: &TestDir, files: &[(&str, Json)]) -> PathBuf {
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": [],
            "root": {"id": "1:1", "name": "Frame"}});
        std::fs::create_dir_all(dir.join("design")).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        for (path, value) in files {
            std::fs::write(dir.join(path), value.to_string()).unwrap();
        }
        dir.join("")
    }

    fn found<'a>(report: &'a Json, category: &str, status: &str) -> Vec<&'a Json> {
        items(&report["findings"])
            .filter(|f| f["category"] == category && f["status"] == status)
            .collect()
    }

    #[test]
    fn a_missing_asset_is_a_missing_asset_finding_with_its_node() {
        let dir = TestDir::new();
        let entry =
            json!({"key": "img", "nodeId": "1:2", "error": "images/img is not in the .fig"});
        let report = verify(&metadata_bundle(
            &dir,
            &[("missing-assets.json", json!([entry]))],
        ))
        .unwrap();
        let f = found(&report, "asset", "missing");
        assert_eq!(
            (f.len(), &f[0]["nodeIds"], &f[0]["evidence"]),
            (1, &json!(["1:2"]), &entry)
        );
    }

    #[test]
    fn an_unresolved_variable_keeps_its_id_and_binding_nodes_apart_from_known_tokens() {
        let dir = TestDir::new();
        let alias = |id: &str| json!({"type": "VARIABLE_ALIAS", "id": id});
        let nodes = json!({"document": {"id": "1:1", "children": [
            {"id": "1:2", "boundVariables": {"fills": [alias("VariableID:9:9"), alias("VariableID:9:9")]}},
            {"id": "1:3", "boundVariables": {"itemSpacing": alias("VariableID:1:5")}},
        ]}});
        let variables = json!({"variables": [{"id": "VariableID:1:5"}], "collections": [],
            "missingVariables": ["VariableID:9:9"]});
        let report = verify(&metadata_bundle(
            &dir,
            &[
                ("design/nodes.json", nodes),
                ("design/variables.json", variables),
            ],
        ))
        .unwrap();
        let missing = found(&report, "variable", "missing");
        let unmatched = found(&report, "variable", "unverified");
        assert_eq!(
            (
                missing.len(),
                &missing[0]["evidence"]["id"],
                &missing[0]["nodeIds"],
                &unmatched[0]["evidence"]
            ),
            (
                1,
                &json!("VariableID:9:9"),
                &json!(["1:2"]),
                &json!({"variableCount": 1})
            )
        );
    }

    #[test]
    fn each_font_face_is_an_unverified_finding_with_its_text_nodes() {
        let dir = TestDir::new();
        let report = verify(&fixture_bundle(&dir)).unwrap();
        let f = found(&report, "font", "unverified");
        assert_eq!(
            (
                f.len(),
                &f[0]["evidence"]["family"],
                f[0]["nodeCount"].as_u64().unwrap() > 0
            ),
            (1, &json!("Figlens Blocks"), true)
        );
    }

    #[test]
    fn render_and_semantic_limits_stay_apart_from_asset_failures() {
        let dir = TestDir::new();
        let diagnostic = json!({"nodeId": "1:2", "feature": "GLASS", "field": "effects", "state": "skipped", "reason": "not drawn"});
        let nodes = json!({"_figlens": {"handoffDiagnostics": {"items": [
                {"feature": "grid", "nodeCount": 7, "exampleNodeIds": ["1:3"], "examplesTruncated": true}]}},
            "document": {"id": "1:1", "children": [{"id": "1:2", "_figlens": {"renderDiagnostics": [diagnostic]}}]}});
        let report = verify(&metadata_bundle(&dir, &[("design/nodes.json", nodes)])).unwrap();
        let render = found(&report, "render", "unsupported");
        let semantics = found(&report, "semantics", "unsupported");
        assert_eq!(
            (
                &render[0]["evidence"],
                &render[0]["source"],
                &semantics[0]["nodeCount"],
                found(&report, "asset", "missing").len()
            ),
            (&diagnostic, &json!("design/nodes.json"), &json!(7), 0)
        );
    }

    #[test]
    fn truncated_state_inventory_and_findings_stay_explicit() {
        let dir = TestDir::new();
        let diagnostics: Vec<_> = (0..MAX_FINDINGS + 1)
            .map(|n| json!({"nodeId": format!("1:{n}"), "state": "approximate"}))
            .collect();
        let nodes =
            json!({"document": {"id": "1:1", "_figlens": {"renderDiagnostics": diagnostics}}});
        let states = json!({"truncated": true, "reasons": ["variant_limit"], "coverage": {}});
        let report = verify(&metadata_bundle(
            &dir,
            &[("design/nodes.json", nodes), ("design/states.json", states)],
        ))
        .unwrap();
        let summary = &report["findingsSummary"];
        assert_eq!(
            (
                // Unverified scope sorts after the unsupported renders, beyond the cut.
                summary["counts"]["scope/unverified"].clone(),
                report["findings"].as_array().unwrap().len(),
                summary["truncated"].clone(),
                summary["counts"]["render/unsupported"].clone(),
            ),
            (json!(1), MAX_FINDINGS, json!(true), json!(MAX_FINDINGS + 1))
        );
    }

    #[test]
    fn oversized_evidence_is_counted_but_not_returned() {
        let dir = TestDir::new();
        let huge = json!({"nodeId": "1:2", "reason": "x".repeat(FINDINGS_BYTES)});
        let nodes = json!({"document": {"id": "1:1", "_figlens": {"renderDiagnostics": [huge]}}});
        let report = verify(&metadata_bundle(&dir, &[("design/nodes.json", nodes)])).unwrap();
        let summary = &report["findingsSummary"];
        assert_eq!(
            (
                &report["findings"],
                &summary["total"],
                &summary["truncated"]
            ),
            (&json!([]), &json!(1), &json!(true))
        );
    }

    #[test]
    fn oversized_bundle_lists_keep_their_totals() {
        let dir = TestDir::new();
        let huge = json!({"key": "x".repeat(LIST_BYTES), "nodeId": "1:2"});
        let path = metadata_bundle(&dir, &[("missing-assets.json", json!([huge]))]);
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": [],
            "root": {"id": "1:1", "name": "x".repeat(2048)}});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        let report = verify(&path).unwrap();
        assert_eq!(
            (
                &report["missingAssets"],
                &report["truncated"],
                &report["findingsSummary"]["scope"]["name"]
            ),
            (&json!([]), &json!({"missingAssets": 1}), &Json::Null)
        );
    }

    #[test]
    fn malformed_handoff_items_and_invalid_paths_do_not_displace_missing_inputs() {
        let dir = TestDir::new();
        let nodes = json!({"_figlens": {"handoffDiagnostics": {"items": [false]}}, "document": {"id": "1:1"}});
        let entry = json!({"key": "img", "nodeId": "1:2", "error": "missing"});
        let path = metadata_bundle(
            &dir,
            &[
                ("design/nodes.json", nodes),
                ("missing-assets.json", json!([entry])),
            ],
        );
        let manifest = json!({"format": "figlens-bundle", "version": 1, "files": [{"path": "../x"}],
            "root": {"id": "1:1", "name": "Frame"}});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        let report = verify(&path).unwrap();
        let order: Vec<_> = items(&report["findings"])
            .map(|f| {
                (
                    f["category"].as_str().unwrap(),
                    f["status"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("asset", "missing"),
                ("semantics", "unsupported"),
                ("integrity", "unverified")
            ]
        );
    }

    #[test]
    fn semantic_coverage_is_reported_once_and_only_when_short() {
        let dir = TestDir::new();
        let report = |coverage: Json| {
            let nodes = json!({"_figlens": {"handoffDiagnostics": {"coverage": coverage,
                "items": [{"feature": "grid", "nodeCount": 1}, {"feature": "slots", "nodeCount": 1}]}},
                "document": {"id": "1:1"}});
            verify(&metadata_bundle(&dir, &[("design/nodes.json", nodes)])).unwrap()
        };
        let short = report(json!("known stored fields"));
        let long = report(json!("x".repeat(2048)));
        assert_eq!(
            (
                &short["findingsSummary"]["semanticsCoverage"],
                found(&short, "semantics", "unsupported")[0]["evidence"].get("coverage"),
                &long["findingsSummary"]["semanticsCoverage"]
            ),
            (&json!("known stored fields"), None, &Json::Null)
        );
    }

    #[test]
    fn a_directory_without_a_bundle_manifest_is_an_error() {
        let dir = TestDir::new();
        assert!(verify(&dir.join("")).is_err());
    }
}
