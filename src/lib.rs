//! Reads Figma `.fig` files (File > Save local copy) without Figma.
//!
//! `canvas.fig` carries its own kiwi schema, so decoding needs no hardcoded
//! knowledge of Figma's fields; only the container framing is fixed.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use kiwi_schema::{Schema, Value};

pub mod bundle;
pub mod compare;
pub mod expand;
pub mod export;
pub mod remote;
pub mod render;
pub mod rest;
pub mod variables;

const MAGIC: &[u8; 8] = b"fig-kiwi";
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

#[derive(Debug)]
pub struct FigError(String);

impl fmt::Display for FigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for FigError {}

fn err<T>(message: impl Into<String>) -> Result<T, Box<dyn Error>> {
    Err(Box::new(FigError(message.into())))
}

/// A decoded `canvas.fig`: the embedded schema plus the raw message bytes.
pub struct Document {
    pub version: u32,
    pub schema: Schema,
    message: Vec<u8>,
}

impl Document {
    /// Opens a `.fig` zip and decodes its `canvas.fig`.
    pub fn open(path: &Path) -> Result<Document, Box<dyn Error>> {
        let mut archive = zip::ZipArchive::new(File::open(path)?)?;
        let mut canvas = Vec::new();
        archive
            .by_name("canvas.fig")
            .map_err(|e| FigError(format!("{}: no canvas.fig ({e})", path.display())))?
            .read_to_end(&mut canvas)?;
        Document::from_canvas(&canvas)
    }

    pub fn from_canvas(canvas: &[u8]) -> Result<Document, Box<dyn Error>> {
        let (version, chunks) = split_chunks(canvas)?;
        if chunks.len() < 2 {
            return err(format!(
                "canvas.fig has {} chunk(s), expected schema and message",
                chunks.len()
            ));
        }
        let schema = Schema::decode(&unpack(chunks[0])?)
            .map_err(|_| FigError("invalid kiwi schema".into()))?;
        Ok(Document {
            version,
            schema,
            message: unpack(chunks[1])?,
        })
    }

    /// Decodes the root `Message` with the file's own schema.
    pub fn message(&self) -> Result<Value<'_>, Box<dyn Error>> {
        let Some(def) = self.schema.def("Message") else {
            return err("schema has no Message definition");
        };
        Value::decode(&self.schema, def.index, &self.message)
            .map_err(|_| FigError("message does not match its schema".into()).into())
    }
}

/// Format version and the chunk slices of a `canvas.fig`.
pub type Chunks<'a> = (u32, Vec<&'a [u8]>);

/// Splits `fig-kiwi` framing: magic, u32 version, then u32-length-prefixed chunks (all little-endian).
pub fn split_chunks(data: &[u8]) -> Result<Chunks<'_>, Box<dyn Error>> {
    if data.len() < 12 || &data[..8] != MAGIC {
        return err("not a fig-kiwi canvas");
    }
    let version = u32::from_le_bytes(data[8..12].try_into()?);
    let mut chunks = Vec::new();
    let mut offset = 12;
    while offset < data.len() {
        let Some(header) = data.get(offset..offset + 4) else {
            return err(format!("truncated chunk header at byte {offset}"));
        };
        let len = u32::from_le_bytes(header.try_into()?) as usize;
        let Some(chunk) = data.get(offset + 4..offset + 4 + len) else {
            return err(format!(
                "chunk at byte {offset} claims {len} bytes past the end"
            ));
        };
        chunks.push(chunk);
        offset += 4 + len;
    }
    Ok((version, chunks))
}

/// Chunks are zstd (newer files) or raw deflate (older files).
pub fn unpack(chunk: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut out = Vec::new();
    if chunk.starts_with(&ZSTD_MAGIC) {
        ruzstd::decoding::StreamingDecoder::new(chunk)?.read_to_end(&mut out)?;
    } else {
        flate2::read::DeflateDecoder::new(chunk).read_to_end(&mut out)?;
    }
    Ok(out)
}

/// `12-345` (URL form), `12:345`, or a Figma URL with `node-id=` → `12:345`.
pub fn normalize_id(id: &str) -> String {
    let id = id
        .split_once("node-id=")
        .map_or(id, |(_, rest)| rest.split('&').next().unwrap_or(rest));
    id.replace("%3A", ":").replace('-', ":")
}

/// `sessionID:localID` of a GUID object, the same id Figma shows in URLs.
pub fn guid_key(guid: &Value) -> Option<String> {
    let (s, l) = (
        guid.get("sessionID")?.as_uint(),
        guid.get("localID")?.as_uint(),
    );
    // Figma writes u32::MAX:u32::MAX for "no guid" (e.g. an unset overrideKey); taking it
    // as a key gives unrelated nodes one id, and the tree then contains itself.
    (s != u32::MAX || l != u32::MAX).then(|| format!("{s}:{l}"))
}

/// Node changes indexed by id, with children ordered as Figma orders them.
pub struct Tree<'a> {
    pub nodes: HashMap<String, &'a Value<'a>>,
    pub children: HashMap<String, Vec<String>>,
    /// The message's byte blobs (path commands of geometry and glyphs), by index.
    pub blobs: Vec<&'a Value<'a>>,
}

