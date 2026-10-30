//! Watching (#92 B2): temporary leases on tracked scopes, and when the shared runner refreshes
//! one source. `Detector` is pure state with no I/O: the runner polls the source's stamp, feeds
//! each result to `Detector::observe`, and submits a refresh when told; that job stages, hashes
//! and compares the bytes, so a same-stamp save is caught there, never read here.
//!
//! A lease is one holder process's interest in one source and scope. Its holder (an MCP server
//! or `watch run`, by its owner token) renews all of its leases in one statement every
//! `RENEW`; a lease not renewed within `TTL` is over for everyone, and only ever ends.
// shortcut: no runner calls the detector yet; it lands with the runner in the next B2 commit.
#![cfg_attr(not(test), allow(dead_code))]

use std::time::{Duration, Instant};

use rusqlite::{TransactionBehavior, params};

use super::jobs::{chosen, owner, random_hex};
use super::*;

/// How often the runner stats a watched source.
pub(crate) const POLL: Duration = Duration::from_millis(250);
/// A changed stamp must hold this long before its save counts as finished.
const STABLE: Duration = Duration::from_millis(300);
/// Continuous churn refreshes anyway after this; the job's staged read retries a moving file.
const CHURN: Duration = Duration::from_millis(1200);
/// A save can keep mtime and length (or replace the inode with equal ones): the content is
/// verified this often, so such a change is seen within this bound plus the refresh.
pub(crate) const VERIFY: Duration = Duration::from_secs(60);
/// A missing source (mid replace-save) is reported only once it stays missing this long.
const MISSING: Duration = Duration::from_secs(2);
/// A busy (locked, access-denied) source is `SourceBusy` after this, `SourceUnreadable` after
/// `UNREADABLE`; it is retried with a backoff doubling from `POLL` up to `BACKOFF`.
const BUSY: Duration = Duration::from_secs(2);
const UNREADABLE: Duration = Duration::from_secs(60);
const BACKOFF: Duration = Duration::from_secs(5);

/// One stat of the source.
pub(crate) enum Probe<S> {
    Stamp(S),
    Missing,
    /// Locked or access denied: transient until `UNREADABLE`.
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Watching,
    SourceBusy,
    SourceMissing,
    SourceUnreadable,
}

/// What the runner does after one observation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// Submit one content-verified refresh now.
    pub submit: bool,
    pub state: State,
    /// When to probe again.
    pub next: Instant,
}

pub(crate) struct Detector<S> {
    /// The stamp the last submitted refresh started from; `None` forces one, so a runner that
    /// (re)starts verifies saves made while nobody watched.
    seen: Option<S>,
    /// A newer stamp, since when it held, and since when stamps kept changing.
    pending: Option<(S, Instant, Instant)>,
    verify_at: Instant,
    missing: Option<Instant>,
    busy: Option<(Instant, Duration)>,
    running: bool,
    dirty: bool,
}

impl<S: Clone + PartialEq> Detector<S> {
    pub(crate) fn new(now: Instant) -> Self {
        Detector {
            seen: None,
            pending: None,
            verify_at: now,
            missing: None,
            busy: None,
            running: false,
            dirty: false,
        }
    }

