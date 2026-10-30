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
/// Tracked scopes listed in a poll; past it `trackingTruncated`.
const TRACKING: usize = 100;
/// The retention marks one request may read, each charged 64 bytes plus its key.
const MARK_BYTES: usize = 4 * PAGE_BYTES;
/// The cursor of "the oldest revision kept": a zero revision id.
const OLDEST: [u8; 16] = [0; 16];

mod queries;
pub mod subscriptions;

#[cfg(test)]
pub(crate) use queries::note;
pub use queries::{Continuation, evaluate, evaluate_published};

#[derive(Default)]
pub struct Request<'a> {
    pub source: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub consumer: Option<&'a str>,
    pub cursor: Option<&'a str>,
    pub revision: Option<&'a str>,
    pub limit: Option<usize>,
    /// A subscription's own filters and checkpoint, in place of consumer, source and scope.
    pub subscription: Option<&'a str>,
}

/// The sources a reply may name, and what its cursors are bound to.
struct View {
    workspace: String,
    sources: Vec<String>,
    source: Option<String>,
    scope: Option<String>,
    filter: String,
    key: Option<String>,
    bind: Vec<u8>,
    sub: Option<subscriptions::Sub>,
    /// Schema 7 records impact; an older workspace is read as it is, never upgraded.
    impacts: bool,
}

fn view(storage: &Storage, ws: &Opened, req: &Request) -> Result<View> {
    let sub = match req.subscription {
        Some(_) if req.consumer.is_some() || req.source.is_some() || req.scope.is_some() => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                "subscriptionId carries its own consumer, sourceId and scope; pass none of them with it",
            ));
        }
        Some(id) => Some(subscriptions::visible(storage, ws, id)?),
        None => None,
    };
    // A set's view is its members, which `visible` found all seen; its filter names the set.
    let source = sub.as_ref().map_or(req.source, |s| Some(s.source.as_str()));
    let sources = match sub.as_ref().filter(|s| s.is_set()) {
        Some(s) => s.members.clone().unwrap_or_default(),
        None => jobs::chosen(storage, ws, source)?,
    };
    let source = source.filter(|s| *s != subscriptions::SET);
    // Named as `index track` names it: `1-2` is `1:2`.
    let scope = match &sub {
        Some(s) => s.scope.clone(),
        None => req.scope.map(crate::normalize_id),
    };
    // A checkpoint is kept per view too: progress over more sources must not skip a narrower
    // view's revisions. A subscription's cursors are bound to it and its filter version.
    let filter = format!(
        "{}{}\u{1f}{}\u{1f}{}",
        sub.as_ref().map_or(String::new(), |s| format!(
            "sub\u{1f}{}\u{1f}{}\u{1f}",
            s.id,
            subscriptions::FILTER_VERSION
        )),
        source.unwrap_or(""),
        scope.as_deref().unwrap_or(""),
        crate::search::hex(&Sha256::digest(sources.join("\u{1f}")))
    );
    // Schema 4 introduced change tracking; a read never upgrades.
    let key = match ws.version >= 4 {
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
    let bind = [ws.incarnation.as_bytes(), filter.as_bytes()].join(&0x1f);
    Ok(View {
        workspace: ws.held.id.clone(),
        sources,
        source: source.map(str::to_owned),
        scope,
        filter,
        key,
        bind,
        sub,
        impacts: ws.version >= 7,
    })
}

fn mac(v: &View, tag: &str, payload: &[u8]) -> Option<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(v.key.as_ref()?.as_bytes()).ok()?;
    mac.update(&v.bind);
    mac.update(&[tag.as_bytes(), &[0x1f]].concat());
    mac.update(payload);
    Some(mac)
}

fn cursor(v: &View, tag: &str, payload: &[u8]) -> Json {
    match mac(v, tag, payload) {
        Some(m) => json!(format!(
            "{tag}.{}",
            crate::search::hex(&[payload, &m.finalize().into_bytes()[..16]].concat())
        )),
        None => Json::Null,
    }
}

