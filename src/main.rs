use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use figlens::compare::{FIELDS, compare};
use figlens::rest::nodes;
use figlens::variables::variables;
use figlens::{Document, Tree, normalize_id};

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

mod command;
mod workflow;

const USAGE: &str = "figlens — read Figma .fig files (File > Save local copy) without Figma

usage:
  figlens tree <file.fig> [node] [--depth N] [--json]
                                             list ids, types and names (default root: the document);
                                             --json prints them as nested {id,type,name,children}
  figlens find <file.fig> <query> [--type TYPE] [--node ID] [--limit N]
                                             search stored layer names and text, case-insensitive;
                                             default 20 results (1-200), with paths and truncation flag
  figlens inspect <file.fig> <node> [--depth N] [--images]
                                             compact layout/text summary, instances expanded;
                                             depth 1 (0-10), at most 200 nodes, 200 characters per text;
                                             --images adds source pixels/bytes and source-pixel crop rectangles
  figlens nodes <file.fig> <node>              print a frame as Figma REST-shaped nodes.json
                                             (instances expanded, fills/text/auto-layout included)
  figlens variables <file.fig> [node]          print the variables a frame binds (design/variables.json);
                                             without a node, every local variable
  figlens bundle <file.fig> <node> <out.zip>   write a design bundle for a frame: nodes, variables,
                                             image/SVG assets, reference render and context.md, in one zip;
                                             prints {nodeId,path,warnings}
  figlens bundle ... --file-key KEY --name N   set the manifest's file key and frame name
  figlens bundle <file.fig> --frames <list> --out-dir <dir> [--file-key KEY] [--jobs N]
                                            one decode, many bundles: <list> has one node id per line;
                                             writes <dir>/<nodeId>.zip (`:` as `-`), prints one JSON line
                                             per frame ({nodeId,path,warnings} or {nodeId,error})
  figlens export <file.fig> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]
                                            render a node to out.png|jpg|webp|avif|svg (default 1x,
                                             quality 80); SVG keeps vectors, scale/quality do not apply;
                                             --max-kb lowers lossy quality until it fits;
                                             --max-size scales raster output to fit its longest side;
                                             prints {path,width,height,bytes,quality}
  figlens preview <file.fig> [node] [--out preview.html] [--port N] [--bind IP]
                                             local read-only viewer (loopback, random port by default);
                                             --out requires a node and writes an offline snapshot
  figlens node <file.fig> <node>               print one raw .fig node as JSON
  figlens compare <file.fig> <nodes.json> <node>   diff a frame against a REST nodes.json export
  figlens compare <file.fig> --frames <list.txt>   same for every `node|nodes.json[|variables.json]` line
  figlens mcp [--root DIR]...                  serve workflow guidance, tree, find, inspect, nodes, variables,
                                             bundle and export as MCP tools; raster export returns an inline image;
                                             with --root, files and outputs must be under one of the folders
  figlens cache [clear]                        show, or clear, the preview render cache
                                             (prints {path,bytes} or {path,freed})
  figlens update                               install the latest GitHub release over this binary
  figlens --help | --version

<file.fig>: a local path, or gs://bucket/object (downloaded with your gcloud login, cached in
  ~/.cache/figlens until the object changes).
<node>: a Figma URL with ?node-id=..., 12-345 (URL form) or 12:345.
JSON goes to stdout, errors to stderr; exit status 1 on any error.

examples:
  figlens tree design.fig --depth 2
  figlens nodes design.fig 'https://www.figma.com/design/KEY/Name?node-id=12-345' > frame.json";

/// Rendering recurses through the node tree; rayon's 2 MiB default stacks overflow sooner than
/// the main thread's 8 MiB, so every pool gets the main thread's size.
const STACK: usize = 8 << 20;

