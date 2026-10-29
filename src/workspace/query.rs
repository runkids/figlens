//! Search over a workspace's published indexes: every visible source with a published
//! generation of exactly the requested scope, whose file still has the verified size and time,
//! ranked together. Search reads only; it never builds, so a source without a ready index is
//! reported with the action that prepares it.

use super::*;
use crate::search::{self, Index, Part, Request};

/// The supported query envelope: ready sources one workspace search reads together, and the
/// records of their generations in total. Checked from stored counts before any is loaded.
const QUERY_SOURCES: usize = 10;
const QUERY_RECORDS: usize = 100_000;

/// One registered source as the query found it.
struct Row {
    id: String,
    path: String,
    membership: String,
    gen_id: Option<i64>,
    gen_membership: Option<String>,
    snapshot: Option<String>,
    versions: Option<String>,
    bytes: Option<i64>,
    mtime_ns: Option<i64>,
    verified_ms: Option<i64>,
}

fn mtime_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Searches `req.scope` across the workspace's visible sources (or the `only` ones), ranked as
/// one corpus. Sources that are not ready are listed with their state and next step; stale
/// indexes are not served. Every request re-resolves the workspace's location and binding.
pub fn search(storage: &Storage, select: &Select, only: &[String], req: &Request) -> Result<Json> {
    search_index(storage, select, only, req).map_err(|e| with_fallback(storage, select, req, e))
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
    // Only what the request gave: the default scope is not repeated as if it were chosen.
    let mut arguments = json!({"file": file, "query": req.query});
    if req.scope != "0:0" {
        arguments["node"] = json!(req.scope);
    }
    if let Some(list) = actions.as_array_mut() {
        list.push(
            json!({"action": "stateless_search", "requires": "none", "tool": "search",
            "arguments": arguments,
            "effects": {"note": "parses the .fig in this call; the workspace stays as it is"}}),
        );
    }
    fig.with_recovery(actions).into()
}

