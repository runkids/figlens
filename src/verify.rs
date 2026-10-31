//! Checks a design bundle, zipped or extracted, against its own manifest: which listed files
//! match, which are missing or changed, which assets the bundle could not include, and which
//! font faces only the project and its browser can confirm. It reports; it never blocks.

use std::collections::{BTreeMap, BTreeSet};
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

/// The report without a local font lookup.
pub fn verify(path: &Path) -> Result<Json, Box<dyn Error>> {
    check(path, &[])
}

/// The report with faces looked up in this machine's font folders and, first, `project`.
pub fn verify_with_fonts(path: &Path, project: Option<&Path>) -> Result<Json, Box<dyn Error>> {
    let mut roots = vec![];
    if let Some(dir) = project {
        if !dir.is_dir() {
            return Err(format!("fonts: {} is not a directory", dir.display()).into());
        }
        roots.push((dir.to_owned(), true));
    }
    roots.extend(crate::fonts::system_dirs().into_iter().map(|d| (d, false)));
    check(path, &roots)
}

fn check(path: &Path, font_roots: &[(PathBuf, bool)]) -> Result<Json, Box<dyn Error>> {
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
    let lookup = look_up(&mut fonts, font_roots);
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
    let Findings {
        list: mut all,
        counts,
    } = findings(&Inputs {
        files: &files,
        unhashed,
        missing_assets: &missing_assets,
        nodes: nodes.as_ref(),
        variables: variables.as_ref(),
        states: states.as_ref(),
        fonts: &fonts,
    });
    let browser = "Load each face in the running project with document.fonts.load and the design text and check the returned faces; document.fonts.check alone can pass on a fallback.";
    unverified.push(if lookup.searched.is_empty() {
        format!("Font faces: the bundle has no font files and figlens cannot see the project's fonts. {browser}")
    } else {
        let local = items(&fonts).filter(|f| f["status"] == "local_file").count();
        format!("Font faces: the bundle has no font files. {local} of {} faces matched a font file on this machine (local_file: found by name only, so not proof that the project loads it or that its licence allows shipping it). {browser}", items(&fonts).count())
    });
    unverified.push(
        "Browser delivery: check the deployed asset URLs, decoding and rendered dimensions in the running project.".to_owned(),
    );
    let total: u64 = counts.values().sum();
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
        "truncated": (all.len() as u64) < total,
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
            "fontLookup": {"searched": lookup.searched, "filesRead": lookup.files_read, "truncated": lookup.truncated},
            "unverified": unverified,
            "findings": all,
            "findingsSummary": summary,
            "truncated": truncated,
    }))
}

/// Marks every face `unverified`, or `local_file` with `localFiles` (and `fontFace` for a match
/// inside the project folder) when `roots` hold a font file with its name.
fn look_up(fonts: &mut Json, roots: &[(PathBuf, bool)]) -> crate::fonts::Summary {
    let text = |f: &Json, k: &str| f[k].as_str().unwrap_or("").to_owned();
    let owned: Vec<_> = items(fonts)
        .map(|f| {
            (
                text(f, "family"),
                text(f, "style"),
                text(f, "postScriptName"),
            )
        })
        .collect();
    let wants: Vec<_> = owned
        .iter()
        .map(|(family, style, post_script_name)| crate::fonts::Want {
            family,
            style,
            post_script_name,
        })
        .collect();
    let (hits, summary) = crate::fonts::lookup(roots, &wants);
    let project = roots.iter().find(|r| r.1).map(|r| r.0.as_path());
    for ((face, hits), want) in fonts
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(hits)
        .zip(&wants)
    {
        face["status"] = json!(if hits.is_empty() {
            "unverified"
        } else {
            "local_file"
        });
        face["localFiles"] = hits
            .iter()
            .map(|h| json!({"path": h.path, "matchedBy": h.by, "inProject": h.project}))
            .collect();
        let weight = face["weight"].as_f64().map(|w| w as u64);
        // The first project file a browser can load (not a collection).
        let snippet = project.and_then(|root| {
            hits.iter()
                .filter(|h| h.project)
                .find_map(|hit| crate::fonts::font_face(want, weight, hit, root))
        });
        if let Some(snippet) = snippet {
            face["fontFace"] = json!(snippet);
        }
    }
    summary
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

/// Bound variable id → (its first binding node ids, how many nodes bind it).
type Bound<'a> = BTreeMap<String, (Vec<&'a Json>, usize)>;