fn main() -> ExitCode {
    // Fails only if a pool already exists, which nothing builds before this line.
    let _ = rayon::ThreadPoolBuilder::new()
        .stack_size(STACK)
        .build_global();
    let mut out = String::new();
    let result = run(std::env::args().skip(1).collect(), &mut out);
    // A reader that stops early (`figlens tree ... | head`) is not an error.
    match std::io::stdout().write_all(out.as_bytes()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            eprintln!("figlens: {e}");
            return ExitCode::FAILURE;
        }
        _ => {}
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("figlens: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>, out: &mut String) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            writeln!(out, "{USAGE}")?;
            return Ok(());
        }
        Some("-v" | "-V" | "--version") => {
            writeln!(out, "figlens {}", env!("CARGO_PKG_VERSION"))?;
            return Ok(());
        }
        Some("update") => return update(),
        Some("cache") => {
            let dir = figlens::preview::cache_dir()?;
            let report = match args.get(1).map(String::as_str) {
                None => {
                    serde_json::json!({"path": dir, "bytes": figlens::preview::cache_bytes(&dir)})
                }
                Some("clear") => {
                    serde_json::json!({"path": dir, "freed": figlens::preview::clear_cache(&dir)?})
                }
                Some(other) => return Err(format!(
                    "unknown cache action {other:?}; use `figlens cache` or `figlens cache clear`"
                )
                .into()),
            };
            writeln!(out, "{report}")?;
            return Ok(());
        }
        Some("mcp") => return mcp(&args[1..]),
        _ => {}
    }
    let (command, file) = match (args.first(), args.get(1)) {
        (Some(c), Some(f)) => (c.as_str(), f),
        _ => return Err(USAGE.into()),
    };
    let path = figlens::remote::local(file)?;
    if command == "preview" {
        let (mut node, mut output, mut port, mut bind) =
            (None, None, 0u16, "127.0.0.1".parse::<std::net::IpAddr>()?);
        let mut rest = args[2..].iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--out" | "-o" => output = Some(rest.next().ok_or("--out needs a path")?),
                "--port" => port = rest.next().ok_or("--port needs a number")?.parse()?,
                "--bind" => bind = rest.next().ok_or("--bind needs an IP address")?.parse()?,
                _ if !arg.starts_with('-') && node.is_none() => node = Some(normalize_id(arg)),
                _ => return Err(USAGE.into()),
            }
        }
        if let Some(output) = output {
            figlens::preview::snapshot(
                &path,
                node.as_deref().ok_or("--out requires a node")?,
                Path::new(output),
            )?;
            writeln!(
                out,
                "{}",
                serde_json::json!({"path":output,"snapshot":true})
            )?;
            return Ok(());
        }
        return figlens::preview::serve(
            &path,
            node.as_deref(),
            std::net::SocketAddr::new(bind, port),
        );
    }
    let doc = Document::open(&path).map_err(|e| format!("{file}: {e}"))?;
    let message = doc.message()?;
    let tree = Tree::new(&message);
    match command {
        "bundle" if args.get(2).map(String::as_str) == Some("--frames") => {
            batch(&tree, &path, &args[2..])?;
        }
        "tree" | "find" | "inspect" | "node" | "nodes" | "variables" | "bundle" | "export" => {
            let command = command::Command::from_cli(command, &args[2..], &tree)?;
            out.push_str(&command.execute(&tree, &path)?.text()?);
        }
        "compare" => {
            // A `--frames` line may name the capture's variables.json as a third field.
            let jobs: Vec<(String, String, Option<String>)> = match args.get(2).map(String::as_str)
            {
                Some("--frames") => std::fs::read_to_string(args.get(3).ok_or(USAGE)?)?
                    .lines()
                    .filter_map(|l| {
                        let mut f = l.split('|');
                        let (id, path) = (f.next()?, f.next()?);
                        Some((
                            normalize_id(id),
                            path.to_owned(),
                            f.next().map(str::to_owned),
                        ))
                    })
                    .collect(),
                Some(path) => vec![(
                    normalize_id(args.get(3).ok_or(USAGE)?),
                    path.to_owned(),
                    None,
                )],
                None => return Err(USAGE.into()),
            };
            let (mut vars_jobs, mut vars_clean) = (0, 0);
            let (mut clean, mut fills_clean) = (0, 0);
            let mut fields_clean = [0usize; FIELDS.len()];
            for (root, path, vars_path) in &jobs {
                if !tree.nodes.contains_key(root) {
                    return Err(format!(
                        "node {root} not found; list ids with `figlens tree <file.fig> --depth 2`"
                    )
                    .into());
                }
                let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
                let report = compare(&tree, &oracle, root);
                let status = if report.is_clean() { "OK  " } else { "DIFF" };
                writeln!(
                    out,
                    "{status} {root} (direct {}, in instances {})",
                    report.direct, report.in_instances
                )?;
                for (label, lines) in &report.diffs {
                    writeln!(out, "  {label}: {}", lines.len())?;
                    for line in lines.iter().take(6) {
                        writeln!(out, "    {line}")?;
                    }
                }
                if let Some(vars_path) = vars_path {
                    let theirs = serde_json::from_slice(&std::fs::read(vars_path)?)?;
                    // A variables capture has no document and holds every local variable.
                    let doc = oracle["document"].is_object().then(|| nodes(&tree, root));
                    let ours = variables(&tree, doc.as_ref().map(|d| &d["document"]));
                    let lines = figlens::variables::diff(&ours, &theirs);
                    if !lines.is_empty() {
                        writeln!(out, "  variables: {}", lines.len())?;
                        for line in lines.iter().take(6) {
                            writeln!(out, "    {line}")?;
                        }
                    }
                    vars_jobs += 1;
                    vars_clean += lines.is_empty() as usize;
                }
                clean += report.is_clean() as usize;
                fills_clean += (report.is_clean() && report.fills_clean()) as usize;
                for (n, label) in fields_clean.iter_mut().zip(FIELDS) {
                    *n += report.clean(label) as usize;
                }
            }
            writeln!(
                out,
                "identical: {clean} of {} ({fills_clean} also with identical fills)",
                jobs.len()
            )?;
            let fields: Vec<String> = FIELDS
                .iter()
                .zip(fields_clean)
                .map(|(label, n)| format!("{label} {n}"))
                .collect();
            writeln!(out, "frames with identical fields: {}", fields.join(", "))?;
            if vars_jobs > 0 {
                writeln!(out, "identical variables.json: {vars_clean} of {vars_jobs}")?;
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// `bundle --frames <list> --out-dir <dir> [--file-key K] [--jobs N]`: one decode, the frames
/// written in parallel, one JSON line per frame as each finishes. A frame's failure is its own line.
fn batch(tree: &Tree, fig: &Path, args: &[String]) -> Result<(), Box<dyn Error>> {
    use rayon::prelude::*;
    let (mut list, mut dir, mut file_key, mut jobs) = (None, None, None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().cloned().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--frames" => list = Some(value()?),
            "--out-dir" => dir = Some(value()?),
            "--file-key" => file_key = Some(value()?),
            "--jobs" => jobs = Some(value()?.parse::<usize>()?),
            _ => return Err(USAGE.into()),
        }
    }
    let (list, dir) = (list.ok_or(USAGE)?, dir.ok_or("--frames needs --out-dir")?);
    std::fs::create_dir_all(&dir)?;
    let frames: Vec<String> = std::fs::read_to_string(&list)?
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(normalize_id))
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.unwrap_or(0))
        .stack_size(STACK)
        .build()?;
    let stdout = std::sync::Mutex::new(std::io::stdout());
    pool.install(|| {
        frames.par_iter().for_each(|id| {
            let path = Path::new(&dir).join(format!("{}.zip", id.replace(':', "-")));
            let one = || -> Result<Vec<String>, String> {
                if !tree.nodes.contains_key(id) {
                    return Err(format!("node {id} not found"));
                }
                let key = file_key.as_deref();
                figlens::bundle::write(tree, fig, key, None, id, &path).map_err(|e| e.to_string())
            };
            // A panic in one frame must not take the others down.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(one))
                .unwrap_or_else(|_| Err("figlens crashed on this frame".to_owned()));
            let line = match result {
                Ok(warnings) => {
                    serde_json::json!({ "nodeId": id, "path": path, "warnings": warnings })
                }
                Err(e) => serde_json::json!({ "nodeId": id, "error": e }),
            };
            let mut out = stdout.lock().unwrap_or_else(|e| e.into_inner());
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        });
    });
    Ok(())
}

