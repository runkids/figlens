//! Saved-query subscriptions: a consumer follows one workspace search over one visible source.
//! Their events are the search's own evaluations, never revisions. An evaluation is computed
//! outside any write and published under fences; `changes` only reads stored ones.

use super::*;
use crate::search;

/// The ranked window an evaluation keeps; changes past it are counted, never listed.
pub(super) const TOP_K: usize = 50;
/// Evaluations kept per subscription, and their retained bytes in the whole workspace.
const KEEP: i64 = 16;
const BYTES: i64 = 8 * 1024 * 1024;
/// The most one stored evaluation may take: half a `changes` page, so one with the reply's own
/// fields (tracking, subscription) always fits.
const ROW_BYTES: usize = PAGE_BYTES / 2;

/// What a query subscription saved: the search as given, and its canonical form.
pub(super) struct Saved {
    pub(super) query: String,
    pub(super) types: Vec<String>,
}

impl Saved {
    pub(super) fn parse(anchor: &str) -> Result<Saved> {
        let v: Json = serde_json::from_str(anchor).map_err(|_| stale())?;
        Ok(Saved {
            query: v["query"].as_str().ok_or_else(stale)?.to_owned(),
            types: v["types"]
                .as_array()
                .ok_or_else(stale)?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect(),
        })
    }

    pub(super) fn anchor(&self) -> String {
        json!({"query": self.query, "types": self.types}).to_string()
    }

    /// The workspace search this saved query runs over `scope`.
    pub(super) fn request(&self, scope: &str) -> search::Request {
        search::Request {
            query: self.query.clone(),
            scope: scope.to_owned(),
            types: self.types.clone(),
            include_hidden: false,
            include_internal: false,
            limit: TOP_K,
            cursor: None,
        }
    }

    /// What one saved query is identified by: the search's own normalized phrase and terms,
    /// so two spellings search normalizes alike are one subscription.
    pub(super) fn canonical(&self, scope: &str) -> Result<String> {
        let q = self.request(scope).validate()?;
        Ok(json!([scope, q.phrase(), q.terms(), self.types]).to_string())
    }
}

/// One evaluation before it is published: what it read and what it found.
pub(super) struct Eval {
    source: String,
    generation: i64,
    membership: String,
    snapshot: String,
    ranking: String,
    coverage: Json,
    refs: Vec<String>,
    more: bool,
    /// The query as search normalized it, which a subscription's identity is bound to.
    pub(super) canonical: String,
}

/// A publication as `changes` reads it: generation, membership, and the path, size and mtime
/// of the file it was built from.
type Published = (i64, String, String, Option<i64>, Option<i64>);

