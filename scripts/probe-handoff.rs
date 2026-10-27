//! Local raw-field inventory for handoff research; output can contain private design data.
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::path::Path;

use figlens::{Document, Tree, to_json};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Field {
    occurrences: usize,
    nodes: HashSet<String>,
    examples: Vec<Value>,
    nonzero_examples: Vec<Value>,
}

fn interesting(key: &str, value: &Value) -> bool {
    let key = key.to_ascii_lowercase();
    ["hyperlink", "trim", "grid", "slot", "list"]
        .iter()
        .any(|s| key.contains(s))
        || value
            .as_str()
            .is_some_and(|s| matches!(s, "GRID" | "SLOT" | "ORDERED_LIST" | "UNORDERED_LIST"))
}

fn visit(value: &Value, path: &str, node: &str, fields: &mut BTreeMap<String, Field>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if interesting(key, value) {
                    let field = fields.entry(path.clone()).or_default();
                    field.occurrences += 1;
                    field.nodes.insert(node.into());
                    if field.examples.len() < 3 {
                        field.examples.push(json!({"nodeId":node,"value":value}));
                    }
                    if field.nonzero_examples.len() < 3
                        && (value == true || value.as_f64().is_some_and(|n| n != 0.0))
                    {
                        field
                            .nonzero_examples
                            .push(json!({"nodeId":node,"value":value}));
                    }
                }
                visit(value, &path, node, fields);
            }
        }
        Value::Array(items) => {
            for value in items {
                visit(value, &format!("{path}[]"), node, fields);
            }
        }
        _ => {}
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut check = BTreeMap::new();
    visit(
        &json!({"textData":{"hyperlinks":[{"url":"https://example.com"}]},"type":"SLOT"}),
        "",
        "1:1",
        &mut check,
    );
    assert_eq!(check["textData.hyperlinks"].occurrences, 1);
    assert_eq!(check["type"].nodes.len(), 1);
    assert!(!interesting("fontSize", &json!(16)));
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err("usage: handoff-probe <local.fig>... > samples/report.jsonl".into());
    }
    for file in args {
        let digest: String = Sha256::digest(std::fs::read(&file)?)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut document = Document::open(Path::new(&file))?;
        let message = document.take_message()?;
        let tree = Tree::new(&message);
        let mut ids: Vec<_> = tree.nodes.keys().collect();
        ids.sort();
        let mut fields = BTreeMap::new();
        let mut kinds = BTreeMap::<String, usize>::new();
        let mut slots = Vec::new();
        for id in ids {
            let node = tree.nodes[id];
            let kind = figlens::expand::plugin_type(node);
            *kinds.entry(kind.clone()).or_default() += 1;
            if kind == "SLOT" && slots.len() < 20 {
                slots.push(json!({"nodeId":id,"children":tree.children.get(id),"parent":node.get("parentIndex").map(to_json)}));
            }
            visit(&to_json(node), "", id, &mut fields);
        }
        let fields: BTreeMap<_, _> = fields.into_iter().map(|(path,f)| (path,json!({"occurrences":f.occurrences,"nodeCount":f.nodes.len(),"examples":f.examples,"nonzeroExamples":f.nonzero_examples}))).collect();
        println!(
            "{}",
            json!({"file":file,"sha256":digest,"nodes":tree.nodes.len(),"kinds":kinds,"fields":fields,"slotExamples":slots})
        );
    }
    Ok(())
}