/// An MCP tool parameter: (key, CLI flag or "" for a positional, JSON type, required,
/// description). Positionals keep the CLI's order.
type Param = (&'static str, &'static str, &'static str, bool, &'static str);
const FILE: Param = (
    "file",
    "",
    "string",
    true,
    "the .fig file (File > Save local copy): an absolute path, or gs://bucket/object",
);
const NODE: Param = (
    "node",
    "",
    "string",
    true,
    "a node id (12:345 or 12-345) or a Figma URL with ?node-id=",
);
/// CLI and tools share command execution and result formatting.
const TOOLS: &[(&str, &str, &[Param])] = &[
    (
        "workflow",
        "Read built-in figlens skill guidance for this agent: build for design-to-code and project reuse, images for website asset preparation, plan for scoped implementation planning, planning-details for page/multi-page work or unresolved design dependencies, yolo for explicitly requested autonomous implementation and temporary-plan cleanup. Returns the requested guide only; no design file, project read/write, implementation, verification or cleanup is performed. Ordinary design questions and exports can use the design tools directly.",
        &[(
            "topic",
            "",
            "string",
            true,
            "Guide to read: build, images, plan, planning-details or yolo.",
        )],
    ),
    (
        "tree",
        "List node ids, types and names under a chosen root. A shallow outline locates pages and frames when their names are unknown; find searches known names or visible text directly.",
        &[
            FILE,
            (
                "node",
                "",
                "string",
                false,
                "the root; default the whole document",
            ),
            (
                "depth",
                "--depth",
                "integer",
                false,
                "levels to list below the root",
            ),
            (
                "json",
                "--json",
                "boolean",
                false,
                "nested {id,type,name,children} JSON instead of indented text (about 1.6x larger)",
            ),
        ],
    ),
    (
        "find",
        "Search stored layer names and text (case-insensitive). Returns ids, paths, local sizes and text previews. Does not expand instance children; use inspect on the instance.",
        &[
            FILE,
            (
                "query",
                "",
                "string",
                true,
                "literal substring in a layer name or text",
            ),
            (
                "node",
                "--node",
                "string",
                false,
                "search this subtree; default the document",
            ),
            (
                "type",
                "--type",
                "string",
                false,
                "Figma node type, e.g. FRAME or TEXT",
            ),
            (
                "limit",
                "--limit",
                "integer",
                false,
                "1-200 results, default 20; truncated signals more matches",
            ),
        ],
    ),
    (
        "inspect",
        "Compact expanded node summary: bounds, layout, typography, text previews and child counts. For image decisions, images=true adds original dimensions/bytes and source-pixel rectangles before masks, not blur diagnoses. Depth and a 200-node budget bound the summary; image paints have their own 200-item limit. nodes returns full design data.",
        &[
            FILE,
            NODE,
            (
                "depth",
                "--depth",
                "integer",
                false,
                "0-10 levels below the node, default 1; text previews up to 200 characters",
            ),
            (
                "images",
                "--images",
                "boolean",
                false,
                "include source bitmap dimensions/bytes and per-paint source-pixel crop rectangles; no files written",
            ),
        ],
    ),
    (
        "nodes",
        "A frame as Figma REST nodes.json: instances expanded, fills, text, auto-layout. Can be large.",
        &[FILE, NODE],
    ),
    (
        "variables",
        "The variables a frame binds (collections, modes, aliases); without a node, every local variable.",
        &[
            FILE,
            (
                "node",
                "",
                "string",
                false,
                "the frame; omit for every local variable",
            ),
        ],
    ),
    (
        "bundle",
        "Write a design bundle zip for a frame: nodes, variables, assets, reference render, context.md.",
        &[
            FILE,
            NODE,
            (
                "out",
                "",
                "string",
                true,
                "absolute path of the .zip to write",
            ),
            (
                "fileKey",
                "--file-key",
                "string",
                false,
                "Figma file key for the manifest",
            ),
            (
                "name",
                "--name",
                "string",
                false,
                "frame name for the manifest",
            ),
        ],
    ),
    (
        "export",
        "Export a node to SVG, PNG, JPG, WebP or AVIF. Create the output directory first. Returns metadata and, for raster formats, an inline image. PNG suits inline references. For website assets, follow the target project's existing image pipeline; otherwise start with WebP for bitmaps, sized for display dimensions and DPR using scale/maxSize and quality/maxKb. SVG embeds original images and can be large; keep it for vector artwork. scale, quality and maxSize affect raster formats only; maxSize defaults to 1600. Check returned dimensions/bytes and isError; read content on failure.",
        &[
            FILE,
            NODE,
            (
                "out",
                "",
                "string",
                true,
                "absolute path ending in .png, .jpg, .webp, .avif or .svg",
            ),
            ("scale", "--scale", "number", false, "default 1"),
            (
                "quality",
                "--quality",
                "integer",
                false,
                "1-100, default 80",
            ),
            (
                "maxKb",
                "--max-kb",
                "integer",
                false,
                "lower lossy quality until the file fits; PNG/SVG fail if over budget",
            ),
            (
                "maxSize",
                "--max-size",
                "integer",
                false,
                "longest side in pixels, default 1600; 0 keeps the scaled size",
            ),
        ],
    ),
];

