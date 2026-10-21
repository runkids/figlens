use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use figlens::compare::{FIELDS, compare};
use figlens::rest::nodes;
use figlens::variables::variables;
use figlens::{Document, Tree, normalize_id, to_json};

const USAGE: &str = "figlens — read Figma .fig files (File > Save local copy) without Figma

usage:
  figlens tree <file.fig> [node] [--depth N]   list ids, types and names (default root: the document)
  figlens nodes <file.fig> <node>              print a frame as Figma REST-shaped nodes.json
                                             (instances expanded, fills/text/auto-layout included)
  figlens variables <file.fig> [node]          print the variables a frame binds (design/variables.json);
                                             without a node, every local variable
  figlens bundle <file.fig> <node> <out.zip>   write a design bundle for a frame: nodes, variables,
                                             image/SVG assets, reference render and context.md, in one zip
  figlens bundle ... --file-key KEY --name N   set the manifest's file key and frame name
  figlens bundle <file.fig> --frames <list> --out-dir <dir> [--file-key KEY] [--jobs N]
                                            one decode, many bundles: <list> has one node id per line;
                                             writes <dir>/<nodeId>.zip (`:` as `-`), prints one JSON line
                                             per frame ({nodeId,path} or {nodeId,error})
  figlens export <file.fig> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]
                                            render a node to out.png|jpg|webp|avif (default 1x,
                                             quality 80); --max-kb lowers the quality until it fits
  figlens node <file.fig> <node>               print one raw .fig node as JSON
  figlens compare <file.fig> <nodes.json> <node>   diff a frame against a REST nodes.json export
  figlens compare <file.fig> --frames <list.txt>   same for every `node|nodes.json[|variables.json]` line
  figlens mcp                                  serve tree, nodes, variables, bundle and export as
                                             MCP tools over stdio (for AI agents)
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
        Some("mcp") => return mcp(),
        _ => {}
    }
    let (command, file) = match (args.first(), args.get(1)) {
        (Some(c), Some(f)) => (c.as_str(), f),
        _ => return Err(USAGE.into()),
    };
    let path = figlens::remote::local(file)?;
    let doc = Document::open(&path).map_err(|e| format!("{file}: {e}"))?;
    let message = doc.message()?;
    let tree = Tree::new(&message);
    match command {
        "tree" => {
            let mut root = "0:0".to_owned();
            let mut depth = usize::MAX;
            let mut rest = args[2..].iter();
            while let Some(arg) = rest.next() {
                if arg == "--depth" {
                    depth = rest.next().ok_or("--depth needs a number")?.parse()?;
                } else {
                    root = normalize_id(arg);
                }
            }
            if !tree.nodes.contains_key(&root) {
                return Err(format!(
                    "node {root} not found; list ids with `figlens tree <file.fig> --depth 2`"
                )
                .into());
            }
            print_tree(out, &tree, &root, 0, depth)?;
        }
        "node" => {
            let id = normalize_id(args.get(2).ok_or(USAGE)?);
            let node = tree.nodes.get(&id).ok_or_else(|| {
                format!("node {id} not found; list ids with `figlens tree <file.fig> --depth 2`")
            })?;
            writeln!(out, "{}", serde_json::to_string_pretty(&to_json(node))?)?;
        }
        "nodes" => {
            let id = normalize_id(args.get(2).ok_or(USAGE)?);
            if !tree.nodes.contains_key(&id) {
                return Err(format!(
                    "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                )
                .into());
            }
            writeln!(out, "{}", serde_json::to_string_pretty(&nodes(&tree, &id))?)?;
        }
        "variables" => {
            // Without a node: every local variable.
            let doc = match args.get(2) {
                Some(node) => {
                    let id = normalize_id(node);
                    if !tree.nodes.contains_key(&id) {
                        return Err(format!(
                            "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                        )
                        .into());
                    }
                    Some(nodes(&tree, &id))
                }
                None => None,
            };
            let vars = variables(&tree, doc.as_ref().map(|d| &d["document"]));
            writeln!(out, "{}", serde_json::to_string_pretty(&vars)?)?;
        }
        "bundle" if args.get(2).map(String::as_str) == Some("--frames") => {
            batch(&tree, &path, &args[2..])?;
        }
        "bundle" => {
            let id = normalize_id(args.get(2).ok_or(USAGE)?);
            let out_path = args.get(3).ok_or(USAGE)?;
            if !tree.nodes.contains_key(&id) {
                return Err(format!(
                    "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                )
                .into());
            }
            let (mut file_key, mut name) = (None, None);
            let mut rest = args[4..].iter();
            while let Some(arg) = rest.next() {
                let mut value = || rest.next().cloned().ok_or(format!("{arg} needs a value"));
                match arg.as_str() {
                    "--file-key" => file_key = Some(value()?),
                    "--name" => name = Some(value()?),
                    _ => return Err(USAGE.into()),
                }
            }
            // Without --file-key the key comes from a Figma URL; it names the source in the manifest.
            let key = file_key.as_deref().or_else(|| {
                args[2]
                    .split_once("/design/")
                    .or_else(|| args[2].split_once("/file/"))
                    .and_then(|(_, rest)| rest.split('/').next())
            });
            for w in figlens::bundle::write(
                &tree,
                &path,
                key,
                name.as_deref(),
                &id,
                Path::new(out_path),
            )? {
                eprintln!("figlens: {w}");
            }
        }
        "export" => {
            let id = normalize_id(args.get(2).ok_or(USAGE)?);
            let out_path = Path::new(args.get(3).ok_or(USAGE)?);
            if !tree.nodes.contains_key(figlens::export::root_of(&id)) {
                return Err(format!(
                    "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                )
                .into());
            }
            let format = figlens::export::Format::of(out_path)
                .ok_or("the output must end in .png, .jpg, .webp or .avif")?;
            let (mut scale, mut quality, mut max_kb) = (1.0_f64, 80, None);
            let mut rest = args[4..].iter();
            while let Some(arg) = rest.next() {
                let mut value = || rest.next().ok_or(format!("{arg} needs a number"));
                match arg.as_str() {
                    "--scale" => scale = value()?.trim_end_matches('x').parse()?,
                    "--quality" => quality = value()?.parse()?,
                    "--max-kb" => max_kb = Some(value()?.parse::<usize>()?),
                    _ => return Err(USAGE.into()),
                }
            }
            if !(1..=100).contains(&quality) || scale.is_nan() || scale <= 0.0 {
                return Err("--quality takes 1-100 and --scale a positive number".into());
            }
            let image = figlens::export::export(
                &tree,
                &path,
                &id,
                format,
                scale,
                quality,
                max_kb.map(|kb| kb * 1024),
            )?;
            std::fs::write(out_path, &image.bytes)?;
            writeln!(
                out,
                "{}: {}x{}, {} KB, quality {}",
                out_path.display(),
                image.width,
                image.height,
                image.bytes.len().div_ceil(1024),
                image.quality
            )?;
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
            let one = || -> Result<(), String> {
                if !tree.nodes.contains_key(id) {
                    return Err(format!("node {id} not found"));
                }
                let key = file_key.as_deref();
                for w in figlens::bundle::write(tree, fig, key, None, id, &path)
                    .map_err(|e| e.to_string())?
                {
                    eprintln!("figlens: {id}: {w}");
                }
                Ok(())
            };
            // A panic in one frame must not take the others down.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(one))
                .unwrap_or_else(|_| Err("figlens crashed on this frame".to_owned()));
            let line = match result {
                Ok(()) => serde_json::json!({ "nodeId": id, "path": path }),
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
/// Each tool runs the CLI command of the same name, so tools and commands cannot drift apart.
const TOOLS: &[(&str, &str, &[Param])] = &[
    (
        "tree",
        "List node ids, types and names. Start here with depth 2 to find pages and frames.",
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
        "Render a node to PNG, JPG, WebP or AVIF, like Figma's Export panel.",
        &[
            FILE,
            NODE,
            (
                "out",
                "",
                "string",
                true,
                "absolute path ending in .png, .jpg, .webp or .avif",
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
                "lower the quality until the file fits",
            ),
        ],
    ),
];

/// `figlens mcp`: a Model Context Protocol server, newline-delimited JSON-RPC on stdio.
fn mcp() -> Result<(), Box<dyn Error>> {
    for line in std::io::stdin().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = mcp_reply(&line) {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The reply to one JSON-RPC message; None for a notification.
fn mcp_reply(line: &str) -> Option<serde_json::Value> {
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
        "initialize" => json!({
            "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "figlens", "version": env!("CARGO_PKG_VERSION") }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": TOOLS.iter().map(|(name, about, params)| json!({
            "name": name,
            "description": about,
            "inputSchema": {
                "type": "object",
                "properties": params.iter().map(|(key, _, kind, _, about)|
                    (key.to_string(), json!({ "type": kind, "description": about }))
                ).collect::<serde_json::Map<_, _>>(),
                "required": params.iter().filter(|p| p.3).map(|p| p.0).collect::<Vec<_>>(),
            }
        })).collect::<Vec<_>>() }),
        "tools/call" => call_tool(&req["params"]),
        method => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("unknown method {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Runs a tool as its CLI command; a failure is a tool result with isError, not a protocol error.
fn call_tool(params: &serde_json::Value) -> serde_json::Value {
    let name = params["name"].as_str().unwrap_or("");
    let result = match TOOLS.iter().find(|t| t.0 == name) {
        None => Err(format!("unknown tool {name}")),
        Some((_, _, tool_params)) => {
            let mut args = vec![name.to_owned()];
            for (key, flag, ..) in *tool_params {
                let value = match &params["arguments"][key] {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if !flag.is_empty() {
                    args.push(flag.to_string());
                }
                args.push(value);
            }
            // shortcut: every call decodes the file again; cache the Tree if agents call in bursts.
            let mut out = String::new();
            // A panic on one file must not stop the server.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(args, &mut out))) {
                // `bundle` prints nothing on success; an empty reply reads as a failure to an agent.
                Ok(Ok(())) if out.is_empty() => Ok("done".to_owned()),
                Ok(Ok(())) => Ok(out),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("figlens crashed on this file".to_owned()),
            }
        }
    };
    let (text, is_error) = match result {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    serde_json::json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
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

fn print_tree(
    out: &mut String,
    tree: &Tree,
    id: &str,
    level: usize,
    depth: usize,
) -> std::fmt::Result {
    let node = tree.nodes[id];
    let kind = node.get("type").map(|t| match t {
        kiwi_schema::Value::Enum(_, name) => *name,
        _ => "?",
    });
    let name = node.get("name").map(|n| n.as_string()).unwrap_or("");
    writeln!(
        out,
        "{}{} {} \"{}\"",
        "  ".repeat(level),
        id,
        kind.unwrap_or("?"),
        name
    )?;
    if level < depth {
        for child in tree.children.get(id).into_iter().flatten() {
            print_tree(out, tree, child, level + 1, depth)?;
        }
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
        assert_eq!(names, ["tree", "nodes", "variables", "bundle", "export"]);
    }

    #[test]
    fn mcp_ignores_notifications() {
        assert!(mcp_reply(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
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
