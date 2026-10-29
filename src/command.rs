//! Binary-private command semantics shared by the CLI and MCP adapters.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use figlens::expand::plugin_type;
use figlens::export::SvgIndex;
use figlens::rest::resolve_nodes;
use figlens::variables::variables;
use figlens::{Tree, normalize_id, to_json};
use serde_json::{Value, json};

use crate::USAGE;

mod discover;
#[cfg(test)]
mod instance_tests;
#[cfg(test)]
mod tests;

pub enum Command {
    Tree {
        root: String,
        depth: usize,
        json: bool,
        top_frames: bool,
    },
    Node(String),
    /// The node, and an MCP reply budget that stops oversized JSON before it is built.
    Nodes(String, Option<usize>),
    Find {
        query: String,
        root: String,
        kind: Option<String>,
        limit: usize,
    },
    Inspect {
        node: String,
        depth: usize,
        images: bool,
        states: bool,
        compare_variant: Option<String>,
        /// One line per node instead of JSON.
        flat: bool,
    },
    Variables(Option<String>),
    Bundle {
        node: String,
        out: PathBuf,
        file_key: Option<String>,
        name: Option<String>,
        states: bool,
    },
    Export {
        node: String,
        out: PathBuf,
        format: figlens::export::Format,
        /// Export the layout box (`absoluteBoundingBox`) instead of the render bounds.
        layout: bool,
        scale: f64,
        quality: u8,
        max_kb: Option<usize>,
        /// Longest side in pixels; raster formats are scaled down to fit.
        max_size: Option<u32>,
        /// SVGs to report the files drawing the same or a similar shape.
        match_index: Option<Arc<SvgIndex>>,
    },
}

pub enum Output {
    Text(String),
    /// Text for people and grep, with the same content as JSON for MCP structured output.
    Lines {
        text: String,
        value: Value,
    },
    Json {
        value: Value,
        pretty: bool,
    },
    Image {
        value: Value,
        bytes: Vec<u8>,
        mime: &'static str,
    },
}

impl Output {
    pub fn text(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Text(text) | Self::Lines { text, .. } => Ok(text.clone()),
            Self::Image { value, .. } => Ok(format!("{}\n", serde_json::to_string(value)?)),
            Self::Json { value, pretty } => Ok(format!(
                "{}\n",
                if *pretty {
                    serde_json::to_string_pretty(value)?
                } else {
                    serde_json::to_string(value)?
                }
            )),
        }
    }

    pub fn json(&self) -> Option<&Value> {
        match self {
            Self::Json { value, .. } | Self::Image { value, .. } | Self::Lines { value, .. } => {
                Some(value)
            }
            Self::Text(_) => None,
        }
    }
}

fn require_node(tree: &Tree, id: &str) -> Result<(), Box<dyn Error>> {
    if !tree.nodes.contains_key(id) {
        if id.starts_with('I') {
            resolve_nodes(tree, id)?;
            return Err(figlens::FigError::new(figlens::ErrorKind::UnsupportedNode, format!(
                "node {id} is an instance sublayer; this operation supports stored nodes only; use inspect, nodes, export or bundle with the full ID"
            )).into());
        }
        return Err(figlens::FigError::new(
            figlens::ErrorKind::NodeNotFound,
            format!("node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"),
        )
        .into());
    }
    Ok(())
}

// Only a cheap root check during argument parsing; the shared resolver verifies the full ID.
fn require_target_root(tree: &Tree, id: &str) -> Result<(), Box<dyn Error>> {
    if !tree.nodes.contains_key(figlens::export::root_of(id)) {
        return Err(figlens::FigError::new(
            figlens::ErrorKind::NodeNotFound,
            format!("node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"),
        )
        .into());
    }
    Ok(())
}

