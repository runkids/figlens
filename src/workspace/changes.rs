//! Recorded revisions of tracked scopes, read without writing (`changes`), and consumer
//! checkpoints (`watch ack`). A cursor names one revision and carries an HMAC-SHA256 over the
//! workspace incarnation, the filters and the visible sources, so a cursor from any other
//! binding is `cursor_stale`, never a position in another caller's view.

use super::*;
use crate::search::diff;
use hmac::{Hmac, KeyInit, Mac};
use rusqlite::params;

const PAGE: usize = 20;
const PAGE_MAX: usize = 50;
const DETAIL: usize = 100;
const DETAIL_MAX: usize = 500;
const PAGE_BYTES: usize = 256 * 1024;
/// The cursor of "the oldest revision kept": a zero revision id.
const OLDEST: [u8; 16] = [0; 16];

pub struct Request<'a> {
    pub source: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub consumer: Option<&'a str>,
    pub cursor: Option<&'a str>,
    pub revision: Option<&'a str>,
    pub limit: Option<usize>,
}

/// The sources a reply may name, and what its cursors are bound to.
struct View {
    workspace: String,
    sources: Vec<String>,
    scope: Option<String>,
    filter: String,
    key: Option<String>,
    bind: Vec<u8>,
}

fn view(storage: &Storage, ws: &Opened, req: &Request) -> Result<View> {
    let sources = jobs::chosen(storage, ws, req.source)?;
    let filter = format!(
        "{}\u{1f}{}",
        req.source.unwrap_or(""),
        req.scope.unwrap_or("")
    );
    let key = match ws.version >= SCHEMA {
        true => Some(
            ws.conn
                .query_row(
                    "SELECT value FROM meta WHERE key = 'cursor_secret'",
                    [],
                    |r| r.get(0),
                )
                .map_err(sql)?,
        ),
        false => None,
    };
    let bind = [
        ws.incarnation.as_bytes(),
        filter.as_bytes(),
        &Sha256::digest(sources.join("\u{1f}")),
    ]
    .join(&0x1f);
    Ok(View {
        workspace: ws.held.id.clone(),
        sources,
        scope: req.scope.map(str::to_owned),
        filter,
        key,
        bind,
    })
}

fn mac(v: &View, payload: &[u8]) -> Option<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(v.key.as_ref()?.as_bytes()).ok()?;
    mac.update(&v.bind);
    mac.update(payload);
    Some(mac)
}

fn cursor(v: &View, tag: &str, payload: &[u8]) -> Json {
    match mac(v, payload) {
        Some(m) => json!(format!(
            "{tag}.{}",
            crate::search::hex(&[payload, &m.finalize().into_bytes()[..16]].concat())
        )),
        None => Json::Null,
    }
}

fn stale() -> Box<dyn Error> {
    fail(
        ErrorKind::CursorStale,
        "this cursor was issued for other filters, other visible sources or another workspace; read changes again without it",
    )
}

