//! MCP parameter metadata/preflight and additive tool-error diagnostics.
use std::error::Error;

use figlens::{ErrorKind, FigError};
use serde_json::{Value, json};

/// An MCP parameter: key, JSON type, requiredness and description.
type Param = (&'static str, &'static str, bool, &'static str);
const FILE: Param = (
    "file",
    "string",
    true,
    "the saved .fig: a local path (absolute recommended), or gs://bucket/object using gcloud; remote sources are unavailable with --root",
);
const NODE: Param = (
    "node",
    "string",
    true,
    "a stored node id (12:345 or 12-345), full instance sublayer id (I12:345;67:89) from inspect/nodes, or a Figma URL with ?node-id=",
);
/// CLI and tools share command execution and result formatting.
pub const TOOLS: &[(&str, &str, &[Param])] = &[
    (
        "workflow",
        "Read built-in figlens skill guidance for this agent: build for design-to-code and project reuse, audit for comparing existing UI with a design and correcting confirmed differences, images for website asset preparation, plan for scoped implementation planning, planning-details for page/multi-page work or unresolved design dependencies, yolo for explicitly requested autonomous implementation and temporary-plan cleanup. Returns the requested guide only; no design file, project read/write, implementation, verification or cleanup is performed. Ordinary design questions and exports can use the design tools directly.",
        &[(
            "topic",
            "string",
            true,
            "Guide to read: build, audit, images, plan, planning-details or yolo.",
        )],
    ),
    (
        "tree",
        "List node ids, types and names under a chosen root. A shallow outline locates pages and frames when their names are unknown; find searches known names or visible text directly.",
        &[
            FILE,
            (
                "node",
                "string",
                false,
                "a stored root id; default the whole document; use inspect for instance sublayers",
            ),
            ("depth", "integer", false, "levels to list below the root"),
            (
                "json",
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
                "string",
                true,
                "literal substring in a layer name or text",
            ),
            (
                "node",
                "string",
                false,
                "search this stored subtree; default the document; use inspect for instance sublayers",
            ),
            (
                "type",
                "string",
                false,
                "Figma node type, e.g. FRAME or TEXT",
            ),
            (
                "limit",
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
                "integer",
                false,
                "0-10 levels below the node, default 1; text previews up to 200 characters",
            ),
            (
                "images",
                "boolean",
                false,
                "include source bitmap dimensions/bytes and per-paint source-pixel crop rectangles; no files written",
            ),
            (
                "states",
                "boolean",
                false,
                "include bounded variants and prototype field evidence with exact sources/unknowns; retains instance overrides, infers no business conditions and renders no other variants",
            ),
            (
                "compareVariant",
                "string",
                false,
                "compare node with this other stored variant in the same component set; adds bounded raw-field and descendant-multiset differences, exact sources and unknown pairing; no alternate rendering or business inference",
            ),
        ],
    ),
    (
        "nodes",
        "A node or exact instance sublayer as Figma REST nodes.json: instances expanded, fills, strokes, effects, full text, auto-layout, masks and component data. Use it when inspect's compact summary lacks paints, masks or full text. A whole screen can be hundreds of KB; replies over the MCP reply budget fail with response_budget_exceeded instead of being truncated, so scope to a smaller node, use inspect, or write a bundle.",
        &[FILE, NODE],
    ),
    (
        "variables",
        "The variables a frame binds (collections, modes, aliases); without a node, every local variable.",
        &[
            FILE,
            (
                "node",
                "string",
                false,
                "the node, including full instance sublayer IDs; omit for every local variable",
            ),
        ],
    ),
    (
        "bundle",
        "Write a design bundle zip for one node or exact instance sublayer: nodes, variables, assets, reference render, context.md. When the user requests related component states, pass states=true to include their explicit variants and evidence gaps; no MCP configuration change is needed.",
        &[
            FILE,
            NODE,
            (
                "out",
                "string",
                true,
                "output path for the bundle ZIP; unless the user names one, use .figlens/exports/ under the project root, created with a .gitignore containing * so it stays out of Git; create the parent directory first",
            ),
            (
                "fileKey",
                "string",
                false,
                "Figma file key for the manifest",
            ),
            ("name", "string", false, "frame name for the manifest"),
            (
                "states",
                "boolean",
                false,
                "include design/states.json and a bounded context.md summary; retains inspect's source, unknown and truncation markers; default false",
            ),
        ],
    ),
    (
        "verify",
        "Check a bundle, zipped or extracted, against its manifest before building from it: verified, missing and changed files, the bundle's missing assets, and the font faces the visible text uses (with node ids). Fonts are always unverified here: confirm each face in the running project with document.fonts.load, and check browser delivery of assets there. findings gathers these checks with the bundle's unresolved variables, truncated state inventory, render limitations and unemitted semantics into one handoff preflight list, each with node ids, evidence, impact and next action. Reports without blocking; no design file is read.",
        &[(
            "bundle",
            "string",
            true,
            "path to a bundle .zip or the directory it was extracted to",
        )],
    ),
    (
        "export",
        "Export a node to SVG, PNG, JPG, WebP or AVIF. Create the output directory first. Returns metadata and, for raster formats, an inline image. PNG suits inline references. For website assets, follow the target project's existing image pipeline; otherwise start with WebP for bitmaps, sized for display dimensions and DPR using scale/maxSize and quality/maxKb. SVG embeds original images and can be large; keep it for vector artwork. scale, quality and maxSize affect raster formats only; maxSize defaults to 1600. Check returned dimensions/bytes and isError; read content on failure.",
        &[
            FILE,
            NODE,
            (
                "out",
                "string",
                true,
                "output path ending in .png, .jpg, .jpeg, .webp, .avif or .svg; shipped assets follow the project's asset conventions, reference and scratch files go in .figlens/exports/ under the project root, created with a .gitignore containing * so it stays out of Git; create the parent directory first",
            ),
            ("scale", "number", false, "default 1"),
            ("quality", "integer", false, "1-100, default 80"),
            (
                "maxKb",
                "integer",
                false,
                "KiB (1024 bytes) budget; lower lossy quality until it fits; PNG/SVG fail if over budget",
            ),
            (
                "maxSize",
                "integer",
                false,
                "longest side in pixels, default 1600; 0 keeps the scaled size",
            ),
        ],
    ),
];

/// The MCP outputSchema of the tools whose CLI command prints one JSON object.
pub fn output_schema(tool: &str) -> Option<serde_json::Value> {
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
                "node": {"type": "object", "description": "Compact node with partial layout semantics and type-only property definitions; _figlens.semantics records source/verification. Definitions, definition evidence and _figlens.renderDiagnostics each have at most 20 entries per node; truncated includes omitted nodes or entries"},
                "truncated": {"type": "boolean"},
                "images": {"type": "object", "description": "optional source image evidence when images=true"},
                "states": {"type": "object", "description": "optional variants and prototype field evidence when states=true; independent truncation, source file, node/override provenance and unknown markers; no inferred business logic"},
                "variantComparison": {"type":"object", "description":"optional same-set definition comparison when compareVariant is supplied; raw fields, descendant value multisets, unpaired nodes, unknowns and independent truncation; not a pixel comparison"},
                "handoffDiagnostics": {"type":"object", "description":"optional known stored semantics not emitted by the projection; counts cover the selected expanded subtree independently of summary depth, with at most five example IDs per feature and explicit coverage limits"},
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
                    "description": "missing assets, known rendering limits and truncated optional state inventory; missing-assets.json contains only asset failures; design/nodes.json has per-node _figlens.renderDiagnostics; design/states.json has inventory coverage when requested",
                },
            }),
            ["nodeId", "path", "warnings"].as_slice(),
        ),
        "verify" => (
            serde_json::json!({
                "path": { "type": "string" },
                "files": {
                    "type": "object",
                    "description": "verified count; missing paths; changed entries with expected and actual size/sha256 (actual null when larger than declared, unreadable, with error, or not a regular file inside the bundle); invalid manifest paths",
                },
                "missingAssets": { "type": "array", "description": "missing-assets.json entries" },
                "fonts": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "family, style, weight, postScriptName, nodeIds and status, always unverified",
                },
                "unverified": { "type": "array", "items": { "type": "string" } },
                "findings": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "handoff preflight, missing inputs first: category (integrity, asset, variable, scope, render, semantics, font), status (missing, unsupported or unverified), source file, nodeIds (at most 20, none over 1 KiB) and nodeCount, evidence, impact and action; at most 500 entries and 1 MiB",
                },
                "findingsSummary": {
                    "type": "object",
                    "description": "scope (bundle root nodeId and name), total, returned, truncated, counts per category/status, unexamined metadata files, semanticsCoverage (the handoff inventory's coverage) and the coverage the findings do not establish",
                },
                "truncated": {
                    "type": "object",
                    "description": "original length of each other list cut to stay within the reply budget (files.missing, files.changed, files.invalid, missingAssets, fonts); empty when nothing was cut",
                },
            }),
            [
                "path",
                "files",
                "missingAssets",
                "fonts",
                "unverified",
                "findings",
                "findingsSummary",
                "truncated",
            ]
            .as_slice(),
        ),
        _ => return None,
    };
    Some(serde_json::json!({ "type": "object", "properties": properties, "required": required }))
}

