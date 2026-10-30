//! The visual preparation job (#95): renders one published generation's units into
//! `visual_units`/`visual_rasters` and records its hash layer; with a vision profile it embeds
//! the stored rasters too, in the same worker process (the shipped `figlens-embed`). Only a
//! `visual` job writes these layers; queries read them.

use super::model::{self, set_layer};
use super::visual::{self as cache, POLICY};
use crate::search::visual::{self, QueryImage};
use crate::semantic::{self, Profile, profile};
use crate::{ErrorKind, FigError};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value as Json, json};
use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// One batch of normalized rasters to vectors, in order, in the worker's own process.
pub type EmbedImages = Box<dyn FnMut(&[QueryImage]) -> Result<Vec<Vec<f32>>>>;

/// Units rendered per fenced commit.
const RENDER_BATCH: usize = 8;
/// One job's render budget: wall time, and pixels rendered before normalization (about a
/// thousand full-size 1024-pixel renders). Reaching either leaves the layer `partial`.
const RENDER_TIME: std::time::Duration = std::time::Duration::from_secs(600);
const RENDER_PIXELS: u64 = 1 << 30;

/// A unit a visual job renders: hidden ones and ones on internal pages only as the scope root.
fn renderable(u: &&visual::Unit<'_>) -> bool {
    u.rid == 0 || !(u.hidden || u.internal)
}