    pub(crate) fn observe(&mut self, now: Instant, probe: Probe<S>) -> Step {
        let stamp = match probe {
            Probe::Stamp(stamp) => stamp,
            Probe::Missing => {
                self.busy = None;
                let since = *self.missing.get_or_insert(now);
                let state = if now - since >= MISSING {
                    State::SourceMissing
                } else {
                    State::Watching
                };
                return self.wait(now + POLL, state);
            }
            Probe::Busy => {
                self.missing = None;
                let (since, delay) = match self.busy {
                    Some((since, delay)) => (since, (delay * 2).min(BACKOFF)),
                    None => (now, POLL),
                };
                self.busy = Some((since, delay));
                let state = match now - since {
                    held if held >= UNREADABLE => State::SourceUnreadable,
                    held if held >= BUSY => State::SourceBusy,
                    _ => State::Watching,
                };
                return self.wait(now + delay, state);
            }
        };
        self.missing = None;
        self.busy = None;
        if self.seen.as_ref() != Some(&stamp) {
            match &mut self.pending {
                Some((held, since, _)) if *held != stamp => {
                    *held = stamp.clone();
                    *since = now;
                }
                Some(_) => {}
                None => self.pending = Some((stamp.clone(), now, now)),
            }
        } else {
            self.pending = None;
        }
        let settled = self
            .pending
            .as_ref()
            .is_some_and(|(_, since, churn)| now - *since >= STABLE || now - *churn >= CHURN);
        if settled || (self.pending.is_none() && now >= self.verify_at) {
            self.pending = None;
            self.seen = Some(stamp);
            return self.request(now);
        }
        self.wait(now + POLL, State::Watching)
    }

    /// The submitted refresh ended; `failed` (the file kept changing, or any job error) makes
    /// the next stable stamp refresh again. Returns the follow-up for a change seen meanwhile.
    pub(crate) fn finished(&mut self, now: Instant, failed: bool) -> Step {
        self.running = false;
        if failed {
            self.seen = None;
        }
        if std::mem::take(&mut self.dirty) {
            return self.request(now);
        }
        self.wait(now + POLL, State::Watching)
    }

    fn request(&mut self, now: Instant) -> Step {
        self.verify_at = now + VERIFY;
        if self.running {
            self.dirty = true;
            return self.wait(now + POLL, State::Watching);
        }
        self.running = true;
        Step {
            submit: true,
            state: State::Watching,
            next: now + POLL,
        }
    }

    fn wait(&self, next: Instant, state: State) -> Step {
        Step {
            submit: false,
            state,
            next,
        }
    }
}

/// How often a holder renews its leases, and how long one lasts unrenewed.
pub const RENEW: Duration = Duration::from_secs(10);
pub const TTL: Duration = Duration::from_secs(30);
/// Live leases per workspace, per holder process, and distinct sources they watch.
const MAX_LEASES: i64 = 64;
const MAX_HELD: i64 = 16;
const MAX_SOURCES: i64 = 16;

/// A row that counts: not ended and renewed within `TTL`.
const LIVE: &str = "ended IS NULL AND expires_ms > ?1";

/// Workspaces this process holds leases in, so one renewal pass and one exit release reach
/// them all.
static HELD: std::sync::Mutex<Vec<(Storage, String)>> = std::sync::Mutex::new(Vec::new());

fn remember_held(storage: &Storage, ws_id: &str) {
    if let Ok(mut held) = HELD.lock()
        && !held.iter().any(|(_, id)| id == ws_id)
    {
        held.push((storage.clone(), ws_id.to_owned()));
    }
}

fn lease_json(ws_id: &str, lease: &str, source: &str, scope: &str, expires: i64) -> Json {
    json!({"workspaceId": ws_id, "leaseId": lease, "sourceId": source, "scope": scope,
        "state": "active", "renewEveryMs": RENEW.as_millis() as u64, "expiresAtMs": expires})
}

