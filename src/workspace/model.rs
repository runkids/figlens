//! The stored model layer of semantic search (#94): vectors by profile and chunk text,
//! each generation's references to them, and each layer's published progress. Only an embed
//! job writes a layer; queries read it.

use crate::expand::Bounds;
use crate::search::semantic::{LAYER_BYTES, LAYER_WORK, Layer};
use crate::search::store;
use crate::semantic::{self, Profile, profile};
use crate::{ErrorKind, FigError};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::sync::atomic::AtomicBool;

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

/// One batch of texts to vectors, in order: the embedder a model worker runs in its own process.
pub type Embed = Box<dyn FnMut(&[String]) -> Result<Vec<Vec<f32>>, Box<dyn Error>>>;

/// Records one embed generation loads, as a workspace search reads at most.
const RECORDS: i64 = 100_000;

/// The hidden `__model-worker <workspace dir> <jobId>` of a profile's runtime: runs one embed
/// job's attempt in this process and returns the JSON line it prints. `load` reads the model
/// weights of the profile it is given, after its executable and pinned files were verified
/// again, and returns the runtime's contract (its hello) with the embedder; it is called only
/// when some text has no stored vector yet.
pub fn run_embed_worker(
    dir: &Path,
    job_id: &str,
    load: impl FnOnce(&Profile) -> Result<(Json, Embed), Box<dyn Error>>,
) -> Json {
    super::jobs::worker_with(dir, job_id, |dir, job_id, me, attempt| {
        embed_work(dir, job_id, me, attempt, load)
    })
}

fn fail(message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(ErrorKind::OperationFailed, message).into()
}

