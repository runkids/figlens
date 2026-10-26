use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use figlens::compare::{FIELDS, compare};
use figlens::rest::nodes;
use figlens::variables::variables;
use figlens::{Document, Tree, normalize_id};

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

mod command;
mod contract;
mod mcp;
mod workflow;

const USAGE: &str = "figlens — read Figma .fig files (File > Save local copy) without Figma

usage:
  figlens tree <file.fig> [node] [--depth N] [--json]
                                             list ids, types and names (default root: the document);
                                             --json prints them as nested {id,type,name,children}
  figlens find <file.fig> <query> [--type TYPE] [--node ID] [--limit N]
                                             search stored layer names and text, case-insensitive;
                                             default 20 results (1-200), with paths and truncation flag
  figlens inspect <file.fig> <node> [--depth N] [--images] [--states]
                                             compact layout/text summary, instances expanded;
                                             depth 1 (0-10), at most 200 nodes, 200 characters per text;
                                             --images adds source pixels/bytes and source-pixel crop rectangles;
                                             --states lists related component variants and evidence gaps
  figlens nodes <file.fig> <node>              print a frame as Figma REST-shaped nodes.json
                                             (instances expanded, fills/text/auto-layout included)
  figlens variables <file.fig> [node]          print the variables a frame binds (design/variables.json);
                                             without a node, every local variable
  figlens bundle <file.fig> <node> <out.zip> [--states]
                                             write a design bundle for a frame: nodes, variables,
                                             image/SVG assets, reference render and context.md, in one zip;
                                             prints {nodeId,path,warnings}
  figlens bundle ... --file-key KEY --name N   set the manifest's file key and frame name;
                                             --states adds related variants and evidence gaps to the bundle
  figlens bundle <file.fig> --frames <list> --out-dir <dir> [--file-key KEY] [--jobs N] [--states]
                                            one decode, many bundles: <list> has one node id per line;
                                             writes <dir>/<nodeId>.zip (`:` as `-`), prints one JSON line
                                             per frame ({nodeId,path,warnings} or {nodeId,error});
                                             --jobs limits concurrent frames (0/default: all cores), not rendering
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
        nodes/inspect/export/bundle also accept full instance sublayer IDs from inspect/nodes.
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
    let path = figlens::remote::local(file).map_err(|e| contract::source(file, e))?;
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
    let mut doc = Document::open(&path).map_err(|e| contract::source(file, e))?;
    let message = doc.take_message()?;
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
    let (mut list, mut dir, mut file_key, mut jobs, mut states) = (None, None, None, None, false);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().cloned().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--frames" => list = Some(value()?),
            "--out-dir" => dir = Some(value()?),
            "--file-key" => file_key = Some(value()?),
            "--jobs" => jobs = Some(value()?.parse::<usize>()?),
            "--states" => states = true,
            _ => return Err(USAGE.into()),
        }
    }
    let (list, dir) = (list.ok_or(USAGE)?, dir.ok_or("--frames needs --out-dir")?);
    std::fs::create_dir_all(&dir)?;
    let frames: Vec<String> = std::fs::read_to_string(&list)?
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(normalize_id))
        .collect();
    let jobs = jobs
        .filter(|&n| n > 0)
        .unwrap_or_else(rayon::current_num_threads);
    let stdout = std::sync::Mutex::new(std::io::stdout());
    let write_frame = |id: &String| {
        let path = Path::new(&dir).join(format!("{}.zip", id.replace(':', "-")));
        let one = || -> Result<Vec<String>, String> {
            let key = file_key.as_deref();
            figlens::bundle::write_with_states(tree, fig, key, None, id, &path, states)
                .map_err(|e| e.to_string())
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
    };
    let workers = jobs.min(frames.len());
    let frames = std::sync::Mutex::new(frames.iter());
    // OS workers bound frames without confining inner rendering to a frame-sized rayon pool.
    std::thread::scope(|scope| -> std::io::Result<()> {
        for _ in 0..workers {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(scope, || {
                    loop {
                        let Some(id) = frames.lock().unwrap().next() else {
                            break;
                        };
                        write_frame(id);
                    }
                })?;
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
use contract::TOOLS;
#[cfg(test)]
use mcp::{call_tool, mcp_reply, serve};

/// Start stdio after canonicalizing configured roots.
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
    mcp::serve(std::io::stdin().lock(), std::io::stdout().lock(), &roots)
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