impl<'a> Tree<'a> {
    /// Bytes of blob `index`, as kiwi `commandsBlob` fields name them.
    pub fn blob(&self, index: u64) -> Option<Vec<u8>> {
        match self.blobs.get(index as usize)?.get("bytes")? {
            Value::Array(bytes) => bytes
                .iter()
                .map(|b| match b {
                    Value::Byte(b) => Some(*b),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }

    pub fn new(message: &'a Value<'a>) -> Tree<'a> {
        let blobs = match message.get("blobs") {
            Some(Value::Array(blobs)) => blobs.iter().collect(),
            _ => Vec::new(),
        };
        let mut nodes = HashMap::new();
        let mut children: HashMap<String, Vec<(String, String)>> = HashMap::new();
        if let Some(Value::Array(changes)) = message.get("nodeChanges") {
            for node in changes {
                let Some(id) = node.get("guid").and_then(guid_key) else {
                    continue;
                };
                if let Some(parent) = node.get("parentIndex") {
                    let position = parent
                        .get("position")
                        .map(|p| p.as_string().to_owned())
                        .unwrap_or_default();
                    if let Some(parent_id) = parent.get("guid").and_then(guid_key) {
                        children
                            .entry(parent_id)
                            .or_default()
                            .push((position, id.clone()));
                    }
                }
                nodes.insert(id, node);
            }
        }
        let children = children
            .into_iter()
            .map(|(parent, mut list)| {
                list.sort();
                (parent, list.into_iter().map(|(_, id)| id).collect())
            })
            .collect();
        Tree {
            nodes,
            children,
            blobs,
        }
    }
}

/// Converts a kiwi value to JSON; byte arrays are summarized instead of listed.
pub fn to_json(value: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match value {
        Value::Bool(v) => J::from(*v),
        Value::Byte(v) => J::from(*v),
        Value::Int(v) => J::from(*v),
        Value::UInt(v) => J::from(*v),
        Value::Float(v) => J::from(*v),
        Value::String(v) => J::from(v.as_str()),
        Value::Int64(v) => J::from(*v),
        Value::UInt64(v) => J::from(*v),
        Value::Enum(_, name) => J::from(*name),
        Value::Array(items)
            if !items.is_empty() && items.iter().all(|i| matches!(i, Value::Byte(_))) =>
        {
            J::from(format!("<{} bytes>", items.len()))
        }
        Value::Array(items) => J::Array(items.iter().map(to_json).collect()),
        Value::Object(_, fields) => {
            let mut sorted: Vec<_> = fields.iter().collect();
            sorted.sort_by_key(|(name, _)| *name);
            J::Object(
                sorted
                    .into_iter()
                    .map(|(name, v)| match v {
                        // Image hashes are what REST calls imageRef.
                        Value::Array(bytes) if *name == "hash" => (
                            name.to_string(),
                            J::from(
                                bytes
                                    .iter()
                                    .map(|b| match b {
                                        Value::Byte(b) => format!("{b:02x}"),
                                        _ => String::new(),
                                    })
                                    .collect::<String>(),
                            ),
                        ),
                        _ => (name.to_string(), to_json(v)),
                    })
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn frame(version: u32, chunks: &[&[u8]]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(version.to_le_bytes());
        for chunk in chunks {
            out.extend((chunk.len() as u32).to_le_bytes());
            out.extend(*chunk);
        }
        out
    }

    #[test]
    fn splits_version_and_chunks() {
        let data = frame(106, &[b"schema", b"message"]);
        let (version, chunks) = split_chunks(&data).unwrap();
        assert_eq!(
            (version, chunks),
            (106, vec![&b"schema"[..], &b"message"[..]])
        );
    }

    #[test]
    fn rejects_non_fig_data() {
        assert!(split_chunks(b"PK\x03\x04 not a canvas").is_err());
    }

    #[test]
    fn rejects_chunk_longer_than_data() {
        let mut data = frame(106, &[b"abc"]);
        data.truncate(data.len() - 1);
        assert!(split_chunks(&data).is_err());
    }

    #[test]
    fn unpacks_raw_deflate() {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"kiwi").unwrap();
        assert_eq!(unpack(&encoder.finish().unwrap()).unwrap(), b"kiwi");
    }

    #[test]
    fn normalizes_url_node_ids() {
        assert_eq!(normalize_id("12-345"), "12:345");
    }

    #[test]
    fn takes_node_id_from_a_figma_url() {
        let url = "https://www.figma.com/design/AbC123xyz/Example?node-id=7-42&t=abc";
        assert_eq!(normalize_id(url), "7:42");
    }

    #[test]
    fn unset_guid_is_no_key() {
        let guid = |s, l| {
            Value::Object(
                "GUID",
                std::collections::HashMap::from([
                    ("sessionID", Value::UInt(s)),
                    ("localID", Value::UInt(l)),
                ]),
            )
        };
        assert_eq!(
            (guid_key(&guid(u32::MAX, u32::MAX)), guid_key(&guid(1, 2))),
            (None, Some("1:2".to_owned()))
        );
    }
}