/// `figlens mcp [--root DIR]...`: a Model Context Protocol server, newline-delimited JSON-RPC
/// on stdio. Roots are resolved here, so a missing folder fails before the client connects.
fn mcp(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut roots = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let dir = match (arg.as_str(), args.next()) {
            ("--root", Some(dir)) => dir,
            _ => return Err(USAGE.into()),
        };
        roots.push(std::fs::canonicalize(dir).map_err(|e| format!("--root {dir}: {e}"))?);
    }
    serve(std::io::stdin().lock(), std::io::stdout().lock(), &roots)
}

/// Whether `path` lies under one of `roots`; an output that does not exist yet is judged by
/// its folder. Without roots everything is allowed.
fn allowed(path: &Path, roots: &[PathBuf]) -> bool {
    if roots.is_empty() {
        return true;
    }
    let resolved = match std::fs::canonicalize(path) {
        Ok(resolved) => resolved,
        // Only a path that does not exist at all is judged by its folder: a dangling symlink
        // exists, and writing through it would land wherever it points.
        Err(_) if std::fs::symlink_metadata(path).is_ok() => return false,
        Err(_) => match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => match std::fs::canonicalize(if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            }) {
                Ok(dir) => dir.join(name),
                Err(_) => return false,
            },
            _ => return false,
        },
    };
    roots.iter().any(|root| resolved.starts_with(root))
}

/// The error reply for a tools/call whose `file` or `out` lies outside the roots, or whose
/// arguments leave the tool schema (the legacy coercions could turn any of them into a path).
/// The text is the same whatever exists there, so a refusal reveals nothing about other folders.
fn refused(line: &str, roots: &[PathBuf]) -> Option<serde_json::Value> {
    if roots.is_empty() {
        return None;
    }
    let req: serde_json::Value = serde_json::from_str(line).ok()?;
    if req["method"] != "tools/call" {
        return None;
    }
    let id = req.get("id")?.clone();
    let params = &req["params"];
    let name = params["name"].as_str().unwrap_or("");
    let folders: Vec<_> = roots.iter().map(|r| r.display().to_string()).collect();
    let direct = TOOLS
        .iter()
        .find(|t| t.0 == name)
        .is_some_and(|t| is_direct(name, t.2, &params["arguments"]));
    let text = if !direct {
        "with --root, arguments must match the tool schema".to_owned()
    } else {
        let outside = ["file", "out"]
            .iter()
            .filter_map(|key| params["arguments"][key].as_str())
            .find(|path| !allowed(Path::new(path), roots))?;
        format!(
            "{outside}: outside the folders this server may use ({})",
            folders.join(", ")
        )
    };
    Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": tool_reply(name, Err(text)) }))
}