fn search_index(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
) -> Result<Json> {
    let q = req.validate()?;
    let (_, ws) = opened(storage, select, false)?;
    let ws_id = ws.held.id.clone();
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
    let mut stmt = tx
        .prepare(
            "SELECT s.source_id, s.path, s.membership, p.gen, g.membership, g.snapshot_id,
             g.versions, n.bytes, n.mtime_ns, n.verified_ms
             FROM sources s
             LEFT JOIN published p ON p.source_id = s.source_id AND p.scope = ?1
             LEFT JOIN generations g ON g.gen = p.gen
             LEFT JOIN snapshots n ON n.source_id = s.source_id AND n.snapshot_id = g.snapshot_id
             ORDER BY s.added_at_ms, s.source_id",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map([&req.scope], |r| {
            Ok(Row {
                id: r.get(0)?,
                path: r.get(1)?,
                membership: r.get(2)?,
                gen_id: r.get(3)?,
                gen_membership: r.get(4)?,
                snapshot: r.get(5)?,
                versions: r.get(6)?,
                bytes: r.get(7)?,
                mtime_ns: r.get(8)?,
                verified_ms: r.get(9)?,
            })
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    drop(stmt);
    relocated(storage, select, rows.iter().map(|r| r.path.as_str()), req)?;
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
    let mut states = Vec::new();
    let mut loaded: Vec<(Index, Json, i64, Json)> = Vec::new();
    let mut steps = Vec::new();
    let (mut sources_read, mut records_read) = (0usize, 0usize);
    for row in &rows {
        let mut state = json!({"sourceId": row.id, "path": row.path});
        let job: Option<(String, String, String)> = tx
            .query_row(
                "SELECT job_id, state, owner FROM jobs WHERE source_id = ?1 AND scope = ?2
                 AND state IN ('queued', 'running')",
                [&row.id, &req.scope],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        let job = job.map(|(id, s, holder)| {
            let s = if jobs::alive(&dir, &holder) {
                s
            } else {
                "interrupted".into()
            };
            json!({"jobId": id, "state": s})
        });
        if let Some(job) = &job {
            state["job"] = job.clone();
        }
        let refresh = json!({"tool": "index", "arguments": {"action": "refresh",
            "workspaceId": ws_id, "sourceId": row.id, "scope": req.scope}});
        // Regular-file checks come before any open, so a source replaced by a FIFO cannot block.
        let stamp =
            readable(Path::new(&row.path)).and_then(|p| search::stamp(&p).map_err(Into::into));
        let current =
            row.gen_id.is_some() && row.gen_membership.as_deref() == Some(&row.membership);
        let label = match (&stamp, current, &job) {
            (Err(_), ..) => "unreadable",
            (_, false, Some(j)) if j["state"] != "interrupted" => {
                steps.push(
                    json!({"tool": "jobs", "arguments": {"action": "status", "jobId": j["jobId"]}}),
                );
                "pending"
            }
            (_, false, _) if row.gen_id.is_some() => {
                // Relocated or re-added since its index was built.
                steps.push(refresh.clone());
                "stale"
            }
            (_, false, _) => {
                steps.push(json!({"tool": "index", "arguments": {"action": "refresh",
                    "workspaceId": ws_id, "sourceId": row.id, "scope": req.scope},
                    "why": "this scope is not indexed for this source yet"}));
                state["reason"] = json!("scope_not_indexed");
                "not_prepared"
            }
            _ if row.versions.as_deref() != Some(versions.as_str()) => {
                steps.push(json!({"tool": "index", "arguments": {"action": "rebuild", "workspaceId": ws_id}}));
                "incompatible"
            }
            (Ok(s), ..) if Some(s.1 as i64) != row.bytes || Some(mtime_ns(s.0)) != row.mtime_ns => {
                steps.push(refresh.clone());
                "stale"
            }
            _ => "ready",
        };
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
            let index = search::store::load(&tx, gen_id, q.terms())
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
    let ranked = search::corpus_rank(&parts(&loaded), &q, req, &key)?;
    // Only the shown page reads its records' ancestors, one tree level per query.
    let mut wanted: Vec<Vec<u32>> = vec![Vec::new(); loaded.len()];
    for (part, rid) in ranked.shown() {
        wanted[part].push(rid);
    }
    for ((index, _, gen_id, identity), mut rids) in loaded.iter_mut().zip(wanted) {
        while !rids.is_empty() {
            search::store::add(&tx, *gen_id, index, &rids).map_err(|e| {
                unreadable_index(&ws_id, identity["sourceId"].as_str().unwrap_or(""), e)
            })?;
            rids = rids
                .iter()
                .filter_map(|r| search::parent(index, *r))
                .collect();
            rids.sort_unstable();
            rids.dedup();
        }
    }
    let mut out = search::corpus_page(&parts(&loaded), &q, req, ranked)?;
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
fn relocated<'a>(
    storage: &Storage,
    select: &Select,
    mut paths: impl Iterator<Item = &'a str> + Clone,
    req: &Request,
) -> Result<()> {
    let Select::Locator(Locator::Source(asked)) = select else {
        return Ok(());
    };
    let asked = utf8(asked)?;
    if paths.clone().any(|p| p == asked) {
        return Ok(());
    }
    match paths.find(|p| storage.sees(Path::new(p))) {
        Some(now) => Err(FigError::new(
            ErrorKind::InvalidArguments,
            format!("{asked} is no longer this workspace's source: it was relocated to {now}; search that file"),
        )
        .with_recovery(json!([{"action": "search_relocated", "requires": "none", "tool": "search",
            "arguments": {"file": now, "query": req.query, "node": req.scope}}]))
        .into()),
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
        .take(QUERY_SOURCES)
        .map(|r| {
            json!({"action": "search_one_source", "requires": "none", "tool": "search",
            "arguments": {"workspaceId": ws_id, "query": req.query, "sourceId": r.id}})
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
fn unreadable_index(ws_id: &str, source: &str, e: Box<dyn Error>) -> Box<dyn Error> {
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
