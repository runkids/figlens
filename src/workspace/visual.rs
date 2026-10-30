//! Generation-bound visual units. Only source renders enter this cache; query images never do.

use crate::search::visual::{Fingerprint, QueryImage, RenderedUnit};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Output bounds, white compositing and 256-pixel normalization are pinned by this policy.
pub const POLICY: &str = "visual-export1024-white256-1";
pub const MAX_UNITS: usize = 2_000;
const MAX_PNG: usize = 300 * 1024;
const MAX_REPORT: usize = 256 * 1024;

/// Read an already authorized canonical local PNG without following a link swapped into its
/// path. The caller keeps this image only for the request; this function creates no files.
pub fn query_image(
    path: &std::path::Path,
    crop: Option<crate::search::visual::Crop>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<QueryImage> {
    use crate::search::visual::MAX_BYTES;
    use std::io::Read;
    use std::sync::atomic::Ordering;
    let cancelled = || {
        if cancel.load(Ordering::Relaxed) {
            Err(super::fail(
                crate::ErrorKind::Cancelled,
                "image query cancelled",
            ))
        } else {
            Ok(())
        }
    };
    cancelled()?;
    // Refuse devices and pipes before opening too, including Windows named pipes.
    if !std::fs::metadata(path)?.is_file() {
        return Err(super::fail(
            crate::ErrorKind::InvalidArguments,
            "query image must be a regular PNG",
        ));
    }
    // The platform helper opens the handle without following replaced path components.
    let mut file = super::storage::registered(path).map_err(|e| {
        if e.downcast_ref::<crate::FigError>().is_some_and(|f| f.kind() == crate::ErrorKind::SourceUnreadable) {
            super::fail(crate::ErrorKind::SourceUnreadable,
                "query image path changed through a link; select and authorize its current path, then retry")
        } else {
            e
        }
    })?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(super::fail(
            crate::ErrorKind::InvalidArguments,
            "query image must be a regular PNG of at most 8 MiB",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut block = [0; 64 * 1024];
    loop {
        cancelled()?;
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_BYTES {
            return Err(super::fail(
                crate::ErrorKind::InvalidArguments,
                "query image exceeds 8 MiB",
            ));
        }
        bytes.extend_from_slice(&block[..n]);
    }
    cancelled()?;
    let image = QueryImage::from_png(&bytes, crop)?;
    cancelled()?;
    Ok(image)
}

/// A unit's source render and its evidence. A failed render keeps its text record intact.
pub struct Unit {
    pub rid: u32,
    pub fingerprint: Option<Fingerprint>,
    pub report: Json,
    pub error: Option<String>,
}

/// Writes a rendered source unit in the caller's fenced transaction. Identical normalized
/// pixels share one raster across generations; the reference keeps this generation's evidence.
pub fn save(conn: &Connection, gen_id: i64, rid: u32, rendered: &RenderedUnit) -> Result<[u8; 32]> {
    let fingerprint = rendered.image.fingerprint.encode();
    let mut digest = Sha256::new();
    digest.update(POLICY.as_bytes());
    digest.update(fingerprint);
    let key: [u8; 32] = digest.finalize().into();
    let report = json!({"bounds": rendered.bounds, "diagnostics": rendered.diagnostics,
        "diagnosticsTruncated": rendered.diagnostics_truncated});
    let report = report.to_string();
    if report.len() > MAX_REPORT {
        return Err(super::fail(
            crate::ErrorKind::QueryWorkLimit,
            "visual render evidence exceeds 256 KiB",
        ));
    }
    // A known raster needs neither another PNG allocation nor another large SQLite write.
    let present = conn
        .query_row(
            "SELECT 1 FROM visual_rasters WHERE raster_key = ?1 AND policy = ?2",
            params![key, POLICY],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !present {
        let png = rendered.image.png()?;
        if png.len() > MAX_PNG {
            return Err(super::fail(
                crate::ErrorKind::QueryWorkLimit,
                "normalized visual raster exceeds 300 KiB",
            ));
        }
        conn.execute(
            "INSERT INTO visual_rasters (raster_key, policy, fingerprint, png) VALUES (?1, ?2, ?3, ?4)",
            params![key, POLICY, fingerprint.as_slice(), png],
        )?;
    }
    conn.execute(
        "INSERT INTO visual_units (gen, rid, policy, raster_key, report, error) VALUES (?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT (gen, rid, policy) DO UPDATE SET raster_key = ?4, report = ?5, error = NULL",
        params![gen_id, rid, POLICY, key, report],
    )?;
    Ok(key)
}

/// Writes a failed source unit without deleting or changing any text record.
pub fn failed(conn: &Connection, gen_id: i64, rid: u32, reason: &str) -> Result<()> {
    let reason: String = reason.chars().take(1024).collect();
    conn.execute(
        "INSERT INTO visual_units (gen, rid, policy, raster_key, report, error) VALUES (?1, ?2, ?3, NULL, '{}', ?4)
         ON CONFLICT (gen, rid, policy) DO UPDATE SET raster_key = NULL, report = '{}', error = ?4",
        params![gen_id, rid, POLICY, reason],
    )?;
    Ok(())
}

/// Compact ranking input: no thumbnail or diagnostic JSON is copied during a candidate scan.
pub struct Candidate {
    pub rid: u32,
    pub raster_key: [u8; 32],
    pub fingerprint: Fingerprint,
}

pub fn candidates(conn: &Connection, gen_id: i64) -> Result<Vec<Candidate>> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM visual_units WHERE gen = ?1 AND policy = ?2",
        params![gen_id, POLICY],
        |r| r.get(0),
    )?;
    if !(0..=MAX_UNITS as i64).contains(&count) {
        return Err(super::fail(
            crate::ErrorKind::QueryWorkLimit,
            "visual unit capacity exceeded; index a narrower scope",
        ));
    }
    let mut stmt = conn.prepare(
        "SELECT u.rid, u.raster_key, r.fingerprint FROM visual_units u
         LEFT JOIN visual_rasters r ON r.raster_key = u.raster_key AND r.policy = u.policy
         WHERE u.gen = ?1 AND u.policy = ?2 AND u.error IS NULL ORDER BY u.rid",
    )?;
    let mut rows = stmt.query(params![gen_id, POLICY])?;
    let mut out = Vec::with_capacity(count as usize);
    while let Some(row) = rows.next()? {
        let key = row.get_ref(1)?.as_blob()?;
        let key: [u8; 32] = key.try_into().map_err(|_| {
            super::fail(
                crate::ErrorKind::IndexIncompatible,
                "invalid visual raster key; rebuild this layer",
            )
        })?;
        let fingerprint = Fingerprint::decode(row.get_ref(2)?.as_blob()?)?;
        out.push(Candidate {
            rid: row.get(0)?,
            raster_key: key,
            fingerprint,
        });
    }
    Ok(out)
}

/// Reads at most the supported visual capacity. Fingerprints and evidence are checked as
/// borrowed SQLite values before allocating them; a damaged cache is an error, never absence.
pub fn units(conn: &Connection, gen_id: i64) -> Result<Vec<Unit>> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM visual_units WHERE gen = ?1 AND policy = ?2",
        params![gen_id, POLICY],
        |r| r.get(0),
    )?;
    if !(0..=MAX_UNITS as i64).contains(&count) {
        return Err(super::fail(
            crate::ErrorKind::QueryWorkLimit,
            "visual unit capacity exceeded; index a narrower scope",
        ));
    }
    let mut stmt = conn.prepare(
        "SELECT u.rid, r.fingerprint, u.report, u.error FROM visual_units u
         LEFT JOIN visual_rasters r ON r.raster_key = u.raster_key AND r.policy = u.policy
         WHERE u.gen = ?1 AND u.policy = ?2 ORDER BY u.rid",
    )?;
    let mut rows = stmt.query(params![gen_id, POLICY])?;
    let mut out = Vec::with_capacity(count as usize);
    while let Some(row) = rows.next()? {
        out.push(read(row)?);
    }
    Ok(out)
}