impl Command {
    /// CLI parsing stays after document loading, including node checks before trailing options.
    pub fn from_cli(name: &str, args: &[String], tree: &Tree) -> Result<Self, Box<dyn Error>> {
        let node = || args.first().cloned().ok_or(USAGE);
        Ok(match name {
            "tree" => {
                let (mut root, mut depth) = ("0:0".to_owned(), usize::MAX);
                let (mut json, mut top_frames) = (false, false);
                let mut rest = args.iter();
                while let Some(arg) = rest.next() {
                    if arg == "--depth" {
                        depth = rest.next().ok_or("--depth needs a number")?.parse()?;
                    } else if arg == "--json" {
                        json = true;
                    } else if arg == "--top-frames" {
                        top_frames = true;
                    } else {
                        root = normalize_id(arg);
                    }
                }
                Self::Tree {
                    root,
                    depth,
                    json,
                    top_frames,
                }
            }
            "node" => Self::Node(node()?),
            "nodes" => Self::Nodes(node()?, None),
            "find" => {
                let query = node()?;
                let (mut root, mut kind, mut limit) = ("0:0".to_owned(), None, 20);
                let mut rest = args[1..].iter();
                while let Some(arg) = rest.next() {
                    let value = rest.next().ok_or(format!("{arg} needs a value"))?;
                    match arg.as_str() {
                        "--node" => root = normalize_id(value),
                        "--type" => kind = Some(value.to_uppercase()),
                        "--limit" => limit = value.parse()?,
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Find {
                    query,
                    root,
                    kind,
                    limit,
                }
            }
            "inspect" => {
                let node = node()?;
                let (mut depth, mut images, mut states, mut flat) = (1, false, false, false);
                let mut compare_variant = None;
                let mut rest = args[1..].iter();
                while let Some(flag) = rest.next() {
                    match flag.as_str() {
                        "--depth" => {
                            depth = rest.next().ok_or("--depth needs a number")?.parse()?
                        }
                        "--images" => images = true,
                        "--states" => states = true,
                        "--flat" => flat = true,
                        "--compare-variant" => {
                            compare_variant = Some(
                                rest.next()
                                    .ok_or("--compare-variant needs a node ID")?
                                    .clone(),
                            )
                        }
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Inspect {
                    node,
                    depth,
                    images,
                    states,
                    compare_variant,
                    flat,
                }
            }
            "variables" => Self::Variables(args.first().cloned()),
            "bundle" => {
                let node = node()?;
                let out = args.get(1).cloned().ok_or(USAGE)?;
                require_target_root(tree, &normalize_id(&node))?;
                let (mut file_key, mut name, mut states) = (None, None, false);
                let mut rest = args[2..].iter();
                while let Some(arg) = rest.next() {
                    let mut value = || rest.next().cloned().ok_or(format!("{arg} needs a value"));
                    match arg.as_str() {
                        "--file-key" => file_key = Some(value()?),
                        "--name" => name = Some(value()?),
                        "--states" => states = true,
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Bundle {
                    node,
                    out: out.into(),
                    file_key,
                    name,
                    states,
                }
            }
            "export" => {
                let node = node()?;
                let out = args.get(1).cloned().ok_or(USAGE)?;
                let id = normalize_id(&node);
                require_target_root(tree, &id)?;
                let format = export_format(&out)?;
                let (layout, scale, quality, max_kb, max_size, match_dir) =
                    export_options(&args[2..])?;
                Self::Export {
                    node,
                    out: out.into(),
                    format,
                    layout,
                    scale,
                    quality,
                    max_kb,
                    max_size,
                    match_index: match_index(match_dir, format)?,
                }
            }
            _ => return Err(USAGE.into()),
        })
    }

    /// Schema-shaped MCP arguments enter directly, without inventing CLI positionals or flags.
    pub fn from_tool(
        call: &crate::contract::PreparedCall,
        tree: &Tree,
        output: Option<&Path>,
        match_dir: Option<&Path>,
    ) -> Result<Self, Box<dyn Error>> {
        let string = |key: &str| call.string(key).map(str::to_owned);
        let node = || string("node").ok_or(USAGE);
        Ok(match call.name() {
            "tree" => Self::Tree {
                root: normalize_id(call.string("node").unwrap()),
                depth: call.argument("depth").as_u64().unwrap() as usize,
                json: call.argument("json") == true,
                top_frames: call.argument("topFrames") == true,
            },
            "nodes" => Self::Nodes(node()?, Some(crate::mcp::MAX_RESPONSE_BYTES)),
            "find" => Self::Find {
                query: string("query").ok_or(USAGE)?,
                root: normalize_id(call.string("node").unwrap()),
                kind: string("type").map(|s| s.to_uppercase()),
                limit: call.argument("limit").as_u64().unwrap() as usize,
            },
            "inspect" => Self::Inspect {
                node: node()?,
                depth: call.argument("depth").as_u64().unwrap() as usize,
                images: call.argument("images") == true,
                states: call.argument("states") == true,
                compare_variant: string("compareVariant"),
                flat: call.argument("flat") == true,
            },
            "variables" => Self::Variables(string("node")),
            "bundle" => {
                let node = node()?;
                require_target_root(tree, &normalize_id(&node))?;
                Self::Bundle {
                    node,
                    out: output.ok_or(USAGE)?.to_owned(),
                    file_key: string("fileKey"),
                    name: string("name"),
                    states: call.argument("states") == true,
                }
            }
            "export" => {
                let node = node()?;
                let out = output.ok_or(USAGE)?.to_owned();
                let id = normalize_id(&node);
                require_target_root(tree, &id)?;
                Self::Export {
                    node,
                    out,
                    // Encoding follows the requested extension, not a symlink target's name.
                    format: export_format(call.string("out").unwrap())?,
                    layout: call.string("bounds") == Some("layout"),
                    scale: call.argument("scale").as_f64().unwrap(),
                    quality: call.argument("quality").as_u64().unwrap() as u8,
                    max_kb: call.argument("maxKb").as_u64().map(|n| n as usize),
                    // Tool calls return the image inline, so cap it unless the agent says otherwise.
                    max_size: Some(call.argument("maxSize").as_u64().unwrap() as u32),
                    match_index: match_index(
                        match_dir.map(Path::to_owned),
                        export_format(call.string("out").unwrap())?,
                    )?,
                }
            }
            _ => return Err(USAGE.into()),
        })
    }

    pub fn execute(self, tree: &Tree, fig: &Path) -> Result<Output, Box<dyn Error>> {
        let pretty = |value| Output::Json {
            value,
            pretty: true,
        };
        let compact = |value| Output::Json {
            value,
            pretty: false,
        };
        Ok(match self {
            Self::Tree {
                root,
                top_frames: true,
                json,
                ..
            } => {
                require_node(tree, &root)?;
                // A section root still reports its page; the walk only sees pages below it.
                let mut ancestors =
                    std::iter::successors(Some(root.as_str()), |id| parent(tree, id));
                let page = ancestors.find(|id| plugin_type(tree.nodes[*id]) == "CANVAS");
                let section =
                    (plugin_type(tree.nodes[&root]) == "SECTION").then_some(root.as_str());
                let mut frames = Vec::new();
                top_frames(tree, &root, page, section, &mut frames);
                if json {
                    compact(Value::Array(
                        frames
                            .into_iter()
                            .map(|(id, page, section)| {
                                let mut frame = tree_json(tree, id, 0, 0, true);
                                if let Some(page) = page {
                                    frame["page"] = json!(page);
                                }
                                if let Some(section) = section {
                                    frame["section"] = json!(section);
                                }
                                frame
                            })
                            .collect(),
                    ))
                } else {
                    // The name is JSON-escaped so a newline in it cannot start another id line.
                    let mut out = String::new();
                    for (id, ..) in frames {
                        let (kind, name) = label(tree, id);
                        writeln!(out, "{id} {kind} {}", json!(name))?;
                    }
                    Output::Text(out)
                }
            }
            Self::Tree {
                root, depth, json, ..
            } => {
                require_node(tree, &root)?;
                if json {
                    // A root on a page or section keeps the firstText it has in its parent's listing.
                    let top = parent(tree, &root).is_some_and(|p| {
                        matches!(plugin_type(tree.nodes[p]).as_str(), "CANVAS" | "SECTION")
                    });
                    compact(tree_json(tree, &root, 0, depth, top))
                } else {
                    let mut out = String::new();
                    print_tree(&mut out, tree, &root, 0, depth)?;
                    Output::Text(out)
                }
            }
            Self::Node(node) => {
                let id = normalize_id(&node);
                require_node(tree, &id)?;
                pretty(to_json(tree.nodes[&id]))
            }
            Self::Nodes(node, budget) => {
                let id = normalize_id(&node);
                pretty(match budget {
                    Some(bytes) => figlens::rest::resolve_nodes_within(tree, &id, bytes)?,
                    None => resolve_nodes(tree, &id)?,
                })
            }
            Self::Find {
                query,
                root,
                kind,
                limit,
            } => {
                require_node(tree, &root)?;
                compact(discover::find(tree, &query, &root, kind.as_deref(), limit)?)
            }
            Self::Inspect {
                node,
                depth,
                images,
                states,
                compare_variant,
                flat,
            } => {
                let id = normalize_id(&node);
                if flat {
                    if images || states || compare_variant.is_some() {
                        return Err(
                            "--flat cannot be combined with --images, --states or --compare-variant"
                                .into(),
                        );
                    }
                    let (text, truncated) = discover::inspect_flat(tree, &id, depth)?;
                    return Ok(Output::Lines {
                        value: json!({ "flat": text, "truncated": truncated }),
                        text,
                    });
                }
                let compare_variant = compare_variant.as_deref().map(normalize_id);
                compact(discover::inspect(
                    tree,
                    &id,
                    depth,
                    fig,
                    images,
                    states,
                    compare_variant.as_deref(),
                )?)
            }
            Self::Variables(node) => {
                let doc = match node {
                    Some(node) => {
                        let id = normalize_id(&node);
                        Some(resolve_nodes(tree, &id)?)
                    }
                    None => None,
                };
                pretty(variables(tree, doc.as_ref().map(|d| &d["document"])))
            }
            Self::Bundle {
                node,
                out,
                file_key,
                name,
                states,
            } => {
                let id = normalize_id(&node);
                require_target_root(tree, &id)?;
                // A URL supplies the manifest key when --file-key is absent.
                let key = file_key.as_deref().or_else(|| {
                    node.split_once("/design/")
                        .or_else(|| node.split_once("/file/"))
                        .and_then(|(_, rest)| rest.split('/').next())
                });
                let warnings = figlens::bundle::write_with_states(
                    tree,
                    fig,
                    key,
                    name.as_deref(),
                    &id,
                    &out,
                    states,
                )?;
                compact(
                    json!({ "nodeId": id, "path": out.to_string_lossy(), "warnings": warnings }),
                )
            }
            Self::Export {
                node,
                out,
                format,
                layout,
                scale,
                quality,
                max_kb,
                max_size,
                match_index,
            } => {
                let id = normalize_id(&node);
                require_target_root(tree, &id)?;
                let max_bytes = check_export(scale, quality, max_kb)?;
                let (image, report) = figlens::export::export(
                    tree,
                    fig,
                    &id,
                    format,
                    layout,
                    scale,
                    quality,
                    max_bytes,
                    max_size.filter(|&px| px > 0),
                )?;
                figlens::output::write(fig, &out, |file| {
                    std::io::Write::write_all(file, &image.bytes)?;
                    Ok(())
                })?;
                let mut value = json!({ "path": out.to_string_lossy(), "width": image.width, "height": image.height,
                    "bytes": image.bytes.len(), "quality": image.quality });
                value.as_object_mut().unwrap().extend(report);
                if let Some(index) = match_index {
                    // The export itself may sit in the folder; it is not a match.
                    let own = std::fs::canonicalize(&out).ok();
                    let (matches, similar) = index.compare(&image.bytes, own.as_deref());
                    value["matches"] = json!(matches);
                    value["similar"] = json!(similar);
                }
                let mime = match format {
                    // Model image inputs commonly accept raster formats, not SVG attachments.
                    figlens::export::Format::Svg => return Ok(compact(value)),
                    figlens::export::Format::Png => "image/png",
                    figlens::export::Format::Jpg => "image/jpeg",
                    figlens::export::Format::Webp => "image/webp",
                    figlens::export::Format::Avif => "image/avif",
                };
                Output::Image {
                    value,
                    bytes: image.bytes,
                    mime,
                }
            }
        })
    }
}

/// `--bounds`, `--scale`, `--quality`, `--max-kb`, `--max-size` and `--match-dir`, as
/// `(layout, scale, quality, max_kb, max_size, match_dir)`.
pub fn export_options(args: &[String]) -> Result<ExportOptions, Box<dyn Error>> {
    let (mut layout, mut scale, mut quality, mut max_kb, mut max_size, mut match_dir) =
        (false, 1.0_f64, 80, None, None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--bounds" => {
                layout = match value()?.as_str() {
                    "layout" => true,
                    "render" => false,
                    _ => return Err("--bounds takes layout or render".into()),
                }
            }
            "--scale" => scale = value()?.trim_end_matches('x').parse()?,
            "--quality" => quality = value()?.parse()?,
            "--max-kb" => max_kb = Some(value()?.parse::<usize>()?),
            "--max-size" => max_size = Some(value()?.parse::<u32>()?),
            "--match-dir" => match_dir = Some(PathBuf::from(value()?)),
            _ => return Err(USAGE.into()),
        }
    }
    check_export(scale, quality, max_kb)?;
    Ok((layout, scale, quality, max_kb, max_size, match_dir))
}

type ExportOptions = (bool, f64, u8, Option<usize>, Option<u32>, Option<PathBuf>);

/// Rejects out-of-range options; returns `--max-kb` in bytes.
fn check_export(
    scale: f64,
    quality: u8,
    max_kb: Option<usize>,
) -> Result<Option<usize>, Box<dyn Error>> {
    if !(1..=100).contains(&quality) || !scale.is_finite() || scale <= 0.0 {
        return Err("--quality takes 1-100 and --scale a positive number".into());
    }
    Ok(max_kb
        .map(|kb| kb.checked_mul(1024).ok_or("--max-kb is too large"))
        .transpose()?)
}

fn search_mode(mode: &str) -> Result<(), Box<dyn Error>> {
    if mode == "text" {
        return Ok(());
    }
    Err(figlens::FigError::new(
        figlens::ErrorKind::InvalidArguments,
        format!("search mode {mode:?} is unavailable; this release searches text only"),
    )
    .into())
}

fn search_types(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(|t| t.trim().to_uppercase())
        .filter(|t| !t.is_empty())
}

/// CLI search arguments, checked before the source is downloaded or read.
pub fn search_request(args: &[String]) -> Result<figlens::search::Request, Box<dyn Error>> {
    let mut request = figlens::search::Request {
        query: args.first().cloned().ok_or(USAGE)?,
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit: figlens::search::DEFAULT_LIMIT,
        cursor: None,
    };
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--include-hidden" => request.include_hidden = true,
            "--include-internal" => request.include_internal = true,
            _ => {
                let value = rest.next().ok_or(format!("{arg} needs a value"))?;
                match arg.as_str() {
                    "--node" => request.scope = normalize_id(value),
                    "--type" => request.types.extend(search_types(value)),
                    "--limit" => request.limit = value.parse().unwrap_or(0),
                    "--cursor" => request.cursor = Some(value.clone()),
                    "--mode" => search_mode(value)?,
                    _ => return Err(USAGE.into()),
                }
            }
        }
    }
    request.validate()?;
    Ok(request)
}

/// The same request from schema-checked MCP arguments.
pub fn search_tool(
    call: &crate::contract::PreparedCall,
) -> Result<figlens::search::Request, Box<dyn Error>> {
    search_mode(call.string("mode").unwrap())?;
    let request = figlens::search::Request {
        query: call.string("query").ok_or(USAGE)?.to_owned(),
        scope: normalize_id(call.string("node").unwrap()),
        types: call
            .string("type")
            .map(search_types)
            .into_iter()
            .flatten()
            .collect(),
        include_hidden: call.argument("includeHidden") == true,
        include_internal: call.argument("includeInternal") == true,
        limit: call.argument("limit").as_u64().unwrap() as usize,
        cursor: call.string("cursor").map(str::to_owned),
    };
    request.validate()?;
    Ok(request)
}

/// Search reads and hashes its own snapshot of `path`, so it never needs a loaded tree.
pub fn search(
    path: &Path,
    request: &figlens::search::Request,
    label: &str,
) -> Result<Output, Box<dyn Error>> {
    let value = figlens::search::search(path, request).map_err(|e| {
        if e.is::<figlens::FigError>() {
            e
        } else {
            crate::contract::source(label, e)
        }
    })?;
    Ok(Output::Json {
        value,
        pretty: false,
    })
}

/// The `--match-dir` folder parsed once, for SVG output only.
pub fn match_index(
    dir: Option<PathBuf>,
    format: figlens::export::Format,
) -> Result<Option<Arc<SvgIndex>>, Box<dyn Error>> {
    let Some(dir) = dir else {
        return Ok(None);
    };
    if format != figlens::export::Format::Svg {
        return Err("--match-dir needs an .svg output".into());
    }
    Ok(Some(Arc::new(SvgIndex::load(&dir)?)))
}

fn export_format(out: &str) -> Result<figlens::export::Format, Box<dyn Error>> {
    figlens::export::Format::of(Path::new(out))
        .ok_or_else(|| "the output must end in .png, .jpg, .webp, .avif or .svg".into())
}

fn print_tree(
    out: &mut String,
    tree: &Tree,
    id: &str,
    level: usize,
    depth: usize,
) -> std::fmt::Result {
    let (kind, name) = label(tree, id);
    writeln!(out, "{}{} {} \"{}\"", "  ".repeat(level), id, kind, name)?;
    if level < depth {
        for child in tree.children.get(id).into_iter().flatten() {
            print_tree(out, tree, child, level + 1, depth)?;
        }
    }
    Ok(())
}

/// `children` is only included where the requested depth reaches. `size` is the local
/// pre-rotation `[width, height]`, absent without one (document, pages); `visible` appears
/// only when false and `layoutMode` (`HORIZONTAL`, `VERTICAL` or `GRID`) only when set, as in `nodes`.
/// A `top` node (a child of a page or section) adds `firstText`, so callers can tell a canvas
/// label (a box around one line of text) from a screen without opening it.
fn tree_json(tree: &Tree, id: &str, level: usize, depth: usize, top: bool) -> Value {
    let (kind, name) = label(tree, id);
    let raw = tree.nodes[id];
    let mut node = json!({ "id": id, "type": kind, "name": name });
    if let Some(size) = discover::local_size(raw) {
        node["size"] = json!(size);
    }
    if matches!(raw.get("visible"), Some(kiwi_schema::Value::Bool(false))) {
        node["visible"] = json!(false);
    }
    if let Some(kiwi_schema::Value::Enum(_, mode)) = raw.get("stackMode")
        && *mode != "NONE"
    {
        node["layoutMode"] = json!(mode);
    }
    if let Some(text) = top.then(|| first_text(tree, id)).flatten() {
        node["firstText"] = json!(text);
    }
    if let Some(children) = tree.children.get(id).filter(|_| level < depth) {
        let top = matches!(kind.as_str(), "CANVAS" | "SECTION");
        node["children"] = children
            .iter()
            .map(|child| tree_json(tree, child, level + 1, depth, top))
            .collect();
    }
    node
}

/// The first visible, non-blank TEXT's characters under `id` (itself included), breadth-first over stored
/// nodes, at most 200 characters. Instances are not expanded, so their text is not seen.
fn first_text(tree: &Tree, id: &str) -> Option<String> {
    let hidden = |id: &str| {
        matches!(
            tree.nodes.get(id).and_then(|n| n.get("visible")),
            Some(kiwi_schema::Value::Bool(false))
        )
    };
    let mut queue = std::collections::VecDeque::from([id]);
    // A crafted file can make a node its own descendant; visit each id once.
    let mut seen = std::collections::HashSet::from([id]);
    while let Some(id) = queue.pop_front() {
        let Some(node) = tree.nodes.get(id).filter(|_| !hidden(id)) else {
            continue;
        };
        let text = node.get("textData").and_then(|t| t.get("characters"));
        // Blank text is no label; keep looking.
        let text = text.map(|t| t.as_string()).filter(|t| !t.trim().is_empty());
        if let Some(text) = text.filter(|_| plugin_type(node) == "TEXT") {
            return Some(text.chars().take(200).collect());
        }
        for child in tree.children.get(id).into_iter().flatten() {
            if seen.insert(child) {
                queue.push_back(child);
            }
        }
    }
    None
}

/// `tree --top-frames`: ids of the screens below `id`, with their page and innermost section.
/// Screens are frames, components, component sets and instances that are direct children of a
/// page or of a section on one, at any section depth, in tree order; groups, shapes and text are
/// not screens. Internal pages are skipped unless they are the root.
fn top_frames<'t>(
    tree: &'t Tree,
    id: &str,
    page: Option<&'t str>,
    section: Option<&'t str>,
    out: &mut Vec<(&'t str, Option<&'t str>, Option<&'t str>)>,
) {
    let kind = plugin_type(tree.nodes[id]);
    let holds_screens = matches!(kind.as_str(), "CANVAS" | "SECTION");
    for child in tree.children.get(id).into_iter().flatten() {
        let node = tree.nodes[child.as_str()];
        match plugin_type(node).as_str() {
            "CANVAS"
                if kind == "DOCUMENT"
                    && node.get("internalOnly").map(|v| v.as_bool()) != Some(true) =>
            {
                top_frames(tree, child, Some(child), None, out)
            }
            "SECTION" if holds_screens => top_frames(tree, child, page, Some(child), out),
            "FRAME" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE" if holds_screens => {
                out.push((child, page, section))
            }
            _ => {}
        }
    }
}

/// The stored parent of `id`. `children` can name a parent the file lacks; that is no parent.
fn parent<'t>(tree: &'t Tree, id: &str) -> Option<&'t str> {
    let mut parents = tree.children.iter();
    parents
        .find(|(_, c)| c.iter().any(|c| c == id))
        .map(|(p, _)| p.as_str())
        .filter(|p| tree.nodes.contains_key(*p))
}

/// The REST type `nodes` and `find` use, so `tree` output can be filtered by the same names.
fn label<'a>(tree: &Tree<'a>, id: &str) -> (String, &'a str) {
    let node = tree.nodes[id];
    let name = node.get("name").map(|n| n.as_string()).unwrap_or("");
    (plugin_type(node), name)
}