/// Whether the job-wide render budget is used up after `elapsed` and `pixels` rendered.
fn spent(elapsed: std::time::Duration, pixels: u64) -> bool {
    elapsed > RENDER_TIME || pixels >= RENDER_PIXELS
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

/// The hidden `__visual-worker <workspace dir> <jobId>`: runs one visual job's attempt in this
/// process and returns the JSON line it prints. `load` reads the vision model of the profile it
/// is given, after its executable and pinned files were verified again, and returns the
/// runtime's contract (its hello) with the embedder; it is called only for a vision job with
/// some raster that has no stored vector yet. A hash-only job never calls it.
pub fn run_visual_worker(
    dir: &Path,
    job_id: &str,
    load: impl FnOnce(&Profile) -> Result<(Json, EmbedImages)>,
) -> Json {
    super::jobs::worker_with(dir, job_id, |dir, job_id, me, attempt| {
        visual_work(dir, job_id, me, attempt, load)
    })
}

/// `render_unit`; test builds fail the job's `first` unit while `FIGLENS_TEST_RENDER_FAIL` is
/// set, so process tests reach per-unit coverage deterministically.
fn render(tree: &crate::Tree, bytes: &[u8], id: &str, first: bool) -> Result<visual::RenderedUnit> {
    if cfg!(debug_assertions) && first && std::env::var_os("FIGLENS_TEST_RENDER_FAIL").is_some() {
        return Err("unsupported node (test)".into());
    }
    visual::render_unit(tree, bytes, id)
}

/// One stored unit: record id, raster key (none for a render error), raster of this renderer.
type Stored = (u32, Option<[u8; 32]>, bool);

/// Every stored unit of `gen_id`: record id, raster key (none for a render error) and whether
/// its raster came from this renderer.
fn stored_units(conn: &Connection, gen_id: i64) -> Result<Vec<Stored>> {
    let mut stmt = conn
        .prepare(
            "SELECT rid, raster_key, json_extract(report, '$.renderer') IS ?3 FROM visual_units
             WHERE gen = ?1 AND policy = ?2 ORDER BY rid",
        )
        .map_err(sql)?;
    stmt.query_map(params![gen_id, POLICY, cache::RENDERER], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .map_err(sql)?
    .collect::<rusqlite::Result<_>>()
    .map_err(sql)
}

/// Renders attempt `attempt` of visual job `job_id` for owner `me`. Every batch commits its
/// units and the layer's `building` progress in one transaction, only while the fence holds.
/// The hash layer is published `ready` or `partial` once rendering ends; a vision job then
/// embeds, and its last write marks the vision layer and the job succeeded together. A crash
/// leaves a layer `building`; a job that ends failed or cancelled marks it `failed`.
fn visual_work(
    dir: &Path,
    job_id: &str,
    me: &str,
    attempt: i64,
    load: impl FnOnce(&Profile) -> Result<(Json, EmbedImages)>,
) -> Result<Json> {
    let (_lock, mut conn, ceiling) = super::jobs::worker_conn(dir, job_id)?;
    let full = |e: Box<dyn Error>| super::jobs::full(e, ceiling);
    type Row = (String, Option<i64>, Option<String>, String, String, String);
    let row: Option<Row> = conn
        .query_row(
            "SELECT incarnation, expected_gen, profile_id, source_id, membership, scope FROM jobs
             WHERE job_id = ?1 AND owner = ?2 AND attempt = ?3 AND state = 'running'
             AND kind = 'visual'",
            params![job_id, me, attempt],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let (incarnation, Some(gen_id), Some(profile_id), source_id, membership, scope) =
        row.ok_or_else(|| fail("another attempt owns this job now"))?
    else {
        return Err(fail("the visual job names no generation or profile"));
    };
    if super::meta(&conn, "incarnation").map_err(sql)?.as_deref() != Some(incarnation.as_str()) {
        return Err(fail("the workspace was recreated"));
    }
    let vision = profile_id != visual::VERSION;
    let storage = crate::workspace::storage(None)?;
    // The job still owns this launch and no cancel landed; the source keeps its membership; its
    // scope still publishes this generation; and a vision job's profile is still the active one.
    let fence = |conn: &Connection| -> Result<()> {
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
                "superseded: the job was cancelled or taken over, or its scope published another generation or changed source; run index visual again",
            ));
        }
        if vision && profile::active_visual(&storage)?.is_none_or(|p| p.id != profile_id) {
            return Err(fail(
                "superseded: the configured vision profile changed while the layer was prepared",
            ));
        }
        Ok(())
    };
    let stage = |conn: &Connection, stage: &str| -> Result<()> {
        conn.execute(
            "UPDATE jobs SET stage = ?2 WHERE job_id = ?1 AND owner = ?3 AND attempt = ?4
             AND state = 'running'",
            params![job_id, stage, me, attempt],
        )
        .map_err(sql)?;
        super::jobs::hold(stage);
        Ok(())
    };
    // `write` and the job's progress in one transaction under the fence; `finished` also marks
    // the job succeeded.
    let commit = |conn: &mut Connection,
                  write: &mut dyn FnMut(&Connection) -> Result<()>,
                  done: usize,
                  total: usize,
                  finished: bool|
     -> Result<()> {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
        fence(&tx)?;
        write(&tx).map_err(full)?;
        tx.execute(
            "UPDATE jobs SET done = ?2, total = ?3,
             state = CASE WHEN ?4 THEN 'succeeded' ELSE state END,
             stage = CASE WHEN ?4 THEN 'publish' ELSE stage END,
             gen = CASE WHEN ?4 THEN ?5 ELSE gen END,
             finished_ms = CASE WHEN ?4 THEN ?6 ELSE finished_ms END
             WHERE job_id = ?1",
            params![
                job_id,
                done as i64,
                total as i64,
                finished,
                gen_id,
                now_ms()
            ],
        )
        .map_err(sql)?;
        tx.commit().map_err(|e| full(sql(e)))?;
        // The same WAL backpressure as a text build: a reader cannot pin an ever-growing log.
        super::jobs::reclaim_wal(conn, dir)
    };
    let set = |conn: &Connection, profile: &str, state: &str, counts: &Json| -> Result<()> {
        set_layer(
            conn,
            gen_id,
            profile,
            POLICY,
            state,
            counts,
            job_id,
            now_ms(),
        )
        .map_err(sql)
    };
    let state = |conn: &Connection, profile: &str| -> Result<Option<String>> {
        super::visual_search::layer_state(conn, dir, gen_id, profile)
    };
    // A finished hash layer of this renderer is never written again, so hash search keeps
    // reading it and a later failure of this job cannot mark it failed.
    let hash_done = matches!(
        state(&conn, visual::VERSION)?.as_deref(),
        Some("ready" | "partial")
    );
    if state(&conn, visual::VERSION)?.as_deref() == Some("ready")
        && (!vision || state(&conn, &profile_id)?.as_deref() == Some("ready"))
    {
        commit(&mut conn, &mut |_| Ok(()), 0, 0, true)?;
        return Ok(json!({"gen": gen_id, "renderedNow": 0, "embeddedNow": 0}));
    }
    let (counts, hash_state, selection, rendered_now) = if hash_done {
        let row: (i64, i64, i64, Option<String>, String) = conn
            .query_row(
                "SELECT chunks, embedded, refused, stop, state FROM model_layers WHERE gen = ?1
                 AND profile_id = ?2 AND policy = ?3",
                params![gen_id, visual::VERSION, POLICY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(sql)?;
        let counts = json!({"chunks": row.0, "texts": row.0, "embedded": row.1,
            "refused": row.2, "stop": row.3});
        (counts, row.4, None, 0)
    } else {
        // Render: the registered source's bytes, which must be the published generation's.
        let path: String = conn
            .query_row(
                "SELECT path FROM sources WHERE source_id = ?1 AND membership = ?2",
                [&source_id, &membership],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?
            .ok_or_else(|| {
                FigError::new(ErrorKind::InvalidArguments, "the source left the workspace")
            })?;
        let path = PathBuf::from(path);
        let (bytes, _) = super::storage::staged(&path)?;
        let published: String = conn
            .query_row(
                "SELECT snapshot_id FROM generations WHERE gen = ?1",
                [gen_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if crate::search::snapshot_id(&bytes) != published {
            return Err(FigError::new(
                ErrorKind::SourceStale,
                "the .fig changed since this index was published; refresh it, then run index visual again",
            )
            .into());
        }
        stage(&conn, "parse")?;
        let document = crate::Document::from_bytes(&bytes, &path.display().to_string())?;
        let message = document.message()?;
        let tree = crate::Tree::new(&message);
        let index = crate::search::index_stored(&tree, &scope, crate::search::deadline())?;
        let mut bounds = crate::expand::Bounds {
            work: crate::search::semantic::LAYER_WORK,
            bytes: crate::search::semantic::LAYER_BYTES,
            depth: 0,
            deadline: crate::search::deadline(),
            stop: None,
        };
        let selected = visual::scope_units(&index, &mut bounds)?;
        let mut stop = selected.reason;
        // Hidden units and units on internal pages are left out (they would spend the unit
        // cap), unless one is the selected scope itself.
        let mut units: Vec<(u32, String)> = selected
            .units
            .iter()
            .filter(renderable)
            .map(|u| (u.rid, u.id.to_owned()))
            .collect();
        drop(selected);
        drop(index);
        if units.len() > cache::MAX_UNITS {
            units.truncate(cache::MAX_UNITS);
            stop = stop.or(Some("visual_capacity"));
        }
        // A record id names the same node in the published generation, or this build is not its.
        for (rid, id) in &units {
            let stored: Option<String> = conn
                .query_row(
                    "SELECT node_id FROM records WHERE gen = ?1 AND rid = ?2",
                    params![gen_id, rid],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?;
            if stored.as_deref() != Some(id.as_str()) {
                return Err(FigError::new(
                    ErrorKind::IndexIncompatible,
                    "the published index does not match its source's records; rebuild it, then run index visual again",
                )
                .into());
            }
        }
        let total = units.len();
        let chosen: HashSet<u32> = units.iter().map(|u| u.0).collect();
        // This selection's rendered units (of this renderer) and render errors.
        let tally = |conn: &Connection| -> Result<(usize, usize, HashSet<u32>)> {
            let (mut rendered, mut refused, mut kept) = (0, 0, HashSet::new());
            for (rid, key, current) in stored_units(conn, gen_id)? {
                if !chosen.contains(&rid) {
                    continue;
                }
                match key {
                    Some(_) if current => {
                        rendered += 1;
                        kept.insert(rid);
                    }
                    Some(_) => {}
                    None => refused += 1,
                }
            }
            Ok((rendered, refused, kept))
        };
        let layer = |conn: &Connection, stop: Option<&str>| -> Result<(Json, &'static str)> {
            let (rendered, refused, _) = tally(conn)?;
            // A unit that did not render is uncovered, never a failed job: the rest still serve.
            let state = if refused > 0 || stop.is_some() || rendered + refused < total {
                "partial"
            } else {
                "ready"
            };
            Ok((
                json!({"chunks": total, "texts": total, "embedded": rendered,
                    "refused": refused, "stop": stop}),
                state,
            ))
        };
        // Rasters this generation already stored from this renderer are kept, never rendered again.
        let (_, _, kept) = tally(&conn)?;
        stage(&conn, "render")?;
        // One job-wide budget across units, charged before each render.
        let (began, mut pixels, mut rendered_now) = (std::time::Instant::now(), 0u64, 0);
        'render: for (n, batch) in units.chunks(RENDER_BATCH).enumerate() {
            let todo: Vec<&(u32, String)> = batch
                .iter()
                .filter(|(rid, _)| !kept.contains(rid))
                .collect();
            if todo.is_empty() {
                continue;
            }
            // Before rendering a batch nobody can publish.
            fence(&conn)?;
            let mut renders = Vec::with_capacity(todo.len());
            for (rid, id) in todo {
                if spent(began.elapsed(), pixels) {
                    stop = Some("render_budget");
                }
                if stop == Some("render_budget") {
                    break;
                }
                let render = render(&tree, &bytes, id, *rid == units[0].0);
                if let Ok(r) = &render {
                    pixels += r.pixels;
                }
                renders.push((*rid, render));
            }
            rendered_now += renders.iter().filter(|r| r.1.is_ok()).count();
            let done = (n * RENDER_BATCH + batch.len()).min(total);
            commit(
                &mut conn,
                &mut |tx| {
                    for (rid, render) in &renders {
                        // A unit that cannot render, or whose render is over its budget, is
                        // recorded as uncovered; its text record stays searchable.
                        let saved = match render {
                            Ok(r) => cache::save(tx, gen_id, *rid, r).map(|_| ()),
                            Err(e) => Err(e.to_string().into()),
                        };
                        match saved {
                            Ok(()) => {}
                            Err(e) if e.is::<rusqlite::Error>() => return Err(e),
                            Err(e) => cache::failed(tx, gen_id, *rid, &e.to_string())?,
                        }
                    }
                    let (counts, _) = layer(tx, stop)?;
                    set(tx, visual::VERSION, "building", &counts)
                },
                done,
                total,
                false,
            )?;
            if stop == Some("render_budget") {
                break 'render;
            }
        }
        drop(tree);
        drop(message);
        drop(document);
        drop(bytes);
        let (done_counts, state) = layer(&conn, stop)?;
        // Published on its own as soon as rendering ends: a vision failure after it fails only
        // the vision layer.
        commit(
            &mut conn,
            &mut |tx| set(tx, visual::VERSION, state, &done_counts),
            total,
            total,
            false,
        )?;
        (done_counts, state.to_owned(), Some(chosen), rendered_now)
    };
    if !vision {
        commit(&mut conn, &mut |_| Ok(()), 0, 0, true)?;
        return Ok(json!({"gen": gen_id, "layer": counts, "renderedNow": rendered_now}));
    }
    let total = counts["chunks"].as_u64().unwrap_or(0) as usize;
    let stop = counts["stop"].clone();

    // Embed: the stored rasters, keyed by their content address, in batches under the fence.
    stage(&conn, "load")?;
    let profile = profile::active_visual(&storage)?
        .filter(|p| p.id == profile_id)
        .ok_or_else(|| {
            fail("the configured vision profile changed since this visual job was submitted")
        })?;
    // Only this job's selection (the whole stored layer when it was already finished).
    let keys: Vec<(u32, [u8; 32])> = stored_units(&conn, gen_id)?
        .into_iter()
        .filter(|u| selection.as_ref().is_none_or(|s| s.contains(&u.0)))
        .filter_map(|(rid, key, _)| key.map(|k| (rid, k)))
        .collect();
    let mut unique: Vec<(u32, [u8; 32])> = Vec::new();
    let mut seen = HashSet::new();
    for (rid, key) in &keys {
        if seen.insert(*key) {
            unique.push((*rid, *key));
        }
    }
    let hashes: Vec<[u8; 32]> = unique.iter().map(|u| u.1).collect();
    // Identical pixels under the same profile and policy keep their vector across generations.
    let stored = model::vectors(&conn, &profile_id, &hashes).map_err(sql)?;
    let reused: Vec<([u8; 32], Vec<u8>)> = stored.into_iter().filter(|v| !v.1.is_empty()).collect();
    let mut embedded = reused.len();
    let vision_counts = |embedded: usize| {
        json!({"chunks": total, "texts": hashes.len(), "embedded": embedded,
            "refused": counts["refused"], "stop": stop})
    };
    commit(
        &mut conn,
        &mut |tx| {
            model::save(tx, gen_id, &profile_id, POLICY, &reused).map_err(sql)?;
            set(tx, &profile_id, "building", &vision_counts(embedded))
        },
        embedded,
        hashes.len(),
        false,
    )?;
    let known: HashSet<[u8; 32]> = reused.iter().map(|r| r.0).collect();
    let missing: Vec<(u32, [u8; 32])> = unique
        .into_iter()
        .filter(|u| !known.contains(&u.1))
        .collect();
    stage(&conn, "embed")?;
    let mut embedded_now = 0;
    if !missing.is_empty() {
        let never = std::sync::atomic::AtomicBool::new(false);
        profile
            .unchanged(&never)
            .map_err(|e| fail(format!("{e}; run models configure again")))?;
        let (hello, mut embed) = load(&profile)?;
        if hello != profile.contract {
            return Err(fail(
                "the runtime's contract changed since configure; run models configure again",
            ));
        }
        for batch in missing.chunks(semantic::MAX_IMAGES) {
            // Before the model spends time on a batch nobody can publish.
            fence(&conn)?;
            let mut images = Vec::with_capacity(batch.len());
            for (rid, _) in batch {
                images.push(cache::image(&conn, gen_id, *rid)?.ok_or_else(|| {
                    fail("a stored visual raster is missing; run index visual again")
                })?);
            }
            let got = embed(&images)?;
            if got.len() != images.len() {
                return Err(fail("the model returned the wrong number of vectors"));
            }
            let mut rows = Vec::with_capacity(batch.len());
            for ((_, key), v) in batch.iter().zip(got) {
                semantic::check_vector(&v, profile.dim)?;
                rows.push((*key, semantic::encode(&v)));
            }
            embedded += rows.len();
            embedded_now += rows.len();
            commit(
                &mut conn,
                &mut |tx| {
                    model::save(tx, gen_id, &profile_id, POLICY, &rows).map_err(sql)?;
                    set(tx, &profile_id, "building", &vision_counts(embedded))
                },
                embedded,
                hashes.len(),
                false,
            )?;
        }
    }
    let vision = vision_counts(embedded);
    commit(
        &mut conn,
        &mut |tx| set(tx, &profile_id, &hash_state, &vision),
        embedded,
        hashes.len(),
        true,
    )?;
    Ok(json!({"gen": gen_id, "layer": counts, "vision": vision,
        "renderedNow": rendered_now, "embeddedNow": embedded_now}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_internal_units_render_only_as_the_scope_root() {
        let unit = |rid, hidden, internal| visual::Unit {
            rid,
            id: "1:2",
            hidden,
            internal,
        };
        let kept: Vec<u32> = [
            unit(0, true, false),
            unit(1, true, false),
            unit(2, false, true),
            unit(3, false, false),
        ]
        .iter()
        .filter(renderable)
        .map(|u| u.rid)
        .collect();
        assert_eq!(kept, [0, 3]);
    }

    #[test]
    fn the_render_budget_ends_at_its_time_or_pixel_limit() {
        let minute = std::time::Duration::from_secs(60);
        assert!(!spent(minute, RENDER_PIXELS - 1));
        assert!(spent(minute, RENDER_PIXELS));
        assert!(spent(RENDER_TIME + minute, 0));
    }
}