/// One unit of generation `gen_id`: what a shown visual search result reports about its render.
pub fn unit(conn: &Connection, gen_id: i64, rid: u32) -> Result<Option<Unit>> {
    let mut stmt = conn.prepare_cached(
        "SELECT u.rid, r.fingerprint, u.report, u.error FROM visual_units u
         LEFT JOIN visual_rasters r ON r.raster_key = u.raster_key AND r.policy = u.policy
         WHERE u.gen = ?1 AND u.rid = ?2 AND u.policy = ?3",
    )?;
    let mut rows = stmt.query(params![gen_id, rid, POLICY])?;
    rows.next()?.map(read).transpose()
}

/// A `units`/`unit` row: its evidence checked against its budget as a borrowed SQLite value.
fn read(row: &rusqlite::Row) -> Result<Unit> {
    let report = row.get_ref(2)?.as_str()?;
    if report.len() > MAX_REPORT {
        return Err(super::fail(
            crate::ErrorKind::IndexIncompatible,
            "stored visual evidence exceeds its budget; rebuild this layer",
        ));
    }
    let fingerprint = match row.get_ref(1)? {
        rusqlite::types::ValueRef::Null => None,
        value => Some(Fingerprint::decode(value.as_blob()?)?),
    };
    let error = match row.get_ref(3)? {
        rusqlite::types::ValueRef::Null => None,
        value => {
            let error = value.as_str()?;
            if error.len() > 4096 {
                return Err(super::fail(
                    crate::ErrorKind::IndexIncompatible,
                    "stored visual error exceeds its budget; rebuild this layer",
                ));
            }
            Some(error.to_owned())
        }
    };
    if fingerprint.is_none() && error.is_none() {
        return Err(super::fail(
            crate::ErrorKind::IndexIncompatible,
            "visual raster is missing; rebuild this layer",
        ));
    }
    Ok(Unit {
        rid: row.get(0)?,
        fingerprint,
        report: serde_json::from_str(report)?,
        error,
    })
}