/// The payload of a cursor this view issued, or `cursor_stale`.
fn payload(v: &View, tag: &str, text: &str, len: usize) -> Result<Vec<u8>> {
    let raw = text
        .strip_prefix(tag)
        .and_then(|t| t.strip_prefix('.'))
        .and_then(unhex)
        .filter(|b| b.len() == len + 16)
        .ok_or_else(stale)?;
    let (payload, tail) = raw.split_at(len);
    mac(v, payload)
        .ok_or_else(stale)?
        .verify_truncated_left(tail)
        .map_err(|_| stale())?;
    Ok(payload.to_vec())
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len().is_multiple_of(2) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

/// The newest revision retention deleted among the view's sources and scope.
fn pruned(conn: &Connection, v: &View) -> Result<i64> {
    let mut stmt = conn
        .prepare("SELECT key, CAST(value AS INTEGER) FROM meta WHERE key LIKE 'pruned' || char(31) || '%'")
        .map_err(sql)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(rows
        .into_iter()
        .filter(|(key, _)| {
            let mut parts = key.split('\u{1f}').skip(1);
            let (source, scope) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            v.sources.iter().any(|s| s == source) && v.scope.as_deref().is_none_or(|s| s == scope)
        })
        .map(|(_, seq)| seq)
        .max()
        .unwrap_or(0))
}

/// The position after `id`: its seq, the oldest kept for [`OLDEST`], or `event_gap` when the
/// revision is no longer kept.
fn position(conn: &Connection, v: &View, id: &[u8]) -> Result<i64> {
    if id == OLDEST {
        return pruned(conn, v);
    }
    conn.query_row(
        "SELECT seq FROM revisions WHERE id = ?1
         AND source_id IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR scope = ?3)",
        params![id, json!(v.sources).to_string(), v.scope],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(|| gap(v))
}

fn gap(v: &View) -> Box<dyn Error> {
    let oldest = cursor(v, "c1", &OLDEST);
    FigError::new(
        ErrorKind::EventGap,
        "revisions after this position were deleted by retention before they were read; read changes again from the oldest kept, and treat the scope as possibly changed",
    )
    .with_recovery(json!([{"action": "changes", "requires": "none", "tool": "changes",
        "arguments": {"workspaceId": v.workspace, "cursor": oldest}}]))
    .into()
}

/// One page of revisions after the cursor, the consumer's checkpoint or the oldest kept, or
/// with `revision` one page of that revision's changed nodes. Never writes.
pub fn changes(storage: &Storage, select: &Select, req: &Request) -> Result<Json> {
    let (_, ws) = opened(storage, select, false)?;
    let v = view(storage, &ws, req)?;
    if v.key.is_none() {
        // A workspace from before change tracking has nothing recorded.
        if req.cursor.is_some() {
            return Err(stale());
        }
        return Ok(
            json!({"workspaceId": v.workspace, "revisions": [], "nextCursor": null,
            "hasMore": false, "tracking": [], "monitor": {"state": "inactive"}}),
        );
    }
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    match req.revision {
        Some(revision) => detail(&tx, &v, req, revision),
        None => poll(&tx, &v, req),
    }
}

fn poll(conn: &Connection, v: &View, req: &Request) -> Result<Json> {
    let limit = req.limit.unwrap_or(PAGE);
    if !(1..=PAGE_MAX).contains(&limit) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("limit must be 1 to {PAGE_MAX}"),
        ));
    }
    let after = match (req.cursor, req.consumer) {
        (Some(c), _) => position(conn, v, &payload(v, "c1", c, 16)?)?,
        (None, Some(consumer)) => {
            let at: Option<i64> = conn
                .query_row(
                    "SELECT after_seq FROM consumers WHERE consumer = ?1 AND filter = ?2",
                    [consumer, &v.filter],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?;
            match at {
                Some(at) if at < pruned(conn, v)? => return Err(gap(v)),
                at => at.unwrap_or(0),
            }
        }
        (None, None) => 0,
    };
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.source_id, r.scope, r.kind, r.snapshot_id, r.coverage, r.summary,
             r.detail, r.detail_rows, (SELECT p.id FROM revisions p WHERE p.source_id = r.source_id
             AND p.scope = r.scope AND p.gen = r.from_gen AND p.seq < r.seq)
             FROM revisions r WHERE r.seq > ?1 AND r.source_id IN (SELECT value FROM json_each(?2))
             AND (?3 IS NULL OR r.scope = ?3) ORDER BY r.seq LIMIT ?4",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(
            params![
                after,
                json!(v.sources).to_string(),
                v.scope,
                limit as i64 + 1
            ],
            |r| {
                let text = |i| r.get::<_, String>(i);
                let id: Vec<u8> = r.get(0)?;
                let from: Option<Vec<u8>> = r.get(9)?;
                Ok((
                    id.clone(),
                    json!({"revisionId": crate::search::hex(&id), "sourceId": text(1)?,
                        "scope": text(2)?, "kind": text(3)?,
                        "from": from.map(|f| crate::search::hex(&f)),
                        "snapshotId": r.get::<_, Option<String>>(4)?,
                        "coverage": serde_json::from_str::<Json>(&text(5)?).unwrap_or(Json::Null),
                        "summary": serde_json::from_str::<Json>(&text(6)?).unwrap_or(Json::Null),
                        "detail": text(7)?, "detailRows": r.get::<_, i64>(8)?}),
                ))
            },
        )
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let (mut page, mut used, mut last) = (Vec::new(), 0, None);
    for (id, item) in rows.iter().take(limit) {
        let size = item.to_string().len();
        if !page.is_empty() && used + size > PAGE_BYTES {
            break;
        }
        used += size;
        page.push(item.clone());
        last = Some(id);
    }
    let has_more = page.len() < rows.len();
    Ok(json!({
        "workspaceId": v.workspace,
        "revisions": page,
        "nextCursor": last.map_or(Json::Null, |id| cursor(v, "c1", id)),
        "hasMore": has_more,
        "tracking": tracking(conn, v)?,
        // The watch monitor (B2) runs apart from tracking: recording happens on each build.
        "monitor": {"state": "inactive"},
    }))
}

