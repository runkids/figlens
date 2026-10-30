//! Watching (#92 B2): temporary leases on tracked scopes, and when the shared runner refreshes
//! one source. `Detector` is pure state with no I/O: the runner polls the source's stamp, feeds
//! each result to `Detector::observe`, and submits a refresh when told; that job stages, hashes
//! and compares the bytes, so a same-stamp save is caught there, never read here.
//!
//! A lease is one holder process's interest in one source and scope. Its holder (an MCP server
//! or `watch run`, by its owner token) renews all of its leases in one statement every
//! `RENEW`; a lease not renewed within `TTL` is over for everyone, and only ever ends.
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use rusqlite::{TransactionBehavior, params};

use super::jobs::{chosen, owner, random_hex};
use super::*;

/// How often the runner stats a watched source; a pending stamp is stat'ed again sooner, at
/// the moment it would settle.
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
    /// Locked or access denied, or now leading elsewhere through a link (never followed):
    /// transient until `UNREADABLE`.
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Watching,
    SourceBusy,
    SourceMissing,
    SourceUnreadable,
    /// The source stats fine but its refresh failed twice running: saves are not indexed until
    /// one succeeds.
    RefreshFailed,
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
    /// Refreshes failed since the last one that succeeded; a good stat never clears it.
    failures: u32,
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
            failures: 0,
        }
    }

    /// A scope newly served on this source may lag its last refresh: the next stable stamp
    /// refreshes again, as for a new detector.
    pub(crate) fn invalidate(&mut self) {
        self.seen = None;
    }

    /// `Watching`, or `RefreshFailed` once a retry failed too, so one save caught mid-write is
    /// not reported.
    fn health(&self) -> State {
        if self.failures >= 2 {
            State::RefreshFailed
        } else {
            State::Watching
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
                    self.health()
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
                    _ => self.health(),
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
        // A pending stamp is probed again exactly when it would settle, not a whole poll late.
        let next = match &self.pending {
            Some((_, since, churn)) => (now + POLL).min(*since + STABLE).min(*churn + CHURN),
            None => now + POLL,
        };
        self.wait(next, self.health())
    }

    /// The submitted refresh ended; `failed` (the file kept changing, or any job error) makes
    /// the next stable stamp refresh again. Returns the follow-up for a change seen meanwhile.
    pub(crate) fn finished(&mut self, now: Instant, failed: bool) -> Step {
        self.running = false;
        if failed {
            self.seen = None;
            self.failures += 1;
        } else {
            self.failures = 0;
        }
        if std::mem::take(&mut self.dirty) {
            return self.request(now);
        }
        self.wait(now + POLL, self.health())
    }

    fn request(&mut self, now: Instant) -> Step {
        self.verify_at = now + VERIFY;
        if self.running {
            self.dirty = true;
            return self.wait(now + POLL, self.health());
        }
        self.running = true;
        Step {
            submit: true,
            state: self.health(),
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
/// A lease of the scope's current tracking epoch (an untrack or retrack starts a new one), on
/// an unaliased `watch_leases`.
const VALID: &str = "EXISTS (SELECT 1 FROM tracking t WHERE t.source_id = watch_leases.source_id
    AND t.scope = watch_leases.scope AND t.epoch = watch_leases.epoch
    AND t.state IN ('active', 'baseline_pending'))";

/// Workspaces this process holds leases in, so one renewal pass and one exit release reach
/// them all, each with the mark of its latest lease start.
static HELD: std::sync::Mutex<Vec<(Storage, String, u64)>> = std::sync::Mutex::new(Vec::new());
static STARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Set under `HELD` by `release_at_signal`: a start not yet in `HELD` then makes no lease.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// False once `release_at_signal` ran, so the caller's lease is never made.
fn remember_held(storage: &Storage, ws_id: &str) -> bool {
    let mark = STARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if let Ok(mut held) = HELD.lock() {
        if CLOSING.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        match held.iter_mut().find(|(_, id, _)| id == ws_id) {
            Some(entry) => entry.2 = mark,
            None => held.push((storage.clone(), ws_id.to_owned(), mark)),
        }
    }
    wake();
    true
}

/// Forgets the workspaces a renewal pass found done, unless a lease started in one since that
/// pass read it: its mark moved, and the next pass renews it.
fn forget(held: &mut Vec<(Storage, String, u64)>, done: &[(String, u64)]) {
    held.retain(|(_, id, mark)| !done.iter().any(|(d, seen)| d == id && seen == mark));
}

/// MCP servers' renewal threads (one per server; tests run several in one process), parked
/// until this process holds a lease.
static RENEWERS: std::sync::Mutex<Vec<std::thread::Thread>> = std::sync::Mutex::new(Vec::new());

/// Wakes the renewal threads, to start renewing or to see that they should stop.
pub fn wake() {
    if let Ok(threads) = RENEWERS.lock() {
        threads.iter().for_each(std::thread::Thread::unpark);
    }
}

/// The MCP server's renewal loop: renews every `RENEW` while this process holds leases and
/// parks, touching no database, until `stop` (after `wake`). Nothing runs before a start.
pub fn renew_until(stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    let me = std::thread::current();
    if let Ok(mut threads) = RENEWERS.lock() {
        threads.push(me.clone());
    }
    while !stop.load(Ordering::SeqCst) {
        if HELD.lock().map_or(true, |h| h.is_empty()) {
            std::thread::park();
            continue;
        }
        std::thread::park_timeout(RENEW);
        if !stop.load(Ordering::SeqCst) {
            // A failed renewal keeps the lease until it expires; the next one retries.
            let _ = renew();
        }
    }
    if let Ok(mut threads) = RENEWERS.lock() {
        threads.retain(|t| t.id() != me.id());
    }
}

fn lease_json(ws_id: &str, lease: &str, source: &str, scope: &str, expires: i64) -> Json {
    json!({"workspaceId": ws_id, "leaseId": lease, "sourceId": source, "scope": scope,
        "state": "active", "renewEveryMs": RENEW.as_millis() as u64, "expiresAtMs": expires})
}

/// `watch start`: a lease on tracked `scope` of one source for this process, which must renew
/// it (see `renew`), and the workspace's runner when none runs. A `request_id` replays its
/// lease while it lives, and reports it ended after that; it never starts an ended lease again.
pub fn start(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
    request_id: Option<&str>,
) -> Result<Json> {
    let scope = &crate::normalize_id(scope);
    let (mut reply, dir, created) = lease(storage, select, source, scope, request_id)?;
    if reply["state"] == "active" {
        let ws_id = reply["workspaceId"].as_str().unwrap_or_default().to_owned();
        match ensure_runner(storage, &ws_id, &dir) {
            Ok(runner) => reply["runner"] = runner,
            // The caller gets no leaseId to stop, so the lease this call made ends here; a
            // replayed one stays with its earlier start.
            Err(e) if created => {
                if let Some(id) = reply["leaseId"].as_str() {
                    let _ = end(storage, &ws_id, id, "start_failed");
                }
                return Err(e);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(reply)
}

/// Ends one lease of this process with `reason`.
fn end(storage: &Storage, ws_id: &str, lease_id: &str, reason: &str) -> Result<usize> {
    let (_, ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let holder = owner(&ws.held.dir)?;
    ws.conn
        .execute(
            "UPDATE watch_leases SET ended = ?3, authorized_path = NULL
             WHERE lease_id = ?1 AND holder = ?2 AND ended IS NULL",
            params![lease_id, holder, reason],
        )
        .map_err(sql)
}

/// `start`'s lease, in one transaction, the workspace directory, and whether this call created
/// it (rather than replaying a requestId).
fn lease(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
    request_id: Option<&str>,
) -> Result<(Json, PathBuf, bool)> {
    let (_, ws) = opened(storage, select, true)?;
    let dir = ws.held.dir.clone();
    let ws_id = ws.held.id.clone();
    let holder = owner(&ws.held.dir)?;
    // The source is chosen and its path authorized under the write lock, so a relocation
    // commits before or after this lease, never between the check and `authorized_path`.
    let tx = rusqlite::Transaction::new_unchecked(&ws.conn, TransactionBehavior::Immediate)
        .map_err(sql)?;
    let sources = chosen(storage, &ws, source)?;
    super::jobs::hold("watch_admit");
    let source_id = match sources.as_slice() {
        [one] => one,
        [] => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!("workspace {ws_id} has no source this process can see"),
            ));
        }
        _ => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!("workspace {ws_id} has more than one source; pass the sourceId to watch"),
            ));
        }
    };
    let print = crate::search::hex(&Sha256::digest(
        json!([ws_id, source_id, scope]).to_string(),
    ))[..16]
        .to_owned();
    let now = now_ms();
    let incarnation = ws.incarnation.clone();
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
            let live = ended.is_none() && expires > now;
            if live && prior_holder != holder {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!(
                        "requestId {request} names lease {lease}, which another running process holds; start with a new requestId"
                    ),
                ));
            }
            let valid: bool = tx
                .query_row(
                    &format!("SELECT {VALID} FROM watch_leases WHERE lease_id = ?1"),
                    [&lease],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if live && valid {
                return Ok((
                    lease_json(&ws_id, &lease, source_id, scope, expires),
                    dir,
                    false,
                ));
            }
            let why = match ended {
                Some(why) => why,
                None if !live => "expired".into(),
                None => {
                    // Untracked or retracked since: it never serves again, so it ends here.
                    tx.execute(
                        "UPDATE watch_leases SET ended = 'untracked', authorized_path = NULL
                         WHERE lease_id = ?1",
                        [&lease],
                    )
                    .map_err(sql)?;
                    tx.commit().map_err(sql)?;
                    "untracked".into()
                }
            };
            let ended = json!({"workspaceId": ws_id, "leaseId": lease, "sourceId": source_id,
                "scope": scope, "state": "ended", "reason": why,
                "nextStep": {"tool": "watch", "arguments": {"action": "start",
                    "workspaceId": ws_id, "sourceId": source_id, "scope": scope},
                    "why": "an ended lease never starts again; start a new one with a new requestId"}});
            return Ok((ended, dir, false));
        }
    }
    let tracked: Option<(String, i64)> = tx
        .query_row(
            "SELECT state, epoch FROM tracking WHERE source_id = ?1 AND scope = ?2",
            [source_id, scope],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let Some(("active" | "baseline_pending", epoch)) = tracked
        .as_ref()
        .map(|(state, epoch)| (state.as_str(), *epoch))
    else {
        return Err(FigError::new(
            ErrorKind::InvalidArguments,
            format!("source {source_id} scope {scope} is not tracked; track it before watching it"),
        )
        .with_recovery(json!([{"action": "track", "requires": "none", "tool": "index",
            "arguments": {"action": "track", "workspaceId": ws_id, "sourceId": source_id, "scope": scope}}]))
        .into());
    };
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
         authorized_path, request_id, fingerprint, renewed_ms, expires_ms, epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            expires,
            epoch
        ],
    )
    .map_err(sql)?;
    // Held before the commit: `release_at_signal` either sees this workspace, its update then
    // waiting for this commit, or refuses the start here and the lease rolls back.
    if !remember_held(storage, &ws_id) {
        return Err(fail(
            ErrorKind::WorkspaceBusy,
            "this server is stopping; start the watch from a running one".to_owned(),
        ));
    }
    tx.commit().map_err(sql)?;
    Ok((
        lease_json(&ws_id, &lease, source_id, scope, expires),
        dir,
        true,
    ))
}