/// Reads one selected source thumbnail for crop/selected-node queries. Input bytes passed to
/// visual search never reach this table, and no arbitrary filesystem location is accepted.
pub fn image(conn: &Connection, gen_id: i64, rid: u32) -> Result<Option<QueryImage>> {
    let mut stmt = conn.prepare(
        "SELECT r.png, r.fingerprint FROM visual_units u JOIN visual_rasters r ON r.raster_key = u.raster_key AND r.policy = u.policy
         WHERE u.gen = ?1 AND u.rid = ?2 AND u.policy = ?3",
    )?;
    let mut rows = stmt.query(params![gen_id, rid, POLICY])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let bytes = row.get_ref(0)?.as_blob()?;
    if bytes.len() > MAX_PNG {
        return Err(super::fail(
            crate::ErrorKind::IndexIncompatible,
            "stored visual raster exceeds its budget; rebuild this layer",
        ));
    }
    let mut image = QueryImage::from_png(bytes, None)?;
    // The PNG is square after aspect-fit; keep the source aspect in exact/hash comparisons.
    image.fingerprint = Fingerprint::decode(row.get_ref(1)?.as_blob()?)?;
    Ok(Some(image))
}

/// Generation cleanup runs after its unit references are deleted, preserving shared rasters.
pub(super) fn collect(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM visual_rasters WHERE NOT EXISTS (
        SELECT 1 FROM visual_units u WHERE u.raster_key = visual_rasters.raster_key)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // The part of the jobs table schema 10 changes.
        conn.execute_batch(
            "CREATE TABLE jobs (kind TEXT, source_id TEXT, scope TEXT, state TEXT,
             expected_gen INTEGER, profile_id TEXT);
             CREATE INDEX jobs_active ON jobs (source_id, scope);",
        )
        .unwrap();
        conn.execute_batch(super::super::database::V10).unwrap();
        conn
    }

    fn rendered() -> RenderedUnit {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 16).unwrap();
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(12, 34, 56, 255));
        RenderedUnit {
            image: QueryImage::from_pixmap(&pixmap, None).unwrap(),
            diagnostics: vec![],
            diagnostics_truncated: false,
            bounds: serde_json::Map::new(),
        }
    }

    #[test]
    fn source_rasters_are_shared_but_generation_evidence_is_separate() {
        let conn = db();
        let a = rendered();
        let mut b = rendered();
        b.diagnostics
            .push(json!({"feature": "blur", "state": "approximate"}));
        assert_eq!(
            save(&conn, 1, 5, &a).unwrap(),
            save(&conn, 2, 9, &b).unwrap()
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM visual_rasters", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let first = units(&conn, 1).unwrap();
        let second = units(&conn, 2).unwrap();
        assert_eq!(first[0].rid, 5);
        assert_eq!(second[0].rid, 9);
        assert!(
            first[0].report["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(second[0].report["diagnostics"][0]["feature"], "blur");
        assert_eq!(first[0].fingerprint, Some(a.image.fingerprint));
        assert_eq!(
            image(&conn, 1, 5).unwrap().unwrap().fingerprint,
            first[0].fingerprint.clone().unwrap()
        );
        conn.execute("DELETE FROM visual_units WHERE gen = 1", [])
            .unwrap();
        collect(&conn).unwrap();
        assert!(image(&conn, 2, 9).unwrap().is_some());
        conn.execute("DELETE FROM visual_units WHERE gen = 2", [])
            .unwrap();
        collect(&conn).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM visual_rasters", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn failed_render_retains_other_units_and_bad_cache_is_not_absence() {
        let conn = db();
        save(&conn, 1, 5, &rendered()).unwrap();
        failed(&conn, 1, 6, "unsupported node").unwrap();
        let got = units(&conn, 1).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].fingerprint.is_some());
        let ranked = candidates(&conn, 1).unwrap();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].rid, 5);
        assert_eq!(Some(&ranked[0].fingerprint), got[0].fingerprint.as_ref());
        assert_eq!(got[1].error.as_deref(), Some("unsupported node"));
        assert!(image(&conn, 1, 6).unwrap().is_none());
        conn.execute("DELETE FROM visual_rasters", []).unwrap();
        assert!(units(&conn, 1).is_err());
        assert!(candidates(&conn, 1).is_err());
    }

    #[test]
    fn local_query_image_is_bounded_cancellable_and_never_changes_its_source() {
        let dir = crate::test_support::TestDir::new();
        let path = dir.join("query.png");
        let png = rendered().image.png().unwrap();
        std::fs::write(&path, &png).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let image = query_image(&path, None, &cancel).unwrap();
        assert_eq!(image.png().unwrap(), png);
        assert_eq!(std::fs::read(&path).unwrap(), png);
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let error = query_image(&path, None, &cancel).err().unwrap();
        assert_eq!(
            error.downcast_ref::<crate::FigError>().unwrap().kind(),
            crate::ErrorKind::Cancelled
        );
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(crate::search::visual::MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(
            query_image(&path, None, &cancel)
                .err()
                .unwrap()
                .to_string()
                .contains("8 MiB")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_query_image_cannot_open_a_replaced_path_or_block_on_a_fifo() {
        use std::os::unix::fs::symlink;
        let dir = crate::test_support::TestDir::new();
        let original = dir.join("original.png");
        let alias = dir.join("query.png");
        std::fs::write(&original, rendered().image.png().unwrap()).unwrap();
        let alias = std::path::absolute(alias).unwrap();
        symlink(std::fs::canonicalize(&original).unwrap(), &alias).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(
            query_image(&alias, None, &cancel)
                .err()
                .unwrap()
                .to_string()
                .contains("query image path")
        );
        let fifo = dir.join("pipe.png");
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a NUL-terminated owned test path.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(query_image(&std::fs::canonicalize(fifo).unwrap(), None, &cancel).is_err());
    }
}
