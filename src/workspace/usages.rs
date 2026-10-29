//! Usages over one source's published index of exactly the requested scope. Like workspace
//! search it reads only: one SQLite read transaction, no parse, no build; a source without a
//! ready index is reported with the step that prepares it.

use super::*;
use crate::search::store::{self, Stored};
use crate::search::usages::{self, Request, Served};

/// Usages of `req.target` in `source`'s published `req.scope` (the only visible source when
/// `source` is `None`). The source must be ready and unchanged since its index was verified.
pub fn usages(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    req: &Request,
) -> Result<Json> {
    let q = req.validate()?;
    let (_, ws) = opened(storage, select, false)?;
    let ws_id = ws.held.id.clone();
    let rebuild = json!([{"action": "rebuild", "requires": "none", "tool": "index",
        "arguments": {"action": "rebuild", "workspaceId": ws_id}}]);
    if ws.version < SCHEMA {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!("workspace {ws_id} has no relations index of this figlens yet; rebuild it, then query usages"),
        )
        .with_recovery(rebuild)
        .into());
    }
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let mut stmt = tx
        .prepare(
            "SELECT s.source_id, s.path, s.membership, p.gen, g.membership, g.snapshot_id,
             g.versions, n.bytes, n.mtime_ns FROM sources s
             LEFT JOIN published p ON p.source_id = s.source_id AND p.scope = ?1
             LEFT JOIN generations g ON g.gen = p.gen
             LEFT JOIN snapshots n ON n.source_id = s.source_id AND n.snapshot_id = g.snapshot_id
             ORDER BY s.added_at_ms, s.source_id",
        )
        .map_err(sql)?;
    type Row = (
        String,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
    );
    let rows: Vec<Row> = stmt
        .query_map([&req.scope], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql)?;
    drop(stmt);
    let target = if req.resource { "resource" } else { "node" };
    // The recovery repeats the caller's whole question against the new path.
    let mut asked = json!({target: req.target, "scope": req.scope, "limit": req.limit});
    if req.outgoing {
        asked["direction"] = json!("out");
    }
    if !req.relations.is_empty() {
        asked["relation"] = json!(req.relations.join(","));
    }
    if req.include_hidden {
        asked["includeHidden"] = json!(true);
    }
    if let Some(snapshot) = &req.snapshot {
        asked["snapshotId"] = json!(snapshot);
    }
    super::query::relocated(
        storage,
        select,
        rows.iter().map(|r| r.1.as_str()),
        "usages",
        asked,
    )?;
    // Invisible sources are dropped before anything about them is read or named.
    let visible: Vec<Row> = rows
        .into_iter()
        .filter(|r| storage.sees(Path::new(&r.1)))
        .collect();
    let row = match source {
        Some(id) => visible.into_iter().find(|r| r.0 == id).ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("workspace {ws_id} has no source {id}"),
            )
        })?,
        None => {
            // The only source, or else the only one with this scope indexed.
            let mut published: Vec<Row> = if visible.len() == 1 {
                visible
            } else {
                visible.into_iter().filter(|r| r.3.is_some()).collect()
            };
            if published.len() != 1 {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!(
                        "pass sourceId: {} sources of workspace {ws_id} have an index of scope {}",
                        published.len(),
                        req.scope
                    ),
                ));
            }
            published.remove(0)
        }
    };
    let (id, path, membership, gen_id, gen_membership, snapshot, versions, bytes, mtime) = row;
    let refresh = json!([{"action": "refresh", "requires": "none", "tool": "index",
        "arguments": {"action": "refresh", "workspaceId": ws_id, "sourceId": id, "scope": req.scope}}]);
    let stamp =
        readable(Path::new(&path)).and_then(|p| crate::search::stamp(&p).map_err(Into::into));
    let Ok(stamp) = stamp else {
        return Err(fail(
            ErrorKind::SourceUnreadable,
            format!("source {id} cannot be read now; restore access to it, then query again"),
        ));
    };
    let current = gen_id.filter(|_| gen_membership.as_deref() == Some(membership.as_str()));
    if current.is_none() {
        let job: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT job_id, owner, attempt FROM jobs WHERE source_id = ?1 AND scope = ?2
                 AND state IN ('queued', 'running')",
                [&id, &req.scope],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        let job =
            job.filter(|(job, owner, attempt)| jobs::job_alive(&ws.held.dir, owner, job, *attempt));
        // A build on its way, or a scope never indexed: a state with its next step, never an
        // empty result set that reads as "no usages".
        let (state, step) = match (&job, gen_id) {
            (Some((job, ..)), _) => (
                "pending",
                json!({"tool": "jobs", "arguments": {"action": "status", "jobId": job}}),
            ),
            (None, None) => ("not_prepared", refresh[0]["arguments"].clone()),
            (None, Some(_)) => ("stale", Json::Null),
        };
        if state != "stale" && req.cursor.is_some() {
            return Err(FigError::new(
                ErrorKind::CursorStale,
                format!("the index this cursor paged is no longer published; query usages again without cursor once source {id} is ready"),
            )
            .into());
        }
        if state != "stale" {
            let step = if state == "pending" {
                step
            } else {
                json!({"tool": "index", "arguments": step})
            };
            return Ok(json!({
                "workspaceId": ws_id, "state": state, "definition": null,
                "direction": if req.outgoing { "out" } else { "in" },
                "relations": q.relations, "scope": {"nodeId": req.scope}, "results": [],
                "coverage": {"complete": false, "layers": {"relations": crate::search::relations(state)},
                    "reasons": [{"code": if state == "pending" { "index_pending" } else { "scope_not_indexed" }}]},
                "hasMore": false, "nextCursor": null,
                "source": {"sourceId": id, "snapshotId": null}, "nextSteps": [step],
            }));
        }
    }
    let Some(gen_id) = current else {
        return Err(FigError::new(
            ErrorKind::SourceStale,
            format!(
                "source {id} was relocated or added again since scope {} was indexed; refresh it, then query usages",
                req.scope
            ),
        )
        .with_recovery(refresh)
        .into());
    };
    if versions.as_deref() != Some(store::versions().as_str()) {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!(
                "the index of {id} was built by another figlens; rebuild it, then query usages"
            ),
        )
        .with_recovery(rebuild)
        .into());
    }
    let mtime_ns = stamp
        .0
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64);
    if Some(stamp.1 as i64) != bytes || Some(mtime_ns) != mtime {
        return Err(FigError::new(
            ErrorKind::SourceStale,
            format!("source {id} changed since it was indexed; refresh it, then query usages"),
        )
        .with_recovery(refresh)
        .into());
    }
    if let Some(paged) = q.cursor_gen().filter(|g| *g != gen_id) {
        let gone = tx
            .query_row("SELECT 1 FROM generations WHERE gen = ?1", [paged], |_| {
                Ok(())
            })
            .optional()
            .map_err(sql)?
            .is_none();
        let (kind, why) = if gone {
            (
                ErrorKind::CursorExpired,
                "the index this cursor paged was deleted",
            )
        } else {
            (
                ErrorKind::CursorStale,
                "the source was reindexed since this cursor",
            )
        };
        return Err(
            FigError::new(kind, format!("{why}; query usages again without cursor")).into(),
        );
    }
    let mut spent = 0;
    let index = store::load(&tx, gen_id, &[], &mut spent)
        .map_err(|e| super::query::unreadable_index(&ws_id, &id, e))?;
    let source = json!({"sourceId": id, "snapshotId": snapshot});
    let mut graph = Stored {
        conn: &tx,
        gen_id,
        index,
        spent,
    };
    let mut out = usages::query(
        &mut graph,
        &q,
        req,
        Served {
            source: &source,
            gen_id,
        },
    )?;
    out["workspaceId"] = json!(ws_id);
    out["source"] = source;
    Ok(out)
}