/// Tracking state of the view's sources, with whether the file changed since its last build.
fn tracking(conn: &Connection, v: &View) -> Result<Json> {
    let mut stmt = conn
        .prepare(
            "SELECT t.source_id, t.scope, t.state, (SELECT id FROM revisions r
             WHERE r.source_id = t.source_id AND r.scope = t.scope ORDER BY seq DESC LIMIT 1),
             s.path, n.bytes, n.mtime_ns, n.verified_ms FROM tracking t
             JOIN sources s ON s.source_id = t.source_id
             LEFT JOIN published p ON p.source_id = t.source_id AND p.scope = t.scope
             LEFT JOIN generations g ON g.gen = p.gen
             LEFT JOIN snapshots n ON n.source_id = t.source_id AND n.snapshot_id = g.snapshot_id
             WHERE t.source_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR t.scope = ?2)
             ORDER BY s.added_at_ms, t.source_id, t.scope",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![json!(v.sources).to_string(), v.scope], |r| {
            let last: Option<Vec<u8>> = r.get(3)?;
            let path: String = r.get(4)?;
            let (bytes, mtime): (Option<i64>, Option<i64>) = (r.get(5)?, r.get(6)?);
            Ok((
                json!({"sourceId": r.get::<_, String>(0)?, "scope": r.get::<_, String>(1)?,
                    "state": r.get::<_, String>(2)?,
                    "lastRevisionId": last.map(|l| crate::search::hex(&l)),
                    "lastCheckedAtMs": r.get::<_, Option<i64>>(7)?}),
                path,
                bytes,
                mtime,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(rows
        .into_iter()
        .map(|(mut row, path, bytes, mtime)| {
            // An empty page never proves the file unchanged: only a build records a change.
            let stamp = crate::search::stamp(Path::new(&path)).ok();
            row["refreshRequired"] = json!(stamp.is_none_or(|(at, len)| {
                Some(len as i64) != bytes
                    || Some(
                        at.duration_since(SystemTime::UNIX_EPOCH)
                            .map_or(0, |d| d.as_nanos() as i64),
                    ) != mtime
            }));
            row
        })
        .collect())
}

fn detail(conn: &Connection, v: &View, req: &Request, revision: &str) -> Result<Json> {
    let limit = req.limit.unwrap_or(DETAIL);
    if !(1..=DETAIL_MAX).contains(&limit) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("limit must be 1 to {DETAIL_MAX} for a revision's detail"),
        ));
    }
    let missing = || {
        fail(
            ErrorKind::InvalidArguments,
            format!("workspace {} has no revision {revision}", v.workspace),
        )
    };
    let id = unhex(revision)
        .filter(|id| id.len() == 16)
        .ok_or_else(missing)?;
    let state: String = conn
        .query_row(
            "SELECT detail FROM revisions WHERE id = ?1
             AND source_id IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR scope = ?3)",
            params![id, json!(v.sources).to_string(), v.scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(missing)?;
    let after = match req.cursor {
        Some(c) => {
            let p = payload(v, "d1", c, 24)?;
            if p[..16] != id[..] {
                return Err(stale());
            }
            i64::from_be_bytes(p[16..].try_into().unwrap())
        }
        None => -1,
    };
    let mut stmt = conn
        .prepare(
            "SELECT ord, frame, node, kind, change, facets, descendants FROM revision_changes
             WHERE rev = ?1 AND ord > ?2 ORDER BY ord LIMIT ?3",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![id, after, limit as i64 + 1], |r| {
            let facets: u8 = r.get(5)?;
            let descendants: i64 = r.get(6)?;
            let mut node = json!({"frame": r.get::<_, Option<String>>(1)?,
                "id": r.get::<_, String>(2)?, "kind": r.get::<_, String>(3)?,
                "change": r.get::<_, String>(4)?});
            if node["change"] == "changed" {
                node["facets"] = json!(diff::facet_names(facets));
                if facets & diff::UNREAD != 0 {
                    node["unread"] = json!(true);
                }
            }
            if descendants > 0 {
                node["descendants"] = json!(descendants);
            }
            Ok((r.get::<_, i64>(0)?, node))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let (mut page, mut used, mut last) = (Vec::new(), 0, None);
    for (ord, node) in rows.iter().take(limit) {
        let size = node.to_string().len();
        if !page.is_empty() && used + size > PAGE_BYTES {
            break;
        }
        used += size;
        page.push(node.clone());
        last = Some(*ord);
    }
    let next = match page.len() < rows.len() {
        true => cursor(v, "d1", &[&id[..], &last.unwrap().to_be_bytes()].concat()),
        false => Json::Null,
    };
    Ok(
        json!({"workspaceId": v.workspace, "revisionId": revision, "detail": state,
        "nodes": page, "nextCursor": next}),
    )
}

/// Moves `consumer`'s checkpoint for these filters forward to `cursor`; an older or equal
/// cursor changes nothing. Replies with the checkpoint now held.
pub fn ack(storage: &Storage, select: &Select, req: &Request) -> Result<Json> {
    let consumer = req
        .consumer
        .filter(|c| !c.is_empty() && c.len() <= 128)
        .ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                "pass consumer: a name of 1 to 128 bytes",
            )
        })?;
    let text = req.cursor.ok_or_else(|| {
        fail(
            ErrorKind::InvalidArguments,
            "pass cursor: the nextCursor of the last changes page handled",
        )
    })?;
    let (_, mut ws) = opened(storage, select, true)?;
    let v = view(storage, &ws, req)?;
    let id = payload(&v, "c1", text, 16)?;
    let tx = ws
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    let seq = position(&tx, &v, &id)?;
    let advanced = tx.execute(
        "INSERT INTO consumers (consumer, filter, after_seq, updated_ms) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (consumer, filter) DO UPDATE SET after_seq = excluded.after_seq,
         updated_ms = excluded.updated_ms WHERE excluded.after_seq > after_seq",
        params![consumer, v.filter, seq, now_ms()],
    )
    .map_err(sql)?
        > 0;
    let at: i64 = tx
        .query_row(
            "SELECT after_seq FROM consumers WHERE consumer = ?1 AND filter = ?2",
            [consumer, &v.filter],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let held: Option<Vec<u8>> = tx
        .query_row("SELECT id FROM revisions WHERE seq = ?1", [at], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(
        json!({"workspaceId": v.workspace, "consumer": consumer, "advanced": advanced,
        "checkpoint": cursor(&v, "c1", held.as_deref().unwrap_or(&OLDEST))}),
    )
}