/// One renewal of every live lease this process holds in workspace `ws_id`, in one statement.
/// Each lease is re-authorized against this process's own roots: a source it no longer sees
/// (or that moved) is kept but unauthorized, so the runner stops reading it for this holder.
/// Returns how many leases it renewed.
fn renew_one(storage: &Storage, ws_id: &str) -> Result<(usize, PathBuf)> {
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
    let checked = json!(paths.iter().map(|(id, _)| id).collect::<Vec<_>>()).to_string();
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
    // An untrack or retrack since the start: the lease never serves again, so it ends.
    tx.execute(
        &format!(
            "UPDATE watch_leases SET ended = 'untracked', authorized_path = NULL
             WHERE {LIVE} AND holder = ?2 AND NOT {VALID}"
        ),
        params![now, holder],
    )
    .map_err(sql)?;
    let renewed = tx
        .execute(
            &format!(
                "UPDATE watch_leases SET renewed_ms = ?1, expires_ms = ?3, authorized_path =
                 CASE WHEN source_id IN (SELECT value FROM json_each(?5))
                 THEN (SELECT json_extract(value, '$[1]') FROM json_each(?4)
                  WHERE json_extract(value, '$[0]') = watch_leases.source_id)
                 ELSE authorized_path END
                 WHERE {LIVE} AND holder = ?2"
            ),
            // A lease started since the read above keeps the authorization its start checked.
            params![now, holder, now + TTL.as_millis() as i64, seen, checked],
        )
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok((renewed, ws.held.dir.clone()))
}

/// Renews this process's leases in every workspace it holds them in, and starts that
/// workspace's runner again when it is gone while they live (one that ended with a host's
/// job object, say). A workspace whose renewal fails keeps its leases until they expire,
/// which is the safe side. Returns each workspace's renewed count.
pub fn renew() -> Vec<(String, Result<usize>)> {
    let held = HELD.lock().map(|h| h.clone()).unwrap_or_default();
    // Every workspace renews before any runner start, which can wait for a stopping runner,
    // so one slow workspace never lets another's leases expire.
    let renewals: Vec<_> = held.iter().map(|(s, id, _)| renew_one(s, id)).collect();
    let renewed: Vec<(String, Result<usize>)> = held
        .iter()
        .zip(renewals)
        .map(|((storage, id, _), renewal)| {
            let renewed = renewal.and_then(|(n, dir)| {
                if n > 0 {
                    ensure_runner(storage, id, &dir)?;
                }
                Ok(n)
            });
            (id.clone(), renewed)
        })
        .collect();
    // A workspace with no lease of this process left (all ended, or it was reset or removed)
    // is forgotten, so the renewal thread parks again; a busy one is retried.
    let done: Vec<(String, u64)> = held
        .iter()
        .zip(&renewed)
        .filter(|(_, (_, n))| {
            n.as_ref()
                .map_or_else(|e| super::database::gone(&**e), |n| *n == 0)
        })
        .map(|((_, id, mark), _)| (id.clone(), *mark))
        .collect();
    if let Ok(mut current) = HELD.lock() {
        forget(&mut current, &done);
    }
    renewed
}