fn default(tool: &str, key: &str) -> Option<Value> {
    match (tool, key) {
        ("tree", "depth") => Some(json!(usize::MAX)),
        ("tree" | "find", "node") => Some(json!("0:0")),
        ("tree", "json") | ("inspect", "images" | "states") | ("bundle", "states") => {
            Some(json!(false))
        }
        ("inspect", "depth") | ("export", "scale") => Some(json!(1)),
        ("find", "limit") => Some(json!(20)),
        ("export", "quality") => Some(json!(80)),
        ("export", "maxSize") => Some(json!(1600)),
        _ => None,
    }
}

const SOURCE_HELP: &str = "Cannot read the requested .fig source; check the path and valid local copy, or gcloud availability, connectivity and authorized object read access.";

// Larger bounds break int64 and JavaScript schema parsers; some model backends reject every tool.
const JSON_SAFE: u64 = (1 << 53) - 1;

pub fn property(tool: &str, param: &Param) -> Value {
    let (key, kind, _, about) = *param;
    let mut p = json!({"type": kind, "description": about});
    let range = match (tool, key) {
        ("inspect", "depth") => Some((0, 10)),
        ("tree", "depth") => Some((0, JSON_SAFE.min(usize::MAX as u64))),
        ("find", "limit") => Some((1, 200)),
        ("export", "quality") => Some((1, 100)),
        ("export", "maxKb") => Some((0, JSON_SAFE.min((usize::MAX / 1024) as u64))),
        ("export", "maxSize") => Some((0, u64::from(u32::MAX))),
        _ => None,
    };
    if let Some((min, max)) = range {
        p["minimum"] = min.into();
        p["maximum"] = max.into();
    }
    if let Some(value) = default(tool, key).filter(|_| (tool, key) != ("tree", "depth")) {
        p["default"] = value;
    }
    if matches!(key, "file" | "out" | "query" | "compareVariant" | "bundle") {
        p["minLength"] = json!(1);
    }
    if key == "query" {
        p["pattern"] = json!(r"\S");
    }
    if tool == "export" && key == "scale" {
        p["exclusiveMinimum"] = json!(0);
    }
    if tool == "workflow" && key == "topic" {
        p["enum"] = json!(crate::workflow::TOPICS);
    }
    p
}

