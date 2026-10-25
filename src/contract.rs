//! MCP parameter metadata/preflight and additive tool-error diagnostics.
use std::error::Error;

use figlens::{ErrorKind, FigError};
use serde_json::{Value, json};

use crate::Param;

const SOURCE_HELP: &str = "Cannot read the requested .fig source; check the path and valid local copy, or gcloud availability, connectivity and authorized object read access.";

pub fn property(tool: &str, param: &Param) -> Value {
    let (key, _, kind, _, about) = *param;
    let mut p = json!({"type": kind, "description": about});
    let range = match (tool, key) {
        ("inspect", "depth") => Some((0, 10)),
        ("tree", "depth") => Some((0, usize::MAX as u64)),
        ("find", "limit") => Some((1, 200)),
        ("export", "quality") => Some((1, 100)),
        ("export", "maxKb") => Some((0, (usize::MAX / 1024) as u64)),
        ("export", "maxSize") => Some((0, u64::from(u32::MAX))),
        _ => None,
    };
    if let Some((min, max)) = range {
        p["minimum"] = min.into();
        p["maximum"] = max.into();
    }
    let default = match (tool, key) {
        ("tree" | "find", "node") => Some(json!("0:0")),
        ("tree", "json") | ("inspect", "images") => Some(json!(false)),
        ("inspect", "depth") | ("export", "scale") => Some(json!(1)),
        ("find", "limit") => Some(json!(20)),
        ("export", "quality") => Some(json!(80)),
        ("export", "maxSize") => Some(json!(1600)),
        _ => None,
    };
    if let Some(value) = default {
        p["default"] = value;
    }
    if key == "file" || key == "out" || key == "query" {
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

/// Normalize JSON integers such as 2.0; leave explicitly recognized legacy values for CLI parsing.
pub fn arguments(tool: &str, params: &[Param], args: &Value) -> Result<(Value, bool), Failure> {
    let object = args
        .as_object()
        .ok_or_else(|| Failure::argument("arguments must be an object"))?;
    if tool == "workflow" && object.len() != 1 {
        return Err(Failure::argument("workflow requires only topic"));
    }
    let mut checked = args.clone();
    let mut legacy = false;
    for param in params {
        let (key, _, kind, required, _) = *param;
        let Some(value) = object.get(key) else {
            if required {
                return Err(Failure::argument(format!(
                    "missing required parameter {key}"
                )));
            }
            continue;
        };
        // Existing unrestricted clients use null/false to omit optional CLI arguments.
        if !required && (value.is_null() || (kind != "boolean" && value == false)) {
            legacy = true;
            continue;
        }
        let schema = property(tool, param);
        let bad = || Failure::argument(format!("invalid parameter {key}; expected {schema}"));
        match kind {
            "integer" => {
                let n = if let Some(text) = value.as_str() {
                    legacy = true;
                    text.parse::<u64>().map_err(|_| bad())?
                } else {
                    integer(value).ok_or_else(bad)?
                };
                if n < schema["minimum"].as_u64().unwrap()
                    || n > schema["maximum"].as_u64().unwrap()
                {
                    return Err(bad());
                }
                if !value.is_string() {
                    checked[key] = n.into();
                }
            }
            "number" => {
                let n = if let Some(text) = value.as_str() {
                    legacy = true;
                    text.trim_end_matches('x')
                        .parse::<f64>()
                        .map_err(|_| bad())?
                } else {
                    value.as_f64().ok_or_else(bad)?
                };
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
        && let Some(out) = args["out"].as_str()
        && figlens::export::Format::of(std::path::Path::new(out)).is_none()
    {
        return Err(Failure::argument(
            "out must end in .png, .jpg, .jpeg, .webp, .avif or .svg",
        ));
    }
    legacy |= matches!(
        (tool, args["node"].as_str()),
        ("tree", Some("--depth" | "--json")) | ("bundle", Some("--frames"))
    );
    Ok((checked, legacy))
}

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
                "Use tools/list types and ranges; omit optional values instead of sending null.",
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
