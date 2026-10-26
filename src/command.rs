//! Binary-private command semantics shared by the CLI and MCP adapters.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    },
    Node(String),
    Nodes(String),
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
        scale: f64,
        quality: u8,
        max_kb: Option<usize>,
        /// Longest side in pixels; raster formats are scaled down to fit.
        max_size: Option<u32>,
    },
}

pub enum Output {
    Text(String),
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
            Self::Text(text) => Ok(text.clone()),
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
            Self::Json { value, .. } | Self::Image { value, .. } => Some(value),
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
                let (mut root, mut depth, mut json) = ("0:0".to_owned(), usize::MAX, false);
                let mut rest = args.iter();
                while let Some(arg) = rest.next() {
                    if arg == "--depth" {
                        depth = rest.next().ok_or("--depth needs a number")?.parse()?;
                    } else if arg == "--json" {
                        json = true;
                    } else {
                        root = normalize_id(arg);
                    }
                }
                Self::Tree { root, depth, json }
            }
            "node" => Self::Node(node()?),
            "nodes" => Self::Nodes(node()?),
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
                let (mut depth, mut images, mut states) = (1, false, false);
                let mut rest = args[1..].iter();
                while let Some(flag) = rest.next() {
                    match flag.as_str() {
                        "--depth" => {
                            depth = rest.next().ok_or("--depth needs a number")?.parse()?
                        }
                        "--images" => images = true,
                        "--states" => states = true,
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Inspect {
                    node,
                    depth,
                    images,
                    states,
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
                let (mut scale, mut quality, mut max_kb, mut max_size) = (1.0_f64, 80, None, None);
                let mut rest = args[2..].iter();
                while let Some(arg) = rest.next() {
                    let mut value = || rest.next().ok_or(format!("{arg} needs a number"));
                    match arg.as_str() {
                        "--scale" => scale = value()?.trim_end_matches('x').parse()?,
                        "--quality" => quality = value()?.parse()?,
                        "--max-kb" => max_kb = Some(value()?.parse::<usize>()?),
                        "--max-size" => max_size = Some(value()?.parse::<u32>()?),
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Export {
                    node,
                    out: out.into(),
                    format,
                    scale,
                    quality,
                    max_kb,
                    max_size,
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
    ) -> Result<Self, Box<dyn Error>> {
        let string = |key: &str| call.string(key).map(str::to_owned);
        let node = || string("node").ok_or(USAGE);
        Ok(match call.name() {
            "tree" => Self::Tree {
                root: normalize_id(call.string("node").unwrap()),
                depth: call.argument("depth").as_u64().unwrap() as usize,
                json: call.argument("json") == true,
            },
            "nodes" => Self::Nodes(node()?),
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
                    scale: call.argument("scale").as_f64().unwrap(),
                    quality: call.argument("quality").as_u64().unwrap() as u8,
                    max_kb: call.argument("maxKb").as_u64().map(|n| n as usize),
                    // Tool calls return the image inline, so cap it unless the agent says otherwise.
                    max_size: Some(call.argument("maxSize").as_u64().unwrap() as u32),
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
            Self::Tree { root, depth, json } => {
                require_node(tree, &root)?;
                if json {
                    compact(tree_json(tree, &root, 0, depth))
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
            Self::Nodes(node) => {
                let id = normalize_id(&node);
                pretty(resolve_nodes(tree, &id)?)
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
            } => {
                let id = normalize_id(&node);
                compact(discover::inspect(tree, &id, depth, fig, images, states)?)
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
                scale,
                quality,
                max_kb,
                max_size,
            } => {
                let id = normalize_id(&node);
                require_target_root(tree, &id)?;
                if !(1..=100).contains(&quality) || !scale.is_finite() || scale <= 0.0 {
                    return Err("--quality takes 1-100 and --scale a positive number".into());
                }
                let max_bytes = max_kb
                    .map(|kb| kb.checked_mul(1024).ok_or("--max-kb is too large"))
                    .transpose()?;
                let image = figlens::export::export(
                    tree,
                    fig,
                    &id,
                    format,
                    scale,
                    quality,
                    max_bytes,
                    max_size.filter(|&px| px > 0),
                )?;
                std::fs::write(&out, &image.bytes).map_err(|e| {
                    figlens::FigError::new(figlens::ErrorKind::OutputWrite, e.to_string())
                })?;
                let value = json!({ "path": out.to_string_lossy(), "width": image.width, "height": image.height,
                    "bytes": image.bytes.len(), "quality": image.quality });
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

/// `children` is only included where the requested depth reaches.
fn tree_json(tree: &Tree, id: &str, level: usize, depth: usize) -> Value {
    let (kind, name) = label(tree, id);
    let mut node = json!({ "id": id, "type": kind, "name": name });
    if let Some(children) = tree.children.get(id).filter(|_| level < depth) {
        node["children"] = children
            .iter()
            .map(|child| tree_json(tree, child, level + 1, depth))
            .collect();
    }
    node
}

fn label<'a>(tree: &Tree<'a>, id: &str) -> (&'a str, &'a str) {
    let node = tree.nodes[id];
    let kind = match node.get("type") {
        Some(kiwi_schema::Value::Enum(_, name)) => *name,
        _ => "?",
    };
    (kind, node.get("name").map(|n| n.as_string()).unwrap_or(""))
}