fn integer(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    let n = value.as_f64()?;
    // u64::MAX rounds up to 2^64 as f64; exclude that endpoint before casting.
    ((0.0..18_446_744_073_709_551_616.0).contains(&n) && n.fract() == 0.0).then_some(n as u64)
}

/// An immutable call whose identity, defaults and arguments have passed the contract.
pub struct PreparedCall {
    name: &'static str,
    arguments: Value,
}

impl PreparedCall {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn argument(&self, key: &str) -> &Value {
        &self.arguments[key]
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.argument(key).as_str()
    }
}

/// Validate before source I/O, normalize mathematical integers and materialize MCP defaults.
pub fn prepare(tool: &str, args: &Value) -> Result<PreparedCall, Failure> {
    let (name, _, params) = TOOLS
        .iter()
        .find(|t| t.0 == tool)
        .ok_or_else(|| Failure::argument("unknown tool"))?;
    let object = args
        .as_object()
        .ok_or_else(|| Failure::argument("arguments must be an object"))?;
    // A misspelled optional parameter would otherwise fall back to its default and still succeed.
    if let Some(key) = object
        .keys()
        .find(|key| !params.iter().any(|p| p.0 == *key))
    {
        let accepted = params.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
        return Err(Failure::argument(format!(
            "unknown parameter {key}; accepted: {accepted}"
        )));
    }
    let mut checked = json!({});
    for param in *params {
        let (key, kind, required, _) = *param;
        let Some(value) = object.get(key) else {
            if required {
                return Err(Failure::argument(format!(
                    "missing required parameter {key}"
                )));
            }
            if let Some(value) = default(tool, key) {
                checked[key] = value;
            }
            continue;
        };
        let schema = property(tool, param);
        checked[key] = value.clone();
        let bad = || Failure::argument(format!("invalid parameter {key}; expected {schema}"));
        match kind {
            "integer" => {
                let n = integer(value).ok_or_else(bad)?;
                if n < schema["minimum"].as_u64().unwrap()
                    || n > schema["maximum"].as_u64().unwrap()
                {
                    return Err(bad());
                }
                checked[key] = n.into();
            }
            "number" => {
                let n = value.as_f64().ok_or_else(bad)?;
                if !n.is_finite() || n <= 0.0 {
                    return Err(bad());
                }
            }
            "boolean" if !value.is_boolean() => return Err(bad()),
            "string" => {
                let text = value.as_str().ok_or_else(bad)?;
                if (schema["minLength"] == 1 && text.is_empty())
                    || (key == "query" && text.trim().is_empty())
                {
                    return Err(bad());
                }
                if let Some(choices) = schema["enum"].as_array()
                    && !choices.contains(value)
                {
                    return Err(bad());
                }
            }
            _ => {}
        }
    }
    if tool == "export"
        && figlens::export::Format::of(std::path::Path::new(checked["out"].as_str().unwrap()))
            .is_none()
    {
        return Err(Failure::argument(
            "out must end in .png, .jpg, .jpeg, .webp, .avif or .svg",
        ));
    }
    Ok(PreparedCall {
        name,
        arguments: checked,
    })
}