fn sql(e: rusqlite::Error) -> Box<dyn Error> {
    e.into()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Embeds attempt `attempt` of embed job `job_id` for owner `me`. Each batch commits its
/// vectors, references and the layer's `building` progress in one transaction, and only while
/// the fence holds; the last write marks the layer `ready` or `partial` and the job succeeded
/// together. A crash or kill leaves the layer `building`, which reads as not ready.
fn embed_work(
    dir: &Path,
    job_id: &str,
    me: &str,
    attempt: i64,
    load: impl FnOnce(&Profile) -> Result<(Json, Embed), Box<dyn Error>>,
) -> Result<Json, Box<dyn Error>> {
    let (_lock, mut conn, ceiling) = super::jobs::worker_conn(dir, job_id)?;
    let full = |e: Box<dyn Error>| super::jobs::full(e, ceiling);
    let row: Option<(String, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT incarnation, expected_gen, profile_id FROM jobs WHERE job_id = ?1
             AND owner = ?2 AND attempt = ?3 AND state = 'running' AND kind = 'embed'",
            params![job_id, me, attempt],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    let (incarnation, Some(gen_id), Some(profile_id)) =
        row.ok_or_else(|| fail("another attempt owns this job now"))?
    else {
        return Err(fail("the embed job names no generation or profile"));
    };
    if super::meta(&conn, "incarnation").map_err(sql)?.as_deref() != Some(incarnation.as_str()) {
        return Err(fail("the workspace was recreated"));
    }
    let storage = crate::workspace::storage(None)?;
    // The job still owns this launch and no cancel landed; the source keeps its membership; its
    // scope still publishes this generation; and the active profile is still the job's.
    let fence = |conn: &Connection| -> Result<(), Box<dyn Error>> {
        let holds = conn
            .query_row(
                "SELECT 1 FROM jobs j
                 JOIN sources s ON s.source_id = j.source_id AND s.membership = j.membership
                 JOIN published p ON p.source_id = j.source_id AND p.scope = j.scope
                  AND p.gen = j.expected_gen
                 WHERE j.job_id = ?1 AND j.owner = ?2 AND j.attempt = ?3
                 AND j.state = 'running' AND j.cancel = 0",
                params![job_id, me, attempt],
                |_| Ok(()),
            )
            .optional()
            .map_err(sql)?;
        if holds.is_none() {
            return Err(fail(
                "superseded: the job was cancelled or taken over, or its scope published another generation or changed source; run index embed again",
            ));
        }
        match profile::active(&storage)? {
            Some(p) if p.id == profile_id => Ok(()),
            _ => Err(fail(
                "superseded: the configured model profile changed while the layer was embedded",
            )),
        }
    };
    let stage = |conn: &Connection, stage: &str| -> Result<(), Box<dyn Error>> {
        conn.execute(
            "UPDATE jobs SET stage = ?2 WHERE job_id = ?1 AND owner = ?3 AND attempt = ?4
             AND state = 'running'",
            params![job_id, stage, me, attempt],
        )
        .map_err(sql)?;
        super::jobs::hold(stage);
        Ok(())
    };
    // `counts` committed as the layer's `state` with the job's progress, under the fence.
    let commit = |conn: &mut Connection,
                  rows: &[([u8; 32], Vec<u8>)],
                  state: &str,
                  counts: &Json|
     -> Result<(), Box<dyn Error>> {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
        fence(&tx)?;
        save(&tx, gen_id, &profile_id, rows).map_err(|e| full(sql(e)))?;
        set_layer(&tx, gen_id, &profile_id, state, counts, job_id, now_ms()).map_err(sql)?;
        let done = state != "building";
        tx.execute(
            "UPDATE jobs SET done = ?2, total = ?3,
             state = CASE WHEN ?4 THEN 'succeeded' ELSE state END,
             stage = CASE WHEN ?4 THEN 'publish' ELSE stage END,
             gen = CASE WHEN ?4 THEN ?5 ELSE gen END,
             finished_ms = CASE WHEN ?4 THEN ?6 ELSE finished_ms END
             WHERE job_id = ?1",
            params![
                job_id,
                counts["embedded"].as_i64().unwrap_or(0) + counts["refused"].as_i64().unwrap_or(0),
                counts["texts"].as_i64(),
                done,
                gen_id,
                now_ms()
            ],
        )
        .map_err(sql)?;
        tx.commit().map_err(|e| full(sql(e)))?;
        // The same WAL backpressure as a text build: a reader cannot pin an ever-growing log.
        super::jobs::reclaim_wal(conn, dir)
    };
    let ready = layer(&conn, gen_id, &profile_id).map_err(sql)?;
    if let Some(ready) = ready.filter(|l| l["state"] == "ready") {
        let counts = json!({"chunks": ready["chunks"], "texts": ready["texts"],
            "embedded": ready["embeddedTexts"], "refused": ready["refused"]});
        commit(&mut conn, &[], "ready", &counts)?;
        return Ok(json!({"gen": gen_id, "layer": counts, "embeddedNow": 0}));
    }
    let profile = profile::active(&storage)?
        .filter(|p| p.id == profile_id)
        .ok_or_else(|| {
            fail("the configured model profile changed since this embed job was submitted")
        })?;
    stage(&conn, "load")?;
    let records: i64 = conn
        .query_row(
            "SELECT records FROM generations WHERE gen = ?1",
            [gen_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if records > RECORDS {
        return Err(FigError::new(
            ErrorKind::QueryWorkLimit,
            format!("the index holds {records} records, more than the {RECORDS} one model layer embeds; index a narrower scope"),
        )
        .into());
    }
    let never = AtomicBool::new(false);
    let mut spent = 0;
    let mut index = store::load_with(&conn, gen_id, &[], &mut spent, &never)?;
    let all: Vec<u32> = (0..records as u32).collect();
    store::add_with(&conn, gen_id, &mut index, &all, &mut spent, &never)?;
    stage(&conn, "extract")?;
    let bounds = Bounds {
        work: LAYER_WORK,
        bytes: LAYER_BYTES,
        depth: 0,
        deadline: crate::search::deadline(),
        stop: None,
    };
    let extracted = Layer::new(&index, Json::Null, profile.dim, &mut { bounds }, &never)?;
    drop(index);
    let hashes: Vec<[u8; 32]> = extracted.texts.iter().map(|t| text_hash(t)).collect();
    let stored = vectors(&conn, &profile_id, &hashes).map_err(sql)?;
    let stop = extracted
        .stop
        .or((extracted.over > 0).then_some("semantic_capacity"));
    let mut counts = json!({"chunks": extracted.units.len(), "texts": hashes.len(),
        "embedded": stored.values().filter(|v| !v.is_empty()).count(),
        "refused": stored.values().filter(|v| v.is_empty()).count(), "stop": stop});
    // Texts stored for another generation are referenced, never embedded again.
    let reused: Vec<([u8; 32], Vec<u8>)> = stored.into_iter().collect();
    commit(&mut conn, &reused, "building", &counts)?;
    let known: std::collections::HashSet<[u8; 32]> = reused.into_iter().map(|(h, _)| h).collect();
    let missing: Vec<usize> = (0..hashes.len())
        .filter(|&i| !known.contains(&hashes[i]))
        .collect();
    stage(&conn, "embed")?;
    let mut embedded_now = 0;
    if !missing.is_empty() {
        profile
            .unchanged(&never)
            .map_err(|e| fail(format!("{e}; run models configure again")))?;
        let (hello, mut embed) = load(&profile)?;
        if hello != profile.contract {
            return Err(fail(
                "the runtime's contract changed since configure; run models configure again",
            ));
        }
        for batch in missing.chunks(semantic::MAX_BATCH) {
            // Before the model spends time on a batch nobody can publish.
            fence(&conn)?;
            let texts: Vec<String> = batch.iter().map(|&i| extracted.texts[i].clone()).collect();
            let got = match embed(&texts) {
                Ok(v) if v.len() == texts.len() => v,
                Ok(_) => return Err(fail("the model returned the wrong number of vectors")),
                // A refused batch is retried one text at a time, so only a text the model
                // refuses (one over its token limit) is stored as uncovered; any other error
                // fails the job, which a later embed retries.
                Err(e) if e.is::<semantic::Refused>() => {
                    let mut one = Vec::with_capacity(texts.len());
                    for t in &texts {
                        one.push(match embed(std::slice::from_ref(t)) {
                            Ok(mut v) if v.len() == 1 => v.remove(0),
                            Ok(_) => {
                                return Err(fail("the model returned the wrong number of vectors"));
                            }
                            Err(e) if e.is::<semantic::Refused>() => Vec::new(),
                            Err(e) => return Err(e),
                        });
                    }
                    one
                }
                Err(e) => return Err(e),
            };
            let mut rows = Vec::with_capacity(batch.len());
            for (&i, v) in batch.iter().zip(got) {
                if v.is_empty() {
                    counts["refused"] = json!(counts["refused"].as_u64().unwrap_or(0) + 1);
                } else {
                    semantic::check_vector(&v, profile.dim)?;
                    counts["embedded"] = json!(counts["embedded"].as_u64().unwrap_or(0) + 1);
                    embedded_now += 1;
                }
                rows.push((hashes[i], semantic::encode(&v)));
            }
            commit(&mut conn, &rows, "building", &counts)?;
        }
    }
    let state = if stop.is_some() { "partial" } else { "ready" };
    commit(&mut conn, &[], state, &counts)?;
    Ok(json!({"gen": gen_id, "layer": counts, "embeddedNow": embedded_now}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // The part of the jobs table schema 9 changes.
        conn.execute_batch(
            "CREATE TABLE jobs (kind TEXT, source_id TEXT, scope TEXT, state TEXT, expected_gen INTEGER);
             CREATE INDEX jobs_active ON jobs (source_id, scope);",
        )
        .unwrap();
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