/// A cursor `tag` over `payload` as the view of `req` issues it, so tests can forge older ones.
#[cfg(test)]
pub(crate) fn sign(
    storage: &Storage,
    select: &Select,
    req: &Request,
    tag: &str,
    payload: &[u8],
) -> Json {
    let (_, ws) = opened(storage, select, false).unwrap();
    cursor(&view(storage, &ws, req).unwrap(), tag, payload)
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
        .filter(|t| t.len() == (len + 16) * 2)
        .and_then(unhex)
        .ok_or_else(stale)?;
    let (payload, tail) = raw.split_at(len);
    mac(v, tag, payload)
        .ok_or_else(stale)?
        .verify_truncated_left(tail)
        .map_err(|_| stale())?;
    Ok(payload.to_vec())
}

pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len().is_multiple_of(2) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

/// What the view's retention marks say: their keyed `receipt`, the `newest` deleted seq, and
/// the newest deleted by a publication after a checkpoint's floor.
struct Marks {
    receipt: [u8; 16],
    newest: i64,
    since_floor: i64,
}

/// Streams the view's retention marks, filtered in SQL to its sources and scope, in key order,
/// paying for each before reading its key. A receipt is never made from part of them.
fn marks(conn: &Connection, v: &View, floor: Option<i64>) -> Result<Marks> {
    // One key range per source (an exact key with a scope, so `1:2` never matches `1:20`),
    // read in key order straight off the primary key: no row is read before it is paid for.
    let mut stmt = conn
        .prepare_cached(
            "SELECT length(CAST(key AS BLOB)), key, CAST(value AS INTEGER),
             CAST(substr(value, instr(value, ' ') + 1) AS INTEGER)
             FROM meta WHERE key >= ?1 AND key < ?2 ORDER BY key",
        )
        .map_err(sql)?;
    let mut sources: Vec<&String> = v.sources.iter().collect();
    sources.sort();
    let mut mac = mac(v, "receipt", &[]).ok_or_else(stale)?;
    let (mut spent, mut newest, mut since_floor) = (0usize, 0, 0);
    for source in sources {
        let (low, high) = match &v.scope {
            Some(scope) => {
                let key = format!("pruned\u{1f}{source}\u{1f}{scope}");
                (key.clone(), format!("{key}\u{1}"))
            }
            None => (
                format!("pruned\u{1f}{source}\u{1f}"),
                format!("pruned\u{1f}{source}\u{20}"),
            ),
        };
        let mut rows = stmt.query(params![low, high]).map_err(sql)?;
        while let Some(r) = rows.next().map_err(sql)? {
            spent += 64 + r.get::<_, i64>(0).map_err(sql)?.max(0) as usize;
            if spent > MARK_BYTES {
                return Err(fail(
                    ErrorKind::QueryWorkLimit,
                    format!(
                        "this view's retention marks exceed {MARK_BYTES} bytes; pass a narrower sourceId or scope"
                    ),
                ));
            }
            let key = r.get_ref(1).and_then(|k| Ok(k.as_str()?)).map_err(sql)?;
            let (seq, by): (i64, i64) = (r.get(2).map_err(sql)?, r.get(3).map_err(sql)?);
            mac.update(&[key.as_bytes(), &[0x1f], &seq.to_le_bytes(), &[0x1e]].concat());
            newest = newest.max(seq);
            if floor.is_none_or(|f| by > f) {
                since_floor = since_floor.max(seq);
            }
        }
    }
    let mut receipt = [0; 16];
    receipt.copy_from_slice(&mac.finalize().into_bytes()[..16]);
    Ok(Marks {
        receipt,
        newest,
        since_floor,
    })
}

/// The seq of the newest revision recorded, kept private in a consumer's checkpoint.
fn floor(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT IFNULL((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'revision_seq'), 0)",
        [],
        |r| r.get(0),
    )
    .map_err(sql)
}

/// A `c1.` position cursor: the revision id it follows and the view's receipt when issued.
fn position_cursor(v: &View, id: &[u8], receipt: &[u8; 16]) -> Json {
    cursor(v, "c1", &[id, receipt].concat())
}

fn read_position(v: &View, text: &str) -> Result<(Vec<u8>, [u8; 16])> {
    let p = payload(v, "c1", text, 32)?;
    Ok((p[..16].to_vec(), p[16..].try_into().map_err(|_| stale())?))
}

