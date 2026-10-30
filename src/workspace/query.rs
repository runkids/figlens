//! Search over a workspace's published indexes: every visible source with a published
//! generation of exactly the requested scope, whose file still has the verified size and time,
//! ranked together. Search reads only; it never builds, so a source without a ready index is
//! reported with the action that prepares it.

use super::*;
use crate::search::{self, Index, Part, Request};
use crate::semantic::{Mode, Profile, profile};
use std::sync::atomic::{AtomicBool, Ordering};

/// The supported query envelope: ready sources one workspace search reads together, and the
/// records of their generations in total. Checked from stored counts before any is loaded.
pub(super) const QUERY_SOURCES: usize = 10;
const QUERY_RECORDS: usize = 100_000;

use super::readiness::{Row, State, rows};

/// Searches `req.scope` across the workspace's visible sources (or the `only` ones), ranked as
/// one corpus. Sources that are not ready are listed with their state and next step; stale
/// indexes are not served. Every request re-resolves the workspace's location and binding.
pub fn search(storage: &Storage, select: &Select, only: &[String], req: &Request) -> Result<Json> {
    search_with(storage, select, only, req, None)
}

/// `search` in `mode`: semantic and hybrid read each source's stored model layer of the active
/// profile (an embed job writes it) and never embed design text here; only the query is
/// embedded, by the warm local runtime.
pub fn search_mode(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
    mode: Mode,
    cancel: &AtomicBool,
) -> Result<Json> {
    search_index(storage, select, only, req, Some(cancel), mode)
        .map_err(|e| with_fallback(storage, select, req, e))
}

/// `search`, ended with `Cancelled` once `cancel` is set: SQLite is interrupted in its
/// statement and the loading and ranking loops stop, so a runner can end a search in progress.
pub(crate) fn search_with(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
    cancel: Option<&AtomicBool>,
) -> Result<Json> {
    search_index(storage, select, only, req, cancel, Mode::Text)
        .map_err(|e| with_fallback(storage, select, req, e))
}

/// Runs `work` while a watcher interrupts SQLite through `handle` once `cancel` is set, and
/// returns `Cancelled` whenever it was set, whatever stopped the work. SQLite drops an interrupt
/// that finds no statement running, so the watcher repeats it until `work` returns; it ends as
/// soon as `work` returns, panics included, and is joined before this returns.
pub(crate) fn interruptible<T>(
    handle: rusqlite::InterruptHandle,
    cancel: &AtomicBool,
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let cancelled = || {
        cancel
            .load(Ordering::SeqCst)
            .then(|| FigError::new(ErrorKind::Cancelled, "search cancelled").into())
    };
    if let Some(e) = cancelled() {
        return Err(e);
    }
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let out = std::thread::scope(|s| {
        s.spawn(move || {
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                wait.recv_timeout(Duration::from_millis(10))
            {
                if cancel.load(Ordering::SeqCst) {
                    handle.interrupt();
                }
            }
        });
        let out = work();
        drop(done);
        out
    });
    cancelled().map_or(out, Err)
}

/// A workspace that cannot be used now still leaves the named `.fig` searchable directly; that
/// is listed only when the request named a file this caller may read and it is readable now.
fn with_fallback(
    storage: &Storage,
    select: &Select,
    req: &Request,
    e: Box<dyn Error>,
) -> Box<dyn Error> {
    let Select::Locator(Locator::Source(file)) = select else {
        return e;
    };
    let fig = match e.downcast::<FigError>() {
        Ok(fig) => *fig,
        Err(e) => return e,
    };
    let unusable = matches!(
        fig.kind(),
        ErrorKind::StorageUnavailable
            | ErrorKind::PermissionDenied
            | ErrorKind::WorkspaceBusy
            | ErrorKind::IndexIncompatible
            | ErrorKind::OperationFailed
    );
    if !unusable || !storage.sees(file) || readable(file).is_err() {
        return fig.into();
    }
    let mut actions = fig.recovery().cloned().unwrap_or_else(|| json!([]));
    let arguments = selectors(json!({"file": file, "query": req.query}), req);
    if let Some(list) = actions.as_array_mut() {
        list.push(
            json!({"action": "stateless_search", "requires": "none", "tool": "search",
            "arguments": arguments,
            "effects": {"note": "parses the .fig in this call; the workspace stays as it is"}}),
        );
    }
    fig.with_recovery(actions).into()
}

