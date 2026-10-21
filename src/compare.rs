//! Diffs the nodes figlens derives from a `.fig` against a plugin capture
//! (`nodes.json`, Figma REST shape) of the same frame.

use std::collections::{HashMap, HashSet};

use serde_json::Value as Json;

use crate::expand::{Vec2, abs_matrix, expand_instances, plugin_type, subtree};
use crate::rest::nodes;
use crate::{Tree, guid_key};

/// Size tolerance in pixels; the plugin reports rounded boxes.
const TOLERANCE: f64 = 0.5;

pub struct Report {
    pub direct: usize,
    pub in_instances: usize,
    /// Diff category → one line per mismatch; empty categories are omitted.
    pub diffs: Vec<(&'static str, Vec<String>)>,
}

/// Style and binding fields compared as normalized JSON; like fills, they are counted apart from structure.
pub const FIELDS: [&str; 15] = [
    "blendMode",
    "effects",
    "rotation",
    "layoutWrap",
    "isMask",
    "boundVariables",
    "componentProperties",
    "styles",
    "style",
    "characterStyleOverrides",
    "styleOverrideTable",
    "lineTypes",
    "lineIndentations",
    "meta",
    "renderBounds",
];

impl Report {
    /// Fill colors bound to variables differ until variable modes are resolved, so fills are counted apart.
    pub fn is_clean(&self) -> bool {
        self.diffs
            .iter()
            .all(|(label, _)| *label == "fills" || FIELDS.contains(label))
    }

    pub fn fills_clean(&self) -> bool {
        self.clean("fills")
    }

    pub fn clean(&self, label: &str) -> bool {
        self.diffs.iter().all(|(l, _)| *l != label)
    }
}

/// `v` with numbers rounded to 4 decimals, so f32 noise does not count.
pub(crate) fn normalized(v: &Json) -> Json {
    match v {
        Json::Number(n) => n
            .as_f64()
            .map_or(v.clone(), |x| serde_json::json!((x * 1e4).round() / 1e4)),
        Json::Array(a) => a.iter().map(normalized).collect(),
        Json::Object(o) => o.iter().map(|(k, v)| (k.clone(), normalized(v))).collect(),
        _ => v.clone(),
    }
}

fn index<'j>(node: &'j Json, out: &mut HashMap<&'j str, &'j Json>) {
    if let Some(id) = node["id"].as_str() {
        out.insert(id, node);
    }
    for child in node["children"].as_array().into_iter().flatten() {
        index(child, out);
    }
}

fn size_differs(ours: Vec2, plugin: &Json) -> bool {
    let bbox = &plugin["absoluteBoundingBox"];
    match (bbox["width"].as_f64(), bbox["height"].as_f64()) {
        (Some(w), Some(h)) => (ours.x - w).abs() > TOLERANCE || (ours.y - h).abs() > TOLERANCE,
        _ => false,
    }
}

/// What a fill looks like, rounded so float noise does not count.
fn paint_key(p: &Json) -> String {
    let c = &p["color"];
    let r = |v: &Json| v.as_f64().map_or(String::new(), |x| format!("{x:.3}"));
    format!(
        "{} {} {} {}/{}/{}/{} {}",
        p["type"].as_str().unwrap_or("?"),
        p["visible"] != Json::Bool(false),
        r(&p["opacity"]),
        r(&c["r"]),
        r(&c["g"]),
        r(&c["b"]),
        r(&c["a"]),
        p["imageRef"].as_str().unwrap_or("")
    )
}

fn fills_key(node: &Json) -> Vec<String> {
    node["fills"]
        .as_array()
        .into_iter()
        .flatten()
        .map(paint_key)
        .collect()
}

fn fields_diff<'p>(
    props: &'p mut Vec<(&'static str, Vec<String>)>,
    label: &'static str,
) -> &'p mut Vec<String> {
    if let Some(i) = props.iter().position(|(l, _)| *l == label) {
        return &mut props[i].1;
    }
    props.push((label, Vec::new()));
    &mut props.last_mut().unwrap().1
}

fn describe(id: &str, kind: &str, name: &str) -> String {
    format!("{id} {kind} \"{name}\"")
}