/// Whether a reader at `at` holding receipt `held` missed a deletion: only when the view was
/// pruned since the receipt was issued, and behind that position.
fn missed(marks: &Marks, held: &[u8; 16], at: i64) -> bool {
    *held != marks.receipt && at < marks.newest
}

/// The position after `id`: its seq, before every revision kept for [`OLDEST`], or `event_gap`
/// when the revision is no longer kept.
fn position(conn: &Connection, v: &View, id: &[u8]) -> Result<i64> {
    if id == OLDEST {
        return Ok(0);
    }
    let seq = conn
        .query_row(
            "SELECT seq FROM revisions WHERE id = ?1
             AND source_id IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR scope = ?3)",
            params![id, json!(v.sources).to_string(), v.scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    seq.ok_or_else(|| gap(conn, v))
}

/// `event_gap`, whose recovery reads from the oldest kept, accepting every deletion so far.
fn gap(conn: &Connection, v: &View) -> Box<dyn Error> {
    let now = match marks(conn, v, None) {
        Ok(marks) => marks.receipt,
        Err(e) => return e,
    };
    // The cursor is bound to these filters, so the recovery repeats them.
    let mut arguments =
        json!({"workspaceId": v.workspace, "cursor": position_cursor(v, &OLDEST, &now)});
    match &v.sub {
        Some(sub) => arguments["subscriptionId"] = json!(sub.id),
        None => {
            if let Some(source) = &v.source {
                arguments["sourceId"] = json!(source);
            }
            if let Some(scope) = &v.scope {
                arguments["scope"] = json!(scope);
            }
        }
    }
    FigError::new(
        ErrorKind::EventGap,
        "revisions after this position were deleted by retention before they were read; read changes again from the oldest kept, and treat the scope as possibly changed",
    )
    .with_recovery(json!([{"action": "changes", "requires": "none", "tool": "changes",
        "arguments": arguments}]))
    .into()
}

/// One page of revisions after the cursor, the consumer's checkpoint or the oldest kept, or
/// with `revision` one page of that revision's changed nodes. Never writes.
pub fn changes(storage: &Storage, select: &Select, req: &Request) -> Result<Json> {
    let (_, ws) = opened(storage, select, false)?;
    // Visible sources and every row read come from one registry snapshot.
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let v = view(storage, &ws, req)?;
    jobs::hold("changes");
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
    match (req.revision, &v.sub) {
        (Some(_), Some(sub)) if sub.is_query() => Err(fail(
            ErrorKind::InvalidArguments,
            "a query subscription records evaluations, not revisions; read it without revision",
        )),
        (Some(revision), _) => detail(&tx, &v, req, revision),
        (None, sub) => {
            let mut reply = match sub {
                Some(sub) if sub.is_query() => queries::poll(&tx, &v, sub, req)?,
                _ => poll(&tx, &v, req)?,
            };
            reply["monitor"] =
                super::watch::monitor(&tx, &ws.held.dir, &v.sources, v.scope.as_deref())?;
            Ok(reply)
        }
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
    let (after, marks) = match (req.cursor, req.consumer) {
        (None, _) if let Some(sub) = &v.sub => {
            let marks = marks(conn, v, Some(sub.floor))?;
            if sub.after_seq < marks.since_floor {
                return Err(gap(conn, v));
            }
            (sub.after_seq, marks)
        }
        (Some(c), _) => {
            let (id, held) = read_position(v, c)?;
            let at = position(conn, v, &id)?;
            let marks = marks(conn, v, None)?;
            // Retention is per scope: a kept cursor can still sit before revisions deleted
            // since it was issued.
            if missed(&marks, &held, at) {
                return Err(gap(conn, v));
            }
            (at, marks)
        }
        (None, Some(consumer)) => {
            let at: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT after_seq, floor FROM consumers WHERE consumer = ?1 AND filter = ?2",
                    [consumer, &v.filter],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(sql)?;
            let marks = marks(conn, v, at.map(|(_, floor)| floor))?;
            match at {
                Some((at, _)) if at < marks.since_floor => return Err(gap(conn, v)),
                at => (at.map_or(0, |(at, _)| at), marks),
            }
        }
        (None, None) => (0, marks(conn, v, None)?),
    };
    let now = marks.receipt;
    // A node is followed through the layers a change reaches, and the top-level frames holding
    // them on either side, as well as the changed ones, and through any revision whose impact
    // is incomplete or was recorded before impact was, since it cannot rule the node out.
    let (impact, reached) = match v.impacts {
        true => (
            "r.impact",
            "OR (r.kind = 'changes' AND r.impact IS NULL)
             OR json_extract(r.impact, '$.complete') IS 0 OR EXISTS
             (SELECT 1 FROM revision_impacts i WHERE i.rev = r.id
             AND (i.node = ?5 OR i.old_frame = ?5 OR i.new_frame = ?5))",
        ),
        false => ("NULL", ""),
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.id, r.source_id, r.scope, r.kind, r.snapshot_id, r.coverage, r.summary,
             r.detail, r.detail_rows, (SELECT p.id FROM revisions p WHERE p.source_id = r.source_id
             AND p.scope = r.scope AND p.gen = r.from_gen AND p.seq < r.seq), {impact}
             FROM revisions r WHERE r.seq > ?1 AND r.source_id IN (SELECT value FROM json_each(?2))
             AND (?3 IS NULL OR r.scope = ?3)
             AND (?5 IS NULL OR r.kind = 'baseline' OR r.detail <> 'complete'
             OR json_extract(r.coverage, '$.complete') IS NOT 1 OR EXISTS
             (SELECT 1 FROM revision_changes c WHERE c.rev = r.id AND (c.node = ?5 OR c.frame = ?5))
             {reached})
             ORDER BY r.seq LIMIT ?4",
        ))
        .map_err(sql)?;
    let rows = stmt
        .query_map(
            params![
                after,
                json!(v.sources).to_string(),
                v.scope,
                limit as i64 + 1,
                v.sub.as_ref().and_then(subscriptions::Sub::node),
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
                        "detail": text(7)?, "detailRows": r.get::<_, i64>(8)?,
                        // Null when nothing was compared or it was recorded before impact was.
                        "impact": r.get::<_, Option<String>>(10)?
                            .and_then(|t| serde_json::from_str::<Json>(&t).ok())}),
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
    let (tracked, truncated) = tracking(conn, v)?;
    let mut reply = json!({
        "workspaceId": v.workspace,
        "revisions": page,
        "nextCursor": last.map_or(Json::Null, |id| position_cursor(v, id, &now)),
        "hasMore": has_more,
        "tracking": tracked,
        "trackingTruncated": truncated,
    });
    if let Some(sub) = &v.sub {
        reply["subscription"] = subscriptions::describe(conn, sub)?;
    }
    Ok(reply)
}

/// Tracking state of the view's sources, with whether the file changed since its last build:
/// at most [`TRACKING`] scopes, and whether more were left out.
fn tracking(conn: &Connection, v: &View) -> Result<(Json, bool)> {
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
             ORDER BY s.added_at_ms, t.source_id, t.scope LIMIT ?3",
        )
        .map_err(sql)?;
    let mut rows = stmt
        .query_map(
            params![json!(v.sources).to_string(), v.scope, TRACKING as i64 + 1],
            |r| {
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
            },
        )
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let truncated = rows.len() > TRACKING;
    rows.truncate(TRACKING);
    let rows = rows
        .into_iter()
        .map(|(mut row, path, bytes, mtime)| {
            row["refreshRequired"] = json!(refresh_required(&path, bytes, mtime));
            row
        })
        .collect();
    Ok((rows, truncated))
}