/// `watch start`: a lease on tracked `scope` of one source for this process, which must renew
/// it (see `renew`). A `request_id` replays its lease while it lives, and reports it ended
/// after that; it never starts an ended lease again.
pub fn start(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
    request_id: Option<&str>,
) -> Result<Json> {
    let (_, mut ws) = opened(storage, select, true)?;
    let ws_id = ws.held.id.clone();
    let holder = owner(&ws.held.dir)?;
    let sources = chosen(storage, &ws, source)?;
    let [source_id] = sources.as_slice() else {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {ws_id} has more than one source; pass the sourceId to watch"),
        ));
    };
    let print = crate::search::hex(&Sha256::digest(
        json!([ws_id, source_id, scope]).to_string(),
    ))[..16]
        .to_owned();
    let now = now_ms();
    let incarnation = ws.incarnation.clone();
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    if let Some(request) = request_id {
        let prior: Option<(String, String, String, i64, Option<String>)> = tx
            .query_row(
                "SELECT lease_id, fingerprint, holder, expires_ms, ended FROM watch_leases
                 WHERE request_id = ?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(sql)?;
        if let Some((lease, prior_print, prior_holder, expires, ended)) = prior {
            if prior_print != print {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!("requestId {request} names a different watch"),
                ));
            }
            if ended.is_none() && expires > now && prior_holder == holder {
                return Ok(lease_json(&ws_id, &lease, source_id, scope, expires));
            }
            let why = ended.unwrap_or_else(|| "expired".into());
            return Ok(
                json!({"workspaceId": ws_id, "leaseId": lease, "sourceId": source_id,
                "scope": scope, "state": "ended", "reason": why,
                "nextStep": {"tool": "watch", "arguments": {"action": "start",
                    "workspaceId": ws_id, "sourceId": source_id, "scope": scope},
                    "why": "an ended lease never starts again; start a new one with a new requestId"}}),
            );
        }
    }
    let tracked: Option<String> = tx
        .query_row(
            "SELECT state FROM tracking WHERE source_id = ?1 AND scope = ?2",
            [source_id, scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if !matches!(tracked.as_deref(), Some("active" | "baseline_pending")) {
        return Err(FigError::new(
            ErrorKind::InvalidArguments,
            format!("source {source_id} scope {scope} is not tracked; track it before watching it"),
        )
        .with_recovery(json!([{"action": "track", "requires": "none", "tool": "index",
            "arguments": {"action": "track", "workspaceId": ws_id, "sourceId": source_id, "scope": scope}}]))
        .into());
    }
    let path: String = tx
        .query_row(
            "SELECT path FROM sources WHERE source_id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let (all, mine, others): (i64, i64, i64) = tx
        .query_row(
            &format!(
                "SELECT count(*), count(*) FILTER (WHERE holder = ?2),
                 count(DISTINCT source_id) FILTER (WHERE source_id != ?3)
                 FROM watch_leases WHERE {LIVE}"
            ),
            params![now, holder, source_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(sql)?;
    if all >= MAX_LEASES || mine >= MAX_HELD || others >= MAX_SOURCES {
        return Err(fail(
            ErrorKind::WorkspaceBusy,
            format!(
                "workspace {ws_id} is at a watch limit ({MAX_LEASES} leases, {MAX_HELD} per process, {MAX_SOURCES} sources); stop a lease first"
            ),
        ));
    }
    let lease = random_hex();
    let expires = now + TTL.as_millis() as i64;
    tx.execute(
        "INSERT INTO watch_leases (lease_id, holder, source_id, scope, incarnation,
         authorized_path, request_id, fingerprint, renewed_ms, expires_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            lease,
            holder,
            source_id,
            scope,
            incarnation,
            path,
            request_id,
            print,
            now,
            expires
        ],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    remember_held(storage, &ws_id);
    Ok(lease_json(&ws_id, &lease, source_id, scope, expires))
}

/// One renewal of every live lease this process holds in workspace `ws_id`, in one statement.
/// Each lease is re-authorized against this process's own roots: a source it no longer sees
/// (or that moved) is kept but unauthorized, so the runner stops reading it for this holder.
/// Returns how many leases it renewed.
fn renew_one(storage: &Storage, ws_id: &str) -> Result<usize> {
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let holder = owner(&ws.held.dir)?;
    let now = now_ms();
    let paths: Vec<(String, String)> = ws
        .conn
        .prepare(&format!(
            "SELECT DISTINCT s.source_id, s.path FROM watch_leases l JOIN sources s
             ON s.source_id = l.source_id WHERE {LIVE} AND l.holder = ?2"
        ))
        .and_then(|mut s| {
            s.query_map(params![now, holder], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .map_err(sql)?;
    let seen: Vec<Json> = paths
        .into_iter()
        .filter(|(_, path)| storage.sees(Path::new(path)))
        .map(|(id, path)| json!([id, path]))
        .collect();
    let seen = Json::Array(seen).to_string();
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let renewed = tx
        .execute(
            &format!(
                "UPDATE watch_leases SET renewed_ms = ?1, expires_ms = ?3, authorized_path =
                 (SELECT json_extract(value, '$[1]') FROM json_each(?4)
                  WHERE json_extract(value, '$[0]') = watch_leases.source_id)
                 WHERE {LIVE} AND holder = ?2"
            ),
            params![now, holder, now + TTL.as_millis() as i64, seen],
        )
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(renewed)
}

/// Renews this process's leases in every workspace it holds them in; a workspace whose
/// renewal fails keeps its leases until they expire, which is the safe side.
pub fn renew() -> Vec<(String, Result<usize>)> {
    let held = HELD.lock().map(|h| h.clone()).unwrap_or_default();
    held.iter()
        .map(|(storage, id)| (id.clone(), renew_one(storage, id)))
        .collect()
}

/// `watch stop`: ends one lease this process holds.
pub fn stop(storage: &Storage, select: &Select, lease_id: &str) -> Result<Json> {
    let (_, ws) = opened(storage, select, true)?;
    let holder = owner(&ws.held.dir)?;
    let ended = ws
        .conn
        .execute(
            "UPDATE watch_leases SET ended = 'stopped', authorized_path = NULL
             WHERE lease_id = ?1 AND holder = ?2 AND ended IS NULL",
            params![lease_id, holder],
        )
        .map_err(sql)?;
    if ended == 0 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("this process holds no live lease {lease_id}"),
        ));
    }
    Ok(
        json!({"workspaceId": ws.held.id, "leaseId": lease_id, "state": "ended", "reason": "stopped"}),
    )
}

/// At this process's exit: ends every lease it holds, in each workspace in one statement.
/// It never stops a runner; one with no live lease left exits by itself.
pub fn release() {
    let held = HELD.lock().map(|h| h.clone()).unwrap_or_default();
    for (storage, id) in held {
        if let Ok((_, ws)) = opened(&storage, &Select::Id(id), true)
            && let Ok(holder) = owner(&ws.held.dir)
        {
            let _ = ws.conn.execute(
                "UPDATE watch_leases SET ended = 'released', authorized_path = NULL
                 WHERE holder = ?1 AND ended IS NULL",
                [holder],
            );
        }
    }
}

/// `watch status`: this workspace's leases on sources the caller sees, read-only.
pub fn status(storage: &Storage, select: &Select) -> Result<Json> {
    let (_, ws) = opened(storage, select, false)?;
    let me = owner(&ws.held.dir).ok();
    let now = now_ms();
    let mut leases = Vec::new();
    if ws.version >= 5 {
        let mut stmt = ws
            .conn
            .prepare(
                "SELECT l.lease_id, l.holder, l.source_id, l.scope, l.expires_ms, l.ended,
                 l.authorized_path IS NOT NULL AND l.authorized_path = s.path, s.path
                 FROM watch_leases l LEFT JOIN sources s ON s.source_id = l.source_id
                 ORDER BY l.renewed_ms DESC, l.lease_id LIMIT ?1",
            )
            .map_err(sql)?;
        let rows = stmt
            .query_map([MAX_LEASES * 4], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (lease, holder, source, scope, expires, ended, authorized, path) =
                row.map_err(sql)?;
            if !path.is_some_and(|p| storage.sees(Path::new(&p))) {
                continue;
            }
            let state = match (&ended, expires > now, authorized) {
                (Some(_), ..) => "ended",
                (None, false, _) => "expired",
                (None, true, Some(true)) => "active",
                (None, true, _) => "unauthorized",
            };
            let mut lease = json!({"leaseId": lease, "sourceId": source, "scope": scope,
                "state": state, "expiresAtMs": expires, "mine": me.as_deref() == Some(&holder)});
            if let Some(reason) = ended {
                lease["reason"] = json!(reason);
            }
            leases.push(lease);
        }
    }
    Ok(json!({"workspaceId": ws.held.id, "leases": leases}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use crate::workspace::{PrepareOptions, prepare, resolve};
    use std::ffi::OsString;

    /// A prepared workspace with `n` copies of the public fixture, each source's scope `0:1`
    /// marked tracked; its storage, selector, directory and source ids.
    fn tracked(dir: &TestDir, n: usize) -> (Storage, Select, PathBuf, Vec<String>) {
        let home = std::path::absolute(dir.join("home")).unwrap();
        let home = home.to_str().unwrap().to_owned();
        let vars = move |name: &str| (name == "HOME").then(|| OsString::from(&home));
        let storage = resolve(None, &vars, false, &|| None).unwrap();
        let project = std::path::absolute(dir.join("project")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let mut select = None;
        for i in 0..n {
            let fig = project.join(format!("{i}.fig"));
            std::fs::copy("examples/first-success/design.fig", &fig).unwrap();
            let report =
                prepare(&storage, &fig, Some(&project), PrepareOptions::default()).unwrap();
            let id = report.report["workspaceId"].as_str().unwrap().to_owned();
            select = Some(Select::Id(id));
        }
        let select = select.expect("n > 0");
        let (_, ws) = opened(&storage, &select, true).unwrap();
        let ids: Vec<String> = ws
            .conn
            .prepare("SELECT source_id FROM sources ORDER BY added_at_ms, source_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for id in &ids {
            ws.conn
                .execute(
                    "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
                     VALUES (?1, '0:1', 'active', 1, 'test', 0)",
                    [id],
                )
                .unwrap();
        }
        (storage, select, ws.held.dir.clone(), ids)
    }

    fn rows(dir: &Path, q: &str) -> i64 {
        let conn = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row(q, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_request_id_replays_its_live_lease_and_never_restarts_an_ended_one() {
        let dir = TestDir::new();
        let (storage, select, _, ids) = tracked(&dir, 1);
        let first = start(&storage, &select, None, "0:1", Some("r1")).unwrap();
        let again = start(&storage, &select, None, "0:1", Some("r1")).unwrap();
        assert_eq!(
            (again["leaseId"].clone(), again["state"].clone()),
            (first["leaseId"].clone(), json!("active"))
        );
        stop(&storage, &select, first["leaseId"].as_str().unwrap()).unwrap();
        let ended = start(&storage, &select, Some(&ids[0]), "0:1", Some("r1")).unwrap();
        assert_eq!(
            (ended["state"].clone(), ended["reason"].clone()),
            (json!("ended"), json!("stopped"))
        );
    }

    #[test]
    fn a_request_id_for_another_scope_is_refused() {
        let dir = TestDir::new();
        let (storage, select, dir, ids) = tracked(&dir, 1);
        open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap()
            .execute(
                "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
                 VALUES (?1, '0:2', 'active', 1, 'test', 0)",
                [&ids[0]],
            )
            .unwrap();
        start(&storage, &select, None, "0:1", Some("r1")).unwrap();
        assert!(start(&storage, &select, None, "0:2", Some("r1")).is_err());
    }

    #[test]
    fn an_untracked_scope_is_refused_with_the_track_step() {
        let dir = TestDir::new();
        let (storage, select, ..) = tracked(&dir, 1);
        let e = start(&storage, &select, None, "0:9", None).unwrap_err();
        let fig = e.downcast_ref::<FigError>().unwrap();
        assert_eq!(fig.recovery().unwrap()[0]["action"], "track");
    }

    #[test]
    fn a_holder_stops_only_its_own_lease() {
        let dir = TestDir::new();
        let (storage, select, dir, _) = tracked(&dir, 1);
        let lease = start(&storage, &select, None, "0:1", None).unwrap();
        open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap()
            .execute("UPDATE watch_leases SET holder = 'other'", [])
            .unwrap();
        assert!(stop(&storage, &select, lease["leaseId"].as_str().unwrap()).is_err());
    }

    #[test]
    fn renewal_extends_this_holders_leases_and_skips_ended_ones() {
        let dir = TestDir::new();
        let (storage, select, dir, ids) = tracked(&dir, 2);
        let a = start(&storage, &select, Some(&ids[0]), "0:1", None).unwrap();
        start(&storage, &select, Some(&ids[1]), "0:1", None).unwrap();
        stop(&storage, &select, a["leaseId"].as_str().unwrap()).unwrap();
        open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap()
            .execute("UPDATE watch_leases SET expires_ms = expires_ms - 1", [])
            .unwrap();
        let ws = select_id(&select);
        let renewed = renew();
        assert_eq!(
            renewed
                .iter()
                .find(|(id, _)| *id == ws)
                .unwrap()
                .1
                .as_ref()
                .unwrap(),
            &1
        );
    }

    #[test]
    fn a_lease_on_a_source_outside_the_roots_is_unauthorized_at_renewal() {
        let dir = TestDir::new();
        let (storage, select, dir, _) = tracked(&dir, 1);
        start(&storage, &select, None, "0:1", None).unwrap();
        let elsewhere = std::path::absolute(dir.join("elsewhere")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let confined = storage
            .clone()
            .confined(&[elsewhere.canonicalize().unwrap()]);
        renew_one(&confined, &select_id(&select)).unwrap();
        assert_eq!(
            rows(
                &dir,
                "SELECT count(*) FROM watch_leases WHERE authorized_path IS NULL"
            ),
            1
        );
        let status = status(&storage, &select).unwrap();
        assert_eq!(status["leases"][0]["state"], "unauthorized");
    }

    #[test]
    fn status_writes_nothing() {
        let dir = TestDir::new();
        let (storage, select, dir, _) = tracked(&dir, 1);
        start(&storage, &select, None, "0:1", None).unwrap();
        let before = std::fs::read(dir.join(DB)).unwrap();
        let wal = std::fs::metadata(dir.join(format!("{DB}-wal")))
            .map(|m| m.len())
            .ok();
        let report = status(&storage, &select).unwrap();
        assert_eq!(report["leases"][0]["state"], "active");
        assert_eq!(std::fs::read(dir.join(DB)).unwrap(), before);
        // A reader may create the empty `-wal` file; it never adds frames.
        let after = std::fs::metadata(dir.join(format!("{DB}-wal"))).map_or(0, |m| m.len());
        assert_eq!(after, wal.unwrap_or(0));
    }

    #[test]
    fn removing_a_source_ends_its_leases() {
        let dir = TestDir::new();
        let (storage, select, dir, ids) = tracked(&dir, 2);
        start(&storage, &select, Some(&ids[0]), "0:1", None).unwrap();
        crate::workspace::remove(&storage, &select, &ids[0]).unwrap();
        assert_eq!(
            rows(
                &dir,
                "SELECT count(*) FROM watch_leases WHERE ended = 'source_removed'"
            ),
            1
        );
    }

    fn select_id(select: &Select) -> String {
        match select {
            Select::Id(id) => id.clone(),
            Select::Locator(_) => unreachable!("tests select by id"),
        }
    }

    /// Feeds `probes` at `POLL` steps from `t0`; returns the times (ms after `t0`) it submitted.
    fn run(d: &mut Detector<u32>, t0: Instant, probes: &[(u64, Option<u32>)]) -> Vec<u64> {
        let mut submits = Vec::new();
        for &(ms, stamp) in probes {
            let probe = stamp.map_or(Probe::Missing, Probe::Stamp);
            if d.observe(t0 + Duration::from_millis(ms), probe).submit {
                submits.push(ms);
                d.finished(t0 + Duration::from_millis(ms), false);
            }
        }
        submits
    }

    fn ticks(from: u64, to: u64, stamp: Option<u32>) -> Vec<(u64, Option<u32>)> {
        (from..to).step_by(250).map(|ms| (ms, stamp)).collect()
    }

    #[test]
    fn a_new_detector_verifies_the_source_once_its_stamp_holds() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        assert_eq!(run(&mut d, t0, &ticks(0, 3000, Some(1))), [500]);
    }

    #[test]
    fn a_save_refreshes_once_after_its_stamp_holds() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        assert_eq!(run(&mut d, t0, &ticks(1000, 4000, Some(2))), [1500]);
    }

    #[test]
    fn churn_refreshes_at_the_cap_instead_of_waiting_forever() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        let churn: Vec<_> = (0..8)
            .map(|i| (1000 + i * 250, Some(10 + i as u32)))
            .collect();
        assert_eq!(run(&mut d, t0, &churn), [2250]);
    }

    #[test]
    fn an_unchanged_stamp_is_content_verified_every_interval() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        let every = VERIFY.as_millis() as u64;
        assert_eq!(
            run(&mut d, t0, &ticks(0, 2 * every + 750, Some(1))),
            [500, 500 + every, 500 + 2 * every]
        );
    }

    #[test]
    fn a_change_during_a_refresh_gives_exactly_one_follow_up() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        assert!(!d.observe(at(0), Probe::Stamp(1)).submit);
        assert!(d.observe(at(300), Probe::Stamp(1)).submit);
        for (ms, stamp) in [(600, 2), (1000, 2), (1300, 3), (1700, 3)] {
            assert!(!d.observe(at(ms), Probe::Stamp(stamp)).submit);
        }
        assert!(d.finished(at(1800), false).submit);
        assert!(!d.finished(at(2000), false).submit);
    }

    #[test]
    fn a_failed_refresh_is_retried_on_the_next_stable_stamp() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        d.observe(at(0), Probe::Stamp(1));
        assert!(d.observe(at(300), Probe::Stamp(1)).submit);
        d.finished(at(400), true);
        assert!(!d.observe(at(500), Probe::Stamp(1)).submit);
        assert!(d.observe(at(800), Probe::Stamp(1)).submit);
    }

    #[test]
    fn a_missing_source_is_silent_while_brief_then_reported_without_a_refresh() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        let brief = d.observe(at(1000), Probe::Missing);
        assert_eq!((brief.submit, brief.state), (false, State::Watching));
        let long = d.observe(at(3000), Probe::Missing);
        assert_eq!((long.submit, long.state), (false, State::SourceMissing));
        // A replace-save that comes back with a new stamp refreshes once it holds.
        assert_eq!(run(&mut d, t0, &ticks(3250, 4500, Some(2))), [3750]);
    }

    #[test]
    fn a_busy_source_backs_off_to_the_cap_and_never_refreshes() {
        let t0 = Instant::now();
        let mut d: Detector<u32> = Detector::new(t0);
        let mut now = t0;
        let mut seen = Vec::new();
        while now - t0 < UNREADABLE + BACKOFF {
            let step = d.observe(now, Probe::Busy);
            assert!(!step.submit);
            seen.push((step.next - now, step.state));
            now = step.next;
        }
        let delays: Vec<_> = seen.iter().map(|(d, _)| d.as_millis()).take(6).collect();
        assert_eq!(delays, [250, 500, 1000, 2000, 4000, 5000]);
        assert!(seen.iter().all(|(d, _)| *d <= BACKOFF));
        assert_eq!(seen.first().unwrap().1, State::Watching);
        assert!(seen.iter().any(|(_, s)| *s == State::SourceBusy));
        assert_eq!(seen.last().unwrap().1, State::SourceUnreadable);
    }
}