pub fn tools() -> Value {
    json!({ "tools": TOOLS.iter().map(|(name, about, params)| {
        let mut tool = json!({
            "name": name,
            "description": about,
            "inputSchema": {
                "type": "object",
                "properties": params.iter().map(|param|
                    (param.0.to_owned(), property(name, param))
                ).collect::<serde_json::Map<_, _>>(),
                "required": params.iter().filter(|p| p.2).map(|p| p.0).collect::<Vec<_>>(),
                "additionalProperties": false,
            }
        });
        if let Some(schema) = output_schema(name) {
            tool["outputSchema"] = schema;
        }
        if *name == "workflow" {
            tool["annotations"] = json!({ "readOnlyHint": true, "openWorldHint": false });
        }
        tool
    }).collect::<Vec<_>>() })
}

#[derive(Debug)]
pub struct Failure {
    pub kind: ErrorKind,
    pub text: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    pub fn argument(text: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArguments, text)
    }

    pub fn from_error(error: Box<dyn Error>) -> Self {
        let kind = error
            .downcast_ref::<FigError>()
            .map_or(ErrorKind::OperationFailed, FigError::kind);
        let text = match kind {
            ErrorKind::SourceUnreadable | ErrorKind::InvalidData => SOURCE_HELP.to_owned(),
            _ => error.to_string(),
        };
        Self::new(kind, text)
    }

    pub fn metadata(&self) -> Value {
        let (code, next) = match self.kind {
            ErrorKind::InvalidArguments => (
                "invalid_arguments",
                "Use tools/list types and ranges; omit optional values for defaults, or use maxSize: 0 for no side cap.",
            ),
            ErrorKind::SourceUnreadable | ErrorKind::InvalidData => (
                "source_unreadable",
                "Check the source path, export a valid .fig, and verify authorized access; do not bypass permissions.",
            ),
            ErrorKind::NodeNotFound => (
                "node_not_found",
                "Use find/tree on a stored root or inspect the containing instance; copy the complete returned ID.",
            ),
            ErrorKind::UnsupportedNode => (
                "unsupported_node",
                "Use inspect/nodes for instance sublayers, or choose a sized drawable node for export/bundle.",
            ),
            ErrorKind::ParseBudget => (
                "parse_budget_exceeded",
                "Use a smaller local copy or report the named limit for review.",
            ),
            ErrorKind::ResponseBudget => (
                "response_budget_exceeded",
                "Use inspect, bundle one frame, or CLI nodes for full JSON; for inline exports lower maxSize. A completed file is identified by outputPath when present.",
            ),
            ErrorKind::OutputWrite => (
                "output_write_failed",
                "Create an authorized writable output directory; check free space and permissions.",
            ),
            ErrorKind::PermissionDenied => (
                "permission_denied",
                "Use an authorized local source and output inside the configured roots; retain the restrictions.",
            ),
            ErrorKind::OperationFailed => (
                "operation_failed",
                "Inspect the error and supported operation; report a reproducible case without private design data.",
            ),
        };
        json!({"code": code, "nextStep": next})
    }
}

/// Keep local CLI diagnostics and budget provenance; URL-source diagnostics can contain secrets.
pub fn source(file: &str, error: Box<dyn Error>) -> Box<dyn Error> {
    if error
        .downcast_ref::<FigError>()
        .is_some_and(|e| e.kind() == ErrorKind::ParseBudget)
    {
        error
    } else {
        Box::new(FigError::new(
            ErrorKind::SourceUnreadable,
            if file.contains("://") {
                SOURCE_HELP.to_owned()
            } else {
                format!("{file}: {error}")
            },
        ))
    }
}