/// Findings counted per `category/status`, with at most `MAX_FINDINGS` of each built: the
/// stable status sort and the cut never reach further, and building every one would let an
/// untrusted bundle multiply its node ids and evidence.
#[derive(Default)]
struct Findings {
    list: Vec<Json>,
    counts: BTreeMap<String, u64>,
}

impl Findings {
    /// Design strings stay in `evidence`; `impact` and `action` are fixed text. Node ids over
    /// 1 KiB are left out of `nodeIds`, like handoff example ids; `nodeCount` keeps the total.
    fn add(
        &mut self,
        (category, status): (&str, &str),
        source: &str,
        (node_ids, node_count): (&[&Json], usize),
        evidence: impl FnOnce() -> Json,
        impact: &str,
        action: &str,
    ) {
        let n = self
            .counts
            .entry(format!("{category}/{status}"))
            .or_default();
        *n += 1;
        if *n > MAX_FINDINGS as u64 {
            return;
        }
        let ids: Vec<_> = node_ids
            .iter()
            .filter(|id| id.as_str().is_some_and(|id| id.len() <= 1024))
            .take(MAX_NODE_IDS)
            .collect();
        self.list.push(json!({
            "category": category, "status": status, "source": source,
            "nodeIds": ids, "nodeCount": node_count,
            "evidence": evidence(), "impact": impact, "action": action,
        }));
    }
}

/// Render diagnostics in document order, and the nodes binding each id already in `bound`.
fn walk<'a>(node: &'a Json, render: &mut Vec<&'a Json>, bound: &mut Bound<'a>) {
    render.extend(items(&node["_figlens"]["renderDiagnostics"]));
    if !bound.is_empty() {
        let mut ids = BTreeSet::new();
        for (key, value) in node.as_object().into_iter().flatten() {
            if key != "children" {
                bound_ids(value, bound, &mut ids);
            }
        }
        for id in ids {
            let (nodes, count) = bound.get_mut(id).expect("collected from bound");
            *count += 1;
            if nodes.len() < MAX_NODE_IDS {
                nodes.push(&node["id"]);
            }
        }
    }
    for child in items(&node["children"]) {
        walk(child, render, bound);
    }
}

/// The ids in `bound` that a `VARIABLE_ALIAS` under `v` names. Map lookups only: alias lists in
/// an untrusted bundle can be huge.
fn bound_ids<'a>(v: &'a Json, bound: &Bound, ids: &mut BTreeSet<&'a str>) {
    match v {
        Json::Array(a) => a.iter().for_each(|x| bound_ids(x, bound, ids)),
        Json::Object(o) => {
            if o.get("type").and_then(Json::as_str) == Some("VARIABLE_ALIAS")
                && let Some(id) = o.get("id").and_then(Json::as_str)
                && bound.contains_key(id)
            {
                ids.insert(id);
            }
            o.values().for_each(|x| bound_ids(x, bound, ids));
        }
        _ => {}
    }
}

