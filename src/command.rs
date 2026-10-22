//! Binary-private command semantics shared by the CLI and MCP adapters.

use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;

use figlens::rest::nodes;
use figlens::variables::variables;
use figlens::{Tree, normalize_id, to_json};
use serde_json::{Value, json};

use crate::USAGE;

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
    Variables(Option<String>),
    Bundle {
        node: String,
        out: String,
        file_key: Option<String>,
        name: Option<String>,
    },
    Export {
        node: String,
        out: String,
        scale: f64,
        quality: u8,
        max_kb: Option<usize>,
    },
}

pub enum Output {
    Text(String),
    Json { value: Value, pretty: bool },
}

impl Output {
    pub fn text(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Text(text) => Ok(text.clone()),
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
            Self::Json { value, .. } => Some(value),
            Self::Text(_) => None,
        }
    }
}

fn require_node(tree: &Tree, id: &str) -> Result<(), Box<dyn Error>> {
    if !tree.nodes.contains_key(id) {
        return Err(format!(
            "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
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
            "variables" => Self::Variables(args.first().cloned()),
            "bundle" => {
                let node = node()?;
                let out = args.get(1).cloned().ok_or(USAGE)?;
                require_node(tree, &normalize_id(&node))?;
                let (mut file_key, mut name) = (None, None);
                let mut rest = args[2..].iter();
                while let Some(arg) = rest.next() {
                    let mut value = || rest.next().cloned().ok_or(format!("{arg} needs a value"));
                    match arg.as_str() {
                        "--file-key" => file_key = Some(value()?),
                        "--name" => name = Some(value()?),
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Bundle {
                    node,
                    out,
                    file_key,
                    name,
                }
            }
            "export" => {
                let node = node()?;
                let out = args.get(1).cloned().ok_or(USAGE)?;
                let id = normalize_id(&node);
                require_node(tree, figlens::export::root_of(&id)).map_err(|_| {
                    format!(
                        "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                    )
                })?;
                export_format(&out)?;
                let (mut scale, mut quality, mut max_kb) = (1.0_f64, 80, None);
                let mut rest = args[2..].iter();
                while let Some(arg) = rest.next() {
                    let mut value = || rest.next().ok_or(format!("{arg} needs a number"));
                    match arg.as_str() {
                        "--scale" => scale = value()?.trim_end_matches('x').parse()?,
                        "--quality" => quality = value()?.parse()?,
                        "--max-kb" => max_kb = Some(value()?.parse::<usize>()?),
                        _ => return Err(USAGE.into()),
                    }
                }
                Self::Export {
                    node,
                    out,
                    scale,
                    quality,
                    max_kb,
                }
            }
            _ => return Err(USAGE.into()),
        })
    }

    /// Schema-shaped MCP arguments enter directly, without inventing CLI positionals or flags.
    pub fn from_tool(name: &str, args: &Value, tree: &Tree) -> Result<Self, Box<dyn Error>> {
        let string = |key: &str| args[key].as_str().map(str::to_owned);
        let node = || string("node").ok_or(USAGE);
        Ok(match name {
            "tree" => Self::Tree {
                root: normalize_id(string("node").as_deref().unwrap_or("0:0")),
                depth: args
                    .get("depth")
                    .map(|n| n.to_string().parse())
                    .transpose()?
                    .unwrap_or(usize::MAX),
                json: args["json"] == true,
            },
            "nodes" => Self::Nodes(node()?),
            "variables" => Self::Variables(string("node")),
            "bundle" => {
                let node = node()?;
                require_node(tree, &normalize_id(&node))?;
                Self::Bundle {
                    node,
                    out: string("out").ok_or(USAGE)?,
                    file_key: string("fileKey"),
                    name: string("name"),
                }
            }
            "export" => {
                let node = node()?;
                let out = string("out").ok_or(USAGE)?;
                let id = normalize_id(&node);
                require_node(tree, figlens::export::root_of(&id)).map_err(|_| {
                    format!(
                        "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                    )
                })?;
                export_format(&out)?;
                Self::Export {
                    node,
                    out,
                    scale: args
                        .get("scale")
                        .map(|n| n.to_string().parse())
                        .transpose()?
                        .unwrap_or(1.0),
                    quality: args
                        .get("quality")
                        .map(|n| n.to_string().parse())
                        .transpose()?
                        .unwrap_or(80),
                    max_kb: args
                        .get("maxKb")
                        .map(|n| n.to_string().parse())
                        .transpose()?,
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
                require_node(tree, &id)?;
                pretty(nodes(tree, &id))
            }
            Self::Variables(node) => {
                let doc = match node {
                    Some(node) => {
                        let id = normalize_id(&node);
                        require_node(tree, &id)?;
                        Some(nodes(tree, &id))
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
            } => {
                let id = normalize_id(&node);
                require_node(tree, &id)?;
                // A URL supplies the manifest key when --file-key is absent.
                let key = file_key.as_deref().or_else(|| {
                    node.split_once("/design/")
                        .or_else(|| node.split_once("/file/"))
                        .and_then(|(_, rest)| rest.split('/').next())
                });
                let warnings =
                    figlens::bundle::write(tree, fig, key, name.as_deref(), &id, Path::new(&out))?;
                compact(json!({ "nodeId": id, "path": out, "warnings": warnings }))
            }
            Self::Export {
                node,
                out,
                scale,
                quality,
                max_kb,
            } => {
                let id = normalize_id(&node);
                require_node(tree, figlens::export::root_of(&id)).map_err(|_| {
                    format!(
                        "node {id} not found; list ids with `figlens tree <file.fig> --depth 2`"
                    )
                })?;
                let format = export_format(&out)?;
                if !(1..=100).contains(&quality) || scale.is_nan() || scale <= 0.0 {
                    return Err("--quality takes 1-100 and --scale a positive number".into());
                }
                let image = figlens::export::export(
                    tree,
                    fig,
                    &id,
                    format,
                    scale,
                    quality,
                    max_kb.map(|kb| kb * 1024),
                )?;
                std::fs::write(&out, &image.bytes)?;
                compact(
                    json!({ "path": out, "width": image.width, "height": image.height,
                    "bytes": image.bytes.len(), "quality": image.quality }),
                )
            }
        })
    }
}

fn export_format(out: &str) -> Result<figlens::export::Format, Box<dyn Error>> {
    figlens::export::Format::of(Path::new(out))
        .ok_or_else(|| "the output must end in .png, .jpg, .webp or .avif".into())
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
