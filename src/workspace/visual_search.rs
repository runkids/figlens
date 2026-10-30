//! Image queries over a workspace's published visual layers (#95). The query image is owned by
//! the request: read once from an authorized path or a stored source raster, never written.

use super::*;
use crate::search::visual::{self, Crop, QueryImage};
use rusqlite::params;
use std::collections::HashMap;

/// `visual` ranks by the image alone; `hybrid` adds the query text's lexical and semantic channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    Visual,
    Hybrid,
}

impl ImageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageMode::Visual => "visual",
            ImageMode::Hybrid => "hybrid",
        }
    }
}

/// Where the query image comes from: a local PNG the caller may read, encoded PNG bytes a
/// caller already holds (the viewer's upload; no CLI or MCP argument carries them), or the
/// stored raster of a node in exactly one published snapshot of a workspace source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Path(PathBuf),
    Png(Vec<u8>),
    Ref {
        source_id: String,
        snapshot_id: String,
        node_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct ImageQuery {
    pub input: Input,
    pub crop: Option<Crop>,
    pub mode: ImageMode,
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    fail(ErrorKind::InvalidArguments, message)
}

impl ImageQuery {
    /// Checks `mode` against the query `text`: `visual` takes an image alone; `hybrid` takes an
    /// image with or without text, ranking every channel the request can use; without a mode
    /// the text decides. Text and semantic modes take no image.
    pub fn new(input: Input, crop: Option<Crop>, mode: Option<&str>, text: &str) -> Result<Self> {
        let mode = match (mode, !text.is_empty()) {
            (None | Some("visual"), false) => ImageMode::Visual,
            (None, true) | (Some("hybrid"), _) => ImageMode::Hybrid,
            (Some("visual"), true) => {
                return Err(invalid(
                    "mode visual takes an image alone; an image with query text uses mode hybrid",
                ));
            }
            (Some(m), _) => {
                return Err(invalid(format!(
                    "an image search uses mode visual or hybrid, not {m}"
                )));
            }
        };
        Ok(Self { input, crop, mode })
    }

    /// What the reply says about the query image: never the file path.
    pub(super) fn describe(&self) -> Json {
        let crop = self
            .crop
            .map(|c| json!({"x": c.x, "y": c.y, "width": c.width, "height": c.height}));
        match &self.input {
            Input::Path(_) => json!({"kind": "path", "crop": crop}),
            Input::Png(_) => json!({"kind": "bytes", "crop": crop}),
            Input::Ref {
                source_id,
                snapshot_id,
                node_id,
            } => json!({"kind": "ref", "sourceId": source_id, "snapshotId": snapshot_id,
                "nodeId": node_id, "crop": crop}),
        }
    }
}

/// `x,y,width,height` in pixels of the query image, all non-negative and the size positive.
pub fn crop(text: &str) -> Result<Crop> {
    let n: Vec<u32> = text
        .split(',')
        .map(|v| v.trim().parse::<u32>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| invalid("crop takes x,y,width,height in whole pixels"))?;
    let [x, y, width, height] = n[..] else {
        return Err(invalid("crop takes x,y,width,height in whole pixels"));
    };
    positive(x, y, width, height)
}

fn positive(x: u32, y: u32, width: u32, height: u32) -> Result<Crop> {
    if width == 0 || height == 0 {
        return Err(invalid("crop width and height must be positive"));
    }
    Ok(Crop {
        x,
        y,
        width,
        height,
    })
}

/// `{"x", "y", "width", "height"}` in whole pixels, as `crop` takes them.
pub fn crop_value(value: &Json) -> Result<Crop> {
    let n = |k: &str| value[k].as_u64().and_then(|n| u32::try_from(n).ok());
    match (
        value.as_object().map(|o| o.len()),
        n("x"),
        n("y"),
        n("width"),
        n("height"),
    ) {
        (Some(4), Some(x), Some(y), Some(w), Some(h)) => positive(x, y, w, h),
        _ => Err(invalid(
            "crop takes exactly x, y, width and height in whole pixels",
        )),
    }
}

/// `{"sourceId", "snapshotId", "nodeId"}`, each a non-empty string and nothing else.
pub fn reference(value: &Json) -> Result<Input> {
    let field = |k: &str| {
        value[k]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= crate::search::store::ID_BYTES)
            .map(str::to_owned)
    };
    match (
        value.as_object().map(|o| o.len()),
        field("sourceId"),
        field("snapshotId"),
        field("nodeId"),
    ) {
        (Some(3), Some(source_id), Some(snapshot_id), Some(node_id)) => Ok(Input::Ref {
            source_id,
            snapshot_id,
            node_id,
        }),
        _ => Err(invalid(
            "imageRef takes exactly sourceId, snapshotId and nodeId from a search result",
        )),
    }
}

/// The step that builds a source's visual layers for `scope`.
pub(super) fn visual_step(ws_id: &str, source_id: &Json, scope: &str, why: &str) -> Json {
    json!({"tool": "index", "arguments": {"action": "visual", "workspaceId": ws_id,
        "sourceId": source_id, "scope": scope}, "why": why})
}

