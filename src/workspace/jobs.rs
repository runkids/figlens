//! Index jobs: one build of one source and scope, submitted or joined in a short transaction,
//! run by the process that owns it in a child process it can kill, and published by that owner
//! only after re-validating the workspace, its source membership and the expected generation.
//!
//! Lock order, never taken backwards: location lock (C) -> workspace lock (W) -> build lock (B)
//! -> SQLite write transaction. A process's owner lock (`owners/<token>.lock`) lives as long as
//! the process, so a job row is alive exactly while its owner process is.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};

use rusqlite::{TransactionBehavior, params};

use super::*;
use crate::search;

/// Queued and running jobs one workspace accepts across every process.
pub const MAX_ACTIVE: i64 = 16;
/// Terminal jobs kept per workspace for status and request-id replays.
const KEPT: i64 = 50;
/// Published scopes one source keeps; publishing another retires the least recently published.
pub const MAX_SCOPES: i64 = 8;
/// Main database file bytes: SQLite enforces this less `BUILD_HEADROOM` on the build writer
/// (`max_page_count`), or the file's current size when that is already larger, since SQLite
/// never lowers the ceiling below it: such a file only reuses its freed pages. A build that
/// needs a page past the ceiling fails in that batch, which rolls back; the published
/// generation keeps serving.
pub const DISK_CAP: u64 = if cfg!(test) {
    1024 * 1024
} else {
    2 * 1024 * 1024 * 1024
};
/// Bytes of the database's pages in use: freed pages are reused before the file grows, so they
/// are room, else a file at its high-water size could never take a write again after cleanup.
pub(crate) fn used_bytes(conn: &Connection) -> Result<u64> {
    let used: i64 = conn
        .query_row(
            "SELECT (page_count - freelist_count) * page_size
             FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(used as u64)
}
/// Part of `DISK_CAP` no build may use, left for publication, registry and job writes, which
/// SQLite does not cap.
const BUILD_HEADROOM: u64 = if cfg!(test) {
    128 * 1024
} else {
    64 * 1024 * 1024
};
/// WAL bytes a build waits to see reclaimed after each batch before writing the next: a
/// threshold, not a ceiling, since the WAL can pass it by one batch's pages (an estimated
/// `BATCH_BYTES` of rows, or one larger row alone, plus b-tree overhead).
pub const WAL_CAP: u64 = if cfg!(test) {
    16 * 1024
} else {
    64 * 1024 * 1024
};
/// How often an owner checks for a cancel request while its child builds.
const POLL: Duration = Duration::from_millis(100);
/// How long `jobs cancel` waits for the owner to report the job stopped.
pub(super) const CANCEL_WAIT: Duration = Duration::from_secs(2);
/// Longest a queued job waits for its turn to build; past it, it fails `workspace_busy` and a
/// refresh retries it.
const QUEUE_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(600)
};

/// Owner tokens and their held lock files, one per workspace directory this process owns jobs in.
static OWNERS: Mutex<Vec<(PathBuf, String, File)>> = Mutex::new(Vec::new());

pub(super) fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    // Not secret: a token only needs to be unique; the clock and a counter seed SHA-256.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seed = format!(
        "{}-{}-{:?}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        SystemTime::now(),
        std::thread::current().name().unwrap_or("")
    );
    bytes.copy_from_slice(&Sha256::digest(seed)[..16]);
    crate::search::hex(&bytes)
}

/// This process's owner token for `dir` when it already has one; never creates or removes a
/// file, so read-only callers can tell their own rows.
pub(crate) fn registered_owner(dir: &Path) -> Option<String> {
    let owners = OWNERS.lock().ok()?;
    owners
        .iter()
        .find(|(d, ..)| d == dir)
        .map(|(_, token, _)| token.clone())
}

/// This process's owner token for the workspace at `dir`, holding `owners/<token>.lock`
/// exclusively for the life of the process. Stale owner files whose lock is free are removed.
pub(crate) fn owner(dir: &Path) -> Result<String> {
    let mut owners = OWNERS.lock().map_err(|_| "owner registry poisoned")?;
    if let Some((_, token, _)) = owners.iter().find(|(d, ..)| d == dir) {
        return Ok(token.clone());
    }
    let owners_dir = dir.join("owners");
    mkdir(&owners_dir)?;
    // Bounded: a crash leaves one file per process.
    for entry in std::fs::read_dir(&owners_dir)
        .map_err(|e| storage_error(&owners_dir, e))?
        .flatten()
        .take(256)
    {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_file())
            && let Ok(file) = OpenOptions::new().read(true).write(true).open(&path)
            && file.try_lock().is_ok()
        {
            let _ = std::fs::remove_file(&path);
        }
    }
    // Another process's cleanup can lock and delete a new file before its creator locks it;
    // only a file still in place once locked is proof, so a lost race takes a new token.
    for _ in 0..3 {
        let token = random_hex();
        let path = owners_dir.join(format!("{token}.lock"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| storage_error(&path, e))?;
        if file.try_lock().is_ok() && path.exists() {
            owners.push((dir.to_owned(), token.clone(), file));
            return Ok(token);
        }
    }
    Err(fail(
        ErrorKind::OperationFailed,
        "cannot lock a new owner file",
    ))
}