/// Serves JSON-RPC lines. Decoding a large file takes most of a call, so the parsed tree of
/// the last file stays on this frame and serves the next calls on it until the file changes
/// (path, modification time or length) or a call needs a different one.
fn serve(
    input: impl std::io::BufRead,
    mut output: impl std::io::Write,
    roots: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    let mut lines = input
        .lines()
        .filter(|l| !l.as_ref().is_ok_and(|l| l.trim().is_empty()));
    let mut reply = |line: &str, loaded: Option<(&Path, &Tree)>| -> Result<(), Box<dyn Error>> {
        if let Some(reply) = refused(line, roots).or_else(|| mcp_reply_with(line, loaded, roots)) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
        Ok(())
    };
    let mut carried = None;
    loop {
        let Some(line) = carried
            .take()
            .map(Ok)
            .or_else(|| lines.next())
            .transpose()?
        else {
            return Ok(());
        };
        let loaded = direct_file(&line, roots).and_then(|path| Some((stamp(&path)?, path)));
        let Some((stamp_at_open, path)) = loaded else {
            reply(&line, None)?;
            continue;
        };
        // An unreadable file, or one that panics the decoder, is reported by the per-call path.
        let Ok(Ok(doc)) = std::panic::catch_unwind(|| Document::open(&path)) else {
            reply(&line, None)?;
            continue;
        };
        let Ok(Ok(message)) = std::panic::catch_unwind(|| doc.message()) else {
            reply(&line, None)?;
            continue;
        };
        let Ok(tree) = std::panic::catch_unwind(|| Tree::new(&message)) else {
            reply(&line, None)?;
            continue;
        };
        reply(&line, Some((&path, &tree)))?;
        loop {
            let Some(next) = lines.next().transpose()? else {
                return Ok(());
            };
            match direct_file(&next, roots) {
                // Pings, listings, legacy and remote calls serve themselves; the tree stays.
                None => reply(&next, None)?,
                Some(next_path) if next_path == path && stamp(&path) == Some(stamp_at_open) => {
                    reply(&next, Some((&path, &tree)))?
                }
                Some(_) => {
                    carried = Some(next);
                    break;
                }
            }
        }
    }
}

/// The resolved file of a tools/call that the schema path serves, so `serve` can keep its
/// parsed tree for the next call on the same file.
fn direct_file(line: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    if refused(line, roots).is_some() {
        return None;
    }
    let req: serde_json::Value = serde_json::from_str(line).ok()?;
    // A notification (no id) gets no reply, so its file is never opened.
    if req["method"] != "tools/call" || req.get("id").is_none() {
        return None;
    }
    let params = &req["params"];
    let name = params["name"].as_str()?;
    let tool_params = TOOLS.iter().find(|t| t.0 == name)?.2;
    if !is_direct(name, tool_params, &params["arguments"]) {
        return None;
    }
    let file = params["arguments"]["file"].as_str()?;
    // Remote files resolve through gcloud; the per-call path handles them.
    (!file.starts_with("gs://")).then(|| PathBuf::from(file))
}

/// Modification time and length, so an overwritten save is parsed again.
fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Whether the arguments fit the tool's schema, so the call is served directly rather than
/// through the legacy CLI coercions.
fn is_direct(name: &str, tool_params: &[Param], arguments: &serde_json::Value) -> bool {
    if name == "workflow" && !arguments.as_object().is_some_and(|args| args.len() == 1) {
        return false;
    }
    tool_params
        .iter()
        .all(|(key, _, kind, required, _)| match arguments.get(key) {
            None => !required,
            Some(value) => match *kind {
                "string" => value.is_string(),
                "boolean" => value.is_boolean(),
                _ => value.is_number(),
            },
        })
        && !matches!(
            (name, arguments["node"].as_str()),
            ("tree", Some("--depth" | "--json")) | ("bundle", Some("--frames"))
        )
}

/// The reply to one JSON-RPC message; None for a notification.
#[cfg(test)]
fn mcp_reply(line: &str) -> Option<serde_json::Value> {
    mcp_reply_with(line, None, &[])
}

