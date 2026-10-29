//! Index jobs: one build of one source and scope, submitted or joined in a short transaction,
//! run by the process that owns it in a child process it can kill, and published by that owner
//! only after re-validating the workspace, its source membership and the expected generation.
//!
//! Lock order, never taken backwards: location lock (C) -> workspace lock (W) -> build lock (B)
//! -> SQLite write transaction. A process's owner lock (`owners/<token>.lock`) lives as long as
//! the process, so a job row is alive exactly while its owner process is.

use std::io::Read as _;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{TransactionBehavior, params};

use super::*;
use crate::search;

/// Queued and running jobs one workspace accepts across every process.
pub const MAX_ACTIVE: i64 = 16;
/// Terminal jobs kept per workspace for status and request-id replays.
const KEPT: i64 = 50;
/// Published scopes one source keeps; publishing another retires the least recently published.
pub const MAX_SCOPES: i64 = 8;
/// Database plus WAL bytes a workspace may reach; a build that would pass it fails, keeping the
/// published generation.
pub const DISK_CAP: u64 = 2 * 1024 * 1024 * 1024;
/// WAL bytes a build waits to see reclaimed before writing its next batch.
pub const WAL_CAP: u64 = 64 * 1024 * 1024;
/// How often an owner checks for a cancel request while its child builds.
const POLL: Duration = Duration::from_millis(100);
/// How long `jobs cancel` waits for the owner to report the job stopped.
const CANCEL_WAIT: Duration = Duration::from_secs(2);

/// Owner tokens and their held lock files, one per workspace directory this process owns jobs in.
static OWNERS: Mutex<Vec<(PathBuf, String, File)>> = Mutex::new(Vec::new());