/// Whether the file at `path` differs from the verified snapshot's size and mtime, or cannot
/// be read: an empty page never proves the file unchanged, only a build records a change.
fn refresh_required(path: &str, bytes: Option<i64>, mtime: Option<i64>) -> bool {
    let stamp = crate::search::stamp(Path::new(path)).ok();
    stamp.is_none_or(|(at, len)| {
        Some(len as i64) != bytes
            || Some(
                at.duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as i64),
            ) != mtime
    })
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
    let id = Some(revision)
        .filter(|r| r.len() == 32)
        .and_then(unhex)
        .ok_or_else(missing)?;
    let state: Option<String> = conn
        .query_row(
            "SELECT detail FROM revisions WHERE id = ?1
             AND source_id IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR scope = ?3)",
            params![id, json!(v.sources).to_string(), v.scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    // Only retention deletes a revision figlens issued, and it leaves a mark.
    let state = match state {
        Some(state) => state,
        None if marks(conn, v, None)?.newest > 0 => return Err(gap(conn, v)),
        None => return Err(missing()),
    };
    // A `d2.` cursor names its section, change rows (0) then the impact list (1), and the last
    // position read in it; a `d1.` cursor, issued before impact, is a change row position.
    let (section, after) = match req.cursor {
        Some(c) if c.starts_with("d1.") => {
            let p = payload(v, "d1", c, 24)?;
            if p[..16] != id[..] {
                return Err(stale());
            }
            (0, i64::from_be_bytes(p[16..].try_into().unwrap()))
        }
        Some(c) => {
            let p = payload(v, "d2", c, 25)?;
            if p[..16] != id[..] || p[16] > 1 {
                return Err(stale());
            }
            (p[16], i64::from_be_bytes(p[17..].try_into().unwrap()))
        }
        None => (0, -1),
    };
    let mut entries = Vec::new();
    if section == 0 {
        entries = changed(conn, &id, after, limit + 1)?;
    }
    if v.impacts && entries.len() <= limit {
        let after = if section == 1 { after } else { -1 };
        entries.extend(impacted(conn, &id, after, limit + 1 - entries.len())?);
    }
    let (mut page, mut used, mut last) = (Vec::new(), 0, None);
    for (at, node) in entries.iter().take(limit) {
        let size = node.to_string().len();
        if !page.is_empty() && used + size > PAGE_BYTES {
            break;
        }
        used += size;
        page.push(node.clone());
        last = Some(*at);
    }
    let next = match (page.len() < entries.len(), last) {
        (true, Some((section, ord))) => {
            cursor(v, "d2", &[&id[..], &[section], &ord.to_be_bytes()].concat())
        }
        _ => Json::Null,
    };
    Ok(
        json!({"workspaceId": v.workspace, "revisionId": revision, "detail": state,
        "nodes": page, "nextCursor": next}),
    )
}