/// Whether this process still holds a lease in any workspace (as of its last renewal).
pub fn holding() -> bool {
    HELD.lock().is_ok_and(|h| !h.is_empty())
}

/// `watch stop`: ends one lease this process holds.
pub fn stop(storage: &Storage, select: &Select, lease_id: &str) -> Result<Json> {
    let ws_id = {
        let (_, ws) = opened(storage, select, false)?;
        ws.held.id.clone()
    };
    if end(storage, &ws_id, lease_id, "stopped")? == 0 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("this process holds no live lease {lease_id}"),
        ));
    }
    Ok(json!({"workspaceId": ws_id, "leaseId": lease_id, "state": "ended", "reason": "stopped"}))
}

/// `release` for a process a signal is ending while its calls may still run: no start makes a
/// lease after it.
pub fn release_at_signal() {
    if let Ok(_held) = HELD.lock() {
        CLOSING.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    release();
}

/// At this process's exit: ends every lease it holds, in each workspace in one statement.
/// It never stops a runner; one with no live lease left exits by itself, and when this
/// process held the last ones it waits up to `CANCEL_WAIT` for that exit.
pub fn release() {
    let held = HELD.lock().map(|h| h.clone()).unwrap_or_default();
    for (storage, id, _) in held {
        if let Ok((_, ws)) = opened(&storage, &Select::Id(id), true)
            && let Ok(holder) = owner(&ws.held.dir)
        {
            let _ = ws.conn.execute(
                "UPDATE watch_leases SET ended = 'released', authorized_path = NULL
                 WHERE holder = ?1 AND ended IS NULL",
                [holder],
            );
            let live = ws.conn.query_row(
                &format!("SELECT count(*) FROM watch_leases WHERE {LIVE} AND {VALID}"),
                [now_ms()],
                |r| r.get::<_, i64>(0),
            );
            if live.is_ok_and(|n| n == 0) {
                let start = Instant::now();
                while runner_alive(&ws.held.dir) == Some(true)
                    && start.elapsed() < super::jobs::CANCEL_WAIT
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

/// `watch status`: this workspace's leases on sources the caller sees, read-only.
pub fn status(storage: &Storage, select: &Select) -> Result<Json> {
    let (_, ws) = opened(storage, select, false)?;
    let me = super::jobs::registered_owner(&ws.held.dir);
    let now = now_ms();
    let mut leases = Vec::new();
    let mut truncated = false;
    // One read snapshot: the visible sources and their lease rows come from the same commit.
    let _snapshot = ws.conn.unchecked_transaction().map_err(sql)?;
    if ws.version >= 5 {
        // Visible sources first, then the bound: live leases (at most MAX_LEASES) come before
        // retained ended ones, so a hidden source's history never pushes a visible row out.
        let seen: Vec<String> = ws
            .conn
            .prepare("SELECT source_id, path FROM sources")
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(sql)?
            .into_iter()
            .filter(|(_, path)| storage.sees(Path::new(path)))
            .map(|(id, _)| id)
            .collect();
        super::jobs::hold("watch_status");
        let mut stmt = ws
            .conn
            .prepare(
                "SELECT l.lease_id, l.holder, l.source_id, l.scope, l.expires_ms, l.ended,
                 CASE WHEN t.epoch IS NOT l.epoch OR t.state = 'disabled' THEN 0
                 ELSE l.authorized_path IS NOT NULL AND l.authorized_path = s.path END,
                 t.epoch IS l.epoch AND t.state != 'disabled'
                 FROM watch_leases l LEFT JOIN sources s ON s.source_id = l.source_id
                 LEFT JOIN tracking t ON t.source_id = l.source_id AND t.scope = l.scope
                 WHERE l.source_id IN (SELECT value FROM json_each(?1))
                 ORDER BY l.ended IS NOT NULL, l.renewed_ms DESC, l.lease_id LIMIT ?2",
            )
            .map_err(sql)?;
        let rows = stmt
            .query_map(params![json!(seen).to_string(), MAX_LEASES * 4 + 1], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, Option<bool>>(7)?.unwrap_or(false),
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (lease, holder, source, scope, expires, ended, authorized, tracked) =
                row.map_err(sql)?;
            if leases.len() as i64 == MAX_LEASES * 4 {
                truncated = true;
                break;
            }
            let state = match (&ended, expires > now, authorized) {
                (Some(_), ..) => "ended",
                (None, false, _) => "expired",
                (None, true, Some(true)) => "active",
                // Untracked (or retracked) since it started: it never counts again.
                (None, true, _) if !tracked => "untracked",
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
    let runner = match runner_alive(&ws.held.dir) {
        Some(true) if ws.version >= 5 && closing(&ws.conn)? => "stopping",
        Some(true) => "running",
        Some(false) => "stopped",
        None => "unknown",
    };
    let mut runner = json!({"state": runner});
    if ws.version >= 5
        && let Some((attempt, placement, beat)) = ws
            .conn
            .query_row(
                "SELECT attempt, placement, beat_ms FROM watch_runner WHERE one = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?
    {
        runner["attempt"] = json!(attempt);
        runner["placement"] = json!(placement);
        runner["beatMs"] = json!(beat);
    }
    let mut reply = json!({"workspaceId": ws.held.id, "runner": runner, "leases": leases});
    if truncated {
        reply["truncated"] = json!(true);
    }
    Ok(reply)
}

/// The watch monitor of `sources` (and `scope`, when one is named) for a `changes` reply,
/// apart from tracking: `watching` needs a running runner, a live lease that authorizes the
/// source's current path, and a monitor row the runner wrote within three persist intervals.
/// The first state that is not `watching`, in source order, is reported; `inactive` when no
/// live lease covers them, with the cadence only when one does. An empty page with any other state never proves the disk unchanged.
pub(super) fn monitor(
    conn: &Connection,
    dir: &Path,
    sources: &[String],
    scope: Option<&str>,
) -> Result<Json> {
    let mut out = json!({"state": "inactive"});
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql)?;
    if version < 5 || sources.is_empty() {
        return Ok(out);
    }
    let now = now_ms();
    let alive = runner_alive(dir);
    let beat: Option<i64> = conn
        .query_row("SELECT beat_ms FROM watch_runner WHERE one = 1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql)?;
    let (mut states, mut stamp, mut verified) = (Vec::new(), None::<i64>, None::<i64>);
    let mut uncovered = false;
    for source in sources {
        let tracked: i64 = conn
            .query_row(
                "SELECT count(*) FROM tracking WHERE source_id = ?1 AND (?2 IS NULL OR scope = ?2)
                 AND state IN ('active', 'baseline_pending')",
                params![source, scope],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let (live, authorized): (i64, i64) = conn
            .query_row(
                &format!(
                    "SELECT count(DISTINCT l.scope),
                     count(DISTINCT l.scope) FILTER (WHERE l.authorized_path = s.path)
                     FROM {SERVED_FROM} WHERE l.ended IS NULL AND l.expires_ms > ?1
                     AND t.epoch = l.epoch AND t.state = 'active'
                     AND l.source_id = ?2 AND (?3 IS NULL OR l.scope = ?3)"
                ),
                params![now, source, scope],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql)?;
        // A tracked scope of the view that no authorizing lease serves is not refreshed.
        uncovered |= authorized < tracked;
        if live == 0 {
            continue;
        }
        if authorized == 0 {
            states.push("unauthorized".to_owned());
            continue;
        }
        let row: Option<(String, Option<i64>, Option<i64>, i64)> = conn
            .query_row(
                "SELECT state, stamp_checked_ms, content_verified_ms, written_ms
                 FROM watch_monitor WHERE source_id = ?1",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(sql)?;
        let fresh = |at: i64| now - at < 3 * PERSIST.as_millis() as i64;
        let state = match (alive, &row) {
            (Some(true), Some((state, .., written))) if fresh(*written) => state.clone(),
            // A runner writes a newly served source's first row at its next persist.
            (Some(true), None) if beat.is_some_and(fresh) => "watching".to_owned(),
            _ => "stale".to_owned(),
        };
        if let Some((_, s, v, _)) = row {
            stamp = match (stamp, s) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            verified = match (verified, v) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        states.push(state);
    }
    if !states.is_empty() {
        let first = match states.iter().find(|s| *s != "watching") {
            Some(state) => state.as_str(),
            None if uncovered => "partial",
            None => "watching",
        };
        out = json!({"state": first, "pollEveryMs": POLL.as_millis() as u64,
            "contentVerifyEveryMs": VERIFY.as_millis() as u64,
            "persistEveryMs": PERSIST.as_millis() as u64});
        out["stampCheckedAtMs"] = json!(stamp);
        out["contentVerifiedAtMs"] = json!(verified);
    }
    Ok(out)
}

// ---- Runner ----

/// Leases the runner serves (`l` joined to sources `s` and tracking `t`): live, of this
/// incarnation, authorizing the source's current path, and of the scope's current tracking
/// epoch while it is active. A lease from before an untrack never counts again, and nothing is
/// refreshed while the baseline is pending. `?1` is now, `?2` the incarnation.
const SERVED: &str = "l.ended IS NULL AND l.expires_ms > ?1 AND l.incarnation = ?2
    AND l.authorized_path = s.path AND t.state = 'active' AND t.epoch = l.epoch";
const SERVED_FROM: &str = "watch_leases l JOIN sources s ON s.source_id = l.source_id
    JOIN tracking t ON t.source_id = l.source_id AND t.scope = l.scope";

/// The runner persists monitor rows, beats and collects garbage this often.
const PERSIST: Duration = Duration::from_secs(10);
/// It re-reads the served lease union every poll, so it stops within one poll of the last
/// lease's end.
const UNION: Duration = POLL;
/// Ended and expired lease rows stay this long, so their requestIds still replay as ended.
const KEEP_ENDED_MS: i64 = 3_600_000;
/// And at most this many, newest first, so a start/stop loop cannot grow the table; past
/// either bound a requestId is unknown and starts a new lease (as a job's does once collected).
const KEEP_ENDED_ROWS: i64 = 1024;
/// The runner records how it was started, so status never claims an independence it lacks.
const PLACEMENT: &str = "FIGLENS_WATCH_PLACEMENT";

fn runner_lock(dir: &Path) -> PathBuf {
    dir.join("watch").join("runner.lock")
}

/// Whether a runner holds `runner.lock`: `Some(true)` held, `Some(false)` free or missing,
/// `None` unknown. Read-only, like `jobs::alive`; unknown must never start a second runner.
fn runner_alive(dir: &Path) -> Option<bool> {
    let file = match File::open(runner_lock(dir)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(false),
        Err(_) => return None,
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            Some(false)
        }
        Err(TryLockError::WouldBlock) => Some(true),
        Err(_) => None,
    }
}

/// The hidden runner's arguments: the workspace id and, when the caller had one, its explicit
/// storage directory.
fn runner_args(storage: &Storage, ws_id: &str) -> Vec<std::ffi::OsString> {
    let mut args = vec![ws_id.into()];
    if let Some(dir) = &storage.explicit {
        args.extend(["--storage".into(), dir.clone().into_os_string()]);
    }
    args
}

/// How long a runner may take to say `ready`, and a start waits for a stopping one to exit.
const READY: Duration = Duration::from_secs(10);

/// `READY`, or in debug builds `FIGLENS_TEST_WATCH_READY_MS`, so process tests reach a timeout.
fn ready_limit() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("FIGLENS_TEST_WATCH_READY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    READY
}

/// Debug builds only: `FIGLENS_TEST_WATCH=<fault>` makes the launcher fail (`launch_fails`) or
/// the runner never say ready (`runner_silent`), so process tests cover a failed start.
fn fault(name: &str) -> bool {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("FIGLENS_TEST_WATCH") {
        let (at, file) = v
            .split_once(':')
            .map_or((v.as_str(), None), |(a, f)| (a, Some(f)));
        if at == name && file.is_none_or(|f| Path::new(f).exists()) {
            return true;
        }
    }
    let _ = name;
    false
}

/// Whether the runner holding `runner.lock` has decided to exit (it no longer serves leases).
fn closing(conn: &Connection) -> Result<bool> {
    Ok(conn
        .query_row("SELECT closing FROM watch_runner WHERE one = 1", [], |r| {
            r.get::<_, i64>(0)
        })
        .optional()
        .map_err(sql)?
        == Some(1))
}

/// A child's first stdout line, at most 64 bytes, within `limit`; on timeout the child is
/// killed and awaited, and `None` returned.
fn first_line(child: &mut std::process::Child, limit: Duration) -> Option<String> {
    use std::io::Read as _;
    let out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut std::io::BufReader::new(out.take(64)), &mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(limit) {
        Ok(line) => Some(line.trim().to_owned()),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Starts the workspace's runner unless one serves it, through a launcher that exits once the
/// runner is ready, so the runner is never this process's descendant and a host that kills
/// this process's tree leaves it running. A runner that is stopping is waited for (up to
/// `READY`) and then replaced; an unknown lock state is reported, never called running.
/// Returns the runner's state and placement. Called outside any database transaction.
pub fn ensure_runner(storage: &Storage, ws_id: &str, dir: &Path) -> Result<Json> {
    let deadline = Instant::now() + ready_limit();
    loop {
        match runner_alive(dir) {
            None => return Ok(json!({"state": "unknown"})),
            Some(true) => {
                let (_, ws) = opened(storage, &Select::Id(ws_id.to_owned()), false)?;
                if !closing(&ws.conn)? {
                    return Ok(json!({"state": "running"}));
                }
                // It decided to exit before this lease was there; start the next once it has.
                super::jobs::hold("watch_start_closing");
                if Instant::now() >= deadline {
                    return Ok(json!({"state": "stopping"}));
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Some(false) => {}
        }
        let exe = std::env::current_exe()?;
        let mut launcher = std::process::Command::new(exe)
            .arg("__watch-launch")
            .args(runner_args(storage, ws_id))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                fail(
                    ErrorKind::OperationFailed,
                    format!("cannot start the watch runner: {e}"),
                )
            })?;
        let line = first_line(&mut launcher, ready_limit() + Duration::from_secs(1));
        let ok = launcher.wait().is_ok_and(|s| s.success());
        match (
            ok,
            line.as_deref()
                .map(|l| l.split_whitespace().collect::<Vec<_>>()),
        ) {
            (true, Some(words)) if words.len() == 2 && words[0] == "ready" => {
                return Ok(json!({"state": "running", "placement": words[1]}));
            }
            // Another start took the lock first: look again (it may be one that is stopping).
            (true, Some(words)) if words == ["busy"] && Instant::now() < deadline => {}
            (_, line) => {
                return Err(fail(
                    ErrorKind::OperationFailed,
                    format!(
                        "the watch runner did not start: {}",
                        line.map_or("no reply in time".to_owned(), |w| w.join(" "))
                    ),
                ));
            }
        }
    }
}

/// The hidden `__watch-launch` child: starts the detached runner, prints its first line
/// (`ready <placement>` or `busy`) and exits. On Windows the runner leaves the caller's job
/// object when the job allows it; when the job forbids that, the runner starts inside it and
/// says so (`in_host_job`): it then ends with that job, and a holder's next renewal starts
/// another while leases live. A runner silent past `READY` is killed.
pub fn launch(args: &[std::ffi::OsString]) -> Result<String> {
    if fault("launch_fails") {
        return Err(fail(
            ErrorKind::OperationFailed,
            "test fault: the launcher failed".to_owned(),
        ));
    }
    let exe = std::env::current_exe()?;
    let command = |placement: &str| {
        let mut c = std::process::Command::new(&exe);
        c.arg("__watch-runner")
            .args(args)
            .env(PLACEMENT, placement)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };
    #[cfg(unix)]
    let spawned = {
        use std::os::unix::process::CommandExt as _;
        command("independent").process_group(0).spawn()
    };
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt as _;
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200; // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        const BREAKAWAY: u32 = 0x0100_0000; // CREATE_BREAKAWAY_FROM_JOB
        match command("independent")
            .creation_flags(DETACHED | BREAKAWAY)
            .spawn()
        {
            // The caller's job forbids breakaway: say so instead of claiming independence.
            Err(e) if e.raw_os_error() == Some(5) => {
                command("in_host_job").creation_flags(DETACHED).spawn()
            }
            other => other,
        }
    };
    let mut runner = spawned.map_err(|e| {
        fail(
            ErrorKind::OperationFailed,
            format!("cannot start the watch runner: {e}"),
        )
    })?;
    first_line(&mut runner, ready_limit()).ok_or_else(|| {
        fail(
            ErrorKind::OperationFailed,
            "the watch runner did not report ready in time".to_owned(),
        )
    })
}

/// One refresh the runner's worker thread runs, and its outcome.
struct Refresh {
    source: String,
    path: PathBuf,
    scopes: Vec<String>,
}

struct Finished {
    source: String,
    path: PathBuf,
    failed: bool,
    job: Option<String>,
    error: Option<String>,
}

/// One watched source in the runner.
struct Watched {
    path: PathBuf,
    scopes: Vec<String>,
    detector: Detector<crate::search::Stamp>,
    next: Instant,
    state: State,
    stamp_checked_ms: Option<i64>,
    content_verified_ms: Option<i64>,
    last_job: Option<String>,
    last_error: Option<String>,
}

/// How long the runner sleeps: until the earliest source's next probe, at most `POLL`.
fn pause(now: Instant, next: impl Iterator<Item = Instant>) -> Duration {
    next.map(|at| at.saturating_duration_since(now))
        .fold(POLL, Duration::min)
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Watching => "watching",
        State::SourceBusy => "source_busy",
        State::SourceMissing => "source_missing",
        State::SourceUnreadable => "source_unreadable",
        State::RefreshFailed => "refresh_failed",
    }
}

/// Served (source, scope, path) of this incarnation's leases, or `None` when this runner was
/// fenced out (another attempt took over, or the workspace was reset).
/// `ws` is the runner's one read connection, held open (with the workspace's shared lock, so
/// the database cannot be replaced under it) but with no transaction between reads.
/// Each served `(source, scope, path, published gen)`; the gen is what saved-query discovery
/// compares, so a publication by anyone (this runner, `index refresh`, another host) is seen.
type Served = Vec<(String, String, String, Option<i64>)>;

fn union(ws: &Opened, token: &str, attempt: i64) -> Result<Option<Served>> {
    if fault("union_fails") {
        return Err(fail(
            ErrorKind::OperationFailed,
            "test fault: the union read failed".to_owned(),
        ));
    }
    let ours: Option<(String, i64)> = ws
        .conn
        .query_row(
            "SELECT token, attempt FROM watch_runner WHERE one = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if ours != Some((token.to_owned(), attempt)) {
        return Ok(None);
    }
    let rows = ws
        .conn
        .prepare_cached(&format!(
            "SELECT DISTINCT l.source_id, l.scope, s.path, p.gen FROM {SERVED_FROM}
             LEFT JOIN published p ON p.source_id = l.source_id AND p.scope = l.scope
             WHERE {SERVED} ORDER BY l.source_id, l.scope"
        ))
        .and_then(|mut s| {
            s.query_map(params![now_ms(), ws.incarnation], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect()
        })
        .map_err(sql)?;
    Ok(Some(rows))
}

/// Live leases of this incarnation, on the runner's read connection.
fn live_leases(ws: &Opened) -> Result<i64> {
    ws.conn
        .prepare_cached(&format!(
            "SELECT count(*) FROM watch_leases WHERE {LIVE} AND {VALID} AND incarnation = ?2"
        ))
        .and_then(|mut s| s.query_row(params![now_ms(), ws.incarnation], |r| r.get(0)))
        .map_err(sql)
}

/// Marks this runner `closing` when no live lease of this incarnation is left, in one
/// IMMEDIATE transaction, so a start that commits a lease after it sees a stopping runner and
/// waits for the next; a start that committed first keeps this runner. `None` when fenced.
fn close(storage: &Storage, ws_id: &str, token: &str, attempt: i64) -> Result<Option<bool>> {
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let incarnation = ws.incarnation.clone();
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let ours: Option<(String, i64)> = tx
        .query_row(
            "SELECT token, attempt FROM watch_runner WHERE one = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if ours != Some((token.to_owned(), attempt)) {
        return Ok(None);
    }
    let live: i64 = tx
        .query_row(
            &format!(
                "SELECT count(*) FROM watch_leases WHERE {LIVE} AND {VALID} AND incarnation = ?2"
            ),
            params![now_ms(), incarnation],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if live > 0 {
        return Ok(Some(false));
    }
    tx.execute("UPDATE watch_runner SET closing = 1 WHERE one = 1", [])
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(Some(true))
}

/// Cancels this runner's own attempts that nobody else joined, in one IMMEDIATE transaction
/// that a join's (which marks `shared`) is ordered against; their `run` sees the cancel and
/// stops the worker. Shared attempts are left to finish for the hosts that wait on them.
fn cancel_exclusive(storage: &Storage, ws_id: &str, token: &str) -> Result<usize> {
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let n = tx
        .execute(
            "UPDATE jobs SET cancel = 1 WHERE owner = ?1 AND shared = 0 AND cancel = 0
             AND state IN ('queued', 'running')",
            [token],
        )
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(n)
}

/// Writes every watched source's monitor row, beats and collects ended leases, fenced by this
/// runner's token and attempt. Returns false when fenced out.
fn persist(
    storage: &Storage,
    ws_id: &str,
    token: &str,
    attempt: i64,
    watched: &std::collections::BTreeMap<String, Watched>,
) -> Result<bool> {
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let now = now_ms();
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let beat = tx
        .execute(
            "UPDATE watch_runner SET beat_ms = ?3 WHERE one = 1 AND token = ?1 AND attempt = ?2",
            params![token, attempt, now],
        )
        .map_err(sql)?;
    if beat == 0 {
        return Ok(false);
    }
    for (source, w) in watched {
        tx.execute(
            "INSERT INTO watch_monitor (source_id, state, stamp_checked_ms, content_verified_ms,
             last_job_id, last_error, written_ms) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
             WHERE EXISTS (SELECT 1 FROM sources WHERE source_id = ?1)
             ON CONFLICT (source_id) DO UPDATE SET state = ?2, stamp_checked_ms = ?3,
             content_verified_ms = ?4, last_job_id = ?5, last_error = ?6, written_ms = ?7",
            params![
                source,
                state_name(w.state),
                w.stamp_checked_ms,
                w.content_verified_ms,
                w.last_job,
                w.last_error,
                now
            ],
        )
        .map_err(sql)?;
    }
    tx.execute(
        "DELETE FROM watch_leases WHERE (ended IS NOT NULL OR expires_ms <= ?1)
         AND renewed_ms < ?2",
        params![now, now - KEEP_ENDED_MS],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM watch_leases WHERE lease_id IN (SELECT lease_id FROM watch_leases
         WHERE ended IS NOT NULL OR expires_ms <= ?1 ORDER BY renewed_ms DESC LIMIT -1 OFFSET ?2)",
        params![now, KEEP_ENDED_ROWS],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    // The runner's open reader means no connection close checkpoints the WAL; a passive one
    // here (it never waits for readers) keeps it to about one persist interval of writes.
    let _ = ws
        .conn
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
    Ok(true)
}

/// Whether a lease still wants `scope` of `source` at `path`: the runner's admission fence,
/// read inside the submit's write, so an untrack, relocation or lease end since the last union
/// read admits and joins nothing.
fn served(
    conn: &Connection,
    incarnation: &str,
    source: &str,
    scope: &str,
    path: &Path,
) -> Result<bool> {
    let n: i64 = conn
        .query_row(
            &format!(
                "SELECT count(*) FROM {SERVED_FROM}
                 WHERE {SERVED} AND l.source_id = ?3 AND l.scope = ?4 AND s.path = ?5"
            ),
            params![now_ms(), incarnation, source, scope, path.to_string_lossy()],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(n > 0)
}

/// Waits for a job another process runs to end, never running or cancelling it, and returns
/// its final report; `None` when `quit` was set first.
fn await_joined(storage: &Storage, id: &str, quit: &AtomicBool) -> Result<Option<Json>> {
    loop {
        let now = super::jobs::status(storage, id)?;
        if !matches!(now["state"].as_str(), Some("queued" | "running")) {
            return Ok(Some(now));
        }
        if quit.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(None);
        }
        std::thread::sleep(POLL);
    }
}

use super::changes::{self as saved, Continuation};

type Scope = (String, String);

/// One leased scope's next saved-query Turn: where it resumes, and not before `at`.
struct Due<C> {
    key: Scope,
    from: Option<C>,
    at: Instant,
}

/// How a Turn ended: `Next` with the queue's continuation (none: nothing left at this
/// publication); `Again` when a save or exit interrupted it, writing nothing; `Retry` after a
/// transient busy or storage race; `Drop` for anything else, which the queue records itself.
enum Outcome<C> {
    Next(Option<C>),
    Again(Option<C>),
    Retry(Option<C>),
    Drop,
}

/// Saved-query work of the leased scopes, in memory only: a scope is due when it first appears
/// (runner start, lease admission) or its published generation changes, then gets one bounded
/// Turn at a time, round-robin. A restart recomputes everything from the stored evaluations.
struct Saved<C> {
    seen: std::collections::BTreeMap<Scope, Option<i64>>,
    due: std::collections::VecDeque<Due<C>>,
}

impl<C> Saved<C> {
    fn new() -> Self {
        Saved {
            seen: Default::default(),
            due: Default::default(),
        }
    }

    /// Takes one union read: drops scopes no longer served and queues new or republished ones
    /// from their start, superseding any continuation of an older publication.
    fn observe(&mut self, rows: &Served, now: Instant) {
        let live: std::collections::BTreeSet<Scope> =
            rows.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
        self.seen.retain(|k, _| live.contains(k));
        self.due.retain(|d| live.contains(&d.key));
        for (source, scope, _, published) in rows {
            let key = (source.clone(), scope.clone());
            if self.seen.get(&key) != Some(published) {
                self.seen.insert(key.clone(), *published);
                self.due.retain(|d| d.key != key);
                self.due.push_back(Due {
                    key,
                    from: None,
                    at: now,
                });
            }
        }
    }

    /// The first queued Turn whose time has come.
    fn take(&mut self, now: Instant) -> Option<Due<C>> {
        let i = self.due.iter().position(|d| d.at <= now)?;
        self.due.remove(i)
    }

    /// Requeues a finished Turn's scope at the back, unless it stopped being served or a newer
    /// publication already queued it afresh.
    fn finished(&mut self, key: Scope, outcome: Outcome<C>, now: Instant) {
        if !self.seen.contains_key(&key) || self.due.iter().any(|d| d.key == key) {
            return;
        }
        let (from, at) = match outcome {
            Outcome::Next(None) | Outcome::Drop => return,
            Outcome::Next(Some(c)) => (Some(c), now),
            Outcome::Again(from) => (from, now),
            Outcome::Retry(from) => (from, now + BUSY),
        };
        self.due.push_back(Due { key, from, at });
    }
}

/// Runs one bounded saved-query Turn. `halt` is set when a save waits or the runner exits, and
/// interrupts the Turn's search.
fn turn(
    storage: &Storage,
    ws_id: &str,
    due: Due<Continuation>,
    halt: &AtomicBool,
) -> Outcome<Continuation> {
    if fault("turn_holds") {
        // Test builds: a Turn that only an interrupt ends; `<file>.reached` says it started.
        if let Some((_, file)) = std::env::var("FIGLENS_TEST_WATCH")
            .ok()
            .as_deref()
            .and_then(|v| v.split_once(':'))
        {
            let _ = std::fs::write(format!("{file}.reached"), "");
        }
        while !halt.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        return Outcome::Again(due.from);
    }
    let (source, scope) = &due.key;
    let select = Select::Id(ws_id.to_owned());
    match saved::evaluate_published(storage, &select, source, scope, due.from.clone(), halt) {
        Ok(t) => Outcome::Next(t.next),
        Err(e) => match e.downcast_ref::<FigError>().map(|f| f.kind()) {
            Some(ErrorKind::Cancelled) => Outcome::Again(due.from),
            Some(ErrorKind::WorkspaceBusy | ErrorKind::StorageUnavailable) => {
                Outcome::Retry(due.from)
            }
            _ => {
                eprintln!("figlens watch: saved queries of {source} {scope}: {e}");
                Outcome::Drop
            }
        },
    }
}

/// The runner worker's one job at a time.
enum Work {
    Refresh(Refresh),
    Turn(Box<Due<Continuation>>),
}

enum Done {
    Refresh(Finished),
    Turn(Box<(Scope, Outcome<Continuation>)>),
}

/// Refreshes every still-served scope of one source, one at a time: its own jobs run here to
/// their end; a job another process already runs is awaited, never run or cancelled. A source
/// counts as verified only when every job ended `succeeded`. A published scope whose model
/// layer is missing gets an embed job this runner owns, sent to `models` so the next save
/// never waits for the model.
fn refresh(
    storage: &Storage,
    ws_id: &str,
    job: &Refresh,
    cancel: &AtomicBool,
    quit: &AtomicBool,
    models: &std::sync::mpsc::Sender<String>,
) -> Finished {
    let select = Select::Id(ws_id.to_owned());
    let (mut failed, mut last, mut error) = (false, None, None);
    let mut fail_with = |e: String| {
        failed = true;
        error = Some(e);
    };
    for scope in &job.scopes {
        if quit.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        // Judged inside the admission's write, so a relocation lands wholly before it (nothing
        // admitted or joined) or after it (and cancels the stale build itself).
        let fence = |conn: &Connection, incarnation: &str| {
            served(conn, incarnation, &job.source, scope, &job.path)
        };
        let outcome =
            match super::jobs::refresh_fenced(storage, &select, &job.source, scope, &fence) {
                Ok(Some(outcome)) => outcome,
                Ok(None) => continue,
                Err(e) => {
                    fail_with(e.to_string());
                    continue;
                }
            };
        let ids: Vec<String> = match outcome.report["jobs"].as_array() {
            Some(jobs) => jobs
                .iter()
                .filter_map(|j| j["jobId"].as_str().map(str::to_owned))
                .collect(),
            None => outcome.report["job"]["jobId"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .collect(),
        };
        for id in ids {
            last = Some(id.clone());
            let report = if outcome.run.contains(&id) {
                super::jobs::run(storage, ws_id, &id, cancel, &mut |_| {}).map(Some)
            } else {
                await_joined(storage, &id, quit)
            };
            match report {
                Ok(Some(r)) if r["state"] == "succeeded" => {
                    if let Some(embed) = super::jobs::embed_after(storage, &r) {
                        let _ = models.send(embed);
                    }
                }
                Ok(Some(r)) => {
                    fail_with(r["error"]["code"].as_str().unwrap_or("failed").to_owned())
                }
                Ok(None) => fail_with("runner_stopping".to_owned()),
                Err(e) => fail_with(e.to_string()),
            }
        }
    }
    Finished {
        source: job.source.clone(),
        path: job.path.clone(),
        // No job at all (every scope stopped being served) verifies nothing.
        failed: failed || last.is_none(),
        job: last,
        error,
    }
}

/// The hidden `__watch-runner` process: holds `runner.lock`, takes the next attempt, prints
/// `ready <placement>` (or `busy` when another runner holds the lock) and then polls every
/// leased source until no live lease is left, refreshing one source at a time. It then cancels
/// its unshared attempt and finishes a shared one, so a host that joined it gets its result.
pub fn serve(storage: &Storage, ws_id: &str) -> Result<()> {
    use std::io::Write as _;
    let dir = {
        let (_, ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
        ws.held.dir.clone()
    };
    mkdir(&dir.join("watch"))?;
    let lock_path = runner_lock(&dir);
    let lock = open_lock(&lock_path, true)?.expect("created");
    if lock.try_lock().is_err() {
        println!("busy");
        return Ok(());
    }
    let token = owner(&dir)?;
    let placement = std::env::var(PLACEMENT).unwrap_or_else(|_| "independent".into());
    let attempt: i64 = {
        let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
        let now = now_ms();
        let tx = ws
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        tx.execute(
            "INSERT INTO watch_runner (one, token, attempt, placement, started_ms, beat_ms)
             VALUES (1, ?1, 1, ?2, ?3, ?3) ON CONFLICT (one) DO UPDATE SET token = ?1,
             attempt = attempt + 1, placement = ?2, started_ms = ?3, beat_ms = ?3, closing = 0",
            params![token, placement, now],
        )
        .map_err(sql)?;
        let attempt = tx
            .query_row("SELECT attempt FROM watch_runner WHERE one = 1", [], |r| {
                r.get(0)
            })
            .map_err(sql)?;
        tx.commit().map_err(sql)?;
        attempt
    };
    if fault("runner_silent") {
        std::thread::sleep(Duration::from_secs(60));
        return Ok(());
    }
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "ready {placement}");
        let _ = out.flush();
    }
    // One read connection for the union reads. Holding it keeps the workspace's shared lock,
    // so a reset first ends every lease (see `registry::reset`) and waits for this exit.
    let (_, reader) = opened(storage, &Select::Id(ws_id.to_owned()), false)?;
    // Never set: an exit cancels only unshared attempts, through the job row.
    let cancel = AtomicBool::new(false);
    // Set on exit: submit nothing more and stop waiting for a job another process runs.
    let quit = AtomicBool::new(false);
    let idle = AtomicBool::new(false);
    // Set to interrupt a saved-query Turn: a save waits, or the runner exits.
    let halt = AtomicBool::new(false);
    let (to_worker, jobs) = std::sync::mpsc::channel::<Work>();
    let (done, finished) = std::sync::mpsc::channel::<Done>();
    // Embed jobs this runner owns, run apart from refreshes. Like refreshes they take `cancel`,
    // never `quit`: an exit cancels only unshared jobs through their rows (`cancel_exclusive`),
    // so an embed another caller joined still finishes.
    let (to_model, models) = std::sync::mpsc::channel::<String>();
    std::thread::scope(|scope| -> Result<()> {
        let (cancel, quit, idle, halt) = (&cancel, &quit, &idle, &halt);
        scope.spawn(move || {
            for id in models {
                let _ = super::jobs::run(storage, ws_id, &id, cancel, &mut |_| {});
            }
        });
        scope.spawn(move || {
            for work in jobs {
                let _ = done.send(match work {
                    Work::Refresh(job) => {
                        Done::Refresh(refresh(storage, ws_id, &job, cancel, quit, &to_model))
                    }
                    Work::Turn(due) => Done::Turn(Box::new((
                        due.key.clone(),
                        turn(storage, ws_id, *due, halt),
                    ))),
                });
            }
            idle.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let mut watched = std::collections::BTreeMap::<String, Watched>::new();
        let (mut read_at, mut persist_at) = (Instant::now(), Instant::now() + PERSIST);
        let mut busy = false;
        let mut saved = Saved::new();
        let mut turning = false;
        // Sources whose refresh waits for the worker, served in arrival order so a source that
        // keeps changing cannot take every turn.
        let mut waiting = std::collections::VecDeque::<String>::new();
        // Any end, an error included, runs the shutdown below.
        let result = (|| -> Result<()> {
            loop {
                let now = Instant::now();
                if now >= read_at {
                    read_at = now + UNION;
                    let Some(rows) = union(&reader, &token, attempt)? else {
                        return Ok(());
                    };
                    saved.observe(&rows, now);
                    // No lease served: exit unless a live one (say, on a pending baseline) remains;
                    // the reader checks first so such a lease costs no write transaction per poll.
                    if rows.is_empty() && live_leases(&reader)? == 0 {
                        match close(storage, ws_id, &token, attempt)? {
                            Some(false) => {}
                            Some(true) => {
                                // Test builds: hold between the closing commit and the lock's release.
                                super::jobs::hold("watch_closing");
                                return Ok(());
                            }
                            None => return Ok(()),
                        }
                    }
                    let mut wanted =
                        std::collections::BTreeMap::<String, (String, Vec<String>)>::new();
                    for (source, scope, path, _) in rows {
                        wanted
                            .entry(source)
                            .or_insert((path, Vec::new()))
                            .1
                            .push(scope);
                    }
                    watched.retain(|source, w| {
                        wanted
                            .get(source)
                            .is_some_and(|(path, _)| Path::new(path) == w.path)
                    });
                    waiting.retain(|source| wanted.contains_key(source));
                    for (source, (path, scopes)) in wanted {
                        let w = watched.entry(source).or_insert_with(|| Watched {
                            path: PathBuf::from(path),
                            scopes: Vec::new(),
                            detector: Detector::new(now),
                            next: now,
                            state: State::Watching,
                            stamp_checked_ms: None,
                            content_verified_ms: None,
                            last_job: None,
                            last_error: None,
                        });
                        // A scope added to a watched source catches up now, not at the next verify;
                        // a new source's detector already refreshes first.
                        if !w.scopes.is_empty() && scopes.iter().any(|s| !w.scopes.contains(s)) {
                            w.detector.invalidate();
                            w.next = now;
                        }
                        w.scopes = scopes;
                    }
                }
                while let Ok(done) = finished.try_recv() {
                    let f = match done {
                        Done::Refresh(f) => f,
                        Done::Turn(done) => {
                            let (key, outcome) = *done;
                            turning = false;
                            saved.finished(key, outcome, Instant::now());
                            continue;
                        }
                    };
                    busy = false;
                    // A source relocated meanwhile is watched anew: this result is not its own.
                    if let Some(w) = watched.get_mut(&f.source).filter(|w| w.path == f.path) {
                        if !f.failed {
                            w.content_verified_ms = Some(now_ms());
                        }
                        w.last_job = f.job;
                        w.last_error = f.error;
                        let step = w.detector.finished(Instant::now(), f.failed);
                        // A failing source (an unparseable save, say) retries every BACKOFF, not
                        // every poll; a new save is still seen within that.
                        w.next = if f.failed {
                            Instant::now() + BACKOFF
                        } else {
                            step.next
                        };
                        if step.submit && !waiting.contains(&f.source) {
                            waiting.push_back(f.source);
                        }
                    }
                }
                while !busy && let Some(source) = waiting.pop_front() {
                    if let Some(w) = watched.get(&source) {
                        busy = true;
                        let _ = to_worker.send(Work::Refresh(Refresh {
                            source,
                            path: w.path.clone(),
                            scopes: w.scopes.clone(),
                        }));
                    }
                }
                for (source, w) in watched.iter_mut() {
                    let now = Instant::now();
                    if now < w.next {
                        continue;
                    }
                    let probe = match crate::search::stamp(&w.path) {
                        // A link swapped in for the authorized path is never followed.
                        Ok(_) if !super::storage::leads_to_itself(&w.path) => Probe::Busy,
                        Ok(stamp) => Probe::Stamp(stamp),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
                        Err(_) => Probe::Busy,
                    };
                    w.stamp_checked_ms = Some(now_ms());
                    let step = w.detector.observe(now, probe);
                    w.state = step.state;
                    w.next = step.next;
                    // One refresh at a time; the detector coalesces a change seen meanwhile.
                    if step.submit && !busy {
                        busy = true;
                        let _ = to_worker.send(Work::Refresh(Refresh {
                            source: source.clone(),
                            path: w.path.clone(),
                            scopes: w.scopes.clone(),
                        }));
                    } else if step.submit && !waiting.contains(source) {
                        waiting.push_back(source.clone());
                    }
                }
                // Saves come first: a Turn runs only on an idle worker, and one in flight is
                // interrupted as soon as a refresh is queued behind it.
                if turning && (busy || !waiting.is_empty()) {
                    halt.store(true, std::sync::atomic::Ordering::SeqCst);
                } else if !turning
                    && !busy
                    && waiting.is_empty()
                    && let Some(due) = saved.take(Instant::now())
                {
                    halt.store(false, std::sync::atomic::Ordering::SeqCst);
                    turning = true;
                    let _ = to_worker.send(Work::Turn(Box::new(due)));
                }
                if Instant::now() >= persist_at {
                    persist_at = Instant::now() + PERSIST;
                    if !persist(storage, ws_id, &token, attempt, &watched)? {
                        return Ok(());
                    }
                }
                std::thread::sleep(pause(Instant::now(), watched.values().map(|w| w.next)));
            }
        })();
        // No lease is left (or this runner was fenced out): submit nothing more, cancel its own
        // unshared attempt, let a shared one finish, then release the lock.
        // Repeated until the worker is done, so an attempt it admitted meanwhile is caught too.
        quit.store(true, std::sync::atomic::Ordering::SeqCst);
        halt.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(to_worker);
        while !idle.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = cancel_exclusive(storage, ws_id, &token);
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = persist(storage, ws_id, &token, attempt, &watched);
        result
    })?;
    drop(lock);
    Ok(())
}

#[cfg(test)]
mod tests;