fn random_hex() -> String {
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
    let token = random_hex();
    let path = owners_dir.join(format!("{token}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| storage_error(&path, e))?;
    file.try_lock()
        .map_err(|_| fail(ErrorKind::OperationFailed, "cannot lock a new owner file"))?;
    owners.push((dir.to_owned(), token.clone(), file));
    Ok(token)
}

/// Whether the process owning `token` is alive: its owner lock is held. Read-only; never blocks.
pub(crate) fn alive(dir: &Path, token: &str) -> bool {
    if OWNERS
        .lock()
        .is_ok_and(|o| o.iter().any(|(d, t, _)| d == dir && t == token))
    {
        return true;
    }
    let Ok(file) = File::open(dir.join("owners").join(format!("{token}.lock"))) else {
        return false;
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// A job as `jobs status` reports it; an active job whose owner died reads `interrupted`.
pub(crate) fn report_job(conn: &Connection, dir: &Path, job_id: &str) -> Result<Option<Json>> {
    let row = conn
        .query_row(
            "SELECT kind, source_id, scope, state, stage, done, total, error, owner, attempt,
             cancel, gen, created_ms, heartbeat_ms, finished_ms FROM jobs WHERE job_id = ?1",
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
                    r.get::<_, Option<i64>>(14)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let Some((kind, source, scope, state, stage, done, total, error, owner, attempt, cancel, gen_id, [created, beat], finished)) = row else {
        return Ok(None);
    };
    let active = state == "queued" || state == "running";
    let state = if active && !alive(dir, &owner) {
        "interrupted".to_owned()
    } else {
        state
    };
    let error: Json = error
        .and_then(|e| serde_json::from_str(&e).ok())
        .unwrap_or(Json::Null);
    let ws = job_id.split('.').next().unwrap_or_default();
    let next = match state.as_str() {
        "queued" | "running" => json!([{"tool": "jobs", "arguments": {"action": "status", "jobId": job_id}}]),
        "interrupted" | "failed" | "cancelled" => json!([{"tool": "index", "arguments": {"action": "refresh",
            "workspaceId": ws, "sourceId": source, "scope": scope},
            "why": "a refresh starts the build again as a new attempt; the previous index keeps serving"}]),
        "succeeded" => json!([{"tool": "search", "arguments": {"workspaceId": ws, "node": scope, "query": "<words>"}}]),
        _ => json!([]),
    };
    Ok(Some(json!({"jobId": job_id, "kind": kind, "sourceId": source, "scope": scope,
        "state": state, "stage": stage, "progress": {"units": done, "total": total},
        "cancelRequested": cancel, "attempt": attempt, "generation": gen_id, "error": error,
        "createdMs": created, "heartbeatMs": beat, "finishedMs": finished, "nextSteps": next})))
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

/// Starts or joins the build of `scope` of `source_id`. One active job per source and scope,
/// whatever its kind: a second request joins it, and a job whose owner died is taken over as
/// a new attempt. A `request_id` replays its first job, and refuses a different request.
pub(crate) fn submit(
    ws: &mut Opened,
    kind: &str,
    source_id: &str,
    scope: &str,
    request_id: Option<&str>,
) -> Result<Submitted> {
    let dir = ws.held.dir.clone();
    let me = owner(&dir)?;
    let (id, incarnation) = (ws.held.id.clone(), ws.incarnation.clone());
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let membership: String = tx
        .query_row(
            "SELECT membership FROM sources WHERE source_id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("workspace {id} has no source {source_id}"),
            )
        })?;
    let print = fingerprint(kind, &id, source_id, &membership, scope);
    if let Some(request) = request_id {
        let found: Option<(String, String)> = tx
            .query_row(
                "SELECT job_id, fingerprint FROM jobs WHERE request_id = ?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql)?;
        match found {
            Some((job_id, fp)) if fp == print => {
                tx.commit().map_err(sql)?;
                return Ok(Submitted {
                    job_id,
                    owned: false,
                    joined: true,
                });
            }
            Some(_) => {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!("requestId {request} was used for a different request; use a new requestId"),
                ));
            }
            None => {}
        }
    }
    let active: Option<(String, String)> = tx
        .query_row(
            "SELECT job_id, owner FROM jobs WHERE source_id = ?1 AND scope = ?2
             AND state IN ('queued', 'running')",
            [source_id, scope],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if let Some((job_id, holder)) = active {
        let owned = !alive(&dir, &holder);
        if owned {
            // Its owner died: this process takes it over as a new attempt from the start.
            tx.execute(
                "UPDATE jobs SET owner = ?2, attempt = attempt + 1, state = 'queued', stage = NULL,
                 done = 0, total = NULL, error = NULL, cancel = 0, membership = ?3, heartbeat_ms = ?4
                 WHERE job_id = ?1",
                params![job_id, me, membership, now_ms()],
            )
            .map_err(sql)?;
        } else if kind == "rebuild" {
            // A rebuild joining a queued refresh makes it skip the unchanged-snapshot shortcut.
            tx.execute(
                "UPDATE jobs SET kind = 'rebuild' WHERE job_id = ?1 AND state = 'queued'",
                [&job_id],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        return Ok(Submitted {
            job_id,
            owned,
            joined: !owned,
        });
    }
    let mut live = Vec::new();
    let mut stmt = tx
        .prepare("SELECT job_id, owner FROM jobs WHERE state IN ('queued', 'running')")
        .map_err(sql)?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?
    {
        let (job, holder) = row.map_err(sql)?;
        if alive(&dir, &holder) {
            live.push(job);
        }
    }
    drop(stmt);
    if live.len() as i64 >= MAX_ACTIVE {
        return Err(FigError::new(
            ErrorKind::WorkspaceBusy,
            format!("workspace {id} already has {MAX_ACTIVE} queued or running jobs; retry after one finishes"),
        )
        .with_recovery(json!([{"action": "wait", "requires": "none", "jobIds": live,
            "tool": "jobs", "arguments": {"action": "status", "jobId": live[0]}}]))
        .into());
    }
    let job_id = format!("{id}.{}", random_hex());
    let now = now_ms();
    tx.execute(
        "INSERT INTO jobs (job_id, request_id, fingerprint, kind, source_id, membership, scope,
         incarnation, state, owner, created_ms, heartbeat_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', ?9, ?10, ?10)",
        params![job_id, request_id, print, kind, source_id, membership, scope, incarnation, me, now],
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
    tx.commit().map_err(sql)?;
    Ok(Submitted {
        job_id,
        owned: true,
        joined: false,
    })
}

/// Deletes generation `gen_id` and everything it holds, in one transaction.
fn drop_gen(conn: &Connection, gen_id: i64) -> rusqlite::Result<()> {
    for table in ["records", "terms", "edges", "generations"] {
        conn.execute(&format!("DELETE FROM {table} WHERE gen = ?1"), [gen_id])?;
    }
    Ok(())
}

/// Deletes the unpublished generations job `job_id` started.
fn drop_building(conn: &mut Connection, job_id: &str) -> Result<()> {
    let gens: Vec<i64> = conn
        .prepare("SELECT gen FROM generations WHERE job_id = ?1 AND state = 'building'")
        .and_then(|mut s| {
            s.query_map([job_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()
        })
        .map_err(sql)?;
    for gen_id in gens {
        let tx = conn.transaction().map_err(sql)?;
        drop_gen(&tx, gen_id).map_err(sql)?;
        tx.commit().map_err(sql)?;
    }
    Ok(())
}

/// Ends job `job_id` in a terminal state, if this process still owns it and it is active.
fn finish(conn: &Connection, job_id: &str, me: &str, state: &str, error: Option<Json>) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET state = ?3, error = ?4, finished_ms = ?5 WHERE job_id = ?1 AND owner = ?2
         AND state IN ('queued', 'running')",
        params![job_id, me, state, error.map(|e| e.to_string()), now_ms()],
    )
    .map_err(sql)?;
    Ok(())
}

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

/// The child process that builds one job. It exits when its stdin closes, which happens when
/// this process exits or dies on every OS, so the build never outlives its owner.
fn spawn(dir: &Path, job_id: &str) -> Result<Child> {
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg("__index-worker")
        .arg(dir)
        .arg(job_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| fail(ErrorKind::OperationFailed, format!("cannot start the index worker: {e}")))
}

/// Test builds only: `FIGLENS_TEST_HOLD=<stage>:<file>` keeps the worker in `stage` while
/// `file` exists, so process tests reach each stage deterministically.
fn hold(stage: &str) {
    #[cfg(debug_assertions)]
    if let Some((at, file)) = std::env::var("FIGLENS_TEST_HOLD")
        .ok()
        .as_deref()
        .and_then(|v| v.split_once(':'))
        && at == stage
    {
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
    let select = Select::Id(ws_id.to_owned());
    let (_, mut ws) = opened(storage, &select, true)?;
    let dir = ws.held.dir.clone();
    let me = owner(&dir)?;
    let mine: Option<(String, String)> = ws
        .conn
        .query_row(
            "SELECT owner, state FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if mine.as_ref().is_none_or(|(o, s)| *o != me || s != "queued") {
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    }
    // Wait for the build lock, holding it only while this job is the oldest live queued job,
    // so no process holds it while waiting for its turn.
    let build_path = dir.join("build.lock");
    let build = open_lock(&build_path, true)?.expect("created");
    let mut beat = Instant::now();
    loop {
        if cancelled(&ws.conn, job_id, cancel)? {
            finish(&ws.conn, job_id, &me, "cancelled", None)?;
            return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
        }
        if build.try_lock().is_ok() {
            let mut head = None;
            let mut stmt = ws
                .conn
                .prepare("SELECT job_id, owner FROM jobs WHERE state = 'queued' ORDER BY created_ms, job_id")
                .map_err(sql)?;
            for row in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(sql)?
            {
                let (job, holder) = row.map_err(sql)?;
                if alive(&dir, &holder) {
                    head = Some(job);
                    break;
                }
            }
            drop(stmt);
            if head.as_deref() == Some(job_id) {
                break;
            }
            let _ = build.unlock();
        }
        if beat.elapsed() > Duration::from_secs(1) {
            ws.conn
                .execute("UPDATE jobs SET heartbeat_ms = ?2 WHERE job_id = ?1", params![job_id, now_ms()])
                .map_err(sql)?;
            beat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let started = ws
        .conn
        .execute(
            "UPDATE jobs SET state = 'running', stage = 'stage', heartbeat_ms = ?3,
             expected_gen = (SELECT p.gen FROM published p JOIN jobs j ON j.job_id = ?1
                             WHERE p.source_id = j.source_id AND p.scope = j.scope)
             WHERE job_id = ?1 AND owner = ?2 AND state = 'queued' AND cancel = 0",
            params![job_id, me, now_ms()],
        )
        .map_err(sql)?;
    if started == 0 {
        finish(&ws.conn, job_id, &me, "cancelled", None)?;
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    }
    drop_building(&mut ws.conn, job_id)?;
    let mut child = spawn(&dir, job_id)?;
    let mut out = String::new();
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let mut last = Json::Null;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if cancelled(&ws.conn, job_id, cancel)? {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        if let Some(now) = report_job(&ws.conn, &dir, job_id)?
            && (now["stage"] != last["stage"] || now["progress"] != last["progress"])
        {
            progress(&now);
            last = now;
        }
        if beat.elapsed() > Duration::from_secs(1) {
            ws.conn
                .execute("UPDATE jobs SET heartbeat_ms = ?2 WHERE job_id = ?1", params![job_id, now_ms()])
                .map_err(sql)?;
            beat = Instant::now();
        }
        std::thread::sleep(POLL);
    };
    let output = reader.join().unwrap_or_default();
    let built: Json = output
        .lines()
        .last()
        .and_then(|l| serde_json::from_str(l).ok())
        .unwrap_or(Json::Null);
    match status {
        None => {
            // The child has exited: nothing computes and nothing more is written for this job.
            finish(&ws.conn, job_id, &me, "cancelled", None)?;
            drop_building(&mut ws.conn, job_id)?;
            return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
        }
        Some(s) if !s.success() || built["error"].is_object() || !built["gen"].is_i64() => {
            let error = if built["error"].is_object() {
                built["error"].clone()
            } else {
                error_json("operation_failed", format!("the index worker exited with {s}"))
            };
            finish(&ws.conn, job_id, &me, "failed", Some(error))?;
            drop_building(&mut ws.conn, job_id)?;
            return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
        }
        Some(_) => {}
    }
    // Publication: release the build and workspace locks, then take the location lock shared and
    // the workspace lock again (the lock order), re-open and re-validate everything, and publish
    // in one short transaction. The owner lock persists, so nobody takes the job over meanwhile.
    drop(build);
    drop(ws);
    publish(storage, ws_id, job_id, &me, &built)
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
pub(crate) fn publish(
    storage: &Storage,
    ws_id: &str,
    job_id: &str,
    me: &str,
    built: &Json,
) -> Result<Json> {
    let _location = location_shared(storage, ws_id)?;
    // `opened` re-reads the location record and refuses a database it no longer binds.
    let (_, mut ws) = opened(storage, &Select::Id(ws_id.to_owned()), true)?;
    let dir = ws.held.dir.clone();
    let gen_id = built["gen"].as_i64().expect("checked");
    let unchanged = built["unchanged"] == true;
    let incarnation = ws.incarnation.clone();
    let tx = ws
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let job: Option<(String, String, String, String, String, Option<i64>, bool, String)> = tx
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
        Some(("workspace_recreated", "the workspace was reset or recreated while the job ran"))
    } else if holder != me || state != "running" {
        Some(("not_owner", "another process took the job over"))
    } else if cancel {
        Some(("cancelled", ""))
    } else if current.as_deref() != Some(membership.as_str()) {
        Some(("membership_changed", "the source was removed, re-added or relocated while the job ran"))
    } else if published != expected {
        Some(("superseded", "another build published this source and scope first"))
    } else {
        None
    };
    if let Some((code, why)) = refusal {
        drop(tx);
        if code == "cancelled" {
            finish(&ws.conn, job_id, me, "cancelled", None)?;
        } else if holder == me {
            finish(&ws.conn, job_id, me, "failed", Some(error_json(code, why)))?;
        }
        drop_building(&mut ws.conn, job_id)?;
        return report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id));
    }
    let now = now_ms();
    tx.execute(
        "INSERT INTO snapshots (source_id, snapshot_id, bytes, mtime_ns, verified_ms)
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (source_id, snapshot_id) DO UPDATE SET
         bytes = excluded.bytes, mtime_ns = excluded.mtime_ns, verified_ms = excluded.verified_ms",
        params![source, built["snapshot"].as_str(), built["bytes"].as_i64(), built["mtimeNs"].as_i64(), now],
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
    // Bounded scopes: the least recently published one beyond the cap is retired.
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
    tx.execute(
        "UPDATE jobs SET state = 'succeeded', stage = 'publish', gen = ?2, finished_ms = ?3
         WHERE job_id = ?1",
        params![job_id, gen_id, now],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    // Retention, outside the publication: one retired generation per source and scope.
    prune(&mut ws.conn, &source, &scope)?;
    report_job(&ws.conn, &dir, job_id)?.ok_or_else(|| unknown_job(job_id))
}

/// Deletes retired generations of `source` and `scope` beyond the newest one, and snapshots no
/// generation names. A cursor over a deleted generation reads `cursor_expired`.
fn prune(conn: &mut Connection, source: &str, scope: &str) -> Result<()> {
    let old: Vec<i64> = conn
        .prepare(
            "SELECT gen FROM generations WHERE source_id = ?1 AND scope = ?2 AND state = 'retired'
             ORDER BY retired_ms DESC, gen DESC LIMIT -1 OFFSET 1",
        )
        .and_then(|mut s| {
            s.query_map([source, scope], |r| r.get(0))?
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
    let row: Option<(String, String)> = ws
        .conn
        .query_row(
            "SELECT state, owner FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let (state, holder) = row.ok_or_else(|| unknown_job(job_id))?;
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
    if !alive(&dir, &holder) {
        ws.conn
            .execute(
                "UPDATE jobs SET state = 'cancelled', cancel = 1, finished_ms = ?2 WHERE job_id = ?1
                 AND state IN ('queued', 'running')",
                params![job_id, now_ms()],
            )
            .map_err(sql)?;
        drop_building(&mut ws.conn, job_id)?;
    } else {
        ws.conn
            .execute(
                "UPDATE jobs SET cancel = 1 WHERE job_id = ?1 AND state IN ('queued', 'running')",
                [job_id],
            )
            .map_err(sql)?;
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
/// stdin closes, so it never outlives the owner that spawned it.
pub fn worker(dir: &Path, job_id: &str) -> Json {
    std::thread::spawn(|| {
        let mut sink = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut sink);
        std::process::exit(3);
    });
    match work(dir, job_id) {
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
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::QueryWorkLimit => "query_work_limit",
    }
}

/// Database plus WAL bytes of the workspace at `dir`.
fn disk(dir: &Path) -> (u64, u64) {
    let size = |name: &str| std::fs::metadata(dir.join(name)).map_or(0, |m| m.len());
    (size(DB), size("index.sqlite-wal"))
}

fn work(dir: &Path, job_id: &str) -> Result<Json> {
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
    let mut conn = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    writable(&conn)?;
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
             WHERE job_id = ?1 AND state = 'running'",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .map_err(sql)?;
    if meta(&conn, "workspace_id").map_err(sql)?.as_deref() != Some(id.as_str())
        || meta(&conn, "incarnation").map_err(sql)?.as_deref() != Some(incarnation.as_str())
    {
        return Err(fail(ErrorKind::OperationFailed, "the workspace was recreated"));
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
        conn.execute(
            "UPDATE jobs SET stage = ?2 WHERE job_id = ?1",
            [job_id, stage],
        )
        .map_err(sql)?;
        hold(stage);
        Ok::<_, Box<dyn Error>>(())
    };
    let path = PathBuf::from(path);
    let (bytes, stamp) = search::stage(&path)?;
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
                "SELECT snapshot_id = ?2 AND versions = ?3 AND complete = 1 FROM generations
                 WHERE gen = ?1",
                params![gen_id, snapshot, versions],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        // Only a complete generation of the same bytes and versions is reused; a partial one
        // (a deadline, say) is rebuilt.
        if same == Some(true) {
            let mut out = snap;
            out["gen"] = json!(gen_id);
            out["unchanged"] = json!(true);
            return Ok(out);
        }
    }
    stage(&conn, "parse")?;
    let label = path.display().to_string();
    let index = search::index_bytes(bytes, &label, &scope, search::deadline())?;
    stage(&conn, "write")?;
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id,
         state, created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'building', ?7)",
        params![source_id, membership, scope, snapshot, versions, job_id, now_ms()],
    )
    .map_err(sql)?;
    let gen_id = conn.last_insert_rowid();
    let mut between = |conn: &Connection, done: usize, total: usize| -> Result<()> {
        conn.execute(
            "UPDATE jobs SET done = ?2, total = ?3 WHERE job_id = ?1",
            params![job_id, done as i64, total as i64],
        )
        .map_err(sql)?;
        // Space: a batch adds at most a few MiB, so checking at every boundary keeps the
        // overshoot to one batch.
        let (db, wal) = disk(dir);
        if db + wal > DISK_CAP {
            return Err(fail(
                ErrorKind::OperationFailed,
                format!("the workspace database reached its {DISK_CAP}-byte cap; clear_cache, remove sources or index a narrower scope"),
            ));
        }
        let wait = Instant::now();
        loop {
            let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
            let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
            if disk(dir).1 <= WAL_CAP {
                return Ok(());
            }
            if wait.elapsed() > LOCK_WAIT {
                return Err(fail(
                    ErrorKind::WorkspaceBusy,
                    format!("a reader keeps the write-ahead log above {WAL_CAP} bytes; retry when other searches finish"),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    search::store::save(&mut conn, gen_id, &index, &mut between)?;
    drop(index);
    stage(&conn, "verify")?;
    // The source is read and hashed again: a save that kept its size and time is caught here.
    let (again, _) = search::stage(&path)?;
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
fn chosen(storage: &Storage, ws: &Opened, only: Option<&str>) -> Result<Vec<String>> {
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
        .filter(|(id, path)| storage.sees(Path::new(path)) && only.is_none_or(|o| o == id))
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
/// the source has published.
fn submit_all(
    storage: &Storage,
    select: &Select,
    kind: &str,
    only: Option<&str>,
    scope: Option<&str>,
    request_id: Option<&str>,
) -> Result<Outcome> {
    let (place, mut ws) = opened(storage, select, true)?;
    let mut plan = Vec::new();
    for source in chosen(storage, &ws, only)? {
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
            format!("workspace {} has no indexed scope to {kind}; pass a scope to index one", ws.held.id),
        ));
    }
    if request_id.is_some() && plan.len() > 1 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "requestId names one job; pass a sourceId and scope with it",
        ));
    }
    let (mut run, mut reports) = (Vec::new(), Vec::new());
    for (source, scope) in plan {
        let job = submit(&mut ws, kind, &source, &scope, request_id)?;
        if job.owned {
            run.push(job.job_id.clone());
        }
        let mut report = report_job(&ws.conn, &ws.held.dir, &job.job_id)?.unwrap_or(Json::Null);
        report["joined"] = json!(job.joined);
        reports.push(report);
    }
    let report = opened_report(storage, &place, &ws, json!({"jobs": reports}))?;
    Ok(Outcome { report, run })
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
    submit_all(storage, select, "refresh", source, scope, request_id)
}

/// `index rebuild`: builds every published scope of every source again, even unchanged ones,
/// keeping the registry and each prior generation until its replacement publishes.
pub fn rebuild(storage: &Storage, select: &Select, request_id: Option<&str>) -> Result<Outcome> {
    submit_all(storage, select, "rebuild", None, None, request_id)
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
            "SELECT g.gen, g.source_id, g.state, j.state, j.owner FROM generations g
             LEFT JOIN jobs j ON j.job_id = g.job_id WHERE g.state IN ('retired', 'building')",
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
            ))
        })
        .map_err(sql)?
    {
        let (gen_id, source, state, job, holder) = row.map_err(sql)?;
        let writing = state == "building"
            && matches!(job.as_deref(), Some("queued" | "running"))
            && holder.is_some_and(|h| alive(&dir, &h));
        // A removed source's generations belong to nobody visible; they go too.
        let owned = visible.contains(&source) || storage.visible.is_none();
        if !writing && owned {
            doomed.push(gen_id);
        }
    }
    drop(stmt);
    let mut records = 0i64;
    for gen_id in &doomed {
        let tx = ws.conn.transaction().map_err(sql)?;
        records += tx
            .query_row("SELECT count(*) FROM records WHERE gen = ?1", [gen_id], |r| r.get::<_, i64>(0))
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
    let cache = place.roots.cache.path.join("workspace-cache").join(&ws.held.id);
    let extra = json!({
        "removed": {"generations": doomed.len(), "records": records},
        "retained": {"publishedGenerations": if storage.visible.is_none() { json!(published) } else { Json::Null },
            "registry": true, "jobs": true, "location": true},
        "failed": [],
        "cache": {"path": cache, "present": cache.exists(),
            "note": "figlens writes no cache files in this version"},
    });
    opened_report(storage, &place, &ws, extra)
}