/// `mcp_reply` with the already parsed tree of the file the call names, if `serve` has it.
fn mcp_reply_with(
    line: &str,
    loaded: Option<(&Path, &Tree)>,
    roots: &[PathBuf],
) -> Option<serde_json::Value> {
    use serde_json::json;
    let Ok(req) = serde_json::from_str::<serde_json::Value>(line) else {
        return Some(json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32700, "message": "parse error" }
        }));
    };
    let id = req.get("id")?.clone();
    let result = match req["method"].as_str().unwrap_or("") {
        // Only tools are served, which every protocol version handles alike, so the client's
        // version is accepted as is.
        "initialize" => {
            let mut instructions = workflow::INSTRUCTIONS.to_owned();
            if !roots.is_empty() {
                let folders: Vec<_> = roots.iter().map(|r| r.display().to_string()).collect();
                instructions +=
                    &format!(" Files and outputs must be under: {}.", folders.join(", "));
            }
            json!({
                "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "figlens", "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": TOOLS.iter().map(|(name, about, params)| {
            let mut tool = json!({
                "name": name,
                "description": about,
                "inputSchema": {
                    "type": "object",
                    "properties": params.iter().map(|(key, _, kind, _, about)|
                        (key.to_string(), json!({ "type": kind, "description": about }))
                    ).collect::<serde_json::Map<_, _>>(),
                    "required": params.iter().filter(|p| p.3).map(|p| p.0).collect::<Vec<_>>(),
                }
            });
            if let Some(schema) = output_schema(name) {
                tool["outputSchema"] = schema;
            }
            if *name == "workflow" {
                tool["inputSchema"]["properties"]["topic"]["enum"] = json!(workflow::TOPICS);
                tool["inputSchema"]["additionalProperties"] = json!(false);
                tool["annotations"] = json!({ "readOnlyHint": true, "openWorldHint": false });
            }
            tool
        }).collect::<Vec<_>>() }),
        "tools/call" => call_tool(&req["params"], loaded),
        method => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("unknown method {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// A failure is a tool result with isError, not a protocol error.
fn call_tool(params: &serde_json::Value, loaded: Option<(&Path, &Tree)>) -> serde_json::Value {
    let name = params["name"].as_str().unwrap_or("");
    if name == "workflow" {
        return tool_reply(
            name,
            workflow::read(&params["arguments"]).map(command::Output::Text),
        );
    }
    let result = match TOOLS.iter().find(|t| t.0 == name) {
        None => Err(format!("unknown tool {name}")),
        Some((_, _, tool_params)) => {
            // A panic on one file must not stop the server.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let arguments = &params["arguments"];
                if is_direct(name, tool_params, arguments) {
                    let run = |tree: &Tree, path: &Path| {
                        command::Command::from_tool(name, arguments, tree)?.execute(tree, path)
                    };
                    if let Some((path, tree)) = loaded {
                        return run(tree, path);
                    }
                    let file = arguments["file"].as_str().ok_or(USAGE)?;
                    let path = figlens::remote::local(file)?;
                    let doc = Document::open(&path).map_err(|e| format!("{file}: {e}"))?;
                    let message = doc.message()?;
                    let tree = Tree::new(&message);
                    run(&tree, &path)
                } else {
                    // Preserve the old permissive coercions for arguments outside the tool schema.
                    let mut args = vec![name.to_owned()];
                    for (key, flag, ..) in *tool_params {
                        let value = match &arguments[key] {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(true) => {
                                args.push(flag.to_string());
                                continue;
                            }
                            _ => continue,
                        };
                        if !flag.is_empty() {
                            args.push(flag.to_string());
                        }
                        args.push(value);
                    }
                    if name == "export" {
                        if arguments.get("maxSize").is_none() {
                            args.extend(["--max-size".to_owned(), "1600".to_owned()]);
                        }
                        // Keep the rendered bytes for inline images on legacy calls too.
                        let file = args.get(1).ok_or(USAGE)?;
                        let path = figlens::remote::local(file)?;
                        let doc = Document::open(&path).map_err(|e| format!("{file}: {e}"))?;
                        let message = doc.message()?;
                        let tree = Tree::new(&message);
                        command::Command::from_cli(name, &args[2..], &tree)?.execute(&tree, &path)
                    } else {
                        let mut out = String::new();
                        run(args, &mut out)?;
                        Ok(command::Output::Text(out))
                    }
                }
            }))
            .map_err(|_| "figlens crashed on this file".to_owned())
            .and_then(|result: Result<command::Output, Box<dyn Error>>| {
                result.map_err(|e| e.to_string())
            })
        }
    };
    tool_reply(name, result)
}

fn tool_reply(name: &str, result: Result<command::Output, String>) -> serde_json::Value {
    let mut structured = None;
    let mut image = None;
    let result = result.and_then(|output| {
        let text = output.text().map_err(|e| e.to_string())?;
        if let command::Output::Image { bytes, mime, .. } = &output {
            image = Some(serde_json::json!({
                "type": "image", "data": figlens::render::base64(bytes), "mimeType": mime,
            }));
        }
        if output_schema(name).is_some() {
            structured = output.json().cloned().or_else(|| {
                // Only legacy, out-of-schema calls still return untyped CLI text.
                serde_json::from_str(&text).ok()
            });
        }
        Ok(text)
    });
    let (text, is_error) = match result {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    let mut reply = serde_json::json!({
        "content": [{ "type": "text", "text": text }], "isError": is_error
    });
    if let Some(value) = structured {
        reply["structuredContent"] = value;
    }
    if let Some(image) = image {
        reply["content"].as_array_mut().unwrap().push(image);
    }
    reply
}

/// The MCP outputSchema of the tools whose CLI command prints one JSON object.
fn output_schema(tool: &str) -> Option<serde_json::Value> {
    let (properties, required) = match tool {
        "find" => (
            serde_json::json!({
                "root": {"type": "string"},
                "results": {"type": "array", "items": {"type": "object"}},
                "truncated": {"type": "boolean"},
            }),
            ["root", "results", "truncated"].as_slice(),
        ),
        "inspect" => (
            serde_json::json!({
                "node": {"type": "object"},
                "truncated": {"type": "boolean"},
                "images": {"type": "object", "description": "optional source image evidence when images=true"},
            }),
            ["node", "truncated"].as_slice(),
        ),
        "export" => (
            serde_json::json!({
                "path": { "type": "string" },
                "width": { "type": "integer" },
                "height": { "type": "integer" },
                "bytes": { "type": "integer" },
                "quality": { "type": "integer", "description": "the quality used; --max-kb may lower it" },
            }),
            ["path", "width", "height", "bytes", "quality"].as_slice(),
        ),
        "bundle" => (
            serde_json::json!({
                "nodeId": { "type": "string" },
                "path": { "type": "string" },
                "warnings": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "assets the bundle could not include; missing-assets.json has details",
                },
            }),
            ["nodeId", "path", "warnings"].as_slice(),
        ),
        _ => return None,
    };
    Some(serde_json::json!({ "type": "object", "properties": properties, "required": required }))
}