/// The generation `source`'s `scope` is published at now, and the source's membership.
fn publication(conn: &Connection, source: &str, scope: &str) -> Result<Option<(i64, String)>> {
    conn.query_row(
        "SELECT p.gen, s.membership FROM published p JOIN sources s
         ON s.source_id = p.source_id WHERE p.source_id = ?1 AND p.scope = ?2",
        [source, scope],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(sql)
}

/// The saved search, run with the workspace search's own validation, ranking and budgets over
/// `source`'s published `scope`, outside any write. `Err(reply)` says why there is nothing to
/// evaluate now (not published, stale, unreadable) and what makes it ready.
pub(super) fn compute(
    storage: &Storage,
    select: &Select,
    sub: &str,
    source: &str,
    scope: &str,
    saved: &Saved,
) -> Result<std::result::Result<Eval, Json>> {
    let (_, ws) = opened(storage, select, false)?;
    // Publications only move forward, so the same one before and after the search is the one
    // it read.
    let Some((generation, membership)) = publication(&ws.conn, source, scope)? else {
        return Ok(Err(
            json!({"state": "unavailable", "reason": "scope_not_published",
            "nextSteps": [{"tool": "index", "arguments": {"action": "refresh",
                "workspaceId": ws.held.id, "sourceId": source, "scope": scope}}]}),
        ));
    };
    let canonical = saved.canonical(scope)?;
    let reply = match query::search(storage, select, &[source.to_owned()], &saved.request(scope)) {
        Ok(reply) => reply,
        Err(e) => {
            // A stale, unreadable or too large index: nothing to evaluate until its recovery runs.
            let Some(f) = e.downcast_ref::<FigError>().filter(|f| {
                matches!(
                    f.kind(),
                    ErrorKind::SourceStale
                        | ErrorKind::SourceUnreadable
                        | ErrorKind::QueryWorkLimit
                        | ErrorKind::IndexIncompatible
                )
            }) else {
                return Err(e);
            };
            return Ok(Err(
                json!({"state": "unavailable", "reason": "search_failed",
                "message": f.to_string(), "nextSteps": f.recovery()}),
            ));
        }
    };
    if publication(&ws.conn, source, scope)? != Some((generation, membership.clone())) {
        return Ok(Err(pending("newer_publication", &ws.held.id, sub)));
    }
    if reply["state"] != "ready" {
        return Ok(Err(
            json!({"state": "unavailable", "reason": reply["state"],
            "sources": reply["sources"], "nextSteps": reply["nextSteps"]}),
        ));
    }
    let refs = reply["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(|h| h["ref"]["nodeId"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let coverage = &reply["coverage"];
    Ok(Ok(Eval {
        source: source.to_owned(),
        generation,
        membership,
        snapshot: reply["sources"][0]["snapshotId"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        ranking: reply["ranking"].to_string(),
        coverage: json!({"complete": coverage["complete"], "text": coverage["layers"]["text"]["state"],
            "reasons": coverage["reasons"].as_array().map(|r| r.iter().map(|c| c["code"].clone()).collect::<Vec<_>>())}),
        refs,
        more: reply["hasMore"] == true,
        canonical,
    }))
}

/// The newest stored evaluation of a subscription, as published.
struct Last {
    id: Vec<u8>,
    generation: i64,
    membership: String,
    snapshot: String,
    ranking: String,
    coverage: String,
    refs: Vec<String>,
}

impl Last {
    /// Whether it was evaluated at this publication and ranking.
    fn matches(&self, generation: i64, membership: &str, ranking: &str) -> bool {
        self.generation == generation && self.membership == membership && self.ranking == ranking
    }
}

fn last(conn: &Connection, ws: &str, sub: &str) -> Result<Option<Last>> {
    conn.query_row(
        // Nothing is copied from a row past ROW_BYTES: each column reads null and it is damaged.
        "SELECT CASE WHEN f THEN id END, gen, CASE WHEN f THEN membership END,
         CASE WHEN f THEN snapshot_id END, CASE WHEN f THEN ranking END,
         CASE WHEN f THEN coverage END, CASE WHEN f THEN refs END
         FROM (SELECT id, gen, membership, snapshot_id, ranking, coverage, refs,
           length(CAST(id AS BLOB)) + length(CAST(membership AS BLOB))
           + length(CAST(snapshot_id AS BLOB)) + length(CAST(ranking AS BLOB))
           + length(CAST(coverage AS BLOB)) + length(CAST(refs AS BLOB)) <= ?2 AS f
           FROM query_evaluations WHERE sub = ?1 ORDER BY seq DESC LIMIT 1)",
        params![sub, ROW_BYTES as i64],
        |r| {
            Ok((
                r.get::<_, i64>(1)?,
                (r.get(0)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, Option<String>>(6)?),
            ))
        },
    )
    .optional()
    .map_err(sql)?
    .map(|(generation, row)| {
        // A damaged or oversized row must not read as an empty window, which would enter every
        // result.
        let damaged = || {
            FigError::new(
                ErrorKind::InvalidData,
                "a stored saved-query evaluation is damaged; unsubscribe and subscribe again",
            )
            .with_recovery(json!([{"action": "unsubscribe", "requires": "write", "tool": "watch",
                "arguments": {"action": "unsubscribe", "workspaceId": ws, "subscriptionId": sub}}]))
        };
        let (Some(id), Some(membership), Some(snapshot), Some(ranking), Some(coverage), Some(refs)) = row
        else {
            return Err(damaged().into());
        };
        let refs = serde_json::from_str(&refs).map_err(|_| damaged())?;
        Ok(Last { id, generation, membership, snapshot, ranking, coverage, refs })
    })
    .transpose()
}

/// Why this evaluation differs from the last one. Only new bytes are a source change; a new
/// generation of the same bytes is a rebuild.
fn causes(last: Option<&Last>, e: &Eval) -> Vec<&'static str> {
    let Some(l) = last else {
        return vec!["initial"];
    };
    let mut out = Vec::new();
    if l.snapshot != e.snapshot {
        out.push("source_changed");
    } else if l.generation != e.generation {
        out.push("index_rebuilt");
    }
    if serde_json::from_str::<Json>(&l.coverage).ok().as_ref() != Some(&e.coverage) {
        out.push("coverage_changed");
    }
    if l.ranking != e.ranking {
        out.push("ranking_changed");
    }
    if l.membership != e.membership {
        out.push("corpus_changed");
    }
    out
}

/// Movement within the top-[`TOP_K`] window: entering or leaving it, or moving inside it, does
/// not prove a result was added to or removed from the whole result set.
fn delta(last: Option<&Last>, e: &Eval) -> Json {
    // One subscription follows one sourceId, which relocate keeps: its nodes compare across a
    // move; a removed source deletes the subscription with it.
    let before = last.map_or(&[][..], |l| l.refs.as_slice());
    let pos = |refs: &[String], id: &str| refs.iter().position(|r| r == id);
    // Exact refs: a left result names the old snapshot it ranked in, never the new one.
    let node = |snapshot: &str, id: &String| json!({"sourceId": e.source, "snapshotId": snapshot, "nodeId": id});
    let entered: Vec<Json> = e
        .refs
        .iter()
        .filter(|r| pos(before, r).is_none())
        .map(|r| node(&e.snapshot, r))
        .collect();
    let left: Vec<Json> = last.map_or_else(Vec::new, |l| {
        l.refs
            .iter()
            .filter(|r| pos(&e.refs, r).is_none())
            .map(|r| node(&l.snapshot, r))
            .collect()
    });
    // Positions under two rankings are not comparable: only membership of the window is.
    let comparable = last.is_none_or(|l| l.ranking == e.ranking);
    let moved = comparable.then(|| {
        e.refs
            .iter()
            .enumerate()
            .filter_map(|(to, r)| {
                pos(before, r)
                    .filter(|from| *from != to)
                    .map(|from| json!({"nodeId": r, "from": from + 1, "to": to + 1}))
            })
            .collect::<Vec<_>>()
    });
    // Publications between the two evaluations are coalesced: `from` names the one compared.
    json!({"from": last.map(|l| search::hex(&l.id)),
        "fromSnapshotId": last.map(|l| &l.snapshot), "snapshotId": e.snapshot,
        "enteredTopK": entered, "leftTopK": left, "rankChanged": moved,
        "rankComparable": comparable})
}

pub(super) fn pending(reason: &str, ws: &str, sub: &str) -> Json {
    json!({"state": "pending", "reason": reason,
        "nextSteps": [{"tool": "watch", "arguments": {"action": "evaluate", "workspaceId": ws, "subscriptionId": sub}}]})
}

/// Publishes `e` for subscription `sub` inside the caller's write, when the generation,
/// membership and subscription it was computed for still hold; otherwise it stays pending and
/// nothing is written.
pub(super) fn record(
    tx: &Connection,
    storage: &Storage,
    ws: &str,
    sub: &str,
    e: &Eval,
) -> Result<Json> {
    let held: Option<(String, String, String)> = tx
        .query_row(
            "SELECT source_id, scope, target FROM subscriptions WHERE id = ?1 AND kind = 'query'
             AND filter_version = ?2",
            params![sub, subscriptions::FILTER_VERSION],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    // Removed or moved out of view since it was computed.
    let Some((source, scope, target)) = held else {
        return Err(subscriptions::unknown(sub));
    };
    if registry::visible_path(storage, tx, &source)?.is_none() {
        return Err(subscriptions::unknown(sub));
    }
    // Search now normalizes the saved text differently: this subscription can never match.
    if target != e.canonical {
        return Ok(json!({"state": "unavailable", "reason": "query_changed",
            "nextSteps": [{"tool": "watch", "arguments": {"action": "unsubscribe", "workspaceId": ws, "subscriptionId": sub}},
            {"tool": "watch", "arguments": {"action": "subscribe", "workspaceId": ws, "sourceId": source}}]}));
    }
    let now = publication(tx, &source, &scope)?;
    // A publication or relocation since it was computed: evaluate again, never store this one.
    if now.as_ref() != Some(&(e.generation, e.membership.clone())) {
        return Ok(pending("newer_publication", ws, sub));
    }
    let last = last(tx, ws, sub)?;
    if last
        .as_ref()
        .is_some_and(|l| l.matches(e.generation, &e.membership, &e.ranking))
    {
        return Ok(json!({"state": "current"}));
    }
    // A rebuild that changed nothing this search used is no event: the evaluation is rebound
    // to the new generation, so it neither reads as pending nor evicts a real change.
    if let Some(l) = &last
        && l.snapshot == e.snapshot
        && l.membership == e.membership
        && l.ranking == e.ranking
        && l.refs == e.refs
        && serde_json::from_str::<Json>(&l.coverage).ok().as_ref() == Some(&e.coverage)
    {
        tx.execute(
            "UPDATE query_evaluations SET gen = ?2 WHERE id = ?1",
            params![l.id, e.generation],
        )
        .map_err(sql)?;
        return Ok(json!({"state": "current"}));
    }
    let causes = causes(last.as_ref(), e);
    let delta = delta(last.as_ref(), e);
    let refs = json!(e.refs).to_string();
    // Every hit is in the window only when nothing ranked past it and the index was whole.
    let total = (!e.more && e.coverage["complete"] == true).then_some(e.refs.len() as i64);
    let coverage = e.coverage.to_string();
    let causes_text = json!(causes).to_string();
    let delta_text = delta.to_string();
    let bytes = (refs.len()
        + coverage.len()
        + causes_text.len()
        + delta_text.len()
        + e.ranking.len()
        + e.snapshot.len()
        + e.membership.len()) as i64;
    // The oldest of this subscription's evaluations past what it keeps go first, and their
    // bytes are not counted against the workspace.
    let doomed: (i64, i64) = tx
        .query_row(
            "SELECT IFNULL(MAX(seq), 0), IFNULL(SUM(bytes), 0) FROM (SELECT seq, bytes
             FROM query_evaluations WHERE sub = ?1 ORDER BY seq DESC LIMIT -1 OFFSET ?2)",
            params![sub, KEEP - 1],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    let held_bytes: i64 = tx
        .query_row(
            "SELECT IFNULL(SUM(bytes), 0) FROM query_evaluations",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if bytes as usize > ROW_BYTES {
        return Ok(
            json!({"state": "unavailable", "reason": "evaluation_too_large",
            "message": format!("this evaluation's window takes more than {ROW_BYTES} bytes; subscribe to a narrower query, type or scope"),
            "nextSteps": [{"tool": "watch", "arguments": {"action": "unsubscribe", "workspaceId": ws, "subscriptionId": sub}}]}),
        );
    }
    if held_bytes - doomed.1 + bytes > BYTES || !room(tx, (bytes - doomed.1).max(0) as u64)? {
        return Err(FigError::new(
            ErrorKind::WorkspaceBusy,
            format!("saved-query evaluations already hold {BYTES} bytes in workspace {ws}, or its database reached its {}-byte cap; unsubscribe query subscriptions no longer read", jobs::DISK_CAP),
        )
        .with_recovery(json!([{"action": "list", "requires": "none", "tool": "watch",
            "arguments": {"action": "list", "workspaceId": ws}},
            {"action": "unsubscribe", "requires": "write", "tool": "watch",
            "arguments": {"action": "unsubscribe", "workspaceId": ws, "subscriptionId": "<id>"}}]))
        .into());
    }
    if doomed.0 > 0 {
        tx.execute(
            "DELETE FROM query_evaluations WHERE sub = ?1 AND seq <= ?2",
            params![sub, doomed.0],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE subscriptions SET pruned_seq = MAX(pruned_seq, ?2) WHERE id = ?1",
            params![sub, doomed.0],
        )
        .map_err(sql)?;
    }
    let id: Vec<u8> = tx
        .query_row("SELECT randomblob(16)", [], |r| r.get(0))
        .map_err(sql)?;
    tx.execute(
        "INSERT INTO query_evaluations (id, sub, seq, gen, snapshot_id, membership, ranking,
         coverage, causes, refs, total, delta, bytes, created_ms)
         SELECT ?1, ?2, IFNULL(MAX(seq), 0) + 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
         FROM query_evaluations WHERE sub = ?2",
        params![
            id,
            sub,
            e.generation,
            e.snapshot,
            e.membership,
            e.ranking,
            coverage,
            causes_text,
            refs,
            total,
            delta_text,
            bytes,
            now_ms()
        ],
    )
    .map_err(sql)?;
    Ok(
        json!({"state": "evaluated", "evaluationId": search::hex(&id), "causes": causes,
        "delta": delta}),
    )
}

/// Whether `bytes` more fit under `DISK_CAP`, freed pages counted as room.
fn room(conn: &Connection, bytes: u64) -> Result<bool> {
    Ok(jobs::used_bytes(conn)? + bytes <= jobs::DISK_CAP)
}

/// `watch evaluate`: recomputes a query subscription's search for its current publication and
/// stores the result, unless another evaluation of the same generation was stored first.
pub fn evaluate(storage: &Storage, select: &Select, id: Option<&str>) -> Result<Json> {
    let id = id.ok_or_else(|| {
        fail(
            ErrorKind::InvalidArguments,
            "pass subscriptionId: the query subscription to evaluate",
        )
    })?;
    let (source, scope, saved, ws_id) = {
        let (_, ws) = opened(storage, select, false)?;
        let sub = subscriptions::visible(storage, &ws, id)?;
        let saved = sub.saved()?.ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("subscription {id} follows changes, not a saved query"),
            )
        })?;
        let scope = sub.scope.clone().unwrap_or_else(|| "0:0".into());
        (sub.source, scope, saved, ws.held.id.clone())
    };
    let e = match compute(storage, select, id, &source, &scope, &saved)? {
        Ok(e) => e,
        Err(why) => {
            return Ok(json!({"workspaceId": ws_id, "subscriptionId": id, "evaluation": why}));
        }
    };
    let (_, ws) = opened(storage, select, true)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    let out = record(&tx, storage, &ws_id, id, &e)?;
    tx.commit().map_err(sql)?;
    Ok(json!({"workspaceId": ws_id, "subscriptionId": id, "evaluation": out}))
}