/// Missing inputs first, then known limitations, then checks only the project can make.
fn findings(i: &Inputs) -> Findings {
    let mut out = Findings::default();
    let none = (&[][..], 0);
    let rebuild = "Recreate the bundle from the source .fig with figlens bundle; do not repair bundle files by hand.";
    for path in items(&i.files["missing"]) {
        out.add(
            ("integrity", "missing"),
            "manifest.json",
            none,
            || json!({"path": path}),
            "A listed bundle file is absent; anything read from it is unavailable.",
            rebuild,
        );
    }
    for entry in items(&i.files["changed"]) {
        out.add(("integrity", "missing"), "manifest.json", none, || entry.clone(),
            "A listed bundle file no longer matches its recorded size and sha256; its original content is unavailable.", rebuild);
    }
    for path in items(&i.files["invalid"]) {
        out.add(
            ("integrity", "unverified"),
            "manifest.json",
            none,
            || json!({"path": path}),
            "A manifest path is not a plain relative path, so its file was not checked.",
            rebuild,
        );
    }
    if i.unhashed > 0 {
        out.add(
            ("integrity", "unverified"),
            "manifest.json",
            none,
            || json!({"unhashedFiles": i.unhashed}),
            "Some listed files exceed the verification budget and were not hashed.",
            rebuild,
        );
    }
    for entry in items(i.missing_assets) {
        out.add(("asset", "missing"), "missing-assets.json", (&[&entry["nodeId"]], 1), || entry.clone(),
            "The bundle has no file for this node's image, vector or composite render; its visual acceptance stays pending.",
            "Ask the designer for the original image (for an image not in the .fig, re-save it with File > Save local copy once the image has loaded) and rebuild the bundle. Until then use a clearly labelled placeholder at the node's dimensions and report it; do not substitute another image.");
    }
    let missing_ids: Vec<_> = i
        .variables
        .into_iter()
        .flat_map(|v| items(&v["missingVariables"]))
        .collect();
    let mut bound: Bound = missing_ids
        .iter()
        .filter_map(|id| Some((id.as_str()?.to_owned(), (vec![], 0))))
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
            .map_or((&[][..], 0), |(nodes, count)| (nodes.as_slice(), *count));
        out.add(("variable", "missing"), "design/variables.json", nodes, || json!({"id": id}),
            "Bound properties keep their saved resolved values in design/nodes.json, but the token identity is lost (a deleted or unpublished library variable).",
            "Implement with the resolved values from design/nodes.json and report the missing token; do not invent a token name. Ask the designer to restore or publish the variable for an exact mapping.");
    }
    if let Some(states) = i.states.filter(|s| s["truncated"] == true) {
        out.add(("scope", "unverified"), "design/states.json", none,
            || json!({"reasons": states["reasons"], "coverage": states["coverage"]}),
            "The state inventory stopped early; related variants beyond it were not examined.",
            "Treat unlisted states as unknown, not absent. Inspect specific component ids with figlens inspect or confirm the missing states with the designer.");
    }
    for d in render {
        out.add(("render", "unsupported"), "design/nodes.json", (&[&d["nodeId"]], 1), || d.clone(),
            "references/screen.png skips or approximates this feature at the node, so pixel comparison there is uncertain.",
            "Build from the node's fields, exclude the area from pixel matching, and confirm its appearance against Figma's own export or with the designer.");
    }
    if let Some(nodes) = i.nodes {
        for item in items(&nodes["_figlens"]["handoffDiagnostics"]["items"]) {
            let examples: Vec<_> = items(&item["exampleNodeIds"]).collect();
            let count = item["nodeCount"].as_u64().unwrap_or(examples.len() as u64) as usize;
            out.add(("semantics", "unsupported"), "design/nodes.json", (&examples, count), || item.clone(),
                "The .fig stores data for this feature that design/nodes.json does not emit, so these nodes are under-described.",
                "Confirm the feature's behaviour in Figma or with the designer before implementing it; do not infer it from the emitted fields.");
        }
    }
    for face in items(i.fonts) {
        let nodes: Vec<_> = items(&face["nodeIds"]).collect();
        out.add(("font", "unverified"), "design/nodes.json", (&nodes, nodes.len()),
            || json!({"family": face["family"], "style": face["style"], "weight": face["weight"], "postScriptName": face["postScriptName"], "localFiles": face["localFiles"]}),
            "Text metrics and appearance depend on this face, and the bundle has no font files.",
            "Locate the licensed face in the project or system (evidence.localFiles lists files found by name on this machine, if any). In the running page, call document.fonts.load with the face's weight, style and family and the design text, then check the returned faces; document.fonts.check alone can pass on a fallback. Use a documented fallback until it loads.");
    }
    if resolved > 0 {
        out.add(("variable", "unverified"), "design/variables.json", none, || json!({"variableCount": resolved}),
            "Resolved design tokens are not yet matched to the project's existing token system.",
            "Match each variable in design/variables.json to an existing project token by name and value; report unmatched ones instead of adding new tokens.");
    }
    // Stable: categories keep their order within each status.
    out.list.sort_by_key(|f| match f["status"].as_str() {
        Some("missing") => 0,
        Some("unsupported") => 1,
        _ => 2,
    });
    out
}

#[cfg(test)]
mod tests;