/// `arguments` plus the request's scope and filters, only those it gave: a default is not
/// repeated as if it were chosen. The cursor is left out; it binds the original search.
fn selectors(mut arguments: Json, req: &Request) -> Json {
    if req.scope != "0:0" {
        arguments["node"] = json!(req.scope);
    }
    if !req.types.is_empty() {
        arguments["type"] = json!(req.types.join(","));
    }
    if req.include_hidden {
        arguments["includeHidden"] = json!(true);
    }
    if req.include_internal {
        arguments["includeInternal"] = json!(true);
    }
    if req.limit != 20 {
        arguments["limit"] = json!(req.limit);
    }
    arguments
}

fn search_index(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
    cancel: Option<&AtomicBool>,
    mode: Mode,
) -> Result<Json> {
    let q = req.validate()?;
    let (_, ws) = opened(storage, select, false)?;
    match cancel {
        None => searched(storage, select, only, req, q, ws, &search::NEVER, mode),
        Some(flag) => interruptible(ws.conn.get_interrupt_handle(), flag, || {
            searched(storage, select, only, req, q, ws, flag, mode)
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn searched(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
    q: search::Prepared,
    ws: Opened,
    cancel: &AtomicBool,
    mode: Mode,
) -> Result<Json> {
    // Semantic needs a usable profile; hybrid answers as text without one and says why.
    let (profile, mut downgrade) = match (mode, profile::active(storage)) {
        (Mode::Text, _) => (None, None),
        (_, Ok(Some(p))) => (Some(p), None),
        (Mode::Semantic, Ok(None)) => {
            return Err(FigError::new(
                ErrorKind::ModeUnavailable,
                "semantic search needs a configured local model; run models status for setup",
            )
            .with_recovery(json!([{"action": "models_status", "tool": "models",
                "arguments": {"action": "status"}, "requires": "none"}]))
            .into());
        }
        (Mode::Semantic, Err(e)) => return Err(e),
        (_, Ok(None)) => (None, Some(json!({"code": "no_profile"}))),
        (_, Err(e)) => (
            None,
            Some(json!({"code": "profile_unreadable", "message": e.to_string()})),
        ),
    };
    let stop = || cancel.load(Ordering::Relaxed);
    let ws_id = ws.held.id.clone();
    let (incarnation, version) = (ws.incarnation.clone(), ws.version);
    let dir = ws.held.dir.clone();
    if ws.version < 2 {
        return Ok(not_ready(
            &ws_id,
            req,
            Vec::new(),
            vec![prepare_step(&ws_id, req)],
        ));
    }
    // One read transaction: every source and generation comes from the same commit.
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let rows = rows(&tx, &req.scope)?;
    relocated(
        storage,
        select,
        rows.iter().map(|r| r.path.as_str()),
        "search",
        selectors(json!({"query": req.query}), req),
    )?;
    // Invisible sources are dropped before anything about them is read or counted; the reply
    // says only that the corpus was restricted, never which sources or how many.
    let all = rows.len();
    let rows: Vec<Row> = rows
        .into_iter()
        .filter(|r| storage.sees(Path::new(&r.path)))
        .collect();
    let restricted = rows.len() < all && only.is_empty();
    let rows: Vec<Row> = rows
        .into_iter()
        .filter(|r| only.is_empty() || only.contains(&r.id))
        .collect();
    if let Some(unknown) = only.iter().find(|o| !rows.iter().any(|r| r.id == **o)) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {ws_id} has no source {unknown}"),
        ));
    }
    let versions = search::store::versions();
    let now = now_ms();
    if q.cursor_expired(now) {
        return Err(FigError::new(
            ErrorKind::CursorExpired,
            "this search cursor's lease ended; search again without cursor",
        )
        .into());
    }
    let mut states = Vec::new();
    let mut loaded: Vec<(Index, Json, i64, Json)> = Vec::new();
    let mut steps = Vec::new();
    let (mut sources_read, mut records_read) = (0usize, 0usize);
    // Record text copied by every generation and ancestor load of this query.
    let mut spent = 0usize;
    for row in &rows {
        if stop() {
            return Err(fail(ErrorKind::Cancelled, "search cancelled"));
        }
        let mut state = json!({"sourceId": row.id, "path": row.path});
        let checked = row.check(&tx, &dir, &req.scope, &versions, true)?;
        let job = checked.job;
        if let Some(job) = &job {
            state["job"] = job.clone();
        }
        let refresh = json!({"tool": "index", "arguments": {"action": "refresh",
            "workspaceId": ws_id, "sourceId": row.id, "scope": req.scope}});
        let label = checked.state.label();
        match checked.state {
            State::Pending => steps.push(
                json!({"tool": "jobs", "arguments": {"action": "status", "jobId": job.as_ref().expect("pending job")["jobId"]}}),
            ),
            State::StaleMembership | State::StaleStamp => steps.push(refresh.clone()),
            State::NotPrepared => {
                steps.push(json!({"tool": "index", "arguments": {"action": "refresh",
                    "workspaceId": ws_id, "sourceId": row.id, "scope": req.scope},
                    "why": "this scope is not indexed for this source yet"}));
                state["reason"] = json!("scope_not_indexed");
            }
            State::Incompatible => steps.push(
                json!({"tool": "index", "arguments": {"action": "rebuild", "workspaceId": ws_id}}),
            ),
            State::Unreadable | State::Ready => {}
        }
        state["state"] = json!(label);
        if let Some(snapshot) = &row.snapshot {
            state["snapshotId"] = json!(snapshot);
            state["freshness"] = json!({"verifiedAtMs": row.verified_ms, "stampCheckedAtMs": now,
                "basis": "size and modification time equal to the verified snapshot"});
        }
        if label == "ready" {
            let gen_id = row.gen_id.expect("ready");
            let records: i64 = tx
                .query_row(
                    "SELECT records FROM generations WHERE gen = ?1",
                    [gen_id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            sources_read += 1;
            records_read =
                records_read.saturating_add(usize::try_from(records).unwrap_or(usize::MAX));
            if sources_read > QUERY_SOURCES || records_read > QUERY_RECORDS {
                return Err(work_limit(
                    &ws_id,
                    req,
                    &rows,
                    records as usize > QUERY_RECORDS,
                ));
            }
            let index = search::store::load_with(&tx, gen_id, q.terms(), &mut spent, cancel)
                .map_err(|e| unreadable_index(&ws_id, &row.id, e))?;
            let identity = json!({"sourceId": row.id, "membership": row.membership,
                "snapshotId": row.snapshot, "gen": gen_id, "versions": versions});
            loaded.push((
                index,
                json!({"sourceId": row.id, "snapshotId": row.snapshot}),
                gen_id,
                identity,
            ));
        }
        states.push(state);
    }
    let used: Vec<i64> = loaded.iter().map(|l| l.2).collect();
    if let Some(gens) = q.cursor_gens()
        && gens != used.as_slice()
    {
        let gone = gens.iter().any(|g| {
            tx.query_row("SELECT 1 FROM generations WHERE gen = ?1", [g], |_| Ok(()))
                .optional()
                .ok()
                .flatten()
                .is_none()
        });
        let (kind, why) = if gone {
            (
                ErrorKind::CursorExpired,
                "an index this cursor paged was deleted",
            )
        } else {
            (
                ErrorKind::CursorStale,
                "a source this cursor paged was reindexed or left the search",
            )
        };
        return Err(FigError::new(kind, format!("{why}; search again without cursor")).into());
    }
    if loaded.is_empty() {
        if states.iter().any(|s| s["state"] == "stale") {
            return Err(FigError::new(
                ErrorKind::SourceStale,
                "every searched source changed since it was indexed; refresh it, then search again",
            )
            .with_recovery(json!(
                steps
                    .iter()
                    .map(|s| {
                        let mut s = s.clone();
                        s["requires"] = json!("none");
                        s["action"] = json!("refresh");
                        s
                    })
                    .collect::<Vec<_>>()
            ))
            .into());
        }
        if !states.is_empty() && states.iter().all(|s| s["state"] == "unreadable") {
            return Err(fail(
                ErrorKind::SourceUnreadable,
                "no searched source can be read now; restore access to it, then search again",
            ));
        }
        if steps.is_empty() {
            steps.push(prepare_step(&ws_id, req));
        }
        let mut out = not_ready(&ws_id, req, states, steps);
        if restricted {
            restrict(&mut out["coverage"]);
        }
        return Ok(out);
    }
    let key = json!(loaded.iter().map(|l| &l.3).collect::<Vec<_>>());
    let (ranked, decor) = match &profile {
        Some(p) => {
            let mut cache = crate::semantic::lock(&LAYERS, cancel)
                .map_err(|_| fail(ErrorKind::Cancelled, "search cancelled"))?;
            let ws_key = (ws_id.as_str(), incarnation.as_str(), version);
            let layer = model_layers(
                &tx, ws_key, req, &loaded, p, &mut spent, &mut cache, &mut steps, cancel,
            )?;
            let sides: Vec<search::semantic::Side> = loaded
                .iter()
                .map(|l| {
                    let key = layer_key(&incarnation, l.2);
                    let (_, _, index, layer) = cache
                        .iter()
                        .find(|c| c.0 == key && c.1 == p.id)
                        .expect("loaded above");
                    search::semantic::Side { index, layer }
                })
                .collect();
            match search::semantic::workspace_rank(
                &parts(&loaded),
                &sides,
                mode,
                p,
                layer,
                &q,
                req,
                &key,
                cancel,
            ) {
                Ok((ranked, decor)) => (ranked, Some(decor)),
                // A model that cannot run leaves hybrid's text ranking working.
                Err(e)
                    if mode == Mode::Hybrid
                        && e.downcast_ref::<FigError>().map(FigError::kind)
                            == Some(ErrorKind::OperationFailed) =>
                {
                    downgrade = Some(json!({"code": "runtime_failed", "message": e.to_string()}));
                    (
                        search::corpus_rank(&parts(&loaded), &q, req, &key, cancel)?,
                        None,
                    )
                }
                Err(e) => return Err(e),
            }
        }
        None => (
            search::corpus_rank(&parts(&loaded), &q, req, &key, cancel)?,
            None,
        ),
    };
    // Only the shown page reads its records' ancestors, one tree level per query.
    let mut wanted: Vec<Vec<u32>> = vec![Vec::new(); loaded.len()];
    for (part, rid) in ranked.shown() {
        wanted[part].push(rid);
    }
    for ((index, _, gen_id, identity), mut rids) in loaded.iter_mut().zip(wanted) {
        while !rids.is_empty() {
            search::store::add_with(&tx, *gen_id, index, &rids, &mut spent, cancel).map_err(
                |e| unreadable_index(&ws_id, identity["sourceId"].as_str().unwrap_or(""), e),
            )?;
            rids = rids
                .iter()
                .filter_map(|r| search::parent(index, *r))
                .collect();
            rids.sort_unstable();
            rids.dedup();
        }
    }
    let mut out = search::corpus_page(&parts(&loaded), &q, req, ranked, now)?;
    match (decor, downgrade) {
        (Some(decor), _) => search::semantic::decorate(&mut out, mode, decor),
        (None, Some(reason)) => {
            out["query"]["requestedMode"] = json!(mode.as_str());
            out["query"]["usedModes"] = json!(["text"]);
            out["coverage"]["layers"]["semantic"] =
                json!({"state": "unavailable", "reasons": [reason]});
            // The next page could rank with the model again, so this text page is not replayable.
            out["coverage"]["complete"] = json!(false);
            out["nextCursor"] = Json::Null;
        }
        (None, None) => {}
    }
    let left_out: Vec<&Json> = states.iter().filter(|s| s["state"] != "ready").collect();
    if !left_out.is_empty() {
        let coverage = &mut out["coverage"];
        coverage["complete"] = json!(false);
        coverage["totalRecords"] = Json::Null;
        coverage["layers"]["text"]["state"] = json!("partial");
        coverage["layers"]["relations"] = search::relations("partial");
        if let Some(reasons) = coverage["reasons"].as_array_mut() {
            reasons.push(
                json!({"code": "sources_not_ready", "counts": {"sources": left_out.len()},
                "sourceIds": left_out.iter().map(|s| &s["sourceId"]).collect::<Vec<_>>()}),
            );
        }
    }
    if restricted {
        restrict(&mut out["coverage"]);
    }
    out["workspaceId"] = json!(ws_id);
    out["state"] = json!("ready");
    out["sources"] = json!(states);
    out["nextSteps"] = json!(steps);
    Ok(out)
}

/// A `--file` whose workspace no longer holds that path (its source was relocated) must not
/// serve the moved source under the old name; it names where the source went when visible.
/// Refuses a `--file` that no longer names this workspace's source; the recovery reruns `tool`
/// with `arguments` on the file it was relocated to.
pub(super) fn relocated<'a>(
    storage: &Storage,
    select: &Select,
    mut paths: impl Iterator<Item = &'a str> + Clone,
    tool: &str,
    mut arguments: Json,
) -> Result<()> {
    let Select::Locator(Locator::Source(asked)) = select else {
        return Ok(());
    };
    let asked = utf8(asked)?;
    if paths.clone().any(|p| p == asked) {
        return Ok(());
    }
    match paths.find(|p| storage.sees(Path::new(p))) {
        Some(now) => {
            arguments["file"] = json!(now);
            Err(FigError::new(
                ErrorKind::InvalidArguments,
                format!("{asked} is no longer this workspace's source: it was relocated to {now}; {tool} that file"),
            )
            .with_recovery(json!([{"action": format!("{tool}_relocated"), "requires": "none",
                "tool": tool, "arguments": arguments}]))
            .into())
        }
        None => Ok(()),
    }
}

/// Marks a reply whose corpus left out sources this caller may not see, without naming them.
fn restrict(coverage: &mut Json) {
    coverage["complete"] = json!(false);
    coverage["totalRecords"] = Json::Null;
    if let Some(reasons) = coverage["reasons"].as_array_mut() {
        reasons.push(json!({"code": "authorization_restricted"}));
    }
}

/// The search exceeds the query envelope: the registry and indexes stay as they are, and each
/// visible source can be searched alone; a single source over the envelope needs a narrower
/// indexed scope, a node the caller picks inside the current one.
fn work_limit(ws_id: &str, req: &Request, rows: &[Row], one_too_big: bool) -> Box<dyn Error> {
    let mut actions: Vec<Json> = rows
        .iter()
        .map(|r| {
            json!({"action": "search_one_source", "requires": "none", "tool": "search",
            "arguments": selectors(json!({"workspaceId": ws_id, "query": req.query, "sourceId": r.id}), req)})
        })
        .collect();
    if one_too_big {
        actions.push(json!({"action": "index_narrower_scope", "requires": "node_choice", "tool": "index",
            "arguments": {"action": "refresh", "workspaceId": ws_id, "scope": "<a node inside the current scope>"}}));
    }
    FigError::new(
        ErrorKind::QueryWorkLimit,
        format!("this search would read more than {QUERY_SOURCES} sources or {QUERY_RECORDS} indexed records; search fewer sources or a narrower scope"),
    )
    .with_recovery(json!(actions))
    .into()
}

/// A stored index this binary cannot read: rebuilding it is the recovery. A query budget
/// crossed while loading is passed on as it is: the index is fine.
pub(super) fn unreadable_index(ws_id: &str, source: &str, e: Box<dyn Error>) -> Box<dyn Error> {
    if e.downcast_ref::<FigError>()
        .is_some_and(|f| f.kind() == ErrorKind::QueryWorkLimit)
    {
        return e;
    }
    FigError::new(
        ErrorKind::IndexIncompatible,
        format!("the index of {source} cannot be read: {e}"),
    )
    .with_recovery(
        json!([{"action": "rebuild", "requires": "none", "tool": "index",
        "arguments": {"action": "rebuild", "workspaceId": ws_id}}]),
    )
    .into()
}

fn parts(loaded: &[(Index, Json, i64, Json)]) -> Vec<Part<'_>> {
    loaded
        .iter()
        .map(|(index, source, gen_id, _)| Part {
            index,
            source: source.clone(),
            gen_id: *gen_id,
        })
        .collect()
}

fn prepare_step(ws_id: &str, req: &Request) -> Json {
    json!({"tool": "index", "arguments": {"action": "refresh", "workspaceId": ws_id, "scope": req.scope}})
}

/// A search with no ready source: a structured state with the next step, never an empty result
/// set that reads as "nothing matched".
fn not_ready(ws_id: &str, req: &Request, states: Vec<Json>, steps: Vec<Json>) -> Json {
    let pending = states.iter().any(|s| s["state"] == "pending");
    json!({
        "workspaceId": ws_id,
        "state": if pending { "pending" } else { "not_prepared" },
        "query": {"text": req.query},
        "scope": {"nodeId": req.scope},
        "results": [],
        "coverage": {"complete": false, "scannedRecords": 0, "totalRecords": null,
            "layers": {"text": {"state": if pending { "pending" } else { "not_prepared" }},
                "relations": search::relations(if pending { "pending" } else { "not_prepared" })},
            "reasons": [{"code": if pending { "index_pending" } else { "scope_not_indexed" }}]},
        "hasMore": false,
        "nextCursor": null,
        "sources": states,
        "nextSteps": steps,
    })
}

/// Fully loaded generations and their model layers, by database incarnation, generation,
/// policy and profile (generation numbers repeat across workspaces), kept for the last
/// queried corpus only.
type Layers = Vec<(String, String, Index, search::semantic::Layer)>;
static LAYERS: std::sync::Mutex<Layers> = std::sync::Mutex::new(Vec::new());

/// The cache key of generation `gen_id` of the database `incarnation`.
fn layer_key(incarnation: &str, gen_id: i64) -> String {
    format!("{incarnation}/{gen_id}/{}", super::model::POLICY)
}

/// Makes sure `cache` holds every loaded source's full generation and layer under `p`, reads
/// the vectors its layer still lacks in batches, and returns the sources' combined
/// `coverage.layers.semantic`. Extraction shares one budget across the sources. A source whose
/// layer is not ready adds an embed step; a schema before 9 has no stored layer to read.
#[allow(clippy::too_many_arguments)]
fn model_layers(
    tx: &rusqlite::Transaction,
    ws: (&str, &str, i64),
    req: &Request,
    loaded: &[(Index, Json, i64, Json)],
    p: &Profile,
    spent: &mut usize,
    cache: &mut Layers,
    steps: &mut Vec<Json>,
    cancel: &AtomicBool,
) -> Result<Json> {
    let (ws_id, incarnation, version) = ws;
    let mut bounds = crate::expand::Bounds {
        work: search::semantic::LAYER_WORK,
        bytes: search::semantic::LAYER_BYTES,
        depth: 0,
        deadline: std::time::Instant::now() + Duration::from_secs(15),
        stop: None,
    };
    // Only this corpus stays cached: past corpora are dropped, and the layers kept are charged
    // to this query's budget first, so warming sources one by one cannot exceed it together.
    let wanted: Vec<String> = loaded.iter().map(|l| layer_key(incarnation, l.2)).collect();
    cache.retain(|c| c.1 == p.id && c.3.stop.is_none() && wanted.contains(&c.0));
    for c in cache.iter() {
        if !bounds.charge(0, search::semantic::cached_bytes(&c.2, &c.3)) {
            return Err(fail(
                ErrorKind::QueryWorkLimit,
                "the semantic layers of these sources exceed one search's memory budget; search fewer sources",
            ));
        }
    }
    let mut sources = Vec::new();
    let mut ready = true;
    for (_, source, gen_id, _) in loaded {
        let key = layer_key(incarnation, *gen_id);
        // A layer cut short by the shared budget is a miss: it is extracted again.
        let at = match cache
            .iter()
            .position(|c| c.0 == key && c.1 == p.id && c.3.stop.is_none())
        {
            Some(at) => at,
            None => {
                cache.retain(|c| c.0 != key || c.1 != p.id);
                let records: i64 = tx
                    .query_row(
                        "SELECT records FROM generations WHERE gen = ?1",
                        [gen_id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                let mut index = search::store::load_with(tx, *gen_id, &[], spent, cancel)?;
                let all: Vec<u32> = (0..records.max(0) as u32).collect();
                search::store::add_with(tx, *gen_id, &mut index, &all, spent, cancel)?;
                let layer = search::semantic::Layer::new(
                    &index,
                    json!([key, p.id]),
                    p.dim,
                    &mut bounds,
                    cancel,
                )
                .map_err(|_| fail(ErrorKind::Cancelled, "search cancelled"))?;
                cache.push((key, p.id.clone(), index, layer));
                cache.len() - 1
            }
        };
        let layer = &mut cache[at].3;
        let built = if version >= 9 {
            let missing: Vec<usize> = (0..layer.texts.len())
                .filter(|&i| layer.vectors[i].is_none())
                .collect();
            let hashes: Vec<[u8; 32]> = missing
                .iter()
                .map(|&i| super::model::text_hash(&layer.texts[i]))
                .collect();
            let stored = super::model::vectors(tx, &p.id, &hashes).map_err(sql)?;
            for (&i, hash) in missing.iter().zip(&hashes) {
                if let Some(bytes) = stored.get(hash) {
                    layer.vectors[i] = Some(match bytes.is_empty() {
                        true => Vec::new(),
                        false => crate::semantic::decode(bytes, p.dim)?,
                    });
                }
            }
            super::model::layer(tx, *gen_id, &p.id).map_err(sql)?
        } else {
            None
        };
        let mut cov = search::semantic::layer_coverage(layer, &p.id, [0, 0]);
        let state = match built.as_ref().map(|b| b["state"].as_str().unwrap_or("")) {
            None => "pending",
            Some("ready") if cov["state"] == "ready" => "ready",
            Some(_) => "partial",
        };
        if state != "ready" {
            let why = match state {
                "pending" => "this source's model layer is not built for the configured profile",
                _ => "this source's model layer is unfinished; embed again to complete it",
            };
            steps.push(
                json!({"tool": "index", "arguments": {"action": "embed", "workspaceId": ws_id,
                "sourceId": source["sourceId"], "scope": req.scope}, "why": why}),
            );
        }
        ready &= state == "ready";
        cov["state"] = json!(state);
        cov["sourceId"] = source["sourceId"].clone();
        cov["job"] = built.map_or(Json::Null, |b| b["jobId"].clone());
        sources.push(cov);
    }
    Ok(
        json!({"state": if ready { "ready" } else if sources.iter().any(|s| s["state"] != "pending") { "partial" } else { "pending" },
        "profileId": p.id, "policy": super::model::POLICY, "inputs": ["name", "text"], "sources": sources}),
    )
}
