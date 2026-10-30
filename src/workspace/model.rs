//! The stored model layer of semantic search (#94): vectors by profile and chunk text,
//! each generation's references to them, and each layer's published progress. Only an embed
//! job writes a layer; queries read it.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// How records become model inputs; a new policy is a new layer, never mixed with an old one.
pub const POLICY: &str = "chunks-400-name-text-1";
/// Texts one statement reads or writes.
const BATCH: usize = 500;

/// The key a chunk text's vector is stored under.
pub fn text_hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Deletes vectors no layer references; run after a generation's references are dropped.
pub(super) fn collect(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM model_vectors WHERE NOT EXISTS (SELECT 1 FROM model_refs r
         WHERE r.profile_id = model_vectors.profile_id AND r.text_hash = model_vectors.text_hash)",
        [],
    )?;
    Ok(())
}

/// The stored vectors of `hashes` under `profile`: `Some(vector)`, empty when the runtime
/// refused that text. Read in batches, never one statement per text.
pub fn vectors(
    conn: &Connection,
    profile: &str,
    hashes: &[[u8; 32]],
) -> rusqlite::Result<HashMap<[u8; 32], Vec<u8>>> {
    let mut found = HashMap::new();
    for batch in hashes.chunks(BATCH) {
        let marks = vec!["?"; batch.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT text_hash, vector FROM model_vectors WHERE profile_id = ?1 AND text_hash IN ({marks})"
        ))?;
        let args = std::iter::once(&profile as &dyn rusqlite::ToSql)
            .chain(batch.iter().map(|h| h as &dyn rusqlite::ToSql));
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (hash, vector) = row?;
            if let Ok(hash) = <[u8; 32]>::try_from(hash.as_slice()) {
                found.insert(hash, vector);
            }
        }
    }
    Ok(found)
}

/// Stores one embedded batch of layer (`gen`, `profile`, `POLICY`): each text's vector (empty
/// when refused) and the layer's reference to it. The caller commits it with the layer's
/// progress in one transaction, after checking its fence.
pub fn save(
    conn: &Connection,
    gen_id: i64,
    profile: &str,
    batch: &[([u8; 32], Vec<u8>)],
) -> rusqlite::Result<()> {
    let mut vector = conn.prepare_cached(
        "INSERT OR IGNORE INTO model_vectors (profile_id, text_hash, vector) VALUES (?1, ?2, ?3)",
    )?;
    let mut reference = conn.prepare_cached(
        "INSERT OR IGNORE INTO model_refs (gen, profile_id, policy, text_hash) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (hash, bytes) in batch {
        vector.execute(params![profile, hash, bytes])?;
        reference.execute(params![gen_id, profile, POLICY, hash])?;
    }
    Ok(())
}

/// Records the layer's progress; `state` is `building`, `ready`, `partial` or `failed`.
pub fn set_layer(
    conn: &Connection,
    gen_id: i64,
    profile: &str,
    state: &str,
    counts: &Json,
    job_id: &str,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO model_layers (gen, profile_id, policy, state, chunks, texts, embedded, refused,
         stop, job_id, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (gen, profile_id, policy) DO UPDATE SET state = ?4, chunks = ?5, texts = ?6,
         embedded = ?7, refused = ?8, stop = ?9, job_id = ?10, updated_ms = ?11",
        params![
            gen_id,
            profile,
            POLICY,
            state,
            counts["chunks"].as_i64().unwrap_or(0),
            counts["texts"].as_i64().unwrap_or(0),
            counts["embedded"].as_i64().unwrap_or(0),
            counts["refused"].as_i64().unwrap_or(0),
            counts["stop"].as_str(),
            job_id,
            now_ms
        ],
    )?;
    Ok(())
}

/// The published progress of layer (`gen`, `profile`, `POLICY`), `None` before its job started.
pub fn layer(conn: &Connection, gen_id: i64, profile: &str) -> rusqlite::Result<Option<Json>> {
    conn.query_row(
        "SELECT state, chunks, texts, embedded, refused, stop, job_id, updated_ms FROM model_layers
         WHERE gen = ?1 AND profile_id = ?2 AND policy = ?3",
        params![gen_id, profile, POLICY],
        |r| {
            Ok(
                json!({"state": r.get::<_, String>(0)?, "chunks": r.get::<_, i64>(1)?,
                "texts": r.get::<_, i64>(2)?, "embeddedTexts": r.get::<_, i64>(3)?,
                "refused": r.get::<_, i64>(4)?, "stop": r.get::<_, Option<String>>(5)?,
                "jobId": r.get::<_, Option<String>>(6)?, "updatedMs": r.get::<_, i64>(7)?,
                "policy": POLICY}),
            )
        },
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(super::super::database::V9).unwrap();
        conn
    }

    #[test]
    fn vectors_outlive_one_generation_only_while_another_references_them() {
        let conn = db();
        let (a, b) = (text_hash("a"), text_hash("b"));
        save(&conn, 1, "p", &[(a, vec![1]), (b, vec![2])]).unwrap();
        save(&conn, 2, "p", &[(a, vec![1])]).unwrap();
        conn.execute("DELETE FROM model_refs WHERE gen = 1", [])
            .unwrap();
        collect(&conn).unwrap();
        let left = vectors(&conn, "p", &[a, b]).unwrap();
        assert_eq!(
            (left.contains_key(&a), left.contains_key(&b)),
            (true, false)
        );
    }

    #[test]
    fn a_layer_reports_the_progress_its_job_set() {
        let conn = db();
        assert!(layer(&conn, 1, "p").unwrap().is_none());
        let counts = json!({"chunks": 3, "texts": 2, "embedded": 1, "refused": 0});
        set_layer(&conn, 1, "p", "building", &counts, "job", 5).unwrap();
        let got = layer(&conn, 1, "p").unwrap().unwrap();
        assert_eq!(
            (got["state"].as_str(), got["embeddedTexts"].as_i64()),
            (Some("building"), Some(1))
        );
    }
}