/// The receipt of a query subscription's cursors: it changes whenever evaluations it kept were
/// deleted, so a cursor issued before that is known to have missed them.
fn receipt(v: &View, pruned: i64) -> Result<[u8; 16]> {
    let mut mac = mac(v, "receipt", &pruned.to_le_bytes()).ok_or_else(stale)?;
    mac.update(b"query");
    let mut out = [0; 16];
    out.copy_from_slice(&mac.finalize().into_bytes()[..16]);
    Ok(out)
}

fn pruned(conn: &Connection, sub: &str) -> Result<i64> {
    conn.query_row(
        "SELECT pruned_seq FROM subscriptions WHERE id = ?1",
        [sub],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(stale)
}

/// The seq after evaluation `id` of this subscription, or `event_gap` when it is not kept.
fn at(conn: &Connection, v: &View, sub: &str, id: &[u8], held: &[u8; 16]) -> Result<i64> {
    let pruned = pruned(conn, sub)?;
    let seq = match id == OLDEST {
        true => Some(0),
        false => conn
            .query_row(
                "SELECT seq FROM query_evaluations WHERE sub = ?1 AND id = ?2",
                params![sub, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?,
    };
    match seq {
        Some(seq) if !(seq < pruned && *held != receipt(v, pruned)?) => Ok(seq),
        _ => Err(query_gap(v, sub, pruned)),
    }
}

fn query_gap(v: &View, sub: &str, pruned: i64) -> Box<dyn Error> {
    let cursor = match receipt(v, pruned) {
        Ok(now) => position_cursor(v, &OLDEST, &now),
        Err(e) => return e,
    };
    FigError::new(
        ErrorKind::EventGap,
        "evaluations after this position were deleted by retention before they were read; read changes again from the oldest kept, and treat the result as possibly changed",
    )
    .with_recovery(json!([{"action": "changes", "requires": "none", "tool": "changes",
        "arguments": {"workspaceId": v.workspace, "subscriptionId": sub, "cursor": cursor}}]))
    .into()
}

/// One page of a query subscription's evaluations after the cursor or its checkpoint, with
/// whether its current publication still waits for `watch evaluate`. Never writes.
pub(super) fn poll(
    conn: &Connection,
    v: &View,
    sub: &subscriptions::Sub,
    req: &Request,
) -> Result<Json> {
    let limit = req.limit.unwrap_or(PAGE);
    if !(1..=PAGE_MAX).contains(&limit) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("limit must be 1 to {PAGE_MAX}"),
        ));
    }
    let pruned = pruned(conn, &sub.id)?;
    let now = receipt(v, pruned)?;
    let after = match req.cursor {
        Some(c) => {
            let (id, held) = read_position(v, c)?;
            at(conn, v, &sub.id, &id, &held)?
        }
        None if sub.after_seq < pruned => return Err(query_gap(v, &sub.id, pruned)),
        None => sub.after_seq,
    };
    // Sizes first: no row is copied or parsed before the page has room for it.
    let mut stmt = conn
        .prepare(
            // Actual stored lengths of every emitted column, not the counter stored beside them;
            // 160 covers the reply's own keys.
            "SELECT id, seq, 160 + length(CAST(id AS BLOB)) + length(CAST(snapshot_id AS BLOB))
             + length(CAST(coverage AS BLOB)) + length(CAST(causes AS BLOB))
             + length(CAST(delta AS BLOB))
             FROM query_evaluations WHERE sub = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
        )
        .map_err(sql)?;
    let sizes = stmt
        .query_map(params![sub.id, after, limit as i64 + 1], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let (mut admitted, mut used) = (Vec::new(), 0usize);
    for (id, seq, size) in sizes.iter().take(limit) {
        let size = *size as usize;
        if used + size > PAGE_BYTES {
            if admitted.is_empty() {
                // Only a damaged or foreign row can be this large: let the reader pass it.
                return Err(FigError::new(
                    ErrorKind::ResponseBudget,
                    format!(
                        "evaluation {} takes more than {PAGE_BYTES} bytes; ack past it to read on",
                        search::hex(id)
                    ),
                )
                .with_recovery(
                    json!([{"action": "ack", "requires": "write", "tool": "watch",
                    "arguments": {"action": "ack", "workspaceId": v.workspace,
                    "subscriptionId": sub.id, "cursor": position_cursor(v, id, &now)}}]),
                )
                .into());
            }
            break;
        }
        used += size;
        admitted.push((id.clone(), *seq));
    }
    // One ranged read of the admitted rows, parsed one at a time: a damaged row ends the page
    // before it, so the rows ahead of it stay readable and their cursor passes them.
    let mut page = Vec::with_capacity(admitted.len());
    let mut stmt = conn
        .prepare(
            "SELECT id, snapshot_id, coverage, causes, total, delta, created_ms
             FROM query_evaluations WHERE sub = ?1 AND seq > ?2 AND seq <= ?3 ORDER BY seq",
        )
        .map_err(sql)?;
    let last_seq = admitted.last().map_or(after, |(_, seq)| *seq);
    let mut rows = stmt.query(params![sub.id, after, last_seq]).map_err(sql)?;
    while let Some(r) = rows.next().map_err(sql)? {
        let id: Vec<u8> = r.get(0).map_err(sql)?;
        let text = |i| r.get::<_, String>(i).map_err(sql);
        let parsed = [text(2)?, text(3)?, text(5)?].map(|t| serde_json::from_str::<Json>(&t).ok());
        let [Some(coverage), Some(causes), Some(delta)] = parsed else {
            if !page.is_empty() {
                admitted.truncate(page.len());
                break;
            }
            return Err(FigError::new(
                ErrorKind::InvalidData,
                format!(
                    "stored evaluation {} is damaged; ack past it to read on, or unsubscribe",
                    search::hex(&id)
                ),
            )
            .with_recovery(
                json!([{"action": "ack", "requires": "write", "tool": "watch",
                "arguments": {"action": "ack", "workspaceId": v.workspace,
                "subscriptionId": sub.id, "cursor": position_cursor(v, &id, &now)}}]),
            )
            .into());
        };
        page.push(
            json!({"evaluationId": search::hex(&id), "snapshotId": text(1)?,
            "coverage": coverage, "causes": causes,
            "total": r.get::<_, Option<i64>>(4).map_err(sql)?, "topK": TOP_K, "delta": delta,
            "createdAtMs": r.get::<_, i64>(6).map_err(sql)?}),
        );
    }
    let scope = sub.scope.as_deref().unwrap_or("0:0");
    // The publication and the file behind it: a save, deletion or lost access since that build
    // leaves the evaluation current only for the index, not the file.
    let published: Option<Published> = conn
        .query_row(
            "SELECT p.gen, s.membership, s.path, n.bytes, n.mtime_ns FROM published p
             JOIN sources s ON s.source_id = p.source_id JOIN generations g ON g.gen = p.gen
             LEFT JOIN snapshots n ON n.source_id = p.source_id AND n.snapshot_id = g.snapshot_id
             WHERE p.source_id = ?1 AND p.scope = ?2",
            [&sub.source, scope],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(sql)?;
    // Only the publication it was evaluated at is read, never its stored window.
    let ranking = search::corpus_ranking().to_string();
    let state = match published {
        None => json!({"state": "unavailable", "reason": "scope_not_published"}),
        // Compared in SQL: nothing of the stored row is copied out.
        Some((generation, membership, path, bytes, mtime))
            if conn
                .query_row(
                    "SELECT gen = ?2 AND membership = ?3 AND ranking = ?4 FROM query_evaluations
                     WHERE sub = ?1 ORDER BY seq DESC LIMIT 1",
                    params![sub.id, generation, membership, ranking],
                    |r| r.get::<_, bool>(0),
                )
                .optional()
                .map_err(sql)?
                == Some(true) =>
        {
            let refresh = refresh_required(&path, bytes, mtime);
            let mut out = json!({"state": if refresh { "current_for_index" } else { "current" },
                "refreshRequired": refresh});
            if refresh {
                out["nextSteps"] = json!([{"tool": "index", "arguments": {"action": "refresh",
                    "workspaceId": v.workspace, "sourceId": sub.source, "scope": scope}},
                {"tool": "watch", "arguments": {"action": "evaluate",
                    "workspaceId": v.workspace, "subscriptionId": sub.id}}]);
            }
            out
        }
        _ => pending("not_evaluated", &v.workspace, &sub.id),
    };
    let (tracked, truncated) = tracking(conn, v)?;
    Ok(json!({
        "workspaceId": v.workspace,
        "evaluations": page,
        "nextCursor": admitted.last().map_or(Json::Null, |(id, _)| position_cursor(v, id, &now)),
        "hasMore": admitted.len() < sizes.len(),
        "evaluation": state,
        "tracking": tracked,
        "trackingTruncated": truncated,
        "monitor": {"state": "inactive"},
        "subscription": subscriptions::describe(conn, sub)?,
    }))
}

/// `watch ack` of a query subscription: its checkpoint moves to the evaluation the cursor names.
pub(super) fn ack(tx: &Connection, v: &View, sub: &subscriptions::Sub, text: &str) -> Result<Json> {
    let (id, receipt) = read_position(v, text)?;
    let seq = at(tx, v, &sub.id, &id, &receipt)?;
    let before: i64 = tx
        .query_row(
            "SELECT after_seq FROM subscriptions WHERE id = ?1",
            [&sub.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(stale)?;
    tx.execute(
        "UPDATE subscriptions SET after_seq = MAX(after_seq, ?2), read_ms = ?3 WHERE id = ?1",
        params![sub.id, seq, now_ms()],
    )
    .map_err(sql)?;
    Ok(
        json!({"workspaceId": v.workspace, "consumer": sub.consumer, "subscriptionId": sub.id,
        "advanced": seq > before, "checkpoint": held(tx, v, &sub.id, before.max(seq))?}),
    )
}

/// The cursor after evaluation seq `at`, or null when that evaluation is no longer kept.
pub(super) fn held(conn: &Connection, v: &View, sub: &str, at: i64) -> Result<Json> {
    let now = receipt(v, pruned(conn, sub)?)?;
    if at == 0 {
        return Ok(position_cursor(v, &OLDEST, &now));
    }
    let id: Option<Vec<u8>> = conn
        .query_row(
            "SELECT id FROM query_evaluations WHERE sub = ?1 AND seq = ?2",
            params![sub, at],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    Ok(id.map_or(Json::Null, |id| position_cursor(v, &id, &now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_freed_by_deleted_evaluations_are_room_under_the_disk_cap() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (b BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 300)
             INSERT INTO t SELECT zeroblob(4000) FROM n; DELETE FROM t;",
        )
        .unwrap();
        let high: i64 = conn
            .query_row(
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // At its high-water size the file has no growth left, yet the freed pages take it.
        assert_eq!(
            (
                high as u64 + 100_000 > jobs::DISK_CAP,
                room(&conn, 100_000).unwrap()
            ),
            (true, true)
        );
    }

    #[test]
    fn a_relocated_source_keeps_its_nodes_across_the_new_membership() {
        let refs = |ids: &[&str]| ids.iter().map(|i| i.to_string()).collect::<Vec<_>>();
        let last = Last {
            id: vec![0; 16],
            generation: 1,
            membership: "a".into(),
            snapshot: "old".into(),
            ranking: "r".into(),
            coverage: "{}".into(),
            refs: refs(&["1:1", "1:2"]),
        };
        let e = |membership: &str| Eval {
            source: "s".into(),
            generation: 2,
            membership: membership.into(),
            snapshot: "new".into(),
            ranking: "r".into(),
            coverage: json!({}),
            refs: refs(&["1:2", "1:1"]),
            more: false,
            canonical: String::new(),
        };
        let count = |d: &Json, k: &str| d[k].as_array().map(|a| a.len());
        let (kept, moved) = (delta(Some(&last), &e("a")), delta(Some(&last), &e("b")));
        assert_eq!(
            [
                (
                    count(&kept, "enteredTopK"),
                    count(&kept, "leftTopK"),
                    count(&kept, "rankChanged")
                ),
                (
                    count(&moved, "enteredTopK"),
                    count(&moved, "leftTopK"),
                    count(&moved, "rankChanged")
                )
            ],
            [(Some(0), Some(0), Some(2)), (Some(0), Some(0), Some(2))]
        );
    }
}