/// Whether the process owning `token` is alive: its owner lock is held. Read-only; never blocks.
pub(crate) fn alive(dir: &Path, token: &str) -> bool {
    if OWNERS
        .lock()
        .is_ok_and(|o| o.iter().any(|(d, t, _)| d == dir && t == token))
    {
        return true;
    }
    // Only a missing owner file proves death; any other failure is unknown, and unknown must
    // never let a takeover, a cancel or a cleanup proceed.
    let file = match File::open(dir.join("owners").join(format!("{token}.lock"))) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// Proof that this process is running one attempt of a job: an exclusive lock on
/// `claims/<jobId>.<attempt>.lock`, held from `run` until that attempt's child is known to
/// have exited. Without it a job whose terminal state could not be written would read running
/// for as long as its owner process lives.
struct Claim {
    path: PathBuf,
    _file: File,
}

/// Claims whose child could not be confirmed stopped, kept with that child and the build lock:
/// unknown exit is never released, so nothing takes the job over, deletes what the child may
/// still write or builds beside it. (The child holds its own workspace lock, so no reset runs.)
static RETAINED: Mutex<Vec<(Child, Claim, Option<File>)>> = Mutex::new(Vec::new());

/// Claims taken when this process admitted a job, until its `run` starts: a run that fails
/// before it can write (the workspace busy, say) drops its claim, so the queued job reads
/// interrupted instead of holding the head of the queue for as long as this process lives.
static PENDING: Mutex<Vec<(String, i64, Claim)>> = Mutex::new(Vec::new());

/// Commits the admission of attempt `attempt` of `job_id`, claimed first: a job is admitted
/// only with its claim, so no live owner's job is ever left without one.
fn admit(tx: rusqlite::Transaction, dir: &Path, job_id: &str, attempt: i64) -> Result<()> {
    let claim = claim(dir, job_id, attempt)?;
    if let Err(e) = tx.commit() {
        claim.done();
        return Err(sql(e));
    }
    PENDING
        .lock()
        .map_err(|_| "pending claims poisoned")?
        .push((job_id.to_owned(), attempt, claim));
    Ok(())
}

fn take_pending(job_id: &str) -> Option<(i64, Claim)> {
    let mut pending = PENDING.lock().ok()?;
    let at = pending.iter().position(|(j, ..)| j == job_id)?;
    let (_, attempt, claim) = pending.swap_remove(at);
    Some((attempt, claim))
}

fn claim_path(dir: &Path, job_id: &str, attempt: i64) -> PathBuf {
    dir.join("claims").join(format!("{job_id}.{attempt}.lock"))
}

/// `claims/<jobId>.<attempt>.worker`: created empty by the owner before it spawns that attempt's
/// worker; the worker locks it, writes `started` and keeps the lock until it exits.
fn worker_path(dir: &Path, job_id: &str, attempt: i64) -> PathBuf {
    dir.join("claims")
        .join(format!("{job_id}.{attempt}.worker"))
}

/// Whether the worker of attempt `attempt` may still run. It outlives a dead owner until its
/// stdin watchdog fires, or longer when stopped. No file means it was never spawned; an empty
/// unlocked one means it may not have locked yet, which counts as running.
fn worker_alive(dir: &Path, job_id: &str, attempt: i64) -> bool {
    match File::open(worker_path(dir, job_id, attempt)) {
        Ok(file) => {
            if file.try_lock_shared().is_err() {
                return true;
            }
            let started = file.metadata().is_ok_and(|m| m.len() > 0);
            let _ = file.unlock();
            !started
        }
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

fn claim(dir: &Path, job_id: &str, attempt: i64) -> Result<Claim> {
    mkdir(&dir.join("claims"))?;
    let path = claim_path(dir, job_id, attempt);
    let file = open_lock(&path, true)?.expect("created");
    file.try_lock().map_err(|_| {
        fail(
            ErrorKind::WorkspaceBusy,
            "another run holds this job attempt",
        )
    })?;
    Ok(Claim { path, _file: file })
}

impl Claim {
    /// The attempt's terminal state is recorded; the claim is no longer needed.
    fn done(self) {
        let _ = std::fs::remove_file(self.path.with_extension("worker"));
        let _ = std::fs::remove_file(&self.path);
    }

    /// The child exited but the terminal state could not be recorded: unlocked with its file
    /// kept, the job reads interrupted and a refresh retries it while this process lives.
    fn release(self) {}
}

/// Whether attempt `attempt` of `job_id`, held by `owner`, may still be running: its owner is
/// alive and its claim, once taken, is still held. A queued job not claimed yet is its live
/// owner's to run; anything that cannot be inspected counts as alive.
pub(crate) fn job_alive(dir: &Path, owner: &str, job_id: &str, attempt: i64) -> bool {
    if !alive(dir, owner) {
        return false;
    }
    match File::open(claim_path(dir, job_id, attempt)) {
        Ok(file) => match file.try_lock_shared() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(_) => true,
        },
        Err(_) => true,
    }
}

/// A job as `jobs status` reports it; an active job whose owner died reads `interrupted`.
pub(crate) fn report_job(conn: &Connection, dir: &Path, job_id: &str) -> Result<Option<Json>> {
    let row = conn
        .query_row(
            "SELECT kind, source_id, scope, state, stage, done, total, error, owner, attempt,
             cancel, gen, created_ms, heartbeat_ms, finished_ms, evicted FROM jobs
             WHERE job_id = ?1",
            [job_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, bool>(10)?,
                    r.get::<_, Option<i64>>(11)?,
                    [r.get::<_, i64>(12)?, r.get::<_, i64>(13)?],
                    (
                        r.get::<_, Option<i64>>(14)?,
                        r.get::<_, Option<String>>(15)?,
                    ),
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let Some((
        kind,
        source,
        scope,
        state,
        stage,
        done,
        total,
        error,
        owner,
        attempt,
        cancel,
        gen_id,
        [created, beat],
        (finished, evicted),
    )) = row
    else {
        return Ok(None);
    };
    let active = state == "queued" || state == "running";
    let state = if active && !job_alive(dir, &owner, job_id, attempt) {
        "interrupted".to_owned()
    } else {
        state
    };
    let error: Json = error
        .and_then(|e| serde_json::from_str(&e).ok())
        .unwrap_or(Json::Null);
    let ws = job_id.split('.').next().unwrap_or_default();
    let next = match state.as_str() {
        "queued" | "running" => {
            json!([{"tool": "jobs", "arguments": {"action": "status", "jobId": job_id}}])
        }
        "interrupted" | "failed" | "cancelled" => {
            // A rebuild again, so an unchanged snapshot is not reused instead of repaired; an
            // embed again, so no text build is repeated for a model layer.
            let action = match kind.as_str() {
                "rebuild" => "rebuild",
                "embed" => "embed",
                _ => "refresh",
            };
            json!([{"tool": "index", "arguments": {"action": action,
            "workspaceId": ws, "sourceId": source, "scope": scope},
            "why": format!("a {action} starts the build again as a new attempt; the previous index keeps serving")}])
        }
        "succeeded" if kind == "embed" => {
            json!([{"tool": "search", "arguments": {"workspaceId": ws, "node": scope, "query": "<words>", "mode": "semantic"}}])
        }
        "succeeded" => {
            json!([{"tool": "search", "arguments": {"workspaceId": ws, "node": scope, "query": "<words>"}}])
        }
        _ => json!([]),
    };
    let mut report = json!({"jobId": job_id, "workspaceId": ws, "kind": kind, "sourceId": source, "scope": scope,
        "state": state, "stage": stage, "progress": {"units": done, "total": total},
        "cancelRequested": cancel, "attempt": attempt, "generation": gen_id, "error": error,
        "createdMs": created, "heartbeatMs": beat, "finishedMs": finished, "nextSteps": next});
    // The scopes this job's publication retired under the scope cap, kept with the job.
    if let Some(evicted) = evicted.and_then(|e| serde_json::from_str::<Json>(&e).ok()) {
        report["evictedScopes"] = evicted;
    }
    Ok(Some(report))
}

/// The result of submitting a build: the job and whether this process must run it.
pub(crate) struct Submitted {
    pub job_id: String,
    /// True when this process owns the job and must run it; false when it joined another
    /// live owner's job or replayed a finished one.
    pub owned: bool,
    pub joined: bool,
}

fn fingerprint(kind: &str, ws: &str, source: &str, membership: &str, scope: &str) -> String {
    crate::search::hex(&Sha256::digest(
        json!([kind, ws, source, membership, scope]).to_string(),
    ))[..16]
        .to_owned()
}

/// Marks an active job as one another caller waits for, in the caller's join transaction, so a
/// watch runner never cancels it as its own (schema 5; earlier schemas have no runner).
fn share(tx: &Connection, job_id: &str) -> Result<()> {
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql)?;
    if version >= 5 {
        tx.execute(
            "UPDATE jobs SET shared = 1 WHERE job_id = ?1 AND state IN ('queued', 'running')",
            [job_id],
        )
        .map_err(sql)?;
    }
    Ok(())
}

/// The job and fingerprint `request` was first used for, if any.
fn requested(tx: &Connection, request: &str) -> Result<Option<(String, String)>> {
    tx.query_row(
        "SELECT job_id, fingerprint FROM jobs WHERE request_id = ?1
         UNION ALL SELECT job_id, fingerprint FROM requests WHERE request_id = ?1",
        [request],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(sql)
}

fn other_request(request: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::InvalidArguments,
        format!("requestId {request} was used for a different request; use a new requestId"),
    )
}

/// Records `request_id` for a job it joined or took over, so a retry replays that job; `''`
/// records a track that started none, so its retry replays without submitting.
fn remember(tx: &Connection, request_id: Option<&str>, print: &str, job_id: &str) -> Result<()> {
    if let Some(request) = request_id {
        tx.execute(
            "INSERT INTO requests (request_id, fingerprint, job_id) VALUES (?1, ?2, ?3)",
            params![request, print, job_id],
        )
        .map_err(sql)?;
    }
    Ok(())
}

/// Starts or joins the build of `scope` of `source_id`. One active job per source and scope,
/// whatever its kind: a second request joins it, and a job whose owner died is taken over as
/// a new attempt. A `request_id` replays its first job, and refuses a different request.
pub(crate) fn submit(
    storage: &Storage,
    ws: &mut Opened,
    kind: &str,
    source_id: &str,
    scope: &str,
    request_id: Option<&str>,
) -> Result<Submitted> {
    submit_fenced(storage, ws, kind, source_id, scope, request_id, None)
}

/// A condition an admission must still meet inside its own write, given the transaction and
/// the workspace incarnation.
pub(crate) type Fence<'a> = &'a dyn Fn(&Connection, &str) -> Result<bool>;

/// A fenced admission whose fence no longer held; nothing was replayed, joined or admitted.
#[derive(Debug)]
pub(crate) struct Unfenced;

impl std::fmt::Display for Unfenced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the admission's fence no longer holds")
    }
}

impl Error for Unfenced {}

/// `submit`, refused with `Unfenced` when `fence` fails inside the admission's write.
#[allow(clippy::too_many_arguments)]
fn submit_fenced(
    storage: &Storage,
    ws: &mut Opened,
    kind: &str,
    source_id: &str,
    scope: &str,
    request_id: Option<&str>,
    fence: Option<Fence>,
) -> Result<Submitted> {
    // A tracking baseline is a rebuild, but its requestId names a different operation.
    let (op, kind) = (kind, if kind == "track" { "rebuild" } else { kind });
    let dir = ws.held.dir.clone();
    let me = owner(&dir)?;
    let (id, incarnation) = (ws.held.id.clone(), ws.incarnation.clone());
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    // Visibility is judged again inside this write: a source moved out of view since it was
    // chosen is unknown here, before any replay or admission.
    let membership: String = registry::visible_path(storage, &tx, source_id)?
        .map(|_| {
            tx.query_row(
                "SELECT membership FROM sources WHERE source_id = ?1",
                [source_id],
                |r| r.get(0),
            )
        })
        .transpose()
        .map_err(sql)?
        .ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("workspace {id} has no source {source_id}"),
            )
        })?;
    if let Some(fence) = fence
        && !fence(&tx, &incarnation)?
    {
        return Err(Unfenced.into());
    }
    // An embed job embeds the scope's published generation under the active profile: both are
    // in its key, so an embed of an older generation or profile is never joined or replayed.
    let embed = kind == "embed";
    let (gen_key, profile_key) = if embed {
        let gen_id: i64 = tx
            .query_row(
                "SELECT gen FROM published WHERE source_id = ?1 AND scope = ?2",
                [source_id, scope],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?
            .ok_or_else(|| {
                fail(
                    ErrorKind::InvalidArguments,
                    format!("scope {scope} of source {source_id} has no published index to embed; prepare or refresh it first"),
                )
            })?;
        let profile = crate::semantic::profile::active(storage)?.ok_or_else(no_profile)?;
        (Some(gen_id), Some(profile.id))
    } else {
        (None, None)
    };
    let key = match (gen_key, &profile_key) {
        (Some(g), Some(p)) => format!("{scope}\n{g}\n{p}\n{}", super::model::POLICY),
        _ => scope.to_owned(),
    };
    let print = fingerprint(op, &id, source_id, &membership, &key);
    if let Some(request) = request_id {
        match requested(&tx, request)? {
            Some((job_id, fp)) if fp == print => {
                share(&tx, &job_id)?;
                tx.commit().map_err(sql)?;
                return Ok(Submitted {
                    job_id,
                    owned: false,
                    joined: true,
                });
            }
            Some(_) => return Err(other_request(request)),
            None => {}
        }
    }
    let active: Option<(String, String, i64, String, bool)> = tx
        .query_row(
            "SELECT job_id, owner, attempt, membership, cancel FROM jobs WHERE source_id = ?1
             AND scope = ?2 AND state IN ('queued', 'running') AND (kind = 'embed') = ?3
             AND (?3 = 0 OR (expected_gen = ?4 AND profile_id = ?5))",
            params![source_id, scope, embed, gen_key, profile_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(sql)?;
    if let Some((job_id, holder, attempt, built, cancelled)) = active {
        let owned = !job_alive(&dir, &holder, &job_id, attempt);
        // A live build already cancelled (by its caller, or a closing watch runner) only stops;
        // joining it would end this request cancelled too.
        if !owned && cancelled {
            return Err(FigError::new(
                ErrorKind::WorkspaceBusy,
                format!("a cancelled build of source {source_id} scope {scope} is still stopping; retry after it ends"),
            )
            .with_recovery(json!([{"action": "wait", "requires": "none", "tool": "jobs",
                "arguments": {"action": "status", "jobId": job_id}}]))
            .into());
        }
        // A live build of the source's previous registration cannot publish; joining it would
        // build nothing, and its slot frees once it stops.
        if !owned && built != membership {
            return Err(FigError::new(
                ErrorKind::WorkspaceBusy,
                format!("a build of source {source_id}'s previous path is still stopping; retry after it ends"),
            )
            .with_recovery(json!([{"action": "wait", "requires": "none", "tool": "jobs",
                "arguments": {"action": "status", "jobId": job_id}}]))
            .into());
        }
        if owned {
            // Its owner died: this process takes it over as a new attempt from the start.
            tx.execute(
                "UPDATE jobs SET owner = ?2, attempt = attempt + 1, state = 'queued', stage = NULL,
                 done = 0, total = NULL, error = NULL, cancel = 0, membership = ?3, heartbeat_ms = ?4,
                 seq = (SELECT max(seq) + 1 FROM jobs),
                 kind = CASE WHEN ?5 = 'rebuild' THEN 'rebuild' ELSE kind END WHERE job_id = ?1",
                params![job_id, me, membership, now_ms(), kind],
            )
            .map_err(sql)?;
            // The dead owner's waiters still wait for it, so a takeover counts as shared.
            share(&tx, &job_id)?;
            remember(&tx, request_id, &print, &job_id)?;
            admit(tx, &dir, &job_id, attempt + 1)?;
            return Ok(Submitted {
                job_id,
                owned,
                joined: false,
            });
        } else if kind == "rebuild" {
            // A running refresh already read its kind and may reuse the unchanged snapshot, so
            // a rebuild cannot join it: it runs after it, and its requestId is not recorded.
            let (state, current): (String, String) = tx
                .query_row(
                    "SELECT state, kind FROM jobs WHERE job_id = ?1",
                    [&job_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(sql)?;
            if state == "running" && current != "rebuild" {
                return Err(FigError::new(
                    ErrorKind::WorkspaceBusy,
                    format!("a refresh of source {source_id} scope {scope} is running and may reuse its unchanged snapshot; rebuild after it ends"),
                )
                .with_recovery(json!([{"action": "wait", "requires": "none", "tool": "jobs",
                    "arguments": {"action": "status", "jobId": job_id}},
                    {"action": "rebuild", "requires": "none", "tool": "index",
                    "arguments": {"action": "rebuild", "workspaceId": id, "sourceId": source_id, "scope": scope}}]))
                .into());
            }
            // A rebuild joining a queued refresh makes it skip the unchanged-snapshot shortcut:
            // its worker reads the kind only once the job runs.
            tx.execute(
                "UPDATE jobs SET kind = 'rebuild' WHERE job_id = ?1 AND state = 'queued'",
                [&job_id],
            )
            .map_err(sql)?;
        }
        remember(&tx, request_id, &print, &job_id)?;
        share(&tx, &job_id)?;
        tx.commit().map_err(sql)?;
        return Ok(Submitted {
            job_id,
            owned,
            joined: !owned,
        });
    }
    let (mut live, mut shown) = (0, Vec::new());
    let mut stmt = tx
        .prepare(
            "SELECT j.job_id, j.owner, j.attempt, s.path FROM jobs j
             LEFT JOIN sources s ON s.source_id = j.source_id WHERE j.state IN ('queued', 'running')",
        )
        .map_err(sql)?;
    for row in stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(sql)?
    {
        let (job, holder, attempt, path) = row.map_err(sql)?;
        if job_alive(&dir, &holder, &job, attempt) {
            live += 1;
            // A confined caller is never told about jobs of sources it cannot see.
            if path.is_some_and(|p| storage.sees(Path::new(&p))) {
                shown.push(job);
            }
        }
    }
    drop(stmt);
    if live >= MAX_ACTIVE {
        let mut wait = json!({"action": "wait", "requires": "none", "jobIds": shown});
        if let Some(first) = shown.first() {
            wait["tool"] = json!("jobs");
            wait["arguments"] = json!({"action": "status", "jobId": first});
        }
        return Err(FigError::new(
            ErrorKind::WorkspaceBusy,
            format!("workspace {id} already has {MAX_ACTIVE} queued or running jobs; retry after one finishes"),
        )
        .with_recovery(json!([wait]))
        .into());
    }
    if embed {
        // An embed of an older generation or another profile is obsolete: its fence would
        // refuse its writes anyway, so it stops now instead of at its next batch.
        tx.execute(
            "UPDATE jobs SET cancel = 1 WHERE kind = 'embed' AND source_id = ?1 AND scope = ?2
             AND state IN ('queued', 'running') AND NOT (expected_gen = ?3 AND profile_id = ?4)",
            params![source_id, scope, gen_key, profile_key],
        )
        .map_err(sql)?;
    }
    let job_id = format!("{id}.{}", random_hex());
    let now = now_ms();
    // Text jobs set `expected_gen` when they start; an embed names its generation here.
    tx.execute(
        "INSERT INTO jobs (job_id, request_id, fingerprint, kind, source_id, membership, scope,
         incarnation, state, owner, created_ms, heartbeat_ms, seq, expected_gen, profile_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', ?9, ?10, ?10,
                 (SELECT COALESCE(max(seq), 0) + 1 FROM jobs), ?11, ?12)",
        params![
            job_id,
            request_id,
            print,
            kind,
            source_id,
            membership,
            scope,
            incarnation,
            me,
            now,
            gen_key,
            profile_key
        ],
    )
    .map_err(sql)?;
    // Keep the newest terminal jobs only.
    tx.execute(
        "DELETE FROM jobs WHERE state NOT IN ('queued', 'running') AND job_id NOT IN
         (SELECT job_id FROM jobs WHERE state NOT IN ('queued', 'running')
          ORDER BY created_ms DESC LIMIT ?1)",
        [KEPT],
    )
    .map_err(sql)?;
    // A request that started no job (`''`) is retired where it is recorded.
    tx.execute(
        "DELETE FROM requests WHERE job_id <> '' AND job_id NOT IN (SELECT job_id FROM jobs)",
        [],
    )
    .map_err(sql)?;
    admit(tx, &dir, &job_id, 1)?;
    hold("submit");
    Ok(Submitted {
        job_id,
        owned: true,
        joined: false,
    })
}

/// Deletes generation `gen_id` and everything it holds, in one transaction.
fn drop_gen(conn: &Connection, gen_id: i64) -> rusqlite::Result<()> {
    for table in [
        "records",
        "terms",
        "edges",
        "definitions",
        "facets",
        "facet_sets",
        "fingerprints",
        "model_refs",
        "model_layers",
        "generations",
    ] {
        conn.execute(&format!("DELETE FROM {table} WHERE gen = ?1"), [gen_id])?;
    }
    super::model::collect(conn)
}

/// Deletes the unpublished generations of job `job_id`, one transaction each, only while its
/// row still names `holder`'s `attempt`: a stale run never deletes a newer attempt's work.
fn drop_building(conn: &mut Connection, job_id: &str, holder: &str, attempt: i64) -> Result<()> {
    loop {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let gen_id: Option<i64> = tx
            .query_row(
                "SELECT g.gen FROM generations g JOIN jobs j ON j.job_id = g.job_id
                 WHERE g.job_id = ?1 AND g.state = 'building' AND j.owner = ?2 AND j.attempt = ?3
                 LIMIT 1",
                params![job_id, holder, attempt],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        let Some(gen_id) = gen_id else {
            return Ok(());
        };
        drop_gen(&tx, gen_id).map_err(sql)?;
        tx.commit().map_err(sql)?;
    }
}

/// Ends job `job_id` in a terminal state, if `me`'s `attempt` still owns it and it is active.
fn finish(
    conn: &Connection,
    job_id: &str,
    me: &str,
    attempt: i64,
    state: &str,
    error: Option<Json>,
) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET state = ?4, error = ?5, finished_ms = ?6 WHERE job_id = ?1 AND owner = ?2
         AND attempt = ?3 AND state IN ('queued', 'running')",
        params![
            job_id,
            me,
            attempt,
            state,
            error.map(|e| e.to_string()),
            now_ms()
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// Stops `child` and reaps it; true only when it is known to have exited. Closing its stdin
/// stops it even when the kill fails, so the wait ends either way.
fn stop(child: &mut Child) -> bool {
    drop(child.stdin.take());
    crate::semantic::kill_tree(child);
    let _ = child.kill();
    child.wait().is_ok()
}

/// Ends a job whose run failed with `e`, holding the caller's locks throughout: stops and reaps
/// its child, records `failed`, then drops its unpublished generation. If the child's exit
/// cannot be confirmed, nothing is recorded or deleted and the claim stays held. If only the
/// terminal write fails, the claim is released, so the job reads `interrupted` and a refresh
/// retries it while this process lives.
fn abandon(
    conn: &mut Connection,
    job_id: &str,
    me: &str,
    attempt: i64,
    (child, claim, build_lock): (Option<Child>, Claim, Option<File>),
    e: Box<dyn Error>,
) -> Box<dyn Error> {
    if let Some(mut child) = child
        && !stop(&mut child)
    {
        // Unknown exit: nothing is recorded or deleted, and the claim and build lock stay held.
        if let Ok(mut kept) = RETAINED.lock() {
            kept.push((child, claim, build_lock));
        }
        return e;
    }
    let code = e
        .downcast_ref::<FigError>()
        .map_or("operation_failed", |f| kind_code(f.kind()));
    match finish(
        conn,
        job_id,
        me,
        attempt,
        "failed",
        Some(error_json(code, &e)),
    ) {
        Ok(()) => {
            let _ = drop_building(conn, job_id, me, attempt);
            claim.done();
        }
        Err(_) => claim.release(),
    }
    e
}

/// The launch token a worker child writes under: the owner and attempt that started it.
const LAUNCH: &str = "FIGLENS_INDEX_LAUNCH";

fn error_json(code: &str, message: impl std::fmt::Display) -> Json {
    json!({"code": code, "message": message.to_string()})
}

/// Whether a cancel was requested in this process or in the job row.
fn cancelled(conn: &Connection, job_id: &str, local: &AtomicBool) -> Result<bool> {
    if local.load(Ordering::Relaxed) {
        return Ok(true);
    }
    conn.query_row("SELECT cancel FROM jobs WHERE job_id = ?1", [job_id], |r| {
        r.get::<_, bool>(0)
    })
    .optional()
    .map_err(sql)
    .map(|c| c.unwrap_or(true))
}

/// The child process that builds one job. Its stdin closes when this process exits or dies,
/// which asks it to stop; a stopped child can outlive its owner, so it holds the worker lock.
/// `model` is the profile's runtime for an embed job: it runs `__model-worker` itself, as the
/// root of its own process tree, so stopping it ends whatever the model started too.
fn spawn(dir: &Path, job_id: &str, me: &str, attempt: i64, model: Option<&Path>) -> Result<Child> {
    let exe = match model {
        Some(exe) => exe.to_owned(),
        None => std::env::current_exe()?,
    };
    // Before the child exists, so a cancel never mistakes a starting worker for an exited one.
    mkdir(&dir.join("claims"))?;
    let path = worker_path(dir, job_id, attempt);
    File::create(&path).map_err(|e| storage_error(&path, e))?;
    let mut cmd = Command::new(exe);
    cmd.arg(match model {
        Some(_) => "__model-worker",
        None => "__index-worker",
    })
    .arg(dir)
    .arg(job_id)
    .env(LAUNCH, format!("{me}:{attempt}"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let child = match model {
        Some(_) => crate::semantic::spawn_owned(&mut cmd),
        None => cmd.spawn().map_err(Into::into),
    };
    child.map_err(|e| {
        fail(
            ErrorKind::OperationFailed,
            format!("cannot start the index worker: {e}"),
        )
    })
}

/// The runtime of the active profile, while it is still the one an embed job was submitted for.
fn model_exe(storage: &Storage, profile_id: Option<&str>) -> Result<PathBuf> {
    match crate::semantic::profile::active(storage)? {
        // Checked before it runs: a replaced runtime or model must never be started.
        Some(p) if Some(p.id.as_str()) == profile_id => {
            p.unchanged(&AtomicBool::new(false)).map_err(|e| {
                fail(
                    ErrorKind::OperationFailed,
                    format!("{e}; run models configure again"),
                )
            })?;
            Ok(p.exe)
        }
        _ => Err(fail(
            ErrorKind::OperationFailed,
            "the configured model profile changed since this embed job was submitted; run index embed again",
        )),
    }
}

/// Test builds only: `FIGLENS_TEST_HOLD=<stage>:<file>` keeps the worker in `stage` while
/// `file` exists and creates `<file>.reached` when it gets there, so process tests reach each
/// stage deterministically. `start` is before the worker locks its worker file; `submit` holds
/// the submitting process once its new job is admitted.
pub(super) fn hold(stage: &str) {
    #[cfg(debug_assertions)]
    if let Some((at, file)) = std::env::var("FIGLENS_TEST_HOLD")
        .ok()
        .as_deref()
        .and_then(|v| v.split_once(':'))
        && at == stage
    {
        let _ = std::fs::write(format!("{file}.reached"), "");
        let start = Instant::now();
        while Path::new(file).exists() && start.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let _ = stage;
}

/// Runs job `job_id`, which this process owns, to a terminal state, and returns its report.
/// `cancel` stops it from this process; a `jobs cancel` from any process sets the row's flag,
/// polled every `POLL`. A cancel kills the child, waits for it to exit, then records
/// `cancelled`; publication commits only if no cancel landed first.
pub fn run(
    storage: &Storage,
    ws_id: &str,
    job_id: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Json),
) -> Result<Json> {
    // First, so any early error or panic below drops it and the job reads interrupted.
    let pending = take_pending(job_id);
    let select = Select::Id(ws_id.to_owned());
    let (_, mut ws) = opened(storage, &select, true)?;
    let dir = ws.held.dir.clone();
    let me = owner(&dir)?;
    let mine: Option<(String, String, i64, bool)> = ws
        .conn
        .query_row(
            "SELECT owner, state, attempt, kind = 'embed' FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(sql)?;
    let embed = mine.as_ref().is_some_and(|m| m.3);
    let Some(attempt) = mine
        .filter(|(o, s, ..)| *o == me && s == "queued")
        .map(|(_, _, a, _)| a)
    else {
        // Ended (cancelled while queued, say) or taken over: its claim is no longer needed.
        if let Some((_, claim)) = pending {
            claim.done();
        }
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    };
    // This attempt is this process's to end: every exit from here records its terminal state
    // or, if that write fails, releases its claim once nothing runs for it (see `abandon`).
    let claimed = match pending {
        Some((held, claim)) if held == attempt => Ok(claim),
        stale => {
            if let Some((_, claim)) = stale {
                claim.done();
            }
            claim(&dir, job_id, attempt)
        }
    };
    let claim = match claimed {
        Ok(claim) => claim,
        // Another run in this process holds this attempt, and ends it.
        Err(e)
            if e.downcast_ref::<FigError>()
                .is_some_and(|f| f.kind() == ErrorKind::WorkspaceBusy) =>
        {
            return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
        }
        Err(e) => {
            let error = error_json("operation_failed", &e);
            finish(&ws.conn, job_id, &me, attempt, "failed", Some(error))?;
            return Err(e);
        }
    };
    // Held here, not in `build`, so an error keeps it (and the workspace lock) until the child
    // is stopped and reaped. Embed jobs take turns on their own lock, so a text build never
    // waits for a model job.
    let lock_name = if embed { "embed.lock" } else { "build.lock" };
    let build_lock = match open_lock(&dir.join(lock_name), true) {
        Ok(lock) => lock.expect("created"),
        Err(e) => {
            return Err(abandon(
                &mut ws.conn,
                job_id,
                &me,
                attempt,
                (None, claim, None),
                e,
            ));
        }
    };
    let mut child = None;
    let built = match build(
        storage,
        &mut ws,
        &dir,
        job_id,
        &me,
        attempt,
        &build_lock,
        cancel,
        progress,
        &mut child,
    ) {
        Ok(Some(built)) => built,
        Ok(None) => {
            claim.done();
            return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
        }
        Err(e) => {
            let held = (child, claim, Some(build_lock));
            return Err(abandon(&mut ws.conn, job_id, &me, attempt, held, e));
        }
    };
    if embed {
        drop(build_lock);
        // The worker recorded its layer and the job's success in one fenced write; a job it
        // left running ended without that.
        let error = error_json(
            "operation_failed",
            "the model worker exited without recording its layer",
        );
        finish(&ws.conn, job_id, &me, attempt, "failed", Some(error))?;
        claim.done();
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    }
    // Publication: release the build and workspace locks, then take the location lock shared and
    // the workspace lock again (the lock order), re-open and re-validate everything, and publish
    // in one short transaction. The child has exited; the owner and claim locks persist, so
    // nobody takes the job over meanwhile.
    drop(build_lock);
    drop(ws);
    match publish(storage, ws_id, job_id, &me, attempt, &built) {
        Ok(report) => {
            claim.done();
            Ok(report)
        }
        Err(e) => match opened(storage, &select, true) {
            Ok((_, mut ws)) => Err(abandon(
                &mut ws.conn,
                job_id,
                &me,
                attempt,
                (None, claim, None),
                e,
            )),
            Err(_) => {
                claim.release();
                Err(e)
            }
        },
    }
}

/// Whether `job_id` is the first live queued job in admission order. `seq` is assigned under
/// the submit transaction, at admission and again at takeover, so each owner's jobs are in the
/// order it runs them and no two owners can wait on each other.
/// Embed jobs (`embed`) queue apart from text builds, on their own lock.
fn my_turn(conn: &Connection, dir: &Path, job_id: &str, embed: bool) -> Result<bool> {
    let mut stmt = conn
        .prepare(
            "SELECT job_id, owner, attempt FROM jobs WHERE state = 'queued'
             AND (kind = 'embed') = ?1 ORDER BY seq, rowid",
        )
        .map_err(sql)?;
    for row in stmt
        .query_map([embed], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(sql)?
    {
        let (job, holder, held) = row.map_err(sql)?;
        if job == job_id {
            return Ok(true);
        }
        if job_alive(dir, &holder, &job, held) {
            return Ok(false);
        }
    }
    Ok(false)
}

/// Reads `child`'s whole output on a thread; the receiver gets one message when it ends, which
/// is at the child's exit (or a read error).
fn output(child: &mut Child) -> (std::thread::JoinHandle<String>, Receiver<()>) {
    let mut stdout = child.stdout.take().expect("piped");
    let (ended, eof) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = ended.send(());
        out
    });
    (reader, eof)
}

/// Waits for the worker in `child` to exit, running `tick` (cancel check, progress, heartbeat)
/// between waits, and returns its status; `tick` answering true stops and reaps it (`None`).
/// The end of its output wakes the wait: an exiting process closes it just before it can be
/// reaped, so the next `POLL` is waited in 5 ms steps, then whole again, never spinning on a
/// worker that closed its output early.
fn wait_worker(
    child: &mut Option<Child>,
    eof: &Receiver<()>,
    mut tick: impl FnMut() -> Result<bool>,
) -> Result<Option<std::process::ExitStatus>> {
    const STEP: Duration = Duration::from_millis(5);
    let mut steps = 0;
    loop {
        let running = child.as_mut().expect("spawned");
        if let Some(status) = running.try_wait()? {
            return Ok(Some(status));
        }
        if tick()? {
            // Stopped before the output is joined: the reader ends when the child does. An exit
            // that cannot be confirmed is no cancel: the child stays with the caller.
            if !stop(running) {
                return Err(fail(
                    ErrorKind::OperationFailed,
                    "the index worker could not be confirmed stopped",
                ));
            }
            *child = None;
            return Ok(None);
        }
        match eof.recv_timeout(POLL) {
            Err(RecvTimeoutError::Disconnected) if steps < POLL.as_millis() / STEP.as_millis() => {
                steps += 1;
                std::thread::sleep(STEP);
            }
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
            _ => {}
        }
    }
}

/// Waits for the build lock, then runs attempt `attempt` of job `job_id` in a child until it
/// exits. Returns the child's result to publish, or `None` once the job ended here
/// (cancelled or failed). `child` holds the running child, so a caller seeing an error can
/// stop and reap it.
#[allow(clippy::too_many_arguments)]
fn build(
    storage: &Storage,
    ws: &mut Opened,
    dir: &Path,
    job_id: &str,
    me: &str,
    attempt: i64,
    build: &File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Json),
    child: &mut Option<Child>,
) -> Result<Option<Json>> {
    let (kind, profile_id): (String, Option<String>) = ws
        .conn
        .query_row(
            "SELECT kind, profile_id FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    let embed = kind == "embed";
    // Wait for the build lock, holding it only while this job is the oldest live queued job,
    // so no process holds it while waiting for its turn.
    let (waited, mut beat) = (Instant::now(), Instant::now());
    loop {
        if cancelled(&ws.conn, job_id, cancel)? {
            finish(&ws.conn, job_id, me, attempt, "cancelled", None)?;
            return Ok(None);
        }
        if waited.elapsed() > QUEUE_WAIT {
            return Err(fail(
                ErrorKind::WorkspaceBusy,
                "the job waited too long for its turn to build; refresh again later",
            ));
        }
        if build.try_lock().is_ok() {
            if my_turn(&ws.conn, dir, job_id, embed)? {
                break;
            }
            let _ = build.unlock();
        }
        if beat.elapsed() > Duration::from_secs(1) {
            ws.conn
                .execute(
                    "UPDATE jobs SET heartbeat_ms = ?2 WHERE job_id = ?1",
                    params![job_id, now_ms()],
                )
                .map_err(sql)?;
            beat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let started = ws
        .conn
        .execute(
            "UPDATE jobs SET state = 'running', stage = 'stage', heartbeat_ms = ?3,
             expected_gen = CASE WHEN kind = 'embed' THEN expected_gen ELSE
                            (SELECT p.gen FROM published p JOIN jobs j ON j.job_id = ?1
                             WHERE p.source_id = j.source_id AND p.scope = j.scope) END
             WHERE job_id = ?1 AND owner = ?2 AND attempt = ?4 AND state = 'queued' AND cancel = 0",
            params![job_id, me, now_ms(), attempt],
        )
        .map_err(sql)?;
    if started == 0 {
        finish(&ws.conn, job_id, me, attempt, "cancelled", None)?;
        return Ok(None);
    }
    drop_building(&mut ws.conn, job_id, me, attempt)?;
    // The source can leave the visible roots (a symlink swapped, say) while the job queued.
    if !visible_job(storage, &ws.conn, job_id)? {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "the job's source is no longer inside the visible roots",
        ));
    }
    let model = match embed {
        true => Some(model_exe(storage, profile_id.as_deref())?),
        false => None,
    };
    let running = child.insert(spawn(dir, job_id, me, attempt, model.as_deref())?);
    let (reader, eof) = output(running);
    let mut last = Json::Null;
    let status = wait_worker(child, &eof, || {
        if cancelled(&ws.conn, job_id, cancel)? {
            return Ok(true);
        }
        if let Some(now) = report_job(&ws.conn, dir, job_id)?
            && (now["stage"] != last["stage"] || now["progress"] != last["progress"])
        {
            progress(&now);
            last = now;
        }
        if beat.elapsed() > Duration::from_secs(1) {
            ws.conn
                .execute(
                    "UPDATE jobs SET heartbeat_ms = ?2 WHERE job_id = ?1",
                    params![job_id, now_ms()],
                )
                .map_err(sql)?;
            beat = Instant::now();
        }
        Ok(false)
    })?;
    if let Some(exited) = child.as_ref() {
        crate::semantic::forget_tree(exited);
    }
    let output = reader.join().unwrap_or_default();
    let built: Json = output
        .lines()
        .last()
        .and_then(|l| serde_json::from_str(l).ok())
        .unwrap_or(Json::Null);
    match status {
        None => {
            // Cancelled and reaped: nothing more is written for this attempt.
            finish(&ws.conn, job_id, me, attempt, "cancelled", None)?;
            drop_building(&mut ws.conn, job_id, me, attempt)?;
            return Ok(None);
        }
        // A model worker that met its fence after a cancel landed was cancelled.
        Some(s) if embed && !s.success() && cancelled(&ws.conn, job_id, cancel)? => {
            *child = None;
            finish(&ws.conn, job_id, me, attempt, "cancelled", None)?;
            return Ok(None);
        }
        Some(s) if !s.success() || built["error"].is_object() || !built["gen"].is_i64() => {
            let error = if built["error"].is_object() {
                built["error"].clone()
            } else {
                error_json(
                    "operation_failed",
                    format!("the index worker exited with {s}"),
                )
            };
            *child = None;
            finish(&ws.conn, job_id, me, attempt, "failed", Some(error))?;
            drop_building(&mut ws.conn, job_id, me, attempt)?;
            return Ok(None);
        }
        Some(_) => *child = None,
    }
    Ok(Some(built))
}

fn unknown_job(job_id: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::InvalidArguments,
        format!("no job {job_id}; jobs are kept until 50 newer ones finish"),
    )
}

/// The location lock of workspace `id`, shared: a replacement (exclusive) never interleaves
/// with a publication.
fn location_shared(storage: &Storage, id: &str) -> Result<File> {
    let locations = workspaces(&storage.defaults.config.path, true)?.expect("created");
    let path = locations.join(format!("{id}.location.lock"));
    let file = open_lock(&path, true)?.expect("created");
    lock(&file, &path, false, id)?;
    Ok(file)
}

/// Publishes the generation a child built, after re-validating under the location and
/// workspace locks: the database is still the one the location record binds, with the job's
/// incarnation; this process still owns the running job and no cancel landed; the source keeps
/// the job's membership; and the published generation is still the one the job replaces.
/// owner, state, source_id, scope, membership, expected_gen, cancel, incarnation
type PublishRow = (
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
    bool,
    String,
);

pub(crate) fn publish(
    storage: &Storage,
    ws_id: &str,
    job_id: &str,
    me: &str,
    attempt: i64,
    built: &Json,
) -> Result<Json> {
    let _location = location_shared(storage, ws_id)?;
    // `opened` re-reads the location record and refuses a database it no longer binds.
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let dir = ws.held.dir.clone();
    let gen_id = built["gen"].as_i64().expect("checked");
    let unchanged = built["unchanged"] == true;
    let incarnation = ws.incarnation.clone();
    // A tracked scope's generations are compared before the publication, outside any write
    // transaction; the publication only writes the result if tracking is still as read.
    let planned = match unchanged {
        true => None,
        false => plan(&mut ws.conn, job_id, gen_id)?,
    };
    hold("compare");
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let job: Option<PublishRow> = tx
        .query_row(
            "SELECT owner, state, source_id, scope, membership, expected_gen, cancel, incarnation
             FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let Some((holder, state, source, scope, membership, expected, cancel, job_incarnation)) = job
    else {
        return Err(unknown_job(job_id));
    };
    let current_attempt: i64 = tx
        .query_row(
            "SELECT attempt FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let current: Option<String> = tx
        .query_row(
            "SELECT membership FROM sources WHERE source_id = ?1",
            [&source],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    let published: Option<i64> = tx
        .query_row(
            "SELECT gen FROM published WHERE source_id = ?1 AND scope = ?2",
            [&source, &scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    let refusal = if job_incarnation != incarnation {
        Some((
            "workspace_recreated",
            "the workspace was reset or recreated while the job ran",
        ))
    } else if holder != me || current_attempt != attempt || state != "running" {
        Some(("not_owner", "another process took the job over"))
    } else if cancel {
        Some(("cancelled", ""))
    } else if current.as_deref() != Some(membership.as_str()) {
        Some((
            "membership_changed",
            "the source was removed, re-added or relocated while the job ran",
        ))
    } else if published != expected {
        Some((
            "superseded",
            "another build published this source and scope first",
        ))
    } else {
        None
    };
    if let Some((code, why)) = refusal {
        drop(tx);
        // Both are fenced to this attempt, so a taken-over job and its new attempt's
        // generation are left alone.
        if code == "cancelled" {
            finish(&ws.conn, job_id, me, attempt, "cancelled", None)?;
        } else {
            finish(
                &ws.conn,
                job_id,
                me,
                attempt,
                "failed",
                Some(error_json(code, why)),
            )?;
        }
        drop_building(&mut ws.conn, job_id, me, attempt)?;
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    }
    let now = now_ms();
    tx.execute(
        "INSERT INTO snapshots (source_id, snapshot_id, bytes, mtime_ns, verified_ms)
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (source_id, snapshot_id) DO UPDATE SET
         bytes = excluded.bytes, mtime_ns = excluded.mtime_ns, verified_ms = excluded.verified_ms",
        params![
            source,
            built["snapshot"].as_str(),
            built["bytes"].as_i64(),
            built["mtimeNs"].as_i64(),
            now
        ],
    )
    .map_err(sql)?;
    if !unchanged {
        tx.execute(
            "UPDATE generations SET state = 'published' WHERE gen = ?1 AND state = 'building'",
            [gen_id],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE generations SET state = 'retired', retired_ms = ?2 WHERE gen = ?1",
            params![expected, now],
        )
        .map_err(sql)?;
    }
    tx.execute(
        "INSERT INTO published (source_id, scope, gen, published_ms) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (source_id, scope) DO UPDATE SET gen = excluded.gen, published_ms = excluded.published_ms",
        params![source, scope, gen_id, now],
    )
    .map_err(sql)?;
    // Bounded scopes: the least recently published one beyond the cap is retired, reported and
    // pruned like the job's own scope.
    let evicted: Vec<String> = tx
        .prepare(
            "SELECT scope FROM published WHERE source_id = ?1 ORDER BY published_ms DESC, scope
             LIMIT -1 OFFSET ?2",
        )
        .and_then(|mut s| {
            s.query_map(params![source, MAX_SCOPES], |r| r.get(0))?
                .collect()
        })
        .map_err(sql)?;
    tx.execute(
        "UPDATE generations SET state = 'retired', retired_ms = ?2 WHERE gen IN
         (SELECT gen FROM published WHERE source_id = ?1 ORDER BY published_ms DESC, scope
          LIMIT -1 OFFSET ?3)",
        params![source, now, MAX_SCOPES],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM published WHERE source_id = ?1 AND gen IN
         (SELECT gen FROM generations WHERE state = 'retired')",
        [&source],
    )
    .map_err(sql)?;
    let recorded = match unchanged {
        true => false,
        false => record(&tx, &source, &scope, gen_id, expected, built, planned, now)?,
    };
    let evicted_json = (!evicted.is_empty()).then(|| json!(evicted).to_string());
    tx.execute(
        "UPDATE jobs SET state = 'succeeded', stage = 'publish', gen = ?2, finished_ms = ?3,
         evicted = ?4 WHERE job_id = ?1",
        params![job_id, gen_id, now, evicted_json],
    )
    .map_err(sql)?;
    // The replaced generation's facets were only kept to be compared.
    if let (true, Some(old)) = (recorded, expected) {
        for table in ["facets", "facet_sets"] {
            tx.execute(&format!("DELETE FROM {table} WHERE gen = ?1"), [old])
                .map_err(sql)?;
        }
    }
    tx.commit().map_err(sql)?;
    // Retention, outside the publication: one retired generation per source and scope.
    // An evicted scope serves nothing, so it keeps no retired generation: retention stays at
    // one per published scope, however many scopes a source has seen.
    for scope in &evicted {
        prune(&mut ws.conn, &source, scope, 0)?;
    }
    prune(&mut ws.conn, &source, &scope, 1)?;
    report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id))
}

/// Most revision groups kept per source and scope; older whole groups are deleted.
const REVISIONS: i64 = if cfg!(test) { 3 } else { 1000 };
/// Change-row bytes kept per source and scope; older groups beyond it keep only their summary.
const DETAIL_BYTES: i64 = if cfg!(test) { 1024 } else { 16 * 1024 * 1024 };

/// What a tracked scope's publication will record, compared against the generation it replaces.
struct Planned {
    epoch: i64,
    from: i64,
    /// `Err` is a reason the change is a new baseline; an `Err` inside is a failed comparison.
    what: std::result::Result<std::result::Result<search::diff::Revision, String>, &'static str>,
}

/// Compares the generation job `job_id` expects to replace with its new one `gen_id`, in one
/// read transaction, when the scope is actively tracked; `None` otherwise.
fn plan(conn: &mut Connection, job_id: &str, gen_id: i64) -> Result<Option<Planned>> {
    let tx = conn.transaction().map_err(sql)?;
    let row: Option<(i64, i64, String, Option<String>, String)> = tx
        .query_row(
            "SELECT t.epoch, j.expected_gen, j.source_id, g.snapshot_id, g.versions FROM jobs j
             JOIN tracking t ON t.source_id = j.source_id AND t.scope = j.scope
             AND t.state = 'active' JOIN generations g ON g.gen = j.expected_gen
             WHERE j.job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(sql)?;
    let Some((epoch, from, source, old_snapshot, old_versions)) = row else {
        return Ok(None);
    };
    let (scope, new_snapshot): (String, Option<String>) = tx
        .query_row(
            "SELECT scope, snapshot_id FROM generations WHERE gen = ?1",
            [gen_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    let version: Option<String> = tx
        .query_row(
            "SELECT facet_version FROM facet_sets WHERE gen = ?1",
            [from],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    // A previous generation without facets, or with another field layout or extraction (such
    // as relations that now keep library references), is only a baseline: comparing it would
    // report a layout or extraction change as design changes.
    let what = match version {
        None => Err("previous_untracked"),
        Some(v) if v != search::diff::facet_version() => Err("facet_version_changed"),
        Some(_) if old_versions != search::store::versions() => Err("index_version_changed"),
        Some(_) => {
            let side =
                |snapshot: Option<String>| json!({"sourceId": source, "snapshotId": snapshot});
            Ok(search::diff::revision(
                &tx,
                (from, side(old_snapshot)),
                (gen_id, side(new_snapshot)),
                &scope,
                search::diff::REVISION_BYTES,
            )
            .map_err(|e| e.to_string()))
        }
    };
    Ok(Some(Planned { epoch, from, what }))
}

/// Bytes a change row's detail takes, as the diff admitted it.
fn row_cost(r: &search::diff::Row) -> usize {
    64 + r.frame.len() + r.node.len() + r.kind.len()
}

/// Drops the last change rows until they and `reserve` bytes fit one revision's detail budget,
/// since the impact summary is always stored. Returns whether any was dropped.
fn make_room(rows: &mut Vec<search::diff::Row>, reserve: usize) -> bool {
    let mut used: usize = rows.iter().map(row_cost).sum();
    let before = rows.len();
    while used + reserve > search::diff::REVISION_BYTES {
        let Some(r) = rows.pop() else { break };
        used -= row_cost(&r);
    }
    rows.len() < before
}

/// Records the revision publishing `gen_id` makes, inside the publication `tx`, if tracking is
/// still as `planned` read it: a baseline when tracking just started, else the planned changes.
/// Then retires whole revision groups past retention. Returns whether one was recorded.
#[allow(clippy::too_many_arguments)]
fn record(
    tx: &Connection,
    source: &str,
    scope: &str,
    gen_id: i64,
    expected: Option<i64>,
    built: &Json,
    planned: Option<Planned>,
    now: i64,
) -> Result<bool> {
    let tracking: Option<(String, i64)> = tx
        .query_row(
            "SELECT state, epoch FROM tracking WHERE source_id = ?1 AND scope = ?2",
            [source, scope],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let facetted: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM facet_sets WHERE gen = ?1)",
            [gen_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let Some((state, epoch)) = tracking.filter(|_| facetted) else {
        return Ok(false);
    };
    let what = match (state.as_str(), planned) {
        ("baseline_pending", _) => Err("tracking_started"),
        // An evicted scope published again has nothing to compare with: never a silent gap.
        ("active", None) if expected.is_none() => Err("previous_unpublished"),
        ("active", Some(p)) if p.epoch == epoch && Some(p.from) == expected => p.what,
        // Compared against another generation or tracking: a baseline, never a silent gap.
        ("active", Some(p)) if p.epoch == epoch => Err("previous_changed"),
        ("active", _) => Err("tracking_changed"),
        _ => return Ok(false),
    };
    // A baseline compares nothing, so it has no impact: never an empty complete one.
    let (kind, from, coverage, summary, mut rows, mut truncated, impact, reached) = match what {
        Err(reason) => (
            "baseline",
            None,
            json!({"basis": "baseline", "reason": reason}),
            json!({}),
            Vec::new(),
            false,
            None,
            Vec::new(),
        ),
        Ok(Ok(r)) => (
            "changes",
            expected,
            r.coverage,
            r.summary,
            r.rows,
            r.truncated,
            Some((r.impact.complete, r.impact.reasons)),
            r.impact.hits,
        ),
        // Never a false unchanged: the generation still publishes, the history stays.
        Ok(Err(why)) => (
            "changes",
            expected,
            json!({"complete": false, "reasons": [{"reason": "comparison_failed", "message": why.chars().take(500).collect::<String>()}]}),
            json!({"changes": null}),
            Vec::new(),
            true,
            Some((false, BTreeMap::from([("comparison_failed", 1)]))),
            Vec::new(),
        ),
    };
    let row_bytes = |rows: &[search::diff::Row]| rows.iter().map(row_cost).sum::<usize>() as u64;
    // Its impact summary as it will be stored: the hits left out read `impact_truncated`.
    let noted = |kept: usize, by: &BTreeMap<&'static str, u64>| {
        impact.as_ref().map(|(complete, reasons)| {
            let mut reasons = reasons.clone();
            let omitted = (reached.len() - kept) as u64;
            if omitted > 0 {
                *reasons.entry("impact_truncated").or_default() += omitted;
            }
            search::impact::summary(*complete && omitted == 0, &reasons, by).to_string()
        })
    };
    let len = |t: &Option<String>| t.as_ref().map_or(0, |t| t.len() as u64);
    // The reached layers listed beside the change rows, within one revision's detail budget
    // less the summary at its largest (every class counting every hit, none kept): each pays
    // its copy before it is made, then its entry's exact serialized size.
    use search::impact::Class::{Direct, Effective, EffectiveUnchanged, Possible};
    let every = [Direct, Effective, EffectiveUnchanged, Possible];
    let most = every.map(|c| (c.name(), reached.len() as u64)).into();
    let reserve = len(&noted(0, &most)) as usize;
    truncated |= make_room(&mut rows, reserve);
    let mut left = search::diff::REVISION_BYTES.saturating_sub(row_bytes(&rows) as usize + reserve);
    let (mut kept, mut by, mut hit_bytes) = (0, BTreeMap::new(), 0);
    for h in &reached {
        let via = h.via.as_ref().map(|(r, t)| (*r, t.as_str()));
        let framed: usize = h.frames.iter().flatten().map(String::len).sum();
        let copy = 64 + h.node.len() + via.map_or(0, |(r, t)| r.len() + t.len()) + framed;
        let fits = (copy <= left).then(|| {
            let verdicts = [h.changed, h.unknown, h.equal];
            search::impact::entry(&h.node, h.class.name(), h.side, verdicts, via)
        });
        // Its stored frames are kept beside the entry, so they count as well.
        let Some(n) = fits.and_then(|e| search::impact::serialized(&e, left - framed)) else {
            break;
        };
        let n = n + framed;
        left -= n;
        kept += 1;
        *by.entry(h.class.name()).or_insert(0) += 1;
        hit_bytes += n as u64;
    }
    // Publication writes are not capped by SQLite: detail that would pass the disk cap is left out.
    if used_bytes(tx)? + row_bytes(&rows) + hit_bytes + len(&noted(kept, &by)) > DISK_CAP {
        rows.clear();
        (kept, hit_bytes) = (0, 0);
        by.clear();
        truncated = true;
    }
    truncated |= kept < reached.len();
    let impact = noted(kept, &by);
    let hits = &reached[..kept];
    let detail_bytes = row_bytes(&rows) + hit_bytes + len(&impact);
    let seq: i64 = tx
        .query_row(
            "INSERT INTO meta (key, value) VALUES ('revision_seq', '1') ON CONFLICT (key)
             DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
             RETURNING CAST(value AS INTEGER)",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let id: Vec<u8> = tx
        .query_row(
            "INSERT INTO revisions (id, seq, source_id, scope, epoch, gen, from_gen, snapshot_id,
             kind, coverage, summary, detail, detail_rows, detail_bytes, created_ms, impact)
             VALUES (randomblob(16), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             RETURNING id",
            params![
                seq,
                source,
                scope,
                epoch,
                gen_id,
                from,
                built["snapshot"].as_str(),
                kind,
                coverage.to_string(),
                summary.to_string(),
                if truncated { "truncated" } else { "complete" },
                rows.len() as i64,
                detail_bytes as i64,
                now,
                impact
            ],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let mut insert = tx
        .prepare_cached(
            "INSERT INTO revision_changes (rev, ord, frame, node, kind, change, facets, descendants)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(sql)?;
    for (ord, r) in rows.iter().enumerate() {
        insert
            .execute(params![
                id,
                ord as i64,
                r.frame,
                r.node,
                r.kind,
                r.change,
                r.facets,
                r.descendants
            ])
            .map_err(sql)?;
    }
    let mut insert = tx
        .prepare_cached(
            "INSERT INTO revision_impacts (rev, ord, node, class, side, changed, unknown, equal,
             relation, target, old_frame, new_frame)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(sql)?;
    for (ord, h) in hits.iter().enumerate() {
        let (relation, target) = h.via.as_ref().map(|(r, t)| (*r, t.as_str())).unzip();
        insert
            .execute(params![
                id,
                ord as i64,
                h.node,
                h.class.name(),
                h.side,
                h.changed,
                h.unknown,
                h.equal,
                relation,
                target,
                h.frames[0],
                h.frames[1]
            ])
            .map_err(sql)?;
    }
    if state == "baseline_pending" {
        tx.execute(
            "UPDATE tracking SET state = 'active', baseline_gen = ?3
             WHERE source_id = ?1 AND scope = ?2",
            params![source, scope, gen_id],
        )
        .map_err(sql)?;
    }
    // Whole groups only: the oldest past the count are deleted, the oldest past the detail bytes
    // keep their summary and read as `expired`.
    let old = "SELECT id FROM revisions WHERE source_id = ?1 AND scope = ?2
               ORDER BY seq DESC LIMIT -1 OFFSET ?3";
    // The newest deleted and the publication that deleted it, so a reader behind it that had
    // not accepted that deletion reads `event_gap`.
    tx.execute(
        &format!(
            "INSERT INTO meta (key, value) SELECT 'pruned' || char(31) || ?1 || char(31) || ?2,
             MAX(seq) || ' ' || (SELECT value FROM meta WHERE key = 'revision_seq')
             FROM revisions WHERE id IN ({old}) HAVING COUNT(*) > 0
             ON CONFLICT (key) DO UPDATE SET value = excluded.value"
        ),
        params![source, scope, REVISIONS],
    )
    .map_err(sql)?;
    for table in ["revision_changes", "revision_impacts"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE rev IN ({old})"),
            params![source, scope, REVISIONS],
        )
        .map_err(sql)?;
    }
    tx.execute(
        &format!("DELETE FROM revisions WHERE id IN ({old})"),
        params![source, scope, REVISIONS],
    )
    .map_err(sql)?;
    let over = "SELECT id FROM (SELECT id, SUM(detail_bytes) OVER (ORDER BY seq DESC) AS total
                FROM revisions WHERE source_id = ?1 AND scope = ?2 AND detail != 'expired')
                WHERE total > ?3";
    for table in ["revision_changes", "revision_impacts"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE rev IN ({over})"),
            params![source, scope, DETAIL_BYTES],
        )
        .map_err(sql)?;
    }
    tx.execute(
        &format!("UPDATE revisions SET detail = 'expired' WHERE id IN ({over})"),
        params![source, scope, DETAIL_BYTES],
    )
    .map_err(sql)?;
    Ok(true)
}

/// Deletes retired generations of `source` and `scope` beyond the newest `keep`, and snapshots no
/// generation names. A cursor over a deleted generation reads `cursor_expired`.
fn prune(conn: &mut Connection, source: &str, scope: &str, keep: i64) -> Result<()> {
    let old: Vec<i64> = conn
        .prepare(
            "SELECT gen FROM generations WHERE source_id = ?1 AND scope = ?2 AND state = 'retired'
             ORDER BY retired_ms DESC, gen DESC LIMIT -1 OFFSET ?3",
        )
        .and_then(|mut s| {
            s.query_map(params![source, scope, keep], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()
        })
        .map_err(sql)?;
    for gen_id in old {
        let tx = conn.transaction().map_err(sql)?;
        drop_gen(&tx, gen_id).map_err(sql)?;
        tx.commit().map_err(sql)?;
    }
    conn.execute(
        "DELETE FROM snapshots WHERE source_id = ?1 AND snapshot_id NOT IN
         (SELECT snapshot_id FROM generations WHERE source_id = ?1 AND snapshot_id IS NOT NULL)",
        [source],
    )
    .map_err(sql)?;
    let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
    Ok(())
}

/// The workspace a job id names: `<workspaceId>.<hex>`.
pub(crate) fn job_workspace(job_id: &str) -> Result<String> {
    job_id
        .split_once('.')
        .filter(|(_, hex)| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|(ws, _)| ws.to_owned())
        .ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("{job_id:?} is not a job id; ids look like ws_<16 hex>.<32 hex>"),
            )
        })
}

/// `jobs status`: read-only; a dead owner's active job reads `interrupted` and nothing changes.
pub fn status(storage: &Storage, job_id: &str) -> Result<Json> {
    let ws_id = job_workspace(job_id)?;
    let (_, ws) = opened(storage, &Select::Id(ws_id), false)?;
    if ws.version < 2 || !visible_job(storage, &ws.conn, job_id)? {
        return Err(unknown_job(job_id));
    }
    report_job(&ws.conn, &ws.held.dir, job_id)?.ok_or_else(|| unknown_job(job_id))
}

/// Whether this caller may see job `job_id`: its source is visible, or nothing is filtered.
/// An invisible job reads exactly like an unknown one.
fn visible_job(storage: &Storage, conn: &Connection, job_id: &str) -> Result<bool> {
    let path: Option<Option<String>> = conn
        .query_row(
            "SELECT s.path FROM jobs j LEFT JOIN sources s ON s.source_id = j.source_id
             WHERE j.job_id = ?1",
            [job_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    Ok(match path {
        None => false,
        Some(Some(path)) => storage.sees(Path::new(&path)),
        Some(None) => storage.visible.is_none(),
    })
}

/// `jobs cancel`: requests the cancel, then waits up to `CANCEL_WAIT` for the owner to stop the
/// job. `cancel` is `accepted` (now cancelled), `pending` (still stopping), `too_late` (its
/// publication committed first) or `not_active`. A dead owner's job is cancelled at once, since
/// nothing computes for it.
pub fn cancel(storage: &Storage, job_id: &str) -> Result<Json> {
    let ws_id = job_workspace(job_id)?;
    let (_, mut ws) = opened(storage, &Select::Id(ws_id), true)?;
    if ws.version < 2 || !visible_job(storage, &ws.conn, job_id)? {
        return Err(unknown_job(job_id));
    }
    let dir = ws.held.dir.clone();
    let row: Option<(String, String, i64)> = ws
        .conn
        .query_row(
            "SELECT state, owner, attempt FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    let (state, holder, attempt) = row.ok_or_else(|| unknown_job(job_id))?;
    let out = |report: Json, outcome: &str| {
        let mut report = report;
        report["cancel"] = json!(outcome);
        Ok(report)
    };
    if state != "queued" && state != "running" {
        let report = report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id))?;
        return out(report, "not_active");
    }
    let accepted = Instant::now();
    if !job_alive(&dir, &holder, job_id, attempt) {
        // The dead owner's worker may still run: end the job only once it exited; until then
        // the cancel is pending and a later cancel ends it.
        while worker_alive(&dir, job_id, attempt) && accepted.elapsed() < CANCEL_WAIT {
            std::thread::sleep(Duration::from_millis(20));
        }
        if worker_alive(&dir, job_id, attempt) {
            let report = report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id))?;
            let mut report = out(report, "pending")?;
            report["cancelWaitMs"] = json!(accepted.elapsed().as_millis() as u64);
            return Ok(report);
        }
        // Fenced to the dead owner's attempt: a takeover since the read is left running.
        let ended = ws
            .conn
            .execute(
                "UPDATE jobs SET state = 'cancelled', cancel = 1, finished_ms = ?2 WHERE job_id = ?1
                 AND owner = ?3 AND attempt = ?4 AND state IN ('queued', 'running')",
                params![job_id, now_ms(), holder, attempt],
            )
            .map_err(sql)?;
        if ended == 1 {
            drop_building(&mut ws.conn, job_id, &holder, attempt)?;
        }
    } else {
        // A queued job has no child or generation yet, so it ends here; its owner's start is
        // fenced on `queued`.
        let ended = ws
            .conn
            .execute(
                "UPDATE jobs SET state = 'cancelled', cancel = 1, finished_ms = ?2
                 WHERE job_id = ?1 AND state = 'queued'",
                params![job_id, now_ms()],
            )
            .map_err(sql)?;
        if ended == 0 {
            ws.conn
                .execute(
                    "UPDATE jobs SET cancel = 1 WHERE job_id = ?1 AND state = 'running'",
                    [job_id],
                )
                .map_err(sql)?;
        }
    }
    loop {
        let report = report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id))?;
        let outcome = match report["state"].as_str() {
            Some("cancelled") => "accepted",
            Some("succeeded") => "too_late",
            Some("failed") => "not_active",
            _ if accepted.elapsed() >= CANCEL_WAIT => "pending",
            _ => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        let mut report = report;
        report["cancelWaitMs"] = json!(accepted.elapsed().as_millis() as u64);
        return out(report, outcome);
    }
}

/// The hidden `__index-worker <workspace-dir> <jobId>` child: stages, hashes, parses, builds,
/// writes and verifies one job's generation, printing one JSON line. It exits as soon as its
/// stdin closes, unless it is stopped, and holds the worker lock for its whole life.
pub fn worker(dir: &Path, job_id: &str) -> Json {
    worker_with(dir, job_id, work)
}

/// The life of a worker child around `work(dir, jobId, owner, attempt)`: its stdin watchdog,
/// launch token and worker lock, and the one JSON line it reports.
pub(super) fn worker_with(
    dir: &Path,
    job_id: &str,
    work: impl FnOnce(&Path, &str, &str, i64) -> Result<Json>,
) -> Json {
    // Held until the worker file is locked and marked, so the watchdog never exits in between
    // and leaves an empty file that would read as a worker still starting.
    static STARTUP: Mutex<()> = Mutex::new(());
    let startup = STARTUP.lock().unwrap_or_else(|e| e.into_inner());
    // Test builds: `FIGLENS_TEST_ORPHAN` keeps a worker running after its owner died, as a
    // stopped one would.
    if !(cfg!(debug_assertions) && std::env::var_os("FIGLENS_TEST_ORPHAN").is_some()) {
        std::thread::spawn(|| {
            let mut sink = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut sink);
            let _started = STARTUP.lock();
            std::process::exit(3);
        });
    }
    match launch().and_then(|(me, attempt)| {
        hold("start");
        let path = worker_path(dir, job_id, attempt);
        let mut alive = open_lock(&path, false)?.ok_or("the worker file is missing")?;
        alive.lock().map_err(|e| storage_error(&path, e))?;
        alive
            .write_all(b"started")
            .map_err(|e| storage_error(&path, e))?;
        // Unlocked only by this process's exit, after its output.
        std::mem::forget(alive);
        drop(startup);
        work(dir, job_id, &me, attempt)
    }) {
        Ok(out) => out,
        Err(e) => {
            let code = e
                .downcast_ref::<FigError>()
                .map_or("operation_failed", |f| kind_code(f.kind()));
            json!({"error": error_json(code, e)})
        }
    }
}

/// The contract code of an error kind, as recorded in a failed job.
pub(crate) fn kind_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::InvalidArguments => "invalid_arguments",
        ErrorKind::SourceUnreadable | ErrorKind::InvalidData => "source_unreadable",
        ErrorKind::NodeNotFound => "node_not_found",
        ErrorKind::UnsupportedNode => "unsupported_node",
        ErrorKind::ParseBudget => "parse_budget_exceeded",
        ErrorKind::ResponseBudget => "response_budget_exceeded",
        ErrorKind::OutputWrite => "output_write_failed",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::OperationFailed => "operation_failed",
        ErrorKind::WorkspaceBusy => "workspace_busy",
        ErrorKind::IndexIncompatible => "index_incompatible",
        ErrorKind::StorageUnavailable => "storage_unavailable",
        ErrorKind::SourceStale => "source_stale",
        ErrorKind::CursorStale => "cursor_stale",
        ErrorKind::CursorExpired => "cursor_expired",
        ErrorKind::EventGap => "event_gap",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::QueryWorkLimit => "query_work_limit",
        ErrorKind::ModeUnavailable => "mode_unavailable",
    }
}

/// Database plus WAL bytes of the workspace at `dir`.
fn wal_bytes(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("index.sqlite-wal")).map_or(0, |m| m.len())
}

/// The capacity error for SQLite refusing a page past the build's `ceiling` bytes (or a full
/// disk), so the caller sees what to do instead of `database or disk is full`.
pub(super) fn full(e: Box<dyn Error>, ceiling: u64) -> Box<dyn Error> {
    match e.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(f, _)) if f.code == rusqlite::ErrorCode::DiskFull => {
            fail(
                ErrorKind::OperationFailed,
                format!(
                    "the workspace database reached its {ceiling}-byte build ceiling (a {DISK_CAP}-byte cap less headroom, or the file's size when already larger) or the disk is full; the previous index stays published; clear_cache, remove sources or index a narrower scope"
                ),
            )
        }
        _ => e,
    }
}

/// Ok while job `job_id`'s row still names `me`'s `attempt` running; checked inside each write
/// transaction, so a stale child commits nothing.
fn owns_launch(conn: &Connection, job_id: &str, me: &str, attempt: i64) -> Result<()> {
    let owns = conn
        .query_row(
            "SELECT 1 FROM jobs WHERE job_id = ?1 AND owner = ?2 AND attempt = ?3
             AND state = 'running'",
            params![job_id, me, attempt],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?;
    owns.ok_or_else(|| {
        fail(
            ErrorKind::OperationFailed,
            "another attempt owns this job now",
        )
    })
}

/// The owner and attempt in the `LAUNCH` token `spawn` passed.
fn launch() -> Result<(String, i64)> {
    let launch = std::env::var(LAUNCH).map_err(|_| format!("{LAUNCH} is not set"))?;
    launch
        .rsplit_once(':')
        .and_then(|(o, a)| Some((o.to_owned(), a.parse::<i64>().ok()?)))
        .ok_or_else(|| format!("{LAUNCH} is malformed").into())
}

/// A worker's hold on the workspace at `dir` for job `job_id`: its lock, shared, and a write
/// connection whose growth SQLite stops at the build ceiling, also returned in bytes.
pub(super) fn worker_conn(dir: &Path, job_id: &str) -> Result<(File, Connection, u64)> {
    let id = job_workspace(job_id)?;
    let lock_path = dir
        .parent()
        .ok_or("no workspace parent")?
        .join(format!("{id}.lock"));
    let lock_file = open_lock(&lock_path, false)?.ok_or("the workspace lock is missing")?;
    lock(&lock_file, &lock_path, false, &id)?;
    if read_marker(dir, &id)?.is_none() {
        return Err(preserved(dir, "has no figlens workspace marker"));
    }
    let conn = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    writable(&conn)?;
    // The hard ceiling, on this connection only: pages a failed build freed are reused first, so
    // a retry fits again without a VACUUM. SQLite answers with the ceiling it actually set.
    let page: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(sql)?;
    let pages = (DISK_CAP - BUILD_HEADROOM) / page.max(512) as u64;
    let set: i64 = conn
        .query_row(&format!("PRAGMA max_page_count = {pages}"), [], |r| {
            r.get(0)
        })
        .map_err(sql)?;
    let ceiling = set as u64 * page as u64;
    Ok((lock_file, conn, ceiling))
}

/// Builds attempt `attempt` of job `job_id` for owner `me`. Every write is fenced to that
/// launch, never to whichever attempt the row holds now, so a child outliving a takeover
/// cannot adopt the new attempt.
fn work(dir: &Path, job_id: &str, me: &str, attempt: i64) -> Result<Json> {
    let stale = || {
        fail(
            ErrorKind::OperationFailed,
            "another attempt owns this job now",
        )
    };
    let fenced = |changed: usize| if changed == 1 { Ok(()) } else { Err(stale()) };
    let id = job_workspace(job_id)?;
    let (lock_file, mut conn, ceiling) = worker_conn(dir, job_id)?;
    // Also this connection only: it reclaims the WAL itself after every batch (`between`), so
    // SQLite's checkpoint at each large commit only repeated that, and a 32 MiB cache keeps a
    // batch's pages from spilling into the WAL before its commit.
    conn.execute_batch("PRAGMA wal_autocheckpoint = 0; PRAGMA cache_size = -32768;")
        .map_err(sql)?;
    let (kind, source_id, membership, scope, incarnation, expected): (
        String,
        String,
        String,
        String,
        String,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT kind, source_id, membership, scope, incarnation, expected_gen FROM jobs
             WHERE job_id = ?1 AND owner = ?2 AND attempt = ?3 AND state = 'running'",
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
        .map_err(sql)?
        .ok_or_else(stale)?;
    if meta(&conn, "workspace_id").map_err(sql)?.as_deref() != Some(id.as_str())
        || meta(&conn, "incarnation").map_err(sql)?.as_deref() != Some(incarnation.as_str())
    {
        return Err(fail(
            ErrorKind::OperationFailed,
            "the workspace was recreated",
        ));
    }
    let path: String = conn
        .query_row(
            "SELECT path FROM sources WHERE source_id = ?1 AND membership = ?2",
            [&source_id, &membership],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| fail(ErrorKind::InvalidArguments, "the source left the workspace"))?;
    let stage = |conn: &Connection, stage: &str| {
        fenced(
            conn.execute(
                "UPDATE jobs SET stage = ?2 WHERE job_id = ?1 AND owner = ?3 AND attempt = ?4
                 AND state = 'running'",
                params![job_id, stage, me, attempt],
            )
            .map_err(sql)?,
        )?;
        hold(stage);
        Ok::<_, Box<dyn Error>>(())
    };
    let path = PathBuf::from(path);
    // Read only where the registered path its holders authorized still leads.
    let (bytes, stamp) = super::storage::staged(&path)?;
    stage(&conn, "hash")?;
    let snapshot = search::snapshot_id(&bytes);
    let mtime_ns = stamp
        .0
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64);
    let versions = search::store::versions();
    let snap = json!({"snapshot": snapshot, "bytes": stamp.1, "mtimeNs": mtime_ns});
    if kind != "rebuild"
        && let Some(gen_id) = expected
    {
        let same: Option<bool> = conn
            .query_row(
                "SELECT snapshot_id = ?2 AND versions = ?3 AND membership = ?4 AND complete = 1
                 FROM generations WHERE gen = ?1",
                params![gen_id, snapshot, versions, membership],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        // Only a complete generation of the same bytes, versions and membership is reused: a
        // partial one (a deadline, say) or one built before a relocate or re-add is rebuilt.
        if same == Some(true) {
            let mut out = snap;
            out["gen"] = json!(gen_id);
            out["unchanged"] = json!(true);
            return Ok(out);
        }
    }
    stage(&conn, "parse")?;
    let label = path.display().to_string();
    // Only a tracked scope's build keeps facets; an untracked one writes what it always did.
    let tracking: Option<String> = conn
        .query_row(
            "SELECT state FROM tracking WHERE source_id = ?1 AND scope = ?2
             AND state != 'disabled'",
            [&source_id, &scope],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    let (index, tracked) = match tracking {
        Some(state) => {
            let (index, tracked) = search::index_bytes_tracked(
                bytes,
                &label,
                &scope,
                search::deadline(),
                state == "active",
            )?;
            (index, Some(tracked))
        }
        None => (
            search::index_bytes(bytes, &label, &scope, search::deadline())?,
            None,
        ),
    };
    stage(&conn, "write")?;
    // One statement, so the fence and the insert are atomic.
    fenced(
        conn.execute(
            "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id,
             state, created_ms) SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'building', ?7 WHERE EXISTS
             (SELECT 1 FROM jobs WHERE job_id = ?6 AND owner = ?8 AND attempt = ?9
              AND state = 'running')",
            params![
                source_id,
                membership,
                scope,
                snapshot,
                versions,
                job_id,
                now_ms(),
                me,
                attempt
            ],
        )
        .map_err(sql)?,
    )?;
    let gen_id = conn.last_insert_rowid();
    let mut between = |conn: &Connection, done: usize, total: usize| -> Result<()> {
        // A stale attempt stops here, after at most the one batch it was writing.
        fenced(
            conn.execute(
                "UPDATE jobs SET done = ?2, total = ?3 WHERE job_id = ?1 AND owner = ?4
                 AND attempt = ?5 AND state = 'running'",
                params![job_id, done as i64, total as i64, me, attempt],
            )
            .map_err(sql)?,
        )?;
        // The WAL: reclaimed after every batch, so it passes `WAL_CAP` by about one batch at
        // most; a reader that keeps it from resetting fails the build after `LOCK_WAIT`.
        let wait = Instant::now();
        loop {
            let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
            let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
            if wal_bytes(dir) <= WAL_CAP {
                return Ok(());
            }
            if wait.elapsed() > LOCK_WAIT {
                return Err(fail(
                    ErrorKind::WorkspaceBusy,
                    format!(
                        "a reader keeps the write-ahead log above {WAL_CAP} bytes; retry when other searches finish"
                    ),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let guard = |conn: &Connection| owns_launch(conn, job_id, me, attempt);
    search::store::save(&mut conn, gen_id, &index, &guard, &mut between)
        .map_err(|e| full(e, ceiling))?;
    drop(index);
    if let Some(tracked) = tracked {
        search::diff::save_facets(&mut conn, gen_id, &tracked, &guard, &mut between)
            .map_err(|e| full(e, ceiling))?;
    }
    stage(&conn, "verify")?;
    // The source is read and hashed again: a save that kept its size and time is caught here.
    let (again, _) = super::storage::staged(&path)?;
    if search::snapshot_id(&again) != snapshot {
        return Err(fail(
            ErrorKind::SourceUnreadable,
            "the .fig changed while it was indexed; the previous index stays published; refresh after the save finishes",
        ));
    }
    let _ = lock_file;
    let mut out = snap;
    out["gen"] = json!(gen_id);
    Ok(out)
}

/// The visible sources `only` names (every visible source when empty), in registry order.
pub(super) fn chosen(storage: &Storage, ws: &Opened, only: Option<&str>) -> Result<Vec<String>> {
    let mut stmt = ws
        .conn
        .prepare("SELECT source_id, path FROM sources ORDER BY added_at_ms, source_id")
        .map_err(sql)?;
    let all = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let ids: Vec<String> = all
        .into_iter()
        // The id test first: an explicit source resolves only its own path, which a caller in a
        // write transaction would otherwise do for every source.
        .filter(|(id, path)| only.is_none_or(|o| o == id) && storage.sees(Path::new(path)))
        .map(|(id, _)| id)
        .collect();
    if let Some(only) = only
        && ids.is_empty()
    {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {} has no source {only}", ws.held.id),
        ));
    }
    Ok(ids)
}

/// Starts or joins builds of `kind` for each (source, scope): the given scope, or every scope
/// the source has published; `among` narrows the chosen sources, and `fence` must hold inside
/// each admission's write.
#[allow(clippy::too_many_arguments)]
fn submit_all(
    storage: &Storage,
    select: &Select,
    kind: &str,
    only: Option<&str>,
    scope: Option<&str>,
    request_id: Option<&str>,
    among: Option<&[String]>,
    fence: Option<Fence>,
) -> Result<Outcome> {
    let (place, mut ws) = opened(storage, select, true)?;
    let mut plan = Vec::new();
    for source in chosen(storage, &ws, only)? {
        if among.is_some_and(|among| !among.contains(&source)) {
            continue;
        }
        let scopes: Vec<String> = match scope {
            Some(scope) => vec![scope.to_owned()],
            None => ws
                .conn
                .prepare("SELECT scope FROM published WHERE source_id = ?1 ORDER BY published_ms DESC, scope")
                .and_then(|mut s| s.query_map([&source], |r| r.get(0))?.collect())
                .map_err(sql)?,
        };
        plan.extend(scopes.into_iter().map(|s| (source.clone(), s)));
    }
    if plan.is_empty() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "workspace {} has no indexed scope to {kind}; pass a scope to index one",
                ws.held.id
            ),
        ));
    }
    if request_id.is_some() && plan.len() > 1 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "requestId names one job; pass a sourceId and scope with it",
        ));
    }
    let (mut run, mut reports, mut deferred) = (Vec::new(), Vec::new(), Vec::new());
    let mut because = Json::Null;
    // A deferred item names the exact call that admits it later, so continuing a large plan
    // builds only what is left instead of the same first jobs again.
    let ws_id = ws.held.id.clone();
    let later = |source: &str, scope: &str| {
        json!({"sourceId": source, "scope": scope, "nextStep": {"tool": "index",
            "arguments": {"action": kind, "workspaceId": ws_id, "sourceId": source, "scope": scope},
            "why": "not admitted with this request; run it after some of its jobs finish"}})
    };
    for (source, scope) in plan {
        if !deferred.is_empty() {
            deferred.push(later(&source, &scope));
            continue;
        }
        let job = match submit_fenced(storage, &mut ws, kind, &source, &scope, request_id, fence) {
            Ok(job) => job,
            // Each admitted job is committed and owned by this process, which never lets it
            // go: the caller gets them to run, and the rest (past MAX_ACTIVE, say) is reported.
            Err(e) if !reports.is_empty() => {
                let code = e
                    .downcast_ref::<FigError>()
                    .map_or("operation_failed", |f| kind_code(f.kind()));
                because = error_json(code, e);
                deferred.push(later(&source, &scope));
                continue;
            }
            Err(e) => return Err(e),
        };
        if job.owned {
            run.push(job.job_id.clone());
        }
        let mut report = match report_job(&ws.conn, &ws.held.dir, &job.job_id) {
            Ok(report) => report.unwrap_or(Json::Null),
            Err(e) => return Err(release(&ws, &run, e)),
        };
        report["joined"] = json!(job.joined);
        reports.push(report);
    }
    let partial = !deferred.is_empty();
    let mut jobs =
        json!({"jobs": reports, "admission": if partial { "partial" } else { "complete" }});
    if partial {
        jobs["deferred"] = json!(deferred);
        jobs["deferredBecause"] = because;
    }
    match opened_report(storage, &place, &ws, jobs) {
        Ok(report) => Ok(Outcome { report, run }),
        Err(e) => Err(release(&ws, &run, e)),
    }
}

/// Ends the jobs `run` this process admitted but will not return to run, as `failed` with
/// `e`, so none is left queued under a live owner. Returns `e`.
pub(super) fn release(ws: &Opened, run: &[String], e: Box<dyn Error>) -> Box<dyn Error> {
    if let Ok(me) = owner(&ws.held.dir) {
        let code = e
            .downcast_ref::<FigError>()
            .map_or("operation_failed", |f| kind_code(f.kind()));
        let error = error_json(code, &e).to_string();
        for job_id in run {
            let ended = ws.conn.execute(
                "UPDATE jobs SET state = 'failed', error = ?3, finished_ms = ?4 WHERE job_id = ?1
                 AND owner = ?2 AND state = 'queued'",
                params![job_id, me, error, now_ms()],
            );
            // Not ended: the dropped claim leaves it interrupted, for a refresh to retry.
            if let (Ok(1), Some((_, claim))) = (ended, take_pending(job_id)) {
                claim.done();
            }
        }
    }
    e
}

/// `index refresh`: rebuilds a source's scopes whose bytes changed; an unchanged complete
/// snapshot keeps its published index.
pub fn refresh(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: Option<&str>,
    request_id: Option<&str>,
) -> Result<Outcome> {
    submit_all(
        storage, select, "refresh", source, scope, request_id, None, None,
    )
}

/// `refresh` of one source scope, admitted only while `fence` holds inside the admission's
/// write; `None` when it did not, with nothing admitted, joined or replayed.
pub(crate) fn refresh_fenced(
    storage: &Storage,
    select: &Select,
    source: &str,
    scope: &str,
    fence: Fence,
) -> Result<Option<Outcome>> {
    match submit_all(
        storage,
        select,
        "refresh",
        Some(source),
        Some(scope),
        None,
        None,
        Some(fence),
    ) {
        Err(e) if e.is::<Unfenced>() => Ok(None),
        outcome => outcome.map(Some),
    }
}

/// `index rebuild`: builds every published scope of every source again, even unchanged ones,
/// keeping the registry and each prior generation until its replacement publishes.
pub fn rebuild(storage: &Storage, select: &Select, request_id: Option<&str>) -> Result<Outcome> {
    submit_all(
        storage, select, "rebuild", None, None, request_id, None, None,
    )
}

/// `index rebuild` of one source, or one of its scopes: a deferred item's continuation, which
/// builds it again even when unchanged, as `rebuild` does.
pub fn rebuild_scope(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: Option<&str>,
    request_id: Option<&str>,
) -> Result<Outcome> {
    submit_all(
        storage, select, "rebuild", source, scope, request_id, None, None,
    )
}

/// Semantic search's model layer needs a configured profile; `models status` shows the setup.
pub(crate) fn no_profile() -> Box<dyn Error> {
    FigError::new(
        ErrorKind::ModeUnavailable,
        "embedding needs a configured local model; run models status for setup",
    )
    .with_recovery(json!([{"action": "models_status", "tool": "models",
        "arguments": {"action": "status"}, "requires": "none"}]))
    .into()
}

/// `index embed`: embeds the published generation of each chosen scope (every published one
/// without `scope`) with the active profile, as jobs beside the text builds. Text search keeps
/// serving throughout; nothing is downloaded.
pub fn embed(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: Option<&str>,
    request_id: Option<&str>,
) -> Result<Outcome> {
    if crate::semantic::profile::active(storage)?.is_none() {
        return Err(no_profile());
    }
    submit_all(
        storage, select, "embed", source, scope, request_id, None, None,
    )
}

/// Whether succeeded text job `report`'s generation has no ready model layer under the active
/// profile; `None` without a profile, or when the layer cannot be read.
fn layer_missing(storage: &Storage, report: &Json) -> Option<bool> {
    if report["state"] != "succeeded" || report["kind"] == "embed" {
        return None;
    }
    let profile = crate::semantic::profile::active(storage).ok()??;
    let select = Select::Id(report["workspaceId"].as_str()?.to_owned());
    let (_, ws) = opened(storage, &select, false).ok()?;
    let layer = super::model::layer(&ws.conn, report["generation"].as_i64()?, &profile.id).ok()?;
    Some(layer.is_none_or(|l| l["state"] != "ready"))
}

/// For an owner that keeps running after text job `report` succeeded: submits the embed of the
/// generation it published when a profile is configured and its layer is not ready, returning
/// the job this process must run. Never fails the text job; a later `index embed` retries.
pub fn embed_after(storage: &Storage, report: &Json) -> Option<String> {
    if layer_missing(storage, report) != Some(true) {
        return None;
    }
    let select = Select::Id(report["workspaceId"].as_str()?.to_owned());
    let (source, scope) = (report["sourceId"].as_str(), report["scope"].as_str());
    let done = submit_all(storage, &select, "embed", source, scope, None, None, None).ok()?;
    done.run.into_iter().next()
}

/// The `index embed` call that builds succeeded text job `report`'s model layer, for a caller
/// that ends without running model jobs, so none is left queued for a dead owner.
pub fn embed_step(storage: &Storage, report: &Json) -> Option<Json> {
    layer_missing(storage, report)?.then(|| {
        json!({"tool": "index", "arguments": {"action": "embed", "workspaceId": report["workspaceId"],
            "sourceId": report["sourceId"], "scope": report["scope"]},
            "why": "a model profile is configured and this index has no ready semantic layer; embedding runs the local model and may take minutes"})
    })
}

/// `index track`: turns change tracking on for published scope `scope` of each chosen source and
/// builds its baseline as a rebuild, which never reuses an unchanged snapshot and stores the
/// scope's facets; its publication records the baseline revision. A scope already tracked
/// keeps its epoch and builds nothing unless its baseline is still pending.
pub fn track(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
    request_id: Option<&str>,
) -> Result<Outcome> {
    let (id, rows, started) = set_tracking(storage, select, source, scope, Some(request_id))?;
    // Only the sources whose baseline is pending build: an active one is not rebuilt.
    let mut done = match started.is_empty() {
        false => submit_all(
            storage,
            select,
            "track",
            source,
            Some(scope),
            request_id,
            Some(&started),
            None,
        )?,
        true => Outcome {
            report: json!({"workspaceId": id, "jobs": []}),
            run: Vec::new(),
        },
    };
    done.report["tracking"] = rows;
    Ok(done)
}

/// `index untrack`: later builds of the scope store no facets. Revisions and checkpoints stay.
pub fn untrack(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
) -> Result<Json> {
    let (id, rows, _) = set_tracking(storage, select, source, scope, None)?;
    Ok(json!({"workspaceId": id, "tracking": rows}))
}

/// Sets tracking of `scope` on (`Some`, with the requestId of its single baseline job, if any)
/// or off for each chosen source in one transaction, bumping a row's epoch on every change so
/// work begun under the old state can tell. Turning it on needs the scope published; returns
/// the sources whose baseline is pending.
fn set_tracking(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    scope: &str,
    on: Option<Option<&str>>,
) -> Result<(String, Json, Vec<String>)> {
    let (_, ws) = opened(storage, select, true)?;
    // The write lock is taken first, so the visible sources come from the snapshot it writes.
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    let sources = chosen(storage, &ws, source)?;
    // Refused before any write, so no source is left pending without its job.
    if matches!(on, Some(Some(_))) && sources.len() > 1 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "requestId names one job; pass a sourceId and scope with it",
        ));
    }
    // A requestId first used by another request is refused before tracking changes; a retry of
    // this one replays its job and changes nothing, so after an untrack it never re-enables it.
    let mut bound = None;
    if let (Some(Some(request)), [source_id]) = (on, sources.as_slice()) {
        let membership: String = tx
            .query_row(
                "SELECT membership FROM sources WHERE source_id = ?1",
                [source_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let print = fingerprint("track", &ws.held.id, source_id, &membership, scope);
        match requested(&tx, request)? {
            Some((_, fp)) if fp != print => return Err(other_request(request)),
            Some((job, _)) => {
                let (state, epoch) = tx
                    .query_row(
                        "SELECT state, epoch FROM tracking WHERE source_id = ?1 AND scope = ?2",
                        [source_id, scope],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .optional()
                    .map_err(sql)?
                    .unwrap_or(("untracked".into(), 0));
                let row =
                    json!({"sourceId": source_id, "scope": scope, "state": state, "epoch": epoch});
                // A request that started no job replays as one again.
                let started = (!job.is_empty()).then(|| source_id.clone());
                return Ok((
                    ws.held.id.clone(),
                    json!([row]),
                    started.into_iter().collect(),
                ));
            }
            None => bound = Some((request, print)),
        }
    }
    let on = on.is_some();
    let (mut rows, mut started) = (Vec::new(), Vec::new());
    for source_id in &sources {
        let published: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM published WHERE source_id = ?1 AND scope = ?2)",
                [source_id, scope],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if on && !published {
            return Err(FigError::new(
                ErrorKind::InvalidArguments,
                format!("source {source_id} has no published scope {scope} to track; index it first"),
            )
            .with_recovery(json!([{"action": "refresh", "requires": "none", "tool": "index",
                "arguments": {"action": "refresh", "workspaceId": ws.held.id, "sourceId": source_id, "scope": scope}}]))
            .into());
        }
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM tracking WHERE source_id = ?1 AND scope = ?2",
                [source_id, scope],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        let tracked = state.as_deref().is_some_and(|s| s != "disabled");
        let pending = !tracked || state.as_deref() == Some("baseline_pending");
        // Only a rebuild that reads tracking on records the baseline: one not yet at parse, where
        // it reads tracking, or one admitted after a pending baseline was enabled (a larger job
        // `seq`, which only grows: never the clock). Any other
        // running job with a live owner (a refresh may reuse an unfaceted snapshot) cannot, and
        // joining it would leave the baseline pending: track after it ends.
        let busy: Option<(String, String, i64)> = match on && pending {
            true => tx
                .query_row(
                    "SELECT j.job_id, j.owner, j.attempt FROM jobs j LEFT JOIN tracking t
                     ON t.source_id = j.source_id AND t.scope = j.scope
                     AND t.state = 'baseline_pending'
                     WHERE j.source_id = ?1 AND j.scope = ?2 AND j.state = 'running'
                     AND j.kind <> 'embed' AND NOT (j.kind = 'rebuild' AND (j.stage IN ('stage', 'hash')
                     OR IFNULL(j.seq > t.enabled_seq, 0)))",
                    [source_id, scope],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(sql)?,
            false => None,
        };
        // A build whose owner died never ends by itself: its baseline job takes it over as a
        // new rebuild attempt instead.
        let busy = busy
            .filter(|(job, holder, attempt)| job_alive(&ws.held.dir, holder, job, *attempt))
            .map(|(job, _, _)| job);
        if let Some(job_id) = busy {
            return Err(FigError::new(
                ErrorKind::WorkspaceBusy,
                format!("a build of source {source_id} scope {scope} is running that cannot record the baseline (it may have read tracking off or reuse an unchanged snapshot); track after it ends"),
            )
            .with_recovery(json!([{"action": "wait", "requires": "none", "tool": "jobs",
                "arguments": {"action": "status", "jobId": job_id}},
                {"action": "track", "requires": "none", "tool": "index",
                "arguments": {"action": "track", "workspaceId": ws.held.id, "sourceId": source_id, "scope": scope}}]))
            .into());
        }
        if on && !tracked {
            tx.execute(
                "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms,
                 enabled_seq) VALUES (?1, ?2, 'baseline_pending', 1, ?3, ?4,
                 (SELECT IFNULL(MAX(seq), 0) FROM jobs))
                 ON CONFLICT (source_id, scope) DO UPDATE SET state = 'baseline_pending',
                 epoch = epoch + 1, facet_version = ?3, enabled_ms = ?4,
                 enabled_seq = excluded.enabled_seq, baseline_gen = NULL",
                params![source_id, scope, search::diff::facet_version(), now_ms()],
            )
            .map_err(sql)?;
            started.push(source_id.clone());
        } else if on && state.as_deref() == Some("baseline_pending") {
            // A build that started before tracking stored no facets, so its publication left
            // the baseline pending: tracking again builds it.
            started.push(source_id.clone());
        } else if !on && tracked {
            tx.execute(
                "UPDATE tracking SET state = 'disabled', epoch = epoch + 1
                 WHERE source_id = ?1 AND scope = ?2",
                [source_id, scope],
            )
            .map_err(sql)?;
        }
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT state, epoch FROM tracking WHERE source_id = ?1 AND scope = ?2",
                [source_id, scope],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql)?;
        let (state, epoch) = row.unwrap_or(("untracked".into(), 0));
        rows.push(json!({"sourceId": source_id, "scope": scope, "state": state, "epoch": epoch}));
    }
    // An already tracked scope starts no job, yet its requestId stays this request's, among
    // the newest `KEPT` such as finished jobs are.
    if let (Some((request, print)), true) = (bound, started.is_empty()) {
        remember(&tx, Some(request), &print, "")?;
        tx.execute(
            "DELETE FROM requests WHERE job_id = '' AND rowid NOT IN
             (SELECT rowid FROM requests WHERE job_id = '' ORDER BY rowid DESC LIMIT ?1)",
            [KEPT],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok((ws.held.id.clone(), Json::Array(rows), started))
}

/// `index clear_cache`: deletes only what a build can recreate and nothing serves: retired
/// generations and unpublished ones no live job is writing, plus snapshots none names. The
/// registry, published indexes, jobs and the location stay. figlens writes no cache files yet;
/// their namespace is reported so it never overlaps a database even when all roots coincide.
pub fn clear_cache(storage: &Storage, select: &Select) -> Result<Json> {
    let (place, mut ws) = opened(storage, select, true)?;
    let dir = ws.held.dir.clone();
    let visible = chosen(storage, &ws, None)?;
    let mut doomed = Vec::new();
    let mut stmt = ws
        .conn
        .prepare(
            "SELECT g.gen, g.source_id, g.state, j.state, j.owner, g.job_id, j.attempt
             FROM generations g LEFT JOIN jobs j ON j.job_id = g.job_id
             WHERE g.state IN ('retired', 'building')",
        )
        .map_err(sql)?;
    for row in stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<i64>>(6)?,
            ))
        })
        .map_err(sql)?
    {
        let (gen_id, source, state, job, holder, job_id, attempt) = row.map_err(sql)?;
        let writing = state == "building"
            && matches!(job.as_deref(), Some("queued" | "running"))
            && (holder.is_some_and(|h| job_alive(&dir, &h, &job_id, attempt.unwrap_or(0)))
                // A dead owner's worker can still write its batches.
                || worker_alive(&dir, &job_id, attempt.unwrap_or(0)));
        // A removed source's generations belong to nobody visible; they go too.
        let owned = visible.contains(&source) || storage.visible.is_none();
        if !writing && owned {
            doomed.push(gen_id);
        }
    }
    drop(stmt);
    hold("clear_cache");
    let (mut records, mut removed) = (0i64, 0usize);
    for gen_id in &doomed {
        let tx = ws.conn.transaction().map_err(sql)?;
        // Authorized again in this deletion: a source moved out of view since keeps its data.
        let path: Option<String> = tx
            .query_row(
                "SELECT s.path FROM generations g JOIN sources s ON s.source_id = g.source_id
                 WHERE g.gen = ?1",
                [gen_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        if path.is_some_and(|p| !storage.sees(Path::new(&p))) {
            continue;
        }
        removed += 1;
        records += tx
            .query_row(
                "SELECT count(*) FROM records WHERE gen = ?1",
                [gen_id],
                |r| r.get::<_, i64>(0),
            )
            .map_err(sql)?;
        drop_gen(&tx, *gen_id).map_err(sql)?;
        tx.commit().map_err(sql)?;
    }
    ws.conn
        .execute(
            "DELETE FROM snapshots WHERE snapshot_id NOT IN
             (SELECT snapshot_id FROM generations WHERE snapshot_id IS NOT NULL)",
            [],
        )
        .map_err(sql)?;
    let _ = ws
        .conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    let published: i64 = ws
        .conn
        .query_row("SELECT count(*) FROM published", [], |r| r.get(0))
        .map_err(sql)?;
    let cache = place
        .roots
        .cache
        .path
        .join("workspace-cache")
        .join(&ws.held.id);
    let extra = json!({
        "removed": {"generations": removed, "records": records},
        // Some were kept because their source left this caller's view meanwhile: retry later.
        "partial": removed < doomed.len(),
        "retained": {"publishedGenerations": if storage.visible.is_none() { json!(published) } else { Json::Null },
            "registry": true, "jobs": true, "location": true},
        "failed": [],
        "cache": {"path": cache, "present": cache.exists(),
            "note": "figlens writes no cache files in this version"},
    });
    opened_report(storage, &place, &ws, extra)
}

#[cfg(test)]
mod tests;