/// Compares the subtree under `root` (directly and through instance expansion) with `oracle`.
pub fn compare(tree: &Tree, oracle: &Json, root: &str) -> Report {
    if oracle["document"]["id"].as_str().is_none() {
        return Report {
            direct: 0,
            in_instances: 0,
            diffs: vec![(
                "error",
                vec![format!("{root}: oracle has no document node")],
            )],
        };
    }
    let mut plugin = HashMap::new();
    index(&oracle["document"], &mut plugin);
    let (inside, direct): (Vec<&str>, Vec<&str>) =
        plugin.keys().partition(|id| id.starts_with('I'));
    let ours = subtree(tree, root);
    let ours_set: HashSet<&str> = ours.iter().map(String::as_str).collect();
    let name_of = |id: &str| {
        tree.nodes[id]
            .get("name")
            .map_or("", |n| n.as_string())
            .to_owned()
    };

    let mut d: Vec<(&'static str, Vec<String>)> =
        ["missing", "extra", "name", "type", "size", "component"]
            .into_iter()
            .map(|label| (label, Vec::new()))
            .collect();
    let mut note = |label: &str, line: String| {
        d.iter_mut()
            .find(|(l, _)| *l == label)
            .unwrap()
            .1
            .push(line)
    };

    for &id in &direct {
        let o = plugin[id];
        let Some(node) = tree.nodes.get(id).filter(|_| ours_set.contains(id)) else {
            note(
                "missing",
                describe(
                    id,
                    o["type"].as_str().unwrap_or("?"),
                    o["name"].as_str().unwrap_or(""),
                ),
            );
            continue;
        };
        let (name, kind) = (name_of(id), plugin_type(node));
        if o["name"].as_str() != Some(&name) {
            note("name", format!("{id} ours=\"{name}\" plugin={}", o["name"]));
        }
        if o["type"].as_str() != Some(&kind) {
            note("type", format!("{id} ours={kind} plugin={}", o["type"]));
        }
        if let Some(s) = node.get("size") {
            let bbox = abs_matrix(tree, id, root).bbox(Vec2 {
                x: s.get("x").map_or(0.0, |v| v.as_float() as f64),
                y: s.get("y").map_or(0.0, |v| v.as_float() as f64),
            });
            if size_differs(bbox, o) {
                note(
                    "size",
                    format!(
                        "{id} \"{name}\" ours={:.1}x{:.1} plugin={}x{}",
                        bbox.x,
                        bbox.y,
                        o["absoluteBoundingBox"]["width"],
                        o["absoluteBoundingBox"]["height"]
                    ),
                );
            }
        }
    }
    for id in &ours {
        if !plugin.contains_key(id.as_str()) {
            note(
                "extra",
                describe(id, &plugin_type(tree.nodes[id]), &name_of(id)),
            );
        }
    }
    // The top-level instance must point at the same main component.
    for id in &ours {
        let node = tree.nodes[id];
        if plugin_type(node) != "INSTANCE" {
            continue;
        }
        let main = node
            .get("symbolData")
            .and_then(|s| s.get("symbolID"))
            .and_then(guid_key);
        let theirs = plugin
            .get(id.as_str())
            .and_then(|o| o["componentId"].as_str());
        if theirs != main.as_deref() {
            note(
                "component",
                format!("{id} \"{}\" ours={main:?} plugin={theirs:?}", name_of(id)),
            );
        }
    }

    let expanded = expand_instances(tree, root).nodes;
    let mut inst: Vec<(&'static str, Vec<String>)> = [
        "inst-missing",
        "inst-extra",
        "inst-name",
        "inst-type",
        "inst-size",
    ]
    .into_iter()
    .map(|label| (label, Vec::new()))
    .collect();
    let mut note_inst = |label: &str, line: String| {
        inst.iter_mut()
            .find(|(l, _)| *l == label)
            .unwrap()
            .1
            .push(line)
    };
    for &id in &inside {
        let o = plugin[id];
        let Some(e) = expanded.get(id) else {
            note_inst(
                "inst-missing",
                describe(
                    id,
                    o["type"].as_str().unwrap_or("?"),
                    o["name"].as_str().unwrap_or(""),
                ),
            );
            continue;
        };
        if o["name"].as_str() != Some(&e.name) {
            note_inst(
                "inst-name",
                format!("{id} ours=\"{}\" plugin={}", e.name, o["name"]),
            );
        }
        if o["type"].as_str() != Some(&e.kind) {
            note_inst(
                "inst-type",
                format!("{id} ours={} plugin={}", e.kind, o["type"]),
            );
        }
        if let Some(s) = e.size {
            let bbox = e.matrix.bbox(s);
            if size_differs(bbox, o) {
                note_inst(
                    "inst-size",
                    format!(
                        "{id} \"{}\" ours={:.1}x{:.1} plugin={}x{}",
                        e.name,
                        bbox.x,
                        bbox.y,
                        o["absoluteBoundingBox"]["width"],
                        o["absoluteBoundingBox"]["height"]
                    ),
                );
            }
        }
    }
    for (id, e) in &expanded {
        if !plugin.contains_key(id.as_str()) {
            note_inst("inst-extra", describe(id, &e.kind, &e.name));
        }
    }

    // Properties, compared on the REST-shaped nodes `figlens nodes` prints.
    let rest = nodes(tree, root);
    let mut ours_rest = HashMap::new();
    index(&rest["document"], &mut ours_rest);
    let mut props: Vec<(&'static str, Vec<String>)> = vec![
        ("position", Vec::new()),
        ("fills", Vec::new()),
        ("characters", Vec::new()),
    ];
    for (&id, o) in &plugin {
        let Some(n) = ours_rest.get(id) else {
            continue;
        };
        let (ob, nb) = (&o["absoluteBoundingBox"], &n["absoluteBoundingBox"]);
        let off = |f: &str| match (ob[f].as_f64(), nb[f].as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() > TOLERANCE,
            _ => false,
        };
        if off("x") || off("y") {
            props[0].1.push(format!(
                "{id} ours={},{} plugin={},{}",
                nb["x"], nb["y"], ob["x"], ob["y"]
            ));
        }
        if fills_key(o) != fills_key(n) {
            props[1].1.push(format!(
                "{id} ours={:?} plugin={:?}",
                fills_key(n),
                fills_key(o)
            ));
        }
        if o["characters"] != n["characters"] {
            props[2].1.push(format!(
                "{id} ours={} plugin={}",
                n["characters"], o["characters"]
            ));
        }
        // Render bounds: null where nothing is drawn, else the drawn box within a pixel.
        let (ra, rb) = (&n["absoluteRenderBounds"], &o["absoluteRenderBounds"]);
        let far = ["x", "y", "width", "height"].iter().any(|k| {
            match (ra[*k].as_f64(), rb[*k].as_f64()) {
                (Some(a), Some(b)) => (a - b).abs() > 1.0,
                _ => false,
            }
        });
        if ra.is_null() != rb.is_null() || far {
            fields_diff(&mut props, "renderBounds").push(format!("{id} ours={ra} plugin={rb}"));
        }
        for field in &FIELDS[..FIELDS.len() - 2] {
            let (a, b) = (normalized(&n[*field]), normalized(&o[*field]));
            if a != b {
                fields_diff(&mut props, field).push(format!("{id} ours={a} plugin={b}"));
            }
        }
    }
    // Top-level metadata of the components, component sets and styles both reference.
    for map in ["components", "componentSets", "styles"] {
        for (id, theirs) in oracle[map].as_object().into_iter().flatten() {
            let ours = &rest[map][id];
            let keys = ["name", "key", "remote", "componentSetId", "styleType"];
            // A local style's key is not in the .fig, so a missing key is not a difference.
            let differs = keys
                .iter()
                .any(|k| ours[*k] != theirs[*k] && !(*k == "key" && ours[*k].is_null()));
            if ours.is_object() && differs {
                fields_diff(&mut props, "meta")
                    .push(format!("{map} {id} ours={ours} plugin={theirs}"));
            }
        }
    }

    let mut diffs: Vec<_> = d
        .into_iter()
        .chain(inst)
        .chain(props)
        .filter(|(_, lines)| !lines.is_empty())
        .collect();
    for (_, lines) in &mut diffs {
        lines.sort();
    }
    Report {
        direct: direct.len(),
        in_instances: inside.len(),
        diffs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_oracle_document_reports_an_error_without_canvas_diffs() {
        let tree = Tree {
            nodes: HashMap::new(),
            children: HashMap::new(),
            blobs: Vec::new(),
        };
        let report = compare(&tree, &serde_json::json!({}), "1:2");
        assert!(!report.is_clean());
        assert_eq!(
            report.diffs,
            vec![("error", vec!["1:2: oracle has no document node".to_owned()])]
        );
    }
}
