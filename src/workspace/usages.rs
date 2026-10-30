//! Usages over one source's published index of exactly the requested scope. Like workspace
//! search it reads only: one SQLite read transaction, no parse, no build; a source without a
//! ready index is reported with the step that prepares it.

use super::readiness::{Row, State};
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
    // Gated on the schema that introduced relations, not the current one: a read never upgrades.
    if ws.version < 3 {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!("workspace {ws_id} has no relations index of this figlens yet; rebuild it, then query usages"),
        )
        .with_recovery(rebuild)
        .into());
    }
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let rows = super::readiness::rows(&tx, &req.scope)?;
    // The recovery repeats the caller's whole question against the new path.
    super::query::relocated(
        storage,
        select,
        rows.iter().map(|r| r.path.as_str()),
        "usages",
        req.arguments(),
    )?;
    // Invisible sources are dropped before anything about them is read or named.
    let visible: Vec<Row> = rows
        .into_iter()
        .filter(|r| storage.sees(Path::new(&r.path)))
        .collect();
    let row = match source {
        Some(id) => visible.into_iter().find(|r| r.id == id).ok_or_else(|| {
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
                visible.into_iter().filter(|r| r.gen_id.is_some()).collect()
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
    let checked = row.check(&tx, &ws.held.dir, &req.scope, &store::versions(), false)?;
    let current = row.current();
    let Row { id, snapshot, .. } = row;
    let refresh = json!([{"action": "refresh", "requires": "none", "tool": "index",
        "arguments": {"action": "refresh", "workspaceId": ws_id, "sourceId": id, "scope": req.scope}}]);
    if checked.state == State::Unreadable {
        return Err(fail(
            ErrorKind::SourceUnreadable,
            format!("source {id} cannot be read now; restore access to it, then query again"),
        ));
    }
    if matches!(checked.state, State::Pending | State::NotPrepared) {
        if req.cursor.is_some() {
            return Err(FigError::new(
                ErrorKind::CursorStale,
                format!("the index this cursor paged is no longer published; query usages again without cursor once source {id} is ready"),
            )
            .into());
        }
        // A pending or never-indexed scope is not an empty set of usages.
        let state = checked.state.label();
        let step = if checked.state == State::Pending {
            json!({"tool": "jobs", "arguments": {"action": "status", "jobId": checked.job.expect("pending job")["jobId"]}})
        } else {
            json!({"tool": "index", "arguments": refresh[0]["arguments"]})
        };
        return Ok(json!({
            "workspaceId": ws_id, "state": state, "definition": null,
            "direction": if req.outgoing { "out" } else { "in" },
            "relations": q.relations, "scope": {"nodeId": req.scope}, "results": [],
            "coverage": {
                "complete": false,
                "layers": {"relations": crate::search::relations(state)},
                "reasons": [{"code": if state == "pending" { "index_pending" } else { "scope_not_indexed" }}],
            },
            "hasMore": false, "nextCursor": null,
            "source": {"sourceId": id, "snapshotId": null}, "nextSteps": [step],
        }));
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
    if checked.state == State::Incompatible {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!(
                "the index of {id} was built by another figlens; rebuild it, then query usages"
            ),
        )
        .with_recovery(rebuild)
        .into());
    }
    if checked.state == State::StaleStamp {
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