/// Up to `limit` change rows of revision `id` after row `after`, each with its position.
fn changed(
    conn: &Connection,
    id: &[u8],
    after: i64,
    limit: usize,
) -> Result<Vec<((u8, i64), Json)>> {
    let mut stmt = conn
        .prepare(
            "SELECT ord, frame, node, kind, change, facets, descendants FROM revision_changes
             WHERE rev = ?1 AND ord > ?2 ORDER BY ord LIMIT ?3",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![id, after, limit as i64], |r| {
            let facets: u8 = r.get(5)?;
            let descendants: i64 = r.get(6)?;
            let mut node = json!({"entry": "change", "frame": r.get::<_, Option<String>>(1)?,
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
            Ok(((0, r.get::<_, i64>(0)?), node))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(rows)
}

/// Up to `limit` listed layers of revision `id`'s impact after position `after`.
fn impacted(
    conn: &Connection,
    id: &[u8],
    after: i64,
    limit: usize,
) -> Result<Vec<((u8, i64), Json)>> {
    let mut stmt = conn
        .prepare(
            "SELECT ord, node, class, side, changed, unknown, equal, relation, target
             FROM revision_impacts WHERE rev = ?1 AND ord > ?2 ORDER BY ord LIMIT ?3",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![id, after, limit as i64], |r| {
            let (relation, target): (Option<String>, Option<String>) = (r.get(7)?, r.get(8)?);
            let entry = crate::search::impact::entry(
                &r.get::<_, String>(1)?,
                &r.get::<_, String>(2)?,
                r.get(3)?,
                [r.get(4)?, r.get(5)?, r.get(6)?],
                relation.as_deref().zip(target.as_deref()),
            );
            Ok(((1, r.get::<_, i64>(0)?), entry))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(rows)
}

/// Moves `consumer`'s checkpoint for these filters forward to `cursor`; an older or equal
/// cursor normally leaves it, and one that missed a deletion is `event_gap`, writing nothing.
/// A current cursor before a checkpoint behind an unaccepted deletion moves it back instead.
/// Replies with the checkpoint now held.
pub fn ack(storage: &Storage, select: &Select, req: &Request) -> Result<Json> {
    let consumer = req
        .consumer
        .filter(|c| !c.is_empty() && c.len() <= 128)
        .or(req.subscription.map(|_| ""))
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
    let (_, ws) = opened(storage, select, true)?;
    // The write lock is taken first, so visibility and the write share one snapshot.
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    let v = view(storage, &ws, req)?;
    let (id, held) = read_position(&v, text)?;
    if let Some(sub) = &v.sub
        && sub.is_query()
    {
        let reply = queries::ack(&tx, &v, sub, text)?;
        tx.commit().map_err(sql)?;
        return Ok(reply);
    }
    let seq = position(&tx, &v, &id)?;
    let consumer = v.sub.as_ref().map_or(consumer, |s| s.consumer.as_str());
    let before: Option<(i64, i64)> = match &v.sub {
        // Read again inside this write; a source removal since deletes it.
        Some(s) => Some(
            tx.query_row(
                "SELECT after_seq, floor FROM subscriptions WHERE id = ?1",
                [&s.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql)?
            .ok_or_else(stale)?,
        ),
        None => tx
            .query_row(
                "SELECT after_seq, floor FROM consumers WHERE consumer = ?1 AND filter = ?2",
                [consumer, &v.filter],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql)?,
    };
    if before.is_none() {
        subscriptions::admit(&tx, &v.workspace, consumer)?;
    }
    let marks = marks(&tx, &v, before.map(|(_, floor)| floor))?;
    // A stale receipt behind a deletion cannot accept it; nothing is written.
    if missed(&marks, &held, seq) {
        return Err(gap(&tx, &v));
    }
    // A checkpoint behind a deletion it never accepted moves back to a current cursor's place
    // before it: the consumer then repeats revisions, never skips them.
    let reset = before.is_some_and(|(b, _)| seq < b && b < marks.since_floor);
    // Either way the cursor saw every deletion so far, so the private floor moves to now.
    match &v.sub {
        Some(s) => tx.execute(
            "UPDATE subscriptions SET
             after_seq = CASE WHEN ?4 THEN ?2 ELSE MAX(after_seq, ?2) END,
             floor = MAX(floor, ?3), read_ms = ?5 WHERE id = ?1",
            params![s.id, seq, floor(&tx)?, reset, now_ms()],
        ),
        // The visible sources it was written under let `watch forget` authorize its removal.
        None => tx.execute(
            "INSERT INTO consumers (consumer, filter, after_seq, floor, updated_ms, sources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?7) ON CONFLICT (consumer, filter) DO UPDATE SET
             after_seq = CASE WHEN ?6 THEN excluded.after_seq ELSE MAX(after_seq, excluded.after_seq) END,
             floor = MAX(floor, excluded.floor), updated_ms = excluded.updated_ms,
             sources = excluded.sources
             WHERE ?6 OR excluded.after_seq > after_seq OR excluded.floor > floor",
            params![consumer, v.filter, seq, floor(&tx)?, now_ms(), reset, json!(v.sources).to_string()],
        ),
    }
    .map_err(sql)?;
    // Progress is the visible position only, never a private floor refresh.
    let advanced = seq > before.map_or(0, |(b, _)| b);
    let at = match before {
        Some((b, _)) if !reset => b.max(seq),
        _ => seq,
    };
    // The stored floor now accepts every mark, so the current receipt is honest.
    let checkpoint = held_cursor(&tx, &v, at, &marks.receipt)?;
    tx.commit().map_err(sql)?;
    let mut reply = json!({"workspaceId": v.workspace, "consumer": consumer,
        "advanced": advanced, "checkpoint": checkpoint});
    if let Some(s) = &v.sub {
        reply["subscriptionId"] = json!(s.id);
    }
    Ok(reply)
}

/// The cursor after seq `at` in this view, or null when that revision is no longer kept.
fn held_cursor(conn: &Connection, v: &View, at: i64, receipt: &[u8; 16]) -> Result<Json> {
    if at == 0 {
        return Ok(position_cursor(v, &OLDEST, receipt));
    }
    let held: Option<Vec<u8>> = conn
        .query_row(
            "SELECT id FROM revisions WHERE seq = ?1
             AND source_id IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR scope = ?3)",
            params![at, json!(v.sources).to_string(), v.scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    Ok(held.map_or(Json::Null, |id| position_cursor(v, &id, receipt)))
}