/// Reruns the release installer (scripts/install.sh, or install.ps1 on Windows) into this
/// binary's directory.
fn update() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    // Follow a symlink to the real binary; Windows paths stay plain (no `\\?\` prefix).
    let exe = if cfg!(windows) {
        exe
    } else {
        exe.canonicalize()?
    };
    let dir = exe.parent().ok_or("cannot locate the figlens binary")?;
    let script = if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    };
    // Download completely before running anything: a cut-off script must not run half.
    let fetched = Command::new("curl")
        .args(["-fsSL"])
        .arg(format!(
            "https://raw.githubusercontent.com/runkids/figlens/main/scripts/{script}"
        ))
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| {
            format!(
                "update needs `curl`: {e}; without it, download the archive from \
                 https://github.com/runkids/figlens/releases/latest"
            )
        })?;
    if !fetched.status.success() {
        return Err("could not download the install script".into());
    }
    let ran = if cfg!(windows) {
        // PowerShell runs a multi-line script reliably only from a file. %TEMP% is per user.
        let path = std::env::temp_dir().join(format!("figlens-{}-{script}", std::process::id()));
        std::fs::write(&path, &fetched.stdout)?;
        let ran = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&path)
            .env("FIGLENS_BIN_DIR", dir)
            .status();
        let _ = std::fs::remove_file(&path);
        ran?
    } else {
        // Piped from memory: no file in a shared /tmp that another user could swap.
        let mut sh = Command::new("sh")
            .env("FIGLENS_BIN_DIR", dir)
            .stdin(Stdio::piped())
            .spawn()?;
        sh.stdin
            .take()
            .ok_or("sh has no stdin")?
            .write_all(&fetched.stdout)?;
        sh.wait()?
    };
    if !ran.success() {
        return Err("install script failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_lists_every_tool() {
        let reply = mcp_reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        let names: Vec<_> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "workflow",
                "tree",
                "find",
                "inspect",
                "nodes",
                "variables",
                "bundle",
                "export"
            ]
        );
    }

    #[test]
    fn mcp_declares_output_schemas_for_json_results() {
        let reply = mcp_reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        let with_schema: Vec<_> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["outputSchema"].is_object())
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(with_schema, ["find", "inspect", "bundle", "export"]);
    }

    #[test]
    fn mcp_workflows_work_without_design_files_under_restricted_roots() {
        let dir = test_support::TestDir::new();
        let roots = [dir.join("").canonicalize().unwrap()];
        let mut requests = vec![
            serde_json::json!({"id": 1, "method": "initialize", "params": {}}),
            serde_json::json!({"id": 2, "method": "tools/list"}),
        ];
        for topic in workflow::TOPICS {
            requests.push(
                serde_json::json!({"id": topic, "method": "tools/call", "params": {
                    "name": "workflow", "arguments": {"topic": topic}
                }}),
            );
        }
        let input = requests
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = Vec::new();
        serve(std::io::Cursor::new(input), &mut out, &roots).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("workflow(topic=build)"));
        assert!(instructions.contains("Simple questions and exports"));
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        let tool = tools.iter().find(|t| t["name"] == "workflow").unwrap();
        assert_eq!(
            tool["inputSchema"]["required"],
            serde_json::json!(["topic"])
        );
        assert_eq!(
            tool["inputSchema"]["properties"]["topic"]["enum"],
            serde_json::json!(workflow::TOPICS)
        );
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        for reply in &replies[2..] {
            assert_eq!(reply["result"]["isError"], false, "{reply}");
            let text = reply["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("host's own tools"));
            assert!(!text.contains("](references/") && !text.contains("](planning"));
        }
        assert_eq!(std::fs::read_dir(dir.join("")).unwrap().count(), 0);
    }

    #[test]
    fn mcp_workflow_rejects_invalid_arguments_without_cli_fallback() {
        for arguments in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"topic": 1}),
            serde_json::json!({"topic": "../SKILL.md"}),
            serde_json::json!({"topic": "build", "file": "/private/design.fig"}),
        ] {
            let request = serde_json::json!({"id": 1, "method": "tools/call", "params": {
                "name": "workflow", "arguments": arguments
            }});
            assert_eq!(direct_file(&request.to_string(), &[]), None);
            let result = call_tool(&request["params"], None);
            assert_eq!(result["isError"], true, "{result}");
            assert!(
                !result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("usage:")
            );
        }
    }

    #[test]
    fn tree_json_nests_children_down_to_the_depth() {
        use kiwi_schema::Value;
        use std::collections::HashMap;
        let g = |l| {
            Value::Object(
                "GUID",
                HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(l))]),
            )
        };
        let node = |l: u32, parent: Option<u32>| {
            let mut fields = HashMap::from([
                ("guid", g(l)),
                ("type", Value::Enum("NodeType", "FRAME")),
                ("name", Value::String(format!("n{l}"))),
            ]);
            if let Some(p) = parent {
                fields.insert(
                    "parentIndex",
                    Value::Object(
                        "ParentIndex",
                        HashMap::from([("guid", g(p)), ("position", Value::String("a".into()))]),
                    ),
                );
            }
            Value::Object("NodeChange", fields)
        };
        let message = Value::Object(
            "Message",
            HashMap::from([(
                "nodeChanges",
                Value::Array(vec![node(0, None), node(1, Some(0)), node(2, Some(1))]),
            )]),
        );
        let tree = Tree::new(&message);
        assert_eq!(
            command::Command::from_cli(
                "tree",
                &["--depth".into(), "1".into(), "--json".into()],
                &tree
            )
            .unwrap()
            .execute(&tree, Path::new("unused.fig"))
            .unwrap()
            .json()
            .unwrap(),
            &serde_json::json!({ "id": "0:0", "type": "FRAME", "name": "n0",
                "children": [{ "id": "0:1", "type": "FRAME", "name": "n1" }] })
        );
    }

    #[test]
    fn mcp_ignores_notifications() {
        assert!(mcp_reply(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn roots_confine_files_and_outputs() {
        let inside = test_support::TestDir::new();
        let outside = test_support::TestDir::new();
        let roots = [std::fs::canonicalize(inside.join("")).unwrap()];
        assert!(allowed(&inside.join("a.fig"), &roots));
        assert!(!allowed(&outside.join("a.fig"), &roots));
        assert!(!allowed(&inside.join("../escape.fig"), &roots));
        assert!(!allowed(Path::new("gs://bucket/a.fig"), &roots));
        assert!(allowed(&outside.join("a.fig"), &[]));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("new.png"), inside.join("link.png")).unwrap();
            assert!(!allowed(&inside.join("link.png"), &roots));
        }
        // Out-of-schema arguments would reach the legacy coercions, which can make any of
        // them the file path.
        let legacy = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"tree","arguments":{{"node":{}}}}}}}"#,
            serde_json::json!(outside.join("a.fig"))
        );
        let text = refused(&legacy, &roots).unwrap()["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(text.contains("match the tool schema"), "{text}");

        let call = |file: &Path| {
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"tree","arguments":{{"file":{}}}}}}}"#,
                serde_json::json!(file)
            )
        };
        let input = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{{}}}}\n{}\n{}\n",
            call(&outside.join("a.fig")),
            call(&inside.join("a.fig"))
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &roots).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(&roots[0].display().to_string()),
            "{instructions}"
        );
        let text = replies[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(replies[1]["result"]["isError"], true);
        assert!(text.contains("outside the folders"), "{text}");
        // Inside the root the call runs and fails on the missing file instead.
        let text = replies[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("outside the folders"), "{text}");
    }

    #[test]
    fn direct_file_names_only_schema_calls() {
        let call = |args: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"tree","arguments":{args}}}}}"#
            )
        };
        assert_eq!(
            direct_file(&call(r#"{"file":"/a.fig","depth":1}"#), &[]),
            Some(PathBuf::from("/a.fig"))
        );
        assert_eq!(
            direct_file(&call(r#"{"file":"/a.fig","node":"--depth"}"#), &[]),
            None
        );
        assert_eq!(
            direct_file(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &[]),
            None
        );
        let notification = call(r#"{"file":"/a.fig"}"#).replace(r#""id":1,"#, "");
        assert_eq!(direct_file(&notification, &[]), None);
    }

    #[test]
    fn serve_answers_every_call_when_the_file_cannot_be_opened() {
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tree","arguments":{"file":"/missing.fig"}}}"#;
        let input =
            format!("{call}\n{call}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n");
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &[]).unwrap();
        let replies: Vec<serde_json::Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(replies[1]["result"]["isError"], true);
        assert_eq!(replies[2]["result"], serde_json::json!({}));
    }

    #[test]
    fn mcp_reports_a_failed_call_as_a_tool_error() {
        let reply = mcp_reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"tree","arguments":{"file":"/missing.fig"}}}"#,
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }
}