/// The stored raster of node `node_id` in generation `gen_id`, uncropped, if it is a unit there.
pub(super) fn stored(
    tx: &rusqlite::Transaction,
    gen_id: i64,
    node_id: &str,
) -> Result<Option<QueryImage>> {
    let rid: Option<u32> = tx
        .query_row(
            "SELECT rid FROM records WHERE gen = ?1 AND node_id = ?2",
            params![gen_id, node_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    match rid {
        Some(rid) => super::visual::image(tx, gen_id, rid),
        None => Ok(None),
    }
}

/// The state a generation's visual layer under `profile` was published in: the hash layer is
/// recorded with the visual version as its profile, a vision layer with its profile id.
pub(super) fn layer_state(
    tx: &rusqlite::Transaction,
    gen_id: i64,
    profile: &str,
) -> Result<Option<String>> {
    tx.query_row(
        "SELECT state FROM model_layers WHERE gen = ?1 AND profile_id = ?2 AND policy = ?3",
        params![gen_id, profile, super::visual::POLICY],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)
}

/// The vision vectors stored for generation `gen_id`'s units in `rids` under `profile`, by
/// record id; units the filters left out are not decoded, and a scan `bounds` stops is partial. Identical rasters share one vector,
/// keyed by their raster key.
pub(super) fn vectors(
    tx: &rusqlite::Transaction,
    gen_id: i64,
    profile: &str,
    dim: usize,
    rids: &std::collections::HashSet<u32>,
    bounds: &mut crate::expand::Bounds,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<HashMap<u32, Vec<f32>>> {
    let mut stmt = tx
        .prepare(
            "SELECT u.rid, v.vector FROM visual_units u
             JOIN model_refs r ON r.gen = u.gen AND r.profile_id = ?2 AND r.policy = u.policy
                AND r.text_hash = u.raster_key
             JOIN model_vectors v ON v.profile_id = r.profile_id AND v.text_hash = r.text_hash
             WHERE u.gen = ?1 AND u.policy = ?3 AND u.error IS NULL",
        )
        .map_err(sql)?;
    let mut rows = stmt
        .query(params![gen_id, profile, super::visual::POLICY])
        .map_err(sql)?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next().map_err(sql)? {
        let rid: u32 = row.get(0).map_err(sql)?;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(fail(ErrorKind::Cancelled, "search cancelled"));
        }
        // Each kept vector costs its decode, its later scoring and the floats it retains.
        if rids.contains(&rid) {
            if !bounds.charge(dim, 4 * dim) {
                break;
            }
            let vector = crate::semantic::decode(row.get_ref(1).map_err(sql)?.as_blob()?, dim)?;
            out.insert(rid, vector);
        }
    }
    Ok(out)
}

/// The query image's identity for cursors: its normalized fingerprint and the crop.
pub(super) fn digest(image: &QueryImage, query: &ImageQuery) -> Json {
    json!([
        crate::search::hex(&image.fingerprint.encode()),
        query.describe()["crop"],
        visual::VERSION
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::visual::RenderedUnit;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn a_vector_scan_stops_at_its_bounds() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (kind TEXT, source_id TEXT, scope TEXT, state TEXT,
             expected_gen INTEGER);
             CREATE INDEX jobs_active ON jobs (source_id, scope);",
        )
        .unwrap();
        conn.execute_batch(super::super::database::V9).unwrap();
        conn.execute_batch(super::super::database::V10).unwrap();
        let dim = 4;
        for (rid, shade) in [(1u32, 10u8), (2, 200)] {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(8, 8).unwrap();
            pixmap.fill(resvg::tiny_skia::Color::from_rgba8(shade, 0, 0, 255));
            let rendered = RenderedUnit {
                image: QueryImage::from_pixmap(&pixmap, None).unwrap(),
                diagnostics: vec![],
                diagnostics_truncated: false,
                bounds: serde_json::Map::new(),
            };
            let key = super::super::visual::save(&conn, 7, rid, &rendered).unwrap();
            conn.execute(
                "INSERT INTO model_vectors VALUES ('p', ?1, ?2)",
                params![key.to_vec(), crate::semantic::encode(&[1.0, 0.0, 0.0, 0.0])],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO model_refs VALUES (7, 'p', ?1, ?2)",
                params![super::super::visual::POLICY, key.to_vec()],
            )
            .unwrap();
        }
        let tx = conn.transaction().unwrap();
        let rids = [1, 2].into_iter().collect();
        let scan = |work: usize| {
            let mut bounds = crate::expand::Bounds {
                work,
                bytes: usize::MAX,
                depth: 0,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
                stop: None,
            };
            let found = vectors(
                &tx,
                7,
                "p",
                dim,
                &rids,
                &mut bounds,
                &AtomicBool::new(false),
            );
            (found.unwrap().len(), bounds.stop)
        };
        assert_eq!(scan(2 * dim), (2, None));
        assert_eq!(scan(dim), (1, Some(crate::expand::Stop::Work)));
        let cancelled = vectors(
            &tx,
            7,
            "p",
            dim,
            &rids,
            &mut crate::expand::Bounds {
                work: usize::MAX,
                bytes: usize::MAX,
                depth: 0,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
                stop: None,
            },
            &AtomicBool::new(true),
        );
        assert!(cancelled.is_err());
    }
}
