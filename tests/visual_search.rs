//! Image search through the CLI over real workspaces (#95). The visual layers are written here
//! the way the preparation stores them (`workspace::visual::save` plus a ready hash-layer row),
//! from flat colors rather than renders, so ranking, filters, references and cursors are checked
//! through the shipped binary without a model.

#[path = "../src/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use figlens::search::visual::{QueryImage, RenderedUnit};
use resvg::tiny_skia::{Color, Pixmap};
use serde_json::{Value, json};
use test_support::TestDir;

const BIN: &str = env!("CARGO_BIN_EXE_figlens");
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 160, 0];
const NEAR_RED: [u8; 3] = [250, 6, 0];

struct Env {
    dir: TestDir,
    home: PathBuf,
    workspace: PathBuf,
}

impl Env {
    fn new() -> Self {
        let dir = TestDir::new();
        let root = std::fs::canonicalize(dir.join("")).unwrap();
        let (home, workspace) = (root.join("home"), root.join("project"));
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        Self {
            dir,
            home,
            workspace,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .env("HOME", &self.home)
            .env("LOCALAPPDATA", &self.home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// What a failing command prints: its message line and, when it has one, its recovery.
    fn error(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(!out.status.success(), "{args:?} succeeded");
        let text = String::from_utf8_lossy(&out.stderr);
        let mut lines = text.lines();
        let message = lines.next().unwrap_or_default().to_owned();
        let recovery = lines.next().map_or(Value::Null, |l| {
            serde_json::from_str::<Value>(l).unwrap()["recovery"].clone()
        });
        json!({"message": message, "recovery": recovery})
    }

    /// A copy of the public design, prepared into the project workspace at the document scope.
    fn source(&self, name: &str) -> (PathBuf, String) {
        let path = self.dir.join(name);
        std::fs::copy("examples/first-success/design.fig", &path).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let id = self.prepare(&path, "0:0");
        (path, id)
    }

    fn prepare(&self, path: &Path, scope: &str) -> String {
        let workspace = self.workspace.to_str().unwrap();
        let out = self.json(&[
            "index",
            "prepare",
            path.to_str().unwrap(),
            "--workspace",
            workspace,
            "--scope",
            scope,
        ]);
        out["source"]["sourceId"].as_str().unwrap().to_owned()
    }

    fn database(&self) -> rusqlite::Connection {
        let status = self.json(&[
            "index",
            "status",
            "--workspace",
            self.workspace.to_str().unwrap(),
        ]);
        rusqlite::Connection::open(status["paths"]["database"].as_str().unwrap()).unwrap()
    }

    /// Stores a flat-color unit for each node of `source`'s published generation of `scope` and
    /// marks its hash layer ready, as the visual preparation leaves it.
    fn layer(&self, source: &str, scope: &str, units: &[(&str, [u8; 3])]) {
        self.layer_with(source, scope, units, &[]);
    }

    /// `layer` whose every unit reports `diagnostics`.
    fn layer_with(
        &self,
        source: &str,
        scope: &str,
        units: &[(&str, [u8; 3])],
        diagnostics: &[Value],
    ) {
        let conn = self.database();
        let gen_id: i64 = conn
            .query_row(
                "SELECT gen FROM published WHERE source_id = ?1 AND scope = ?2",
                [source, scope],
                |r| r.get(0),
            )
            .unwrap();
        for (node, rgb) in units {
            let rid: u32 = conn
                .query_row(
                    "SELECT rid FROM records WHERE gen = ?1 AND node_id = ?2",
                    rusqlite::params![gen_id, node],
                    |r| r.get(0),
                )
                .unwrap();
            let rendered = RenderedUnit {
                image: flat(*rgb),
                diagnostics: diagnostics.to_vec(),
                diagnostics_truncated: false,
                bounds: serde_json::Map::new(),
                pixels: 0,
            };
            figlens::workspace::visual::save(&conn, gen_id, rid, &rendered).unwrap();
        }
        conn.execute(
            "INSERT INTO model_layers (gen, profile_id, policy, state, chunks, texts, embedded, refused, stop, job_id, updated_ms)
             VALUES (?1, ?2, ?3, 'ready', ?4, ?4, ?4, 0, NULL, NULL, 0)",
            rusqlite::params![gen_id, figlens::search::visual::VERSION, figlens::workspace::visual::POLICY, units.len() as i64],
        )
        .unwrap();
    }

    /// A query PNG of one flat color, written beside the designs.
    fn png(&self, name: &str, rgb: [u8; 3]) -> PathBuf {
        let path = self.dir.join(name);
        let mut p = Pixmap::new(32, 20).unwrap();
        p.fill(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        std::fs::write(&path, p.encode_png().unwrap()).unwrap();
        std::fs::canonicalize(path).unwrap()
    }

    fn search(&self, extra: &[&str]) -> Value {
        let mut args = vec!["search", "--workspace", self.workspace.to_str().unwrap()];
        args.extend_from_slice(extra);
        self.json(&args)
    }

    fn search_error(&self, extra: &[&str]) -> Value {
        let mut args = vec!["search", "--workspace", self.workspace.to_str().unwrap()];
        args.extend_from_slice(extra);
        self.error(&args)
    }
}

fn flat(rgb: [u8; 3]) -> QueryImage {
    let mut p = Pixmap::new(32, 20).unwrap();
    p.fill(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
    QueryImage::from_pixmap(&p, None).unwrap()
}

fn hits(reply: &Value) -> Vec<(String, String)> {
    reply["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["ref"]["sourceId"].as_str().unwrap().to_owned(),
                r["ref"]["nodeId"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Two sources with the same node ids: one ranking across both, each hit kept with its source.
fn two() -> (Env, String, String) {
    let env = Env::new();
    let (_, a) = env.source("a.fig");
    let (_, b) = env.source("b.fig");
    env.layer(&a, "0:0", &[("0:1", RED), ("0:2", BLUE)]);
    env.layer(&b, "0:0", &[("0:1", GREEN), ("0:2", NEAR_RED)]);
    (env, a, b)
}

#[test]
fn an_image_ranks_every_source_once_with_exact_pixels_first() {
    let (env, a, b) = two();
    let query = env.png("red.png", RED);
    let reply = env.search(&["--image", path(&query)]);
    let found = hits(&reply);
    assert_eq!(
        &found[..2],
        [(a.clone(), "0:1".into()), (b.clone(), "0:2".into())]
    );
    assert_eq!(found.len(), 4, "every unit of both sources is ranked once");
    assert_eq!(
        reply["results"][0]["matches"][0]["basis"],
        "normalized_pixels"
    );
    assert_eq!(
        reply["results"][1]["matches"][0]["basis"],
        "perceptual_hash"
    );
    assert!(
        reply["results"][0]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m.get("span").is_none())
    );
    assert_eq!(reply["query"]["mode"], "visual");
    assert_eq!(reply["query"]["usedModes"], json!(["visual_hash"]));
    assert_eq!(
        reply["query"]["image"],
        json!({"kind": "path", "crop": null}),
        "the path is not echoed"
    );
    assert_eq!(
        reply["coverage"]["layers"]["visual"]["embedding"]["state"],
        "not_configured"
    );
    assert_eq!(
        reply["coverage"]["layers"]["visual"]["approximate"],
        "visual_hash"
    );
    assert_eq!(reply["coverage"]["complete"], true);
    let visual = &reply["results"][0]["visual"];
    assert_eq!(
        (visual["state"].as_str(), visual["error"].clone()),
        (Some("rendered"), Value::Null)
    );
    assert!(visual["diagnostics"].is_array() && visual["bounds"].is_object());
}

#[test]
fn a_reference_searches_by_a_stored_render_whatever_the_source_filter() {
    let (env, a, b) = two();
    let snapshot =
        env.search(&["--image", path(&env.png("q.png", BLUE))])["results"][0]["ref"]["snapshotId"]
            .clone();
    let reference = json!({"sourceId": a, "snapshotId": snapshot, "nodeId": "0:2"}).to_string();
    // Narrowed to b, the reference still reads a's render: b's nearest to blue ranks first.
    let reply = env.search(&["--image-ref", &reference, "--source", &b]);
    assert!(hits(&reply).iter().all(|(s, _)| *s == b));
    assert_eq!(reply["query"]["image"]["kind"], "ref");
    let stale = json!({"sourceId": a, "snapshotId": "sha256:0", "nodeId": "0:2"}).to_string();
    assert!(
        env.search_error(&["--image-ref", &stale])["message"]
            .as_str()
            .unwrap()
            .contains("serves it now")
    );
    let unknown =
        json!({"sourceId": "local:nope", "snapshotId": snapshot, "nodeId": "0:2"}).to_string();
    assert!(
        env.search_error(&["--image-ref", &unknown])["message"]
            .as_str()
            .unwrap()
            .contains("does not serve to this caller")
    );
    // A node that is no visual unit names the step that builds the layer.
    let text = json!({"sourceId": a, "snapshotId": snapshot, "nodeId": "0:0"}).to_string();
    let error = env.search_error(&["--image-ref", &text]);
    assert_eq!(error["recovery"][0]["arguments"]["action"], "visual");
}

/// A reference resolves apart from the searched scope: here its node's render is stored only in
/// the source's other prepared scope, and the search of the document scope still reads it.
#[test]
fn a_reference_reads_its_render_from_another_prepared_scope_of_its_source() {
    let env = Env::new();
    let (design, a) = env.source("a.fig");
    env.prepare(&design, "0:2");
    env.layer(&a, "0:0", &[("0:1", RED)]);
    env.layer(&a, "0:2", &[("0:2", BLUE)]);
    let red = env.png("q.png", RED);
    let snapshot = env.search(&["--image", path(&red)])["results"][0]["ref"]["snapshotId"].clone();
    let reference = json!({"sourceId": a, "snapshotId": snapshot, "nodeId": "0:2"}).to_string();
    let reply = env.search(&["--image-ref", &reference]);
    assert_eq!(hits(&reply), [(a, "0:1".to_owned())]);
    assert_eq!(
        reply["results"][0]["matches"][0]["basis"], "perceptual_hash",
        "blue is no exact copy of red"
    );
}

#[test]
fn filters_apply_before_ranking_and_cursors_bind_the_image() {
    let (env, _, _) = two();
    let red = env.png("red.png", RED);
    // Every unit is a FRAME: a TEXT filter leaves nothing to rank.
    assert_eq!(
        env.search(&["--image", path(&red), "--type", "TEXT"])["results"],
        json!([])
    );
    let first = env.search(&["--image", path(&red), "--limit", "1"]);
    let cursor = first["nextCursor"].as_str().unwrap().to_owned();
    let second = env.search(&["--image", path(&red), "--limit", "1", "--cursor", &cursor]);
    assert_eq!(
        hits(&second)[0],
        hits(&env.search(&["--image", path(&red), "--limit", "2"]))[1]
    );
    let blue = env.png("blue.png", BLUE);
    for other in [
        vec!["--image", path(&blue)],
        vec!["--image", path(&red), "--crop", "0,0,4,4"],
    ] {
        let mut args = other.clone();
        args.extend(["--limit", "1", "--cursor", &cursor]);
        let message = env.search_error(&args)["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("cursor belongs to another query image"),
            "{other:?}: {message}"
        );
    }
}

#[test]
fn a_missing_visual_layer_names_its_step_and_a_partial_page_cannot_continue() {
    let env = Env::new();
    let (_, a) = env.source("a.fig");
    let red = env.png("red.png", RED);
    let error = env.search_error(&["--image", path(&red)]);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("no searched source has a visual layer")
    );
    assert_eq!(error["recovery"][0]["arguments"]["action"], "visual");
    let (_, b) = env.source("b.fig");
    env.layer(&b, "0:0", &[("0:1", RED), ("0:2", BLUE)]);
    let reply = env.search(&["--image", path(&red), "--limit", "1"]);
    assert_eq!(hits(&reply)[0].0, b);
    assert_eq!(
        (
            reply["coverage"]["complete"].clone(),
            reply["nextCursor"].clone()
        ),
        (json!(false), Value::Null)
    );
    assert!(
        reply["nextSteps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["arguments"]["action"] == "visual" && s["arguments"]["sourceId"] == a)
    );
}

/// A partial layer (a render budget or unit cap reached) keeps the page partial, but names no
/// `index visual` step, which would leave it unchanged.
#[test]
fn a_partial_visual_layer_is_partial_without_a_step_that_cannot_change_it() {
    let (env, _, _) = two();
    env.database()
        .execute("UPDATE model_layers SET state = 'partial'", [])
        .unwrap();
    let reply = env.search(&["--image", path(&env.png("red.png", RED))]);
    assert_eq!(
        (
            reply["coverage"]["complete"].clone(),
            reply["nextCursor"].clone()
        ),
        (json!(false), Value::Null)
    );
    let steps = reply["nextSteps"].as_array().cloned().unwrap_or_default();
    assert!(
        steps.iter().all(|s| s["arguments"]["action"] != "visual"),
        "{steps:?}"
    );
    assert_eq!(
        reply["coverage"]["layers"]["visual"]["hash"]["sources"][0]["state"],
        "partial"
    );
}

#[test]
fn image_searches_refuse_incompatible_modes_and_inputs() {
    let (env, a, _) = two();
    let red = env.png("red.png", RED);
    let reference = json!({"sourceId": a, "snapshotId": "sha256:0", "nodeId": "0:1"}).to_string();
    for extra in [
        vec!["Card", "--image", path(&red), "--mode", "visual"],
        vec!["Card", "--image", path(&red), "--mode", "semantic"],
        vec!["Card", "--mode", "visual"],
        vec!["Card", "--crop", "0,0,2,2"],
        vec!["--image", path(&red), "--image-ref", &reference],
        vec!["--image", path(&red), "--crop", "0,0,0,2"],
    ] {
        env.search_error(&extra);
    }
    // A single .fig has no visual layer, and a search never makes one.
    let file = env.dir.join("a.fig");
    let error = env.error(&["search", path(&file), "--image", path(&red)]);
    assert_eq!(
        error["recovery"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["arguments"]["action"].clone())
            .collect::<Vec<_>>(),
        [json!("prepare"), json!("visual")]
    );
}

#[test]
fn hybrid_ranks_the_query_text_with_the_image_and_keeps_lexical_spans_lexical() {
    let (env, a, _) = two();
    let blue = env.png("blue.png", BLUE);
    let reply = env.search(&["Card", "--image", path(&blue)]);
    assert_eq!(reply["query"]["mode"], "hybrid");
    assert_eq!(reply["query"]["usedModes"], json!(["text", "visual_hash"]));
    // a's Card frame is both the lexical match and the exact blue render.
    assert_eq!(hits(&reply)[0], (a, "0:2".to_owned()));
    let bases: Vec<&str> = reply["results"][0]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["basis"].as_str().unwrap())
        .collect();
    assert!(
        bases.contains(&"lexical") && bases.contains(&"normalized_pixels"),
        "{bases:?}"
    );
    // Query text after -- stays text, even when it looks like an option.
    let dashed = env.search(&["--image", path(&blue), "--", "--dark"]);
    assert_eq!(
        (
            dashed["query"]["mode"].as_str(),
            dashed["query"]["text"].as_str()
        ),
        (Some("hybrid"), Some("--dark")),
        "{}",
        dashed["query"]
    );
    // Hybrid with an image and no text uses the visual channels alone.
    assert_eq!(
        env.search(&["--image", path(&blue), "--mode", "hybrid"])["query"]["usedModes"],
        json!(["visual_hash"])
    );
}

#[test]
fn a_query_image_is_never_stored_and_text_search_is_unchanged() {
    let (env, _, _) = two();
    let count = |env: &Env| -> i64 {
        env.database()
            .query_row("SELECT count(*) FROM visual_rasters", [], |r| r.get(0))
            .unwrap()
    };
    let before = count(&env);
    env.search(&["--image", path(&env.png("purple.png", [120, 0, 200]))]);
    assert_eq!(count(&env), before);
    let text = env.search(&["Card"]);
    assert_eq!(text["query"]["mode"], "text");
    assert!(
        text["query"].get("image").is_none() && text["coverage"]["layers"].get("visual").is_none()
    );
    assert!(
        text["ranking"].get("fusion").is_none() && text["ranking"]["version"] != "visual-rrf-1"
    );
}

/// MCP admits a query image only inside its roots and never by URL, and a reference to a source
/// outside them reads nothing; results come only from sources the roots show.
#[test]
fn mcp_image_search_stays_inside_its_roots() {
    use std::io::{BufRead, BufReader, Write};
    let env = Env::new();
    for d in ["designs", "hidden", "queries", "outside"] {
        std::fs::create_dir(env.dir.join(d)).unwrap();
    }
    let (_, a) = env.source("designs/a.fig");
    let (_, b) = env.source("hidden/b.fig");
    env.layer(&a, "0:0", &[("0:1", RED), ("0:2", BLUE)]);
    env.layer(&b, "0:0", &[("0:1", RED), ("0:2", BLUE)]);
    let inside = env.png("queries/red.png", RED);
    let outside = env.png("outside/red.png", RED);
    let snapshot =
        env.search(&["--image", path(&inside), "--source", &b])["results"][0]["ref"]["snapshotId"]
            .clone();
    let ws_id =
        env.json(&["index", "status", "--workspace", path(&env.workspace)])["workspaceId"].clone();
    let root = std::fs::canonicalize(env.dir.join("")).unwrap();
    let mut args = vec!["mcp".to_owned()];
    for d in ["project", "designs", "queries"] {
        args.extend(["--root".to_owned(), root.join(d).display().to_string()]);
    }
    let mut mcp = Command::new(BIN)
        .args(&args)
        .env("HOME", &env.home)
        .env("LOCALAPPDATA", &env.home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    let mut lines = BufReader::new(mcp.stdout.take().unwrap());
    let mut call = |id: u64, arguments: Value| -> (bool, Value) {
        let mut arguments = arguments;
        arguments["workspaceId"] = json!(ws_id);
        let request = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "search", "arguments": arguments}});
        writeln!(stdin, "{request}").unwrap();
        loop {
            let mut line = String::new();
            lines.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == id {
                let result = &reply["result"];
                let text = result["content"][0]["text"].as_str().unwrap_or_default();
                let body = serde_json::from_str(text).unwrap_or_else(|_| json!(text));
                return (result["isError"] == true, body);
            }
        }
    };
    let (failed, reply) = call(
        1,
        json!({"image": inside, "crop": {"x": 0, "y": 0, "width": 8, "height": 8}}),
    );
    assert!(!failed, "{reply}");
    assert!(hits(&reply).iter().all(|(s, _)| *s == a), "{reply}");
    assert!(
        call(2, json!({"image": outside})).0,
        "a query image outside the roots"
    );
    assert!(
        call(3, json!({"image": format!("file://{}", inside.display())})).0,
        "a URL"
    );
    let hidden = json!({"sourceId": b, "snapshotId": snapshot, "nodeId": "0:1"});
    let (failed, reply) = call(4, json!({"imageRef": hidden}));
    assert!(
        failed && reply.to_string().contains("does not serve"),
        "{reply}"
    );
    let visible = json!({"sourceId": a, "snapshotId": snapshot, "nodeId": "0:2"});
    let (failed, reply) = call(5, json!({"imageRef": visible}));
    assert!(
        !failed && reply["query"]["image"]["kind"] == "ref",
        "{reply}"
    );
    drop(stdin);
    mcp.wait().unwrap();
}

/// Render evidence counts toward the search reply budget: a page whose rows carry it in bulk is
/// refused as too large, and a smaller limit answers.
#[test]
fn render_evidence_counts_toward_the_reply_budget() {
    let env = Env::new();
    let nodes: Vec<String> = {
        let (_, a) = env.source("a.fig");
        let conn = env.database();
        let mut stmt = conn
            .prepare("SELECT node_id FROM records r JOIN published p ON p.gen = r.gen WHERE p.source_id = ?1")
            .unwrap();
        let ids = stmt
            .query_map([&a], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(stmt);
        ids
    };
    // About 240 KiB of evidence per unit; five sources' units together pass 8 MiB.
    let bulk = vec![json!({"feature": "x".repeat(240_000)})];
    let units: Vec<(&str, [u8; 3])> = nodes.iter().map(|n| (n.as_str(), RED)).collect();
    let mut ids = vec![env.json(&["index", "status", "--workspace", path(&env.workspace)])["sources"][0]["sourceId"].as_str().unwrap().to_owned()];
    for name in ["b.fig", "c.fig", "d.fig", "e.fig"] {
        ids.push(env.source(name).1);
    }
    for id in &ids {
        env.layer_with(id, "0:0", &units, &bulk);
    }
    let red = env.png("red.png", RED);
    let error = env.search_error(&["--image", path(&red), "--limit", "200"]);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("render evidence"),
        "{error}"
    );
    let reply = env.search(&["--image", path(&red), "--limit", "5"]);
    assert_eq!(reply["results"][0]["visual"]["diagnostics"], json!(bulk));
}

/// A workspace from before the visual tables answers an image search with the step that builds
/// its layer, never a missing-table error; it is read as is, not migrated.
#[test]
fn an_older_workspace_names_the_visual_step_instead_of_failing_on_missing_tables() {
    for (version, drop) in [
        (
            8,
            "DROP TABLE visual_units; DROP TABLE visual_rasters; DROP TABLE model_vectors; DROP TABLE model_refs; DROP TABLE model_layers;",
        ),
        (9, "DROP TABLE visual_units; DROP TABLE visual_rasters;"),
    ] {
        let env = Env::new();
        let (_, a) = env.source("a.fig");
        let red = env.png("red.png", RED);
        let snapshot = env.search(&["Card"])["results"][0]["ref"]["snapshotId"].clone();
        env.database()
            .execute_batch(&format!("{drop} PRAGMA user_version = {version};"))
            .unwrap();
        let error = env.search_error(&["--image", path(&red)]);
        assert_eq!(
            error["recovery"][0]["arguments"]["action"], "visual",
            "schema {version}: {error}"
        );
        let reference = json!({"sourceId": a, "snapshotId": snapshot, "nodeId": "0:1"}).to_string();
        let error = env.search_error(&["--image-ref", &reference]);
        assert_eq!(
            error["recovery"][0]["arguments"]["action"], "visual",
            "schema {version}: {error}"
        );
    }
}

/// The real preparation: `index visual` renders the units, and an export of one of them finds it
/// first in every source as an exact normalized-pixel copy.
#[test]
fn index_visual_layers_answer_an_export_of_their_own_unit() {
    let env = Env::new();
    let (design, a) = env.source("a.fig");
    let (_, b) = env.source("b.fig");
    let workspace = path(&env.workspace);
    env.json(&[
        "index",
        "visual",
        "--workspace",
        workspace,
        "--scope",
        "0:0",
    ]);
    let shot = env.dir.join("screen.png");
    env.json(&["export", path(&design), "0:1", path(&shot)]);
    let reply = env.search(&["--image", path(&shot)]);
    let mut top: Vec<(String, String)> = hits(&reply)[..2].to_vec();
    top.sort();
    let mut want = vec![(a, "0:1".to_owned()), (b, "0:1".to_owned())];
    want.sort();
    assert_eq!(top, want, "{reply}");
    assert!(
        reply["results"].as_array().unwrap()[..2]
            .iter()
            .all(|r| r["matches"][0]["basis"] == "normalized_pixels")
    );
    assert_eq!(
        reply["coverage"]["layers"]["visual"]["hash"]["state"],
        "ready"
    );
}
