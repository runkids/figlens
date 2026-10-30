use super::*;
use crate::test_support::TestDir;
use crate::workspace::{PrepareOptions, prepare, resolve};
use std::collections::BTreeSet;
use std::ffi::OsString;

const FIXTURE: &str = "examples/first-success/design.fig";

/// A prepared workspace with `n` copies of the public fixture as sources, its storage, selector
/// and directory.
fn workspace(dir: &TestDir, n: usize) -> (Storage, Select, PathBuf) {
    let home = std::path::absolute(dir.join("home")).unwrap();
    let home = home.to_str().unwrap().to_owned();
    let vars = move |name: &str| (name == "HOME").then(|| OsString::from(&home));
    let storage = resolve(None, &vars, false, &|| None).unwrap();
    let project = std::path::absolute(dir.join("project")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let mut select = None;
    for i in 0..n {
        let fig = project.join(format!("{i}.fig"));
        std::fs::copy(FIXTURE, &fig).unwrap();
        let report = prepare(&storage, &fig, Some(&project), PrepareOptions::default()).unwrap();
        select = Some(Select::Id(
            report.report["workspaceId"].as_str().unwrap().to_owned(),
        ));
    }
    let select = select.expect("n > 0");
    let (_, ws) = opened(&storage, &select, false).unwrap();
    let dir = ws.held.dir.clone();
    (storage, select, dir)
}

fn connection(dir: &Path) -> Connection {
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap()
}

/// Inserts a job for `source` at `scope` in `state`, held by `holder`'s `attempt`.
fn job(
    conn: &Connection,
    source: &str,
    scope: &str,
    state: &str,
    holder: &str,
    attempt: i64,
) -> String {
    let ws: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'workspace_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let job_id = format!("{ws}.{}", random_hex());
    conn.execute(
        "INSERT INTO jobs (job_id, fingerprint, kind, source_id, membership, scope, incarnation,
         state, owner, attempt, created_ms, heartbeat_ms)
         SELECT ?1, 'test', 'refresh', ?2, COALESCE((SELECT membership FROM sources WHERE source_id = ?2), 'none'),
         ?3, (SELECT value FROM meta WHERE key = 'incarnation'), ?4, ?5, ?6, 0, 0",
        params![job_id, source, scope, state, holder, attempt],
    )
    .unwrap();
    job_id
}

fn building(conn: &Connection, job_id: &str) -> i64 {
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, versions, job_id, state, created_ms)
         SELECT source_id, membership, scope, 'test', job_id, 'building', 0 FROM jobs WHERE job_id = ?1",
        [job_id],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn count(conn: &Connection, query: &str, arg: impl rusqlite::ToSql) -> i64 {
    conn.query_row(query, [arg], |r| r.get(0)).unwrap()
}

const GENERATIONS: &str = "SELECT count(*) FROM generations WHERE job_id = ?1";

fn sources(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT source_id FROM sources ORDER BY source_id")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn state(conn: &Connection, job_id: &str) -> String {
    conn.query_row("SELECT state FROM jobs WHERE job_id = ?1", [job_id], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_plan_past_capacity_finishes_through_its_deferred_steps_without_repeating_admitted_items() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 3);
    let conn = connection(&dir);
    for source in sources(&conn) {
        for page in 0..MAX_SCOPES {
            conn.execute(
                "INSERT INTO published (source_id, scope, gen, published_ms) VALUES (?1, ?2, 0, 0)",
                params![source, format!("{page}:0")],
            )
            .unwrap();
        }
    }
    let target = |job_id: &str| -> (String, String) {
        conn.query_row(
            "SELECT source_id, scope FROM jobs WHERE job_id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    let first = rebuild(&storage, &select, None).unwrap();
    assert_eq!(first.report["admission"], "partial");
    assert_eq!(first.run.len() as i64, MAX_ACTIVE);
    let deferred = first.report["deferred"].as_array().unwrap().clone();
    assert_eq!(deferred.len() as i64, 3 * MAX_SCOPES - MAX_ACTIVE);
    let mut built: BTreeSet<_> = first.run.iter().map(|j| target(j)).collect();
    // The admitted jobs finish; each deferred step then admits exactly its own item.
    conn.execute("UPDATE jobs SET state = 'succeeded'", [])
        .unwrap();
    for item in deferred {
        let arguments = &item["nextStep"]["arguments"];
        assert_eq!(arguments["action"], "rebuild");
        let next = rebuild_scope(
            &storage,
            &select,
            arguments["sourceId"].as_str(),
            arguments["scope"].as_str(),
            None,
        )
        .unwrap();
        assert_eq!(next.report["admission"], "complete");
        assert_eq!(next.run.len(), 1);
        assert!(
            built.insert(target(&next.run[0])),
            "an item was rebuilt again"
        );
        conn.execute("UPDATE jobs SET state = 'succeeded'", [])
            .unwrap();
    }
    assert_eq!(built.len() as i64, 3 * MAX_SCOPES);
}

#[test]
fn a_stale_attempt_never_drops_the_current_attempts_generation() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let mut conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "new", 2);
    building(&conn, &job_id);
    drop_building(&mut conn, &job_id, "old", 1).unwrap();
    assert_eq!(
        count(&conn, GENERATIONS, &job_id),
        1,
        "a stale attempt dropped the new one's work"
    );
    drop_building(&mut conn, &job_id, "new", 2).unwrap();
    assert_eq!(count(&conn, GENERATIONS, &job_id), 0);
}

#[test]
fn a_stale_launch_writes_nothing() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "new", 2);
    let error = work(&dir, &job_id, "old", 1).unwrap_err().to_string();
    assert!(error.contains("another attempt"), "{error}");
    assert_eq!(count(&conn, GENERATIONS, &job_id), 0);
    let stage: Option<String> = conn
        .query_row("SELECT stage FROM jobs WHERE job_id = ?1", [&job_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stage, None);
}

#[test]
fn a_takeover_between_batches_commits_nothing_more_from_the_stale_writer() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let mut conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "old", 1);
    let gen_id = building(&conn, &job_id);
    let bytes = std::fs::read(FIXTURE).unwrap();
    let index = search::index_bytes(bytes, "fixture", "0:0", search::deadline()).unwrap();
    let guard = |conn: &Connection| owns_launch(conn, &job_id, "old", 1);
    // Another attempt takes the job over right after the first committed batch.
    let mut between = |conn: &Connection, _: usize, _: usize| -> Result<()> {
        conn.execute(
            "UPDATE jobs SET owner = 'new', attempt = 2 WHERE job_id = ?1",
            [&job_id],
        )
        .map_err(sql)?;
        Ok(())
    };
    let error = search::store::save(&mut conn, gen_id, &index, &guard, &mut between).unwrap_err();
    assert!(error.to_string().contains("another attempt"), "{error}");
    let terms = "SELECT count(*) FROM terms WHERE gen = ?1";
    assert_eq!(count(&conn, terms, gen_id), 0, "a stale batch committed");
    let complete = "SELECT complete FROM generations WHERE gen = ?1";
    assert_eq!(count(&conn, complete, gen_id), 0);
    // The new attempt clears what the stale one wrote before the takeover: no orphan records.
    drop_building(&mut conn, &job_id, "new", 2).unwrap();
    let records = "SELECT count(*) FROM records WHERE gen = ?1";
    assert_eq!(count(&conn, records, gen_id), 0);
}

#[test]
fn a_refresh_builds_again_when_the_source_membership_changed() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let bytes = std::fs::read(FIXTURE).unwrap();
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id,
         state, complete, created_ms) VALUES (?1, 'before', '0:0', ?2, ?3, 'old', 'published', 1, 0)",
        params![source, search::snapshot_id(&bytes), search::store::versions()],
    )
    .unwrap();
    let old = conn.last_insert_rowid();
    let me = owner(&dir).unwrap();
    let job_id = job(&conn, &source, "0:0", "running", &me, 1);
    conn.execute(
        "UPDATE jobs SET expected_gen = ?2 WHERE job_id = ?1",
        params![job_id, old],
    )
    .unwrap();
    let out = work(&dir, &job_id, &me, 1).unwrap();
    assert_ne!(out["unchanged"], true, "{out}");
    assert_ne!(out["gen"], old, "{out}");
}

/// A stand-in worker running `script`, with its output read as `build` reads it.
#[cfg(unix)]
fn stand_in(script: &str) -> (Option<Child>, std::thread::JoinHandle<String>, Receiver<()>) {
    let mut child = Command::new("sh")
        .args(["-c", script])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let (reader, eof) = output(&mut child);
    (Some(child), reader, eof)
}

#[cfg(unix)]
#[test]
fn a_worker_is_seen_exiting_when_its_output_ends_not_a_poll_later() {
    let (mut child, reader, eof) = stand_in("echo done");
    let started = Instant::now();
    let status = wait_worker(&mut child, &eof, || Ok(false)).unwrap();
    assert!(status.unwrap().success());
    assert!(started.elapsed() < POLL, "{:?}", started.elapsed());
    assert_eq!(reader.join().unwrap(), "done\n");
}

#[cfg(unix)]
#[test]
fn a_worker_that_closes_its_output_early_is_waited_for_without_spinning() {
    let (mut child, reader, eof) = stand_in("exec >&-; sleep 0.5");
    let mut ticks = 0;
    let status = wait_worker(&mut child, &eof, || {
        ticks += 1;
        Ok(false)
    })
    .unwrap();
    assert!(status.unwrap().success());
    // One wake, at most one POLL of 5 ms steps, then whole POLLs: about 25 checks in 0.5 s.
    assert!(ticks <= 30, "{ticks} checks");
    assert_eq!(reader.join().unwrap(), "");
}

#[cfg(unix)]
#[test]
fn a_cancelled_wait_stops_and_reaps_its_worker() {
    let (mut child, reader, eof) = stand_in("exec sleep 30");
    let pid = child.as_ref().unwrap().id().to_string();
    assert!(
        wait_worker(&mut child, &eof, || Ok(true))
            .unwrap()
            .is_none()
    );
    assert!(child.is_none());
    let exists = Command::new("kill")
        .args(["-0", &pid])
        .stderr(Stdio::null())
        .status();
    assert!(!exists.unwrap().success());
    assert_eq!(reader.join().unwrap(), "");
}

#[cfg(unix)]
#[test]
fn a_failed_run_reaps_its_child_before_it_records_and_cleans_up() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let mut conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "me", 1);
    building(&conn, &job_id);
    let held = claim(&dir, &job_id, 1).unwrap();
    let child = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().to_string();
    let error = abandon(
        &mut conn,
        &job_id,
        "me",
        1,
        (Some(child), held, None),
        "spawn failed".into(),
    );
    assert_eq!(error.to_string(), "spawn failed");
    let exists = Command::new("kill")
        .args(["-0", &pid])
        .stderr(Stdio::null())
        .status();
    assert!(
        !exists.unwrap().success(),
        "abandon returned before its child was reaped"
    );
    let recorded: String = conn
        .query_row("SELECT error FROM jobs WHERE job_id = ?1", [&job_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state(&conn, &job_id), "failed");
    assert!(recorded.contains("spawn failed"), "{recorded}");
    assert_eq!(count(&conn, GENERATIONS, &job_id), 0);
    assert!(!claim_path(&dir, &job_id, 1).exists());
}

#[test]
fn a_run_that_cannot_record_its_end_reads_interrupted_and_retries_while_its_owner_lives() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let job_id = job(&conn, &source, "0:0", "running", &me, 1);
    let held = claim(&dir, &job_id, 1).unwrap();
    // The end of the run meets storage that cannot be written.
    let mut read_only = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let _ = abandon(
        &mut read_only,
        &job_id,
        &me,
        1,
        (None, held, None),
        "lost".into(),
    );
    drop(read_only);
    assert_eq!(state(&conn, &job_id), "running");
    let report = report_job(&conn, &dir, &job_id).unwrap().unwrap();
    assert_eq!(report["state"], "interrupted", "{report}");
    // Storage is back: a refresh takes the job over as the next attempt.
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let again = submit(&storage, &mut ws, "refresh", &source, "0:0", None).unwrap();
    assert!(again.owned && again.job_id == job_id);
    let attempt = "SELECT attempt FROM jobs WHERE job_id = ?1";
    assert_eq!(count(&conn, attempt, &job_id), 2);
}

#[cfg(unix)]
#[test]
fn an_owner_file_that_cannot_be_opened_counts_as_alive() {
    use std::os::unix::fs::PermissionsExt;
    let d = TestDir::new();
    let owners = std::path::absolute(d.join("owners")).unwrap();
    std::fs::create_dir_all(&owners).unwrap();
    let file = owners.join("someone.lock");
    std::fs::write(&file, "").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unprivileged = File::open(&file).is_err();
    let dir = owners.parent().unwrap();
    let seen = alive(dir, "someone");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    // Root opens it anyway; there is nothing to observe then.
    if unprivileged {
        assert!(seen, "an unreadable owner file was taken as a dead owner");
    }
    assert!(
        !alive(dir, "nobody"),
        "a missing owner file is a dead owner"
    );
}

/// Submits `scope` of the first source as this process's own queued job.
fn queued(storage: &Storage, select: &Select, scope: &str) -> String {
    let (_, mut ws) = opened(storage, select, true).unwrap();
    let source = sources(&ws.conn)[0].clone();
    submit(storage, &mut ws, "refresh", &source, scope, None)
        .unwrap()
        .job_id
}

#[test]
fn jobs_submitted_in_one_millisecond_build_in_submission_order() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    // The first job's owner is another live process.
    std::fs::create_dir_all(dir.join("owners")).unwrap();
    let other = File::create(dir.join("owners").join("other.lock")).unwrap();
    other.lock().unwrap();
    let first = job(&conn, &source, "0:0", "queued", "other", 1);
    let second = job(&conn, &source, "1:0", "queued", &me, 1);
    // The later job gets the smaller id, so an id order would run it first.
    let ws: String = first.split('.').next().unwrap().to_owned();
    conn.execute(
        "UPDATE jobs SET job_id = ?2 WHERE job_id = ?1",
        params![second, format!("{ws}.0")],
    )
    .unwrap();
    assert!(
        !my_turn(&conn, &dir, &second, false).unwrap(),
        "{first} goes first"
    );
}

#[test]
fn a_run_that_fails_before_it_can_write_leaves_its_job_interrupted() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let job_id = queued(&storage, &select, "0:0");
    let conn = connection(&dir);
    assert_eq!(
        report_job(&conn, &dir, &job_id).unwrap().unwrap()["state"],
        "queued"
    );
    // Another process holds the workspace exclusively, so the run cannot even open it.
    let place = storage.place(&select.id().unwrap(), None).unwrap();
    let held = acquire(&place.roots.state.path, &select, true).unwrap();
    let ws_id = select.id().unwrap();
    let reply = run(
        &storage,
        &ws_id,
        &job_id,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    drop(held);
    let error = reply.unwrap_err().downcast::<FigError>().unwrap();
    assert_eq!(error.kind(), ErrorKind::WorkspaceBusy);
    let report = report_job(&conn, &dir, &job_id).unwrap().unwrap();
    assert_eq!(report["state"], "interrupted", "{report}");
    assert!(!job_alive(&dir, &owner(&dir).unwrap(), &job_id, 1));
}

#[test]
fn cancelling_a_live_owners_queued_job_ends_it_at_once() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    queued(&storage, &select, "0:0");
    let behind = queued(&storage, &select, "1:0");
    let report = cancel(&storage, &behind).unwrap();
    assert_eq!(report["cancel"], "accepted", "{report}");
    assert!(report["cancelWaitMs"].as_u64().unwrap() < 1000, "{report}");
    let reply = run(
        &storage,
        &select.id().unwrap(),
        &behind,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    assert_eq!(reply.unwrap()["state"], "cancelled");
    assert!(!claim_path(&dir, &behind, 1).exists());
}

#[test]
fn a_job_waits_for_its_turn_only_so_long() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let behind = queued(&storage, &select, "1:0");
    // An older job of another live process holds the turn.
    std::fs::create_dir_all(dir.join("owners")).unwrap();
    let other = File::create(dir.join("owners").join("other.lock")).unwrap();
    other.lock().unwrap();
    let conn = connection(&dir);
    job(&conn, &sources(&conn)[0], "0:0", "queued", "other", 1);
    let started = Instant::now();
    let reply = run(
        &storage,
        &select.id().unwrap(),
        &behind,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    let error = reply.unwrap_err().downcast::<FigError>().unwrap();
    assert_eq!(error.kind(), ErrorKind::WorkspaceBusy);
    assert!(started.elapsed() < QUEUE_WAIT * 10);
    assert_eq!(state(&connection(&dir), &behind), "failed");
}

#[test]
fn a_queued_source_that_left_the_visible_roots_is_never_built() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let job_id = queued(&storage, &select, "0:0");
    let elsewhere = std::path::absolute(d.join("elsewhere")).unwrap();
    let narrowed = Storage {
        visible: Some(vec![elsewhere]),
        ..storage
    };
    let reply = run(
        &narrowed,
        &select.id().unwrap(),
        &job_id,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    let error = reply.unwrap_err().to_string();
    assert!(error.contains("visible roots"), "{error}");
    let conn = connection(&dir);
    assert_eq!(state(&conn, &job_id), "failed");
    assert_eq!(count(&conn, GENERATIONS, &job_id), 0);
}

#[test]
fn publishing_past_the_scope_cap_reports_and_prunes_the_evicted_scope() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    for page in 1..=MAX_SCOPES {
        let old = job(&conn, &source, &format!("{page}:0"), "succeeded", &me, 1);
        let gen_id = building(&conn, &old);
        conn.execute(
            "UPDATE generations SET state = 'published' WHERE gen = ?1",
            [gen_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO published (source_id, scope, gen, published_ms) VALUES (?1, ?2, ?3, ?4)",
            params![source, format!("{page}:0"), gen_id, page],
        )
        .unwrap();
        // An older retired generation of the scope that is about to be evicted.
        if page == 1 {
            let older = building(&conn, &old);
            conn.execute(
                "UPDATE generations SET state = 'retired', retired_ms = 0 WHERE gen = ?1",
                [older],
            )
            .unwrap();
        }
    }
    let job_id = job(&conn, &source, "0:0", "running", &me, 1);
    let gen_id = building(&conn, &job_id);
    let built = json!({"gen": gen_id, "snapshot": "sha256:test", "bytes": 1, "mtimeNs": 1});
    let report = publish(&storage, &select.id().unwrap(), &job_id, &me, 1, &built).unwrap();
    assert_eq!(report["state"], "succeeded", "{report}");
    assert_eq!(report["evictedScopes"], json!(["1:0"]), "{report}");
    // Status reads it back from the job, as an MCP caller that never saw the run's reply does.
    let later = status(&storage, &job_id).unwrap();
    assert_eq!(later["evictedScopes"], json!(["1:0"]), "{later}");
    let kept = "SELECT count(*) FROM generations WHERE scope = ?1";
    assert_eq!(
        count(&conn, kept, "1:0"),
        0,
        "an evicted scope kept its generations"
    );
}

#[test]
fn a_job_that_cannot_be_claimed_is_not_admitted() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    // A file where the claims directory goes.
    std::fs::write(dir.join("claims"), "").unwrap();
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let source = sources(&ws.conn)[0].clone();
    assert!(submit(&storage, &mut ws, "refresh", &source, "0:0", None).is_err());
    assert_eq!(count(&ws.conn, "SELECT count(*) FROM jobs WHERE ?1", 1), 0);
}

#[test]
fn a_taken_over_job_queues_behind_another_owner_s_job_so_no_two_owners_wait_on_each_other() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    // This owner (A) admits a new job; owner B has a queued job; A then takes over an older job
    // whose owner died.
    let a1 = queued(&storage, &select, "2:0");
    std::fs::create_dir_all(dir.join("owners")).unwrap();
    let other = File::create(dir.join("owners").join("other.lock")).unwrap();
    other.lock().unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    job(&conn, &source, "0:0", "queued", "dead", 1);
    let b1 = job(&conn, &source, "1:0", "queued", "other", 1);
    conn.execute("UPDATE jobs SET seq = 1 WHERE job_id = ?1", [&b1])
        .unwrap();
    conn.execute("UPDATE jobs SET seq = 2 WHERE job_id = ?1", [&a1])
        .unwrap();
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let Ok(a2) = submit(&storage, &mut ws, "refresh", &source, "0:0", None) else {
        panic!("not taken over");
    };
    assert!(a2.owned);
    // B runs its job first; A waits for it, then runs its own in the order it admitted them.
    assert!(my_turn(&conn, &dir, &b1, false).unwrap());
}

#[test]
fn a_submit_never_joins_a_build_of_the_source_s_previous_registration() {
    let d = TestDir::new();
    let (storage, select, _) = workspace(&d, 1);
    queued(&storage, &select, "0:0");
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    ws.conn
        .execute("UPDATE sources SET membership = 'relocated'", [])
        .unwrap();
    let source = sources(&ws.conn)[0].clone();
    let Err(error) = submit(&storage, &mut ws, "refresh", &source, "0:0", None) else {
        panic!("admitted");
    };
    assert_eq!(
        error.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::WorkspaceBusy
    );
}

/// The workspace's two sources, with storage that sees only the first.
fn one_hidden(d: &TestDir) -> (Storage, Storage, Select, String, String) {
    let (storage, select, _) = workspace(d, 2);
    let (_, ws) = opened(&storage, &select, false).unwrap();
    let paths: Vec<(String, String)> = ws
        .conn
        .prepare("SELECT source_id, path FROM sources ORDER BY path")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let seen = std::fs::canonicalize(&paths[0].1).unwrap();
    let confined = storage.clone().confined(&[seen]);
    let [(shown, _), (hidden, _)] = <[_; 2]>::try_from(paths).unwrap();
    (storage, confined, select, shown, hidden)
}

#[test]
fn a_full_workspace_names_no_job_of_a_source_the_caller_cannot_see() {
    let d = TestDir::new();
    let (storage, confined, select, shown, hidden) = one_hidden(&d);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    for i in 0..MAX_ACTIVE {
        submit(
            &storage,
            &mut ws,
            "refresh",
            &hidden,
            &format!("{i}:0"),
            None,
        )
        .unwrap();
    }
    let (_, mut ws) = opened(&confined, &select, true).unwrap();
    let Err(error) = submit(&confined, &mut ws, "refresh", &shown, "99:0", None) else {
        panic!("admitted");
    };
    let recovery = error
        .downcast_ref::<FigError>()
        .unwrap()
        .recovery()
        .unwrap();
    assert_eq!(recovery[0]["jobIds"], json!([]), "{recovery}");
}

#[test]
fn a_submit_of_a_source_the_caller_cannot_see_is_refused_before_admitting_anything() {
    let d = TestDir::new();
    let (storage, confined, select, _, hidden) = one_hidden(&d);
    // A request already admitted for the hidden source is not replayed to this caller either.
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    submit(&storage, &mut ws, "refresh", &hidden, "0:0", Some("r")).unwrap();
    let (_, mut ws) = opened(&confined, &select, true).unwrap();
    let count = |ws: &Opened| -> i64 {
        ws.conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
            .unwrap()
    };
    let before = count(&ws);
    let refused = submit(&confined, &mut ws, "refresh", &hidden, "0:0", Some("r")).is_err();
    assert_eq!((refused, count(&ws)), (true, before));
}

/// The logical size of the database in bytes, uncheckpointed pages included.
fn used(conn: &Connection) -> u64 {
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    (pages * size) as u64
}

/// A published generation of `scope` for the first source: the last good index a failed build
/// must leave serving.
fn published(conn: &Connection, scope: &str) -> i64 {
    let source = sources(conn)[0].clone();
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, versions, job_id, state, complete,
         created_ms) SELECT source_id, membership, ?2, 'test', 'old', 'published', 1, 0
         FROM sources WHERE source_id = ?1",
        params![source, scope],
    )
    .unwrap();
    let gen_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO published (source_id, scope, gen, published_ms) VALUES (?1, ?2, ?3, 0)",
        params![source, scope, gen_id],
    )
    .unwrap();
    gen_id
}

fn serving(conn: &Connection, scope: &str) -> i64 {
    conn.query_row("SELECT gen FROM published WHERE scope = ?1", [scope], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_build_in_a_file_already_past_its_quota_never_grows_it_and_fits_once_space_is_freed() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let mut conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let good = published(&conn, "0:0");
    // Packed pages of a later generation's rows, so the build's rows split a full page instead
    // of fitting into free space left inside existing pages.
    conn.execute_batch(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 199)
         INSERT INTO records (gen, rid, node_id, name, kind, norm0, norm1, len0, len1, hidden)
         SELECT 999999, i, printf('%0100d', i), '', '', '', '', 0, 0, 0 FROM n;
         WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 199)
         INSERT INTO terms (gen, term, df, postings)
         SELECT 999999, printf('%0100d', i), 0, x'' FROM n;
         VACUUM;",
    )
    .unwrap();
    // Uncapped writes take the file past the build's share of the cap, into the headroom.
    let filler = job(&conn, &source, "1:0", "succeeded", &me, 1);
    let room = (DISK_CAP - BUILD_HEADROOM - used(&conn)) as usize + 32 * 1024;
    conn.execute(
        "UPDATE jobs SET error = ?2 WHERE job_id = ?1",
        params![filler, "x".repeat(room)],
    )
    .unwrap();
    let job_id = job(&conn, &source, "0:0", "running", &me, 1);
    let full = used(&conn);
    assert!(full > DISK_CAP - BUILD_HEADROOM);
    let error = work(&dir, &job_id, &me, 1).unwrap_err().to_string();
    // The ceiling SQLite actually set is the file's size, not the smaller quota.
    assert!(
        error.contains(&format!("{full}-byte build ceiling")),
        "{error}"
    );
    assert_eq!(used(&conn), full, "the build grew the file");
    assert_eq!(serving(&conn, "0:0"), good);
    // The owner drops the failed build; freeing the filler leaves the file at its high-water
    // mark, yet the retry reuses the freed pages.
    drop_building(&mut conn, &job_id, &me, 1).unwrap();
    conn.execute("DELETE FROM jobs WHERE job_id = ?1", [&filler])
        .unwrap();
    conn.execute("UPDATE jobs SET attempt = 2 WHERE job_id = ?1", [&job_id])
        .unwrap();
    let out = work(&dir, &job_id, &me, 2).unwrap();
    assert!(out["gen"].is_i64(), "{out}");
    assert_eq!(
        used(&conn),
        full,
        "the retry grew the file instead of reusing freed pages"
    );
}

#[test]
fn a_reader_holding_the_wal_fails_the_build_busy_within_one_batch_of_the_threshold() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let good = published(&conn, "0:0");
    let job_id = job(&conn, &source, "0:0", "running", &me, 1);
    // A search's read transaction pins the WAL: no checkpoint can reset it.
    let reader = connection(&dir);
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM generations", [], |r| r.get(0))
        .unwrap();
    let error = work(&dir, &job_id, &me, 1).unwrap_err();
    let error = error.downcast::<FigError>().unwrap();
    assert_eq!(error.kind(), ErrorKind::WorkspaceBusy, "{error}");
    let wal = wal_bytes(&dir);
    // One batch writes a page or more into each of the generation's b-trees and indexes.
    assert!(
        wal > WAL_CAP && wal <= WAL_CAP + 128 * 1024,
        "WAL at {wal} bytes"
    );
    assert_eq!(serving(&conn, "0:0"), good);
    reader.execute_batch("COMMIT").unwrap();
    conn.execute("UPDATE jobs SET attempt = 2 WHERE job_id = ?1", [&job_id])
        .unwrap();
    assert!(work(&dir, &job_id, &me, 2).unwrap()["gen"].is_i64());
}

#[test]
fn a_cancel_after_the_owner_died_waits_for_its_worker_to_exit() {
    let d = TestDir::new();
    let (storage, _, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "dead", 1);
    std::fs::create_dir_all(dir.join("claims")).unwrap();
    let mut worker = File::create(worker_path(&dir, &job_id, 1)).unwrap();
    worker.lock().unwrap();
    std::io::Write::write_all(&mut worker, b"started").unwrap();
    assert_eq!(cancel(&storage, &job_id).unwrap()["cancel"], "pending");
    drop(worker);
    assert_eq!(cancel(&storage, &job_id).unwrap()["cancel"], "accepted");
}

#[test]
fn a_rebuild_that_takes_over_an_interrupted_refresh_rebuilds() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let job_id = job(&conn, &source, "0:0", "queued", "dead", 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let Ok(taken) = submit(&storage, &mut ws, "rebuild", &source, "0:0", None) else {
        panic!("not taken over");
    };
    assert_eq!(
        (taken.job_id.as_str(), kind_of(&conn, &job_id).as_str()),
        (job_id.as_str(), "rebuild")
    );
}

#[test]
fn a_request_id_that_joined_a_job_replays_it_after_the_job_ended() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let first = queued(&storage, &select, "0:0");
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let source = sources(&ws.conn)[0].clone();
    let Ok(_) = submit(&storage, &mut ws, "refresh", &source, "0:0", Some("r1")) else {
        panic!("not joined");
    };
    connection(&dir)
        .execute("UPDATE jobs SET state = 'succeeded'", [])
        .unwrap();
    let Ok(again) = submit(&storage, &mut ws, "refresh", &source, "0:0", Some("r1")) else {
        panic!("not replayed");
    };
    assert_eq!(again.job_id, first);
}

fn kind_of(conn: &Connection, job_id: &str) -> String {
    conn.query_row("SELECT kind FROM jobs WHERE job_id = ?1", [job_id], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn clear_cache_keeps_the_generation_an_orphaned_worker_still_writes() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let job_id = job(&conn, &sources(&conn)[0], "0:0", "running", "dead", 1);
    let gen_id = building(&conn, &job_id);
    std::fs::create_dir_all(dir.join("claims")).unwrap();
    let mut worker = File::create(worker_path(&dir, &job_id, 1)).unwrap();
    worker.lock().unwrap();
    std::io::Write::write_all(&mut worker, b"started").unwrap();
    let kept = |storage: &Storage| {
        clear_cache(storage, &select).unwrap();
        connection(&dir)
            .query_row(
                "SELECT count(*) FROM generations WHERE gen = ?1",
                [gen_id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    let while_running = kept(&storage);
    drop(worker);
    assert_eq!((while_running, kept(&storage)), (1, 0));
}

#[test]
fn a_failed_rebuild_recommends_a_rebuild_not_a_refresh() {
    let d = TestDir::new();
    let (_, _, dir) = workspace(&d, 1);
    let conn = connection(&dir);
    let next = |kind: &str| {
        let job_id = job(&conn, &sources(&conn)[0], "0:0", "failed", "dead", 1);
        conn.execute(
            "UPDATE jobs SET kind = ?2 WHERE job_id = ?1",
            [&job_id, kind],
        )
        .unwrap();
        report_job(&conn, &dir, &job_id).unwrap().unwrap()["nextSteps"][0]["arguments"]["action"]
            .clone()
    };
    assert_eq!(
        (next("rebuild"), next("refresh")),
        (json!("rebuild"), json!("refresh"))
    );
}

#[test]
fn a_rebuild_does_not_join_a_running_refresh_or_record_its_request_id() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    std::fs::create_dir_all(dir.join("owners")).unwrap();
    let other = File::create(dir.join("owners").join("other.lock")).unwrap();
    other.lock().unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let refresh = job(&conn, &source, "0:0", "running", "other", 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let Err(busy) = submit(&storage, &mut ws, "rebuild", &source, "0:0", Some("r1")) else {
        panic!("joined a running refresh");
    };
    let fig = busy.downcast_ref::<FigError>().unwrap();
    let steps = fig.recovery().unwrap();
    conn.execute("UPDATE jobs SET state = 'succeeded'", [])
        .unwrap();
    let Ok(retried) = submit(&storage, &mut ws, "rebuild", &source, "0:0", Some("r1")) else {
        panic!("not admitted after the refresh ended");
    };
    assert_eq!(
        (
            fig.kind(),
            steps[1]["arguments"]["action"].as_str(),
            retried.job_id != refresh,
            kind_of(&conn, &retried.job_id)
        ),
        (
            ErrorKind::WorkspaceBusy,
            Some("rebuild"),
            true,
            "rebuild".to_owned()
        )
    );
}

#[test]
fn a_rebuild_joining_a_queued_refresh_makes_it_a_rebuild_through_its_failure() {
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    std::fs::create_dir_all(dir.join("owners")).unwrap();
    let other = File::create(dir.join("owners").join("other.lock")).unwrap();
    other.lock().unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let refresh = job(&conn, &source, "0:0", "queued", "other", 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let Ok(joined) = submit(&storage, &mut ws, "rebuild", &source, "0:0", None) else {
        panic!("not joined");
    };
    conn.execute("UPDATE jobs SET state = 'failed'", [])
        .unwrap();
    let report = report_job(&conn, &dir, &refresh).unwrap().unwrap();
    assert_eq!(
        (
            joined.joined,
            &report["kind"],
            &report["nextSteps"][0]["arguments"]["action"]
        ),
        (true, &json!("rebuild"), &json!("rebuild"))
    );
}

#[test]
fn revision_retention_deletes_and_expires_only_whole_groups() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    ws.conn
        .execute_batch(
            "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
             VALUES ('s', '0:0', 'active', 1, 'v', 0);",
        )
        .unwrap();
    // Five rows of 75 bytes a revision: the newest two fit the test's 1024 detail bytes.
    let row = || search::diff::Row {
        frame: "0:0".into(),
        node: "0:1".into(),
        kind: "FRAME".into(),
        change: "changed",
        facets: 1,
        descendants: 0,
    };
    for gen_id in 2..=6 {
        let planned = Planned {
            epoch: 1,
            from: gen_id - 1,
            what: Ok(Ok(search::diff::Revision {
                coverage: json!({"complete": true}),
                summary: json!({"changes": 5}),
                rows: (0..5).map(|_| row()).collect(),
                truncated: false,
                impact: Default::default(),
            })),
        };
        let tx = ws.conn.transaction().unwrap();
        tx.execute(
            "INSERT OR IGNORE INTO facet_sets VALUES (?1, 'v', 1, 'null')",
            [gen_id],
        )
        .unwrap();
        let built = json!({"snapshot": "x"});
        assert!(
            record(
                &tx,
                "s",
                "0:0",
                gen_id,
                Some(gen_id - 1),
                &built,
                Some(planned),
                0
            )
            .unwrap()
        );
        tx.commit().unwrap();
    }
    let kept: Vec<(i64, String, i64)> = ws
        .conn
        .prepare(
            "SELECT gen, detail, (SELECT COUNT(*) FROM revision_changes WHERE rev = id)
             FROM revisions ORDER BY seq",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    // The newest deleted revision's seq marks the gap for checkpoints behind it, with the seq of
    // the publication that deleted it.
    let pruned: String = ws
        .conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'pruned' || char(31) || 's' || char(31) || '0:0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        (kept, pruned),
        (
            vec![
                (4, "expired".into(), 0),
                (5, "complete".into(), 5),
                (6, "complete".into(), 5)
            ],
            "2 5".into()
        )
    );
}

#[test]
fn an_active_scope_published_again_after_eviction_records_a_baseline() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let tx = ws.conn.transaction().unwrap();
    tx.execute_batch(
        "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
         VALUES ('s', '0:0', 'active', 1, 'v', 0);
         INSERT INTO facet_sets VALUES (2, 'v', 1, 'null');",
    )
    .unwrap();
    record(&tx, "s", "0:0", 2, None, &json!({"snapshot": "x"}), None, 0).unwrap();
    let coverage: String = tx
        .query_row("SELECT coverage FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Json>(&coverage).unwrap(),
        json!({"basis": "baseline", "reason": "previous_unpublished"})
    );
}

#[test]
fn a_revision_at_the_high_water_size_keeps_its_rows_in_pages_cleanup_freed() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    // The file grows past the cap, then frees it all: its size stays, its pages are reusable.
    ws.conn
        .execute_batch(&format!(
            "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
             VALUES ('s', '0:0', 'active', 1, 'v', 0);
             CREATE TABLE filler (b BLOB); INSERT INTO filler VALUES (zeroblob({DISK_CAP}));
             DELETE FROM filler;"
        ))
        .unwrap();
    assert!(used(&ws.conn) > DISK_CAP);
    let row = search::diff::Row {
        frame: "0:0".into(),
        node: "0:1".into(),
        kind: "FRAME".into(),
        change: "changed",
        facets: 1,
        descendants: 0,
    };
    let planned = Planned {
        epoch: 1,
        from: 1,
        what: Ok(Ok(search::diff::Revision {
            coverage: json!({"complete": true}),
            summary: json!({"changes": 1}),
            rows: vec![row],
            truncated: false,
            impact: Default::default(),
        })),
    };
    let tx = ws.conn.transaction().unwrap();
    tx.execute("INSERT INTO facet_sets VALUES (2, 'v', 1, 'null')", [])
        .unwrap();
    record(
        &tx,
        "s",
        "0:0",
        2,
        Some(1),
        &json!({"snapshot": "x"}),
        Some(planned),
        0,
    )
    .unwrap();
    let detail: String = tx
        .query_row("SELECT detail FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(detail, "complete");
}

/// Records generation `gen_id` of tracked scope `0:0` of `source`, replacing `gen_id - 1`.
fn recorded(tx: &Connection, source: &str, gen_id: i64, mut planned: Planned) {
    planned.from = gen_id - 1;
    tx.execute_batch(&format!(
        "INSERT OR IGNORE INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
         VALUES ('{source}', '0:0', 'active', 1, 'v', 0);
         INSERT INTO facet_sets VALUES ({gen_id}, 'v', 1, 'null');"
    ))
    .unwrap();
    let built = json!({"snapshot": "x"});
    record(
        tx,
        source,
        "0:0",
        gen_id,
        Some(gen_id - 1),
        &built,
        Some(planned),
        0,
    )
    .unwrap();
}

/// A compared revision of one changed row whose impact reached `hits`, incomplete for `reasons`.
fn impacted(reasons: &[(&'static str, u64)], hits: Vec<search::impact::Hit>) -> Planned {
    let row = search::diff::Row {
        frame: "0:0".into(),
        node: "0:1".into(),
        kind: "FRAME".into(),
        change: "changed",
        facets: 1,
        descendants: 0,
    };
    Planned {
        epoch: 1,
        from: 0,
        what: Ok(Ok(search::diff::Revision {
            coverage: json!({"complete": true}),
            summary: json!({"changes": 1}),
            rows: vec![row],
            truncated: false,
            impact: search::impact::Impact {
                complete: reasons.is_empty(),
                reasons: reasons.iter().copied().collect(),
                hits,
            },
        })),
    }
}

fn hit(node: &str, class: search::impact::Class) -> search::impact::Hit {
    search::impact::Hit {
        node: node.into(),
        class,
        side: 3,
        changed: 1,
        unknown: 2,
        equal: 4,
        via: Some(("instance_of", "0:9".into())),
        frames: [Some("0:0".into()), Some("0:5".into())],
    }
}

/// A hit's entry exactly as a detail page serializes it, and the frames stored beside it.
fn entry_bytes(h: &search::impact::Hit) -> usize {
    let via = h.via.as_ref().map(|(r, t)| (*r, t.as_str()));
    let verdicts = [h.changed, h.unknown, h.equal];
    let entry = search::impact::entry(&h.node, h.class.name(), h.side, verdicts, via);
    entry.to_string().len() + h.frames.iter().flatten().map(String::len).sum::<usize>()
}

/// The stored impact summary, listed classes, detail and detail bytes of generation `gen_id`.
fn impact_of(conn: &Connection, gen_id: i64) -> (Json, Vec<String>, String, i64) {
    let (impact, detail, bytes, id): (String, String, i64, Vec<u8>) = conn
        .query_row(
            "SELECT impact, detail, detail_bytes, id FROM revisions WHERE gen = ?1",
            [gen_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    let classes = conn
        .prepare(
            "SELECT class || ' ' || old_frame || ' ' || new_frame FROM revision_impacts
             WHERE rev = ?1 ORDER BY ord",
        )
        .unwrap()
        .query_map([id], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    (
        serde_json::from_str(&impact).unwrap(),
        classes,
        detail,
        bytes,
    )
}

#[test]
fn a_revision_lists_each_class_it_reached_charged_at_its_escaped_size() {
    use search::impact::Class::*;
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let tx = ws.conn.transaction().unwrap();
    // A quote and a control character are stored as their escapes in the entry.
    let hits = vec![
        hit("0:1", Direct),
        hit("0:2", Effective),
        hit("0:3", EffectiveUnchanged),
        hit("0:\"4\u{1}", Possible),
    ];
    let entries: usize = hits.iter().map(entry_bytes).sum();
    recorded(&tx, "s", 2, impacted(&[], hits));
    let (impact, classes, detail, bytes) = impact_of(&tx, 2);
    // The change row's estimate, each entry as a detail page serializes it with its stored
    // frames, and the summary.
    let row = 64 + "0:0".len() + "0:1".len() + "FRAME".len();
    assert_eq!(
        (impact.clone(), classes, detail, bytes),
        (
            json!({"complete": true, "reasons": [],
                "listed": {"direct": 1, "effective": 1, "effective_unchanged": 1, "possible": 1}}),
            // Each with its old and new frame, in that order.
            ["direct", "effective", "effective_unchanged", "possible"]
                .map(|c| format!("{c} 0:0 0:5"))
                .to_vec(),
            "complete".into(),
            (row + entries + impact.to_string().len()) as i64
        )
    );
}

#[test]
fn a_revision_and_its_impact_roll_back_together() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let tx = ws.conn.transaction().unwrap();
    let hits = vec![hit("0:1", search::impact::Class::Direct)];
    recorded(&tx, "s", 2, impacted(&[], hits));
    drop(tx);
    let count = |t: &str| -> i64 {
        ws.conn
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!((count("revisions"), count("revision_impacts")), (0, 0));
}

#[test]
fn an_impact_whose_escapes_pass_the_detail_budget_lists_nothing_and_keeps_its_reasons() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let tx = ws.conn.transaction().unwrap();
    // Its copy fits the budget; escaped six times over, its entry does not.
    let control = "\u{1}".repeat(search::diff::REVISION_BYTES / 2);
    let hits = vec![hit(&control, search::impact::Class::Possible)];
    recorded(
        &tx,
        "s",
        2,
        impacted(&[("source_comparison_partial", 1)], hits),
    );
    let (impact, classes, detail, _) = impact_of(&tx, 2);
    assert_eq!(
        (impact, classes.len(), detail),
        (
            json!({"complete": false, "listed": {}, "reasons": [
                {"reason": "impact_truncated", "count": 1},
                {"reason": "source_comparison_partial", "count": 1}]}),
            0,
            "truncated".into()
        )
    );
}

#[test]
fn change_rows_at_the_detail_budget_make_room_for_the_impact_summary() {
    // Eight rows the diff admits, filling the budget exactly.
    let node = "1".repeat(search::diff::REVISION_BYTES / 8 - 64 - "0:0".len() - "FRAME".len());
    let room = |reserve| {
        let mut rows: Vec<_> = (0..8)
            .map(|_| search::diff::Row {
                frame: "0:0".into(),
                node: node.clone(),
                kind: "FRAME".into(),
                change: "changed",
                facets: 1,
                descendants: 0,
            })
            .collect();
        let dropped = make_room(&mut rows, reserve);
        let used: usize = rows.iter().map(row_cost).sum();
        (
            rows.len(),
            dropped,
            used + reserve <= search::diff::REVISION_BYTES,
        )
    };
    assert_eq!((room(0), room(1)), ((8, false, true), (7, true, true)));
}

#[test]
fn an_impact_that_would_pass_the_disk_cap_is_left_out_as_truncated() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    ws.conn
        .execute_batch(&format!(
            "CREATE TABLE filler (b BLOB); INSERT INTO filler VALUES (zeroblob({DISK_CAP}));"
        ))
        .unwrap();
    let tx = ws.conn.transaction().unwrap();
    let hits = vec![
        hit("0:1", search::impact::Class::Direct),
        hit("0:2", search::impact::Class::Possible),
    ];
    recorded(&tx, "s", 2, impacted(&[], hits));
    let (impact, classes, detail, bytes) = impact_of(&tx, 2);
    assert_eq!(
        (impact.clone(), classes.len(), detail, bytes),
        (
            json!({"complete": false, "listed": {},
                "reasons": [{"reason": "impact_truncated", "count": 2}]}),
            0,
            "truncated".into(),
            impact.to_string().len() as i64
        )
    );
}

#[test]
fn an_expired_revision_drops_its_listed_impact_with_its_rows_and_keeps_the_summary() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let tx = ws.conn.transaction().unwrap();
    // Two revisions of about 700 bytes each: the older passes the 1 KiB detail retention.
    let long = "1".repeat(600);
    for gen_id in [2, 3] {
        let hits = vec![hit(&long, search::impact::Class::Possible)];
        recorded(&tx, "s", gen_id, impacted(&[], hits));
    }
    let (old, kept) = (impact_of(&tx, 2), impact_of(&tx, 3));
    assert_eq!(
        (old.0, old.1.len(), old.2, kept.1.len(), kept.2),
        (
            json!({"complete": true, "listed": {"possible": 1}, "reasons": []}),
            0,
            "expired".into(),
            1,
            "complete".into()
        )
    );
}

#[test]
fn a_detail_cursor_resumes_in_its_section_and_an_older_one_reads_change_rows() {
    let dir = TestDir::new();
    let (storage, select, _) = workspace(&dir, 1);
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let source = sources(&ws.conn)[0].clone();
    let tx = ws.conn.transaction().unwrap();
    let hits = ["0:2", "0:3"].map(|n| hit(n, search::impact::Class::Possible));
    recorded(&tx, &source, 2, impacted(&[], hits.to_vec()));
    let id: Vec<u8> = tx
        .query_row("SELECT id FROM revisions", [], |r| r.get(0))
        .unwrap();
    tx.commit().unwrap();
    let revision = crate::search::hex(&id);
    let read = |cursor: Option<&str>| {
        let req = crate::workspace::changes::Request {
            revision: Some(&revision),
            cursor,
            limit: Some(1),
            ..Default::default()
        };
        crate::workspace::changes::changes(&storage, &select, &req)
    };
    let ids = |page: &Json| {
        (
            page["nodes"][0]["entry"].clone(),
            page["nodes"][0]["id"].clone(),
        )
    };
    let first = read(None).unwrap();
    let second = read(first["nextCursor"].as_str()).unwrap();
    let third = read(second["nextCursor"].as_str()).unwrap();
    let signed = |tag: &str, payload: &[u8]| {
        let req = crate::workspace::changes::Request {
            revision: Some(&revision),
            ..Default::default()
        };
        crate::workspace::changes::sign(&storage, &select, &req, tag, payload)
    };
    // A cursor issued before impact names a change row: after row 0, the impact list follows.
    let d1 = signed("d1", &[&id[..], &0i64.to_be_bytes()].concat());
    let older = read(d1.as_str()).unwrap();
    let section = signed("d2", &[&id[..], &[2], &0i64.to_be_bytes()].concat());
    let other = signed("d2", &[&[0; 16][..], &[1], &0i64.to_be_bytes()].concat());
    let kind = |r: Result<Json>| {
        r.unwrap_err()
            .downcast_ref::<FigError>()
            .map(FigError::kind)
    };
    assert_eq!(
        (
            [ids(&first), ids(&second), ids(&third)],
            third["nextCursor"].clone(),
            ids(&older),
            kind(read(section.as_str())),
            kind(read(other.as_str()))
        ),
        (
            [
                (json!("change"), json!("0:1")),
                (json!("impact"), json!("0:2")),
                (json!("impact"), json!("0:3"))
            ],
            Json::Null,
            (json!("impact"), json!("0:2")),
            Some(ErrorKind::CursorStale),
            Some(ErrorKind::CursorStale)
        )
    );
}

#[test]
fn a_publication_evaluates_each_saved_search_once_in_bounded_turns() {
    use crate::workspace::changes::{evaluate_published, subscriptions};
    use std::sync::atomic::AtomicBool;
    let d = TestDir::new();
    let (storage, select, dir) = workspace(&d, 1);
    let me = owner(&dir).unwrap();
    let conn = connection(&dir);
    let source = sources(&conn)[0].clone();
    let build = |attempt| {
        // The fixture's own prepare job holds the scope; each build here is a fresh one.
        conn.execute("DELETE FROM jobs", []).unwrap();
        let job_id = job(&conn, &source, "0:0", "running", &me, attempt);
        let built = work(&dir, &job_id, &me, attempt).unwrap();
        let ws_id = job_id.split('.').next().unwrap();
        publish(&storage, ws_id, &job_id, &me, attempt, &built).unwrap();
    };
    build(1);
    let save = |consumer, query| {
        let req = subscriptions::Subscribe {
            consumer: Some(consumer),
            source: Some(&source),
            scope: None,
            node: None,
            owner: None,
            query: Some(query),
            types: Vec::new(),
            sources: None,
            library: None,
        };
        subscriptions::subscribe(&storage, &select, &req).unwrap();
    };
    // One more subscriber of badge than a write records, two spellings of one search by two
    // consumers, and one other search.
    let many: Vec<String> = (0..33).map(|i| format!("c{i:02}")).collect();
    for c in &many {
        save(c, "badge");
    }
    save("a", "First success");
    save("b", "first  SUCCESS");
    save("a", "card");
    let (go, stop) = (AtomicBool::new(false), AtomicBool::new(true));
    let turn = |from, cancel| {
        let t = evaluate_published(&storage, &select, &source, "0:0", from, cancel).unwrap();
        let n = t.report["subscriptions"].as_array().unwrap().len();
        (
            (t.report["computed"].as_u64().unwrap(), n, t.next.is_some()),
            t.next,
        )
    };
    // Turns from `from` until nothing is due, at most ten.
    let drain = |mut from| {
        let mut seen = Vec::new();
        while seen.len() < 10 {
            let (got, next) = turn(from, &go);
            seen.push(got);
            if next.is_none() {
                break;
            }
            from = next;
        }
        seen
    };
    // Every subscription is current after subscribe: nothing is due until a publication.
    let (idle, _) = turn(None, &go);
    // Same bytes publish no new generation here, so the evaluations are made stale instead, as
    // a publication they were not evaluated at leaves them.
    let stale = |ranking: &str| {
        conn.execute("UPDATE query_evaluations SET ranking = ?1", [ranking])
            .unwrap();
    };
    stale("older");
    let (cancelled, from) = turn(None, &stop);
    let (badge, from) = turn(from, &go);
    // A relocation between turns: the kept badge search belongs to another publication, so
    // the work restarts at the first search, which is badge again.
    conn.execute_batch(
        "UPDATE sources SET membership = 'moved'; UPDATE generations SET membership = 'moved'",
    )
    .unwrap();
    let rest = drain(from);
    let again = turn(None, &go).0;
    // A damaged search is noted once per publication, not rerun by every later discovery.
    conn.execute(
        "UPDATE subscriptions SET anchor = '{' WHERE anchor LIKE '%card%'",
        [],
    )
    .unwrap();
    stale("oldest");
    let damaged = drain(None);
    let after = turn(None, &go).0;
    let note: String = conn
        .query_row(
            "SELECT eval_failure ->> '$.evaluation.state' FROM subscriptions WHERE anchor = '{'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Searches in canonical order: badge in two writes from one search, card, first success.
    let order = vec![
        (1, 32, true),
        (0, 1, true),
        (1, 1, true),
        (1, 2, true),
        (0, 0, false),
    ];
    assert_eq!(
        (
            idle,
            cancelled,
            badge,
            rest,
            again,
            damaged,
            after,
            note.as_str()
        ),
        (
            (0, 0, false),
            (0, 0, true),
            (1, 32, true),
            order.clone(),
            (0, 0, false),
            order,
            (0, 0, false),
            "failed"
        )
    );
    // Behind another writer: a turn fails busy at once, and one cancelled while it waits
    // writes nothing; its continuation then records the rest without searching again.
    use std::sync::atomic::Ordering;
    stale("held");
    let (_, from) = turn(None, &go);
    let holder = connection(&dir);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let busy = evaluate_published(&storage, &select, &source, "0:0", from.clone(), &go)
        .err()
        .and_then(|e| e.downcast_ref::<FigError>().map(FigError::kind));
    let waited = started.elapsed();
    let flag = AtomicBool::new(false);
    let held = std::thread::scope(|s| {
        let turn = s.spawn(|| {
            evaluate_published(&storage, &select, &source, "0:0", from.clone(), &flag).unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        flag.store(true, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(50));
        holder.execute_batch("COMMIT").unwrap();
        let t = turn.join().unwrap();
        (
            t.report["subscriptions"].as_array().unwrap().len(),
            t.next.is_some(),
        )
    });
    let resumed = turn(from, &go).0;
    assert_eq!(
        (busy, waited < LOCK_WAIT, held, resumed),
        (
            Some(ErrorKind::WorkspaceBusy),
            true,
            (0, true),
            (0, 1, true)
        )
    );
    // A failure is noted only at the current publication of a visible source: one met before
    // a relocation never replaces the newer note, and a missing or hidden one is unknown.
    let sub: String = conn
        .query_row(
            "SELECT id FROM subscriptions WHERE consumer = 'c00'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (generation, membership): (i64, String) = conn
        .query_row(
            "SELECT p.gen, s.membership FROM published p JOIN sources s USING (source_id)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let ranking = crate::search::corpus_ranking().to_string();
    let noted = |storage: &Storage, membership: &str, reason: &str| {
        crate::workspace::changes::note(
            &conn,
            storage,
            &sub,
            (generation, membership, &ranking),
            &json!({"state": "unavailable", "reason": reason}),
        )
        .unwrap()
    };
    conn.execute_batch(
        "UPDATE sources SET membership = 'again'; UPDATE generations SET membership = 'again'",
    )
    .unwrap();
    let newer = noted(&storage, "again", "newer");
    let older = noted(&storage, &membership, "older");
    let gone = crate::workspace::changes::note(
        &conn,
        &storage,
        "missing",
        (generation, "again", &ranking),
        &json!({"state": "unavailable"}),
    )
    .is_err();
    let elsewhere = std::path::absolute(d.join("elsewhere")).unwrap();
    let hidden = Storage {
        visible: Some(vec![elsewhere]),
        ..storage
    };
    let unseen = crate::workspace::changes::note(
        &conn,
        &hidden,
        &sub,
        (generation, "again", &ranking),
        &json!({"state": "unavailable", "reason": "unseen"}),
    )
    .is_err();
    let kept: String = conn
        .query_row(
            "SELECT eval_failure ->> '$.evaluation.reason' FROM subscriptions WHERE id = ?1",
            [&sub],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        (newer, older, gone, unseen, kept.as_str()),
        (true, false, true, true, "newer")
    );
}

/// A workspace whose `n` identical sources each have a published build of `0:0` and one visible
/// record using a library component; the sources' paths, ids and generations.
fn library_workspace(
    d: &TestDir,
    n: usize,
) -> (Storage, Select, Connection, Vec<(PathBuf, String, i64)>) {
    let (storage, select, dir) = workspace(d, n);
    let conn = connection(&dir);
    let me = owner(&dir).unwrap();
    let mut out = Vec::new();
    // In the order the query pages them, which two sources added in different milliseconds can
    // make differ from source_id order.
    let ordered: Vec<String> = conn
        .prepare("SELECT source_id FROM sources ORDER BY added_at_ms, source_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for source in ordered {
        let job_id = job(&conn, &source, "0:0", "running", &me, 1);
        let built = work(&dir, &job_id, &me, 1).unwrap();
        publish(&storage, &select.id().unwrap(), &job_id, &me, 1, &built).unwrap();
        let gen_id = built["gen"].as_i64().unwrap();
        uses(&conn, gen_id, r#"["component","ns","k"]"#);
        let path: String = conn
            .query_row(
                "SELECT path FROM sources WHERE source_id = ?1",
                [&source],
                |r| r.get(0),
            )
            .unwrap();
        out.push((PathBuf::from(path), source, gen_id));
    }
    (storage, select, conn, out)
}

/// Adds a `uses_library` edge with `target` to the first visible record of `gen_id`.
fn uses(conn: &Connection, gen_id: i64, target: &str) {
    conn.execute(
        "INSERT INTO edges (gen, rid, relation, seq, target, state, basis, evidence)
         SELECT ?1, rid, 'uses_library', (SELECT count(*) FROM edges WHERE gen = ?1 AND
         rid = r.rid AND relation = 'uses_library'), ?2, 'external', 'cached_copy', '{\"copy\":\"0:9\"}'
         FROM records r WHERE gen = ?1 AND hidden = 0 ORDER BY rid LIMIT 1",
        params![gen_id, target],
    )
    .unwrap();
}

fn reference(key: &str, cursor: Option<&Json>) -> crate::workspace::library::Request {
    crate::workspace::library::Request {
        kind: "component".into(),
        namespace: Some("ns".into()),
        key: key.into(),
        definition: None,
        scope: "0:0".into(),
        include_hidden: false,
        limit: 1,
        cursor: cursor.and_then(Json::as_str).map(str::to_owned),
    }
}

fn library_ids(out: &Json) -> Vec<String> {
    out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"]["sourceId"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn library_uses_page_across_sources_and_never_resolve_the_definition() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    // The same key under another namespace, and a namespace whose join collides, are not uses.
    uses(&conn, srcs[0].2, r#"["component","other","k"]"#);
    uses(&conn, srcs[0].2, r#"["component","ns/k",""]"#);
    let first = library(&storage, &select, None, &reference("k", None)).unwrap();
    assert_eq!(library_ids(&first), [srcs[0].1.clone()]);
    // Without a selected definition nothing is compared or reported about one.
    let text = first.to_string();
    assert!(!text.contains("definitionMatch") && !text.contains("definition_"));
    assert_eq!(
        first["definition"],
        json!({"state": "unresolved", "reason": "library_source_unproven"})
    );
    assert_eq!(
        (
            first["hasMore"].clone(),
            first["coverage"]["complete"].clone()
        ),
        (json!(true), json!(true))
    );
    let second = library(
        &storage,
        &select,
        None,
        &reference("k", Some(&first["nextCursor"])),
    )
    .unwrap();
    assert_eq!(library_ids(&second), [srcs[1].1.clone()]);
    assert_eq!(second["nextCursor"], Json::Null);
    // The cursor binds the reference: another key cannot continue it.
    let other = library(
        &storage,
        &select,
        None,
        &reference("j", Some(&first["nextCursor"])),
    );
    assert_eq!(kind_of_error(other), ErrorKind::CursorStale);
    // One source narrows the reply to its own uses.
    let one = library(&storage, &select, Some(&srcs[1].1), &reference("k", None)).unwrap();
    assert_eq!(library_ids(&one), [srcs[1].1.clone()]);
}

#[test]
fn a_library_use_says_where_it_is_without_opening_the_file() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    // A layer inside a page, unlike the document the helper's use sits on.
    conn.execute(
        "UPDATE edges SET rid = (SELECT rid FROM records WHERE gen = ?1 AND hidden = 0
         AND page IS NOT NULL AND root IS NOT NULL ORDER BY rid LIMIT 1)
         WHERE gen = ?1 AND relation = 'uses_library'",
        [srcs[0].2],
    )
    .unwrap();
    let out = library(&storage, &select, None, &reference("k", None)).unwrap();
    let hit = &out["results"][0];
    assert_eq!(
        (
            hit["hidden"].is_boolean(),
            hit["page"]["id"].is_string(),
            hit["scopeRoot"].is_string(),
            out["coverage"]["sources"][0]["path"]
                .as_str()
                .map(PathBuf::from),
        ),
        (true, true, true, Some(srcs[0].0.clone()))
    );
}

/// Renames the record holding `gen_id`'s library use to `name`.
fn rename_use(conn: &Connection, gen_id: i64, name: &str) {
    conn.execute(
        "UPDATE records SET name = ?2 WHERE gen = ?1 AND rid IN
         (SELECT rid FROM edges WHERE gen = ?1 AND relation = 'uses_library')",
        params![gen_id, name],
    )
    .unwrap();
}

#[test]
fn a_first_library_use_too_large_for_a_page_is_refused_by_its_utf8_bytes() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    // 1.1M characters, 3.3 MB: within the page counted in characters, over it in bytes.
    rename_use(&conn, srcs[0].2, &"漢".repeat(1_100_000));
    let out = library(&storage, &select, None, &reference("k", None));
    assert_eq!(kind_of_error(out), ErrorKind::ResponseBudget);
}

#[test]
fn a_library_use_is_measured_as_it_is_sent_so_both_mcp_copies_fit() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    // 2 MB stored, 4 MB once each quote is escaped: past the page as sent.
    rename_use(&conn, srcs[0].2, &"\"".repeat(2_000_000));
    let out = library(&storage, &select, None, &reference("k", None));
    let error = out.unwrap_err();
    let error = error.downcast_ref::<FigError>().unwrap();
    let steps: Vec<_> = error
        .recovery()
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["action"].clone(), s["arguments"]["sourceId"].clone()))
        .collect();
    assert_eq!(
        (error.kind(), steps),
        (
            ErrorKind::ResponseBudget,
            vec![
                (json!("usages_source"), json!(srcs[0].1)),
                (json!("inspect_use"), Json::Null)
            ]
        )
    );
}

/// Replaces `gen_id`'s stored page list with `pages`, a JSON value.
fn set_pages(conn: &Connection, gen_id: i64, pages: &str) {
    conn.execute(
        "UPDATE generations SET info = json_set(info, '$.pages', json(?2)) WHERE gen = ?1",
        params![gen_id, pages],
    )
    .unwrap();
}

#[test]
fn a_source_without_a_use_never_reads_its_page_list() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    conn.execute("DELETE FROM edges WHERE relation = 'uses_library'", [])
        .unwrap();
    // A list no reader could parse: reading it at all would fail the question.
    set_pages(&conn, srcs[0].2, r#""not a list""#);
    let out = library(&storage, &select, None, &reference("k", None)).unwrap();
    assert_eq!(library_ids(&out), Vec::<String>::new());
}

#[test]
fn a_page_list_too_large_for_the_page_is_refused_before_it_is_read() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    let page = format!(r#"["0:1","{}",false]"#, "p".repeat(1024));
    let pages = format!("[{}]", vec![page; 3200].join(","));
    set_pages(&conn, srcs[0].2, &pages);
    let out = library(&storage, &select, None, &reference("k", None));
    assert_eq!(kind_of_error(out), ErrorKind::ResponseBudget);
}

#[test]
fn a_first_use_that_no_longer_fits_beside_its_page_list_is_refused_not_left_without_a_cursor() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    // The list fits the page, but leaves less room than the use needs.
    let page = format!(r#"["0:1","{}",false]"#, "p".repeat(1024));
    set_pages(
        &conn,
        srcs[0].2,
        &format!("[{}]", vec![page; 2900].join(",")),
    );
    rename_use(&conn, srcs[0].2, &"n".repeat(200_000));
    let out = library(&storage, &select, None, &reference("k", None));
    assert_eq!(kind_of_error(out), ErrorKind::ResponseBudget);
}

#[test]
fn a_library_source_names_its_freshness_or_an_unreadable_file_first() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, _conn, srcs) = library_workspace(&d, 1);
    let ready = library(&storage, &select, None, &reference("k", None)).unwrap();
    let freshness = &ready["coverage"]["sources"][0]["freshness"];
    // A scope never indexed for a file that is gone is unreadable, as search reports it.
    std::fs::remove_file(&srcs[0].0).unwrap();
    let request = crate::workspace::library::Request {
        scope: "0:1".into(),
        ..reference("k", None)
    };
    let gone = library(&storage, &select, None, &request).unwrap();
    assert_eq!(
        (
            freshness["verifiedAtMs"].is_i64(),
            freshness["stampCheckedAtMs"].is_i64(),
            gone["coverage"]["sources"][0]["state"].clone(),
        ),
        (true, true, json!("unreadable"))
    );
}

#[test]
fn a_library_question_needs_a_scope() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, _conn, _srcs) = library_workspace(&d, 1);
    let request = crate::workspace::library::Request {
        scope: String::new(),
        ..reference("k", None)
    };
    let out = library(&storage, &select, None, &request);
    assert_eq!(kind_of_error(out), ErrorKind::InvalidArguments);
}

#[test]
fn a_later_library_use_too_large_for_the_page_starts_the_next_one() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    rename_use(&conn, srcs[1].2, &"a".repeat(3 * 1024 * 1024 - 64));
    let request = crate::workspace::library::Request {
        limit: 2,
        ..reference("k", None)
    };
    let first = library(&storage, &select, None, &request).unwrap();
    assert_eq!(
        (library_ids(&first), first["nextCursor"].is_string()),
        (vec![srcs[0].1.clone()], true)
    );
}

#[test]
fn a_use_with_no_namespace_leaves_the_reply_partial_without_being_listed() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    conn.execute("DELETE FROM edges WHERE relation = 'uses_library'", [])
        .unwrap();
    uses(&conn, srcs[0].2, r#"["component",null,"k"]"#);
    let out = library(&storage, &select, None, &reference("k", None)).unwrap();
    assert_eq!(library_ids(&out), Vec::<String>::new());
    assert_eq!(out["coverage"]["complete"], false);
    assert_eq!(
        out["coverage"]["reasons"],
        json!([{"code": "namespace_unknown", "sourceIds": [srcs[0].1]}])
    );
}

#[test]
fn a_reference_too_long_to_keep_leaves_the_reply_partial() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 1);
    conn.execute(
        "UPDATE edges SET target = NULL, state = 'unsupported' WHERE relation = 'uses_library'",
        [],
    )
    .unwrap();
    let out = library(&storage, &select, None, &reference("k", None)).unwrap();
    assert_eq!(library_ids(&out), Vec::<String>::new());
    assert_eq!(
        out["coverage"]["reasons"],
        json!([{"code": "index_partial", "sourceIds": [srcs[0].1]}])
    );
}

#[test]
fn a_revoked_root_or_a_new_generation_ends_a_library_cursor() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let first = library(&storage, &select, None, &reference("k", None)).unwrap();
    let cursor = Some(&first["nextCursor"]);
    // Identical bytes, but only the first file is under the caller's roots now.
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let out = library(&narrow, &select, None, &reference("k", None)).unwrap();
    assert_eq!(library_ids(&out), [srcs[0].1.clone()]);
    assert_eq!(out["coverage"]["complete"], false);
    assert_eq!(
        out["coverage"]["reasons"],
        json!([{"code": "authorization_restricted"}])
    );
    assert_eq!(out["coverage"]["sources"].as_array().unwrap().len(), 1);
    assert_eq!(
        kind_of_error(library(&narrow, &select, None, &reference("k", cursor))),
        ErrorKind::CursorStale
    );
    // A changed filter cannot continue it either.
    let hidden = crate::workspace::library::Request {
        include_hidden: true,
        ..reference("k", cursor)
    };
    assert_eq!(
        kind_of_error(library(&storage, &select, None, &hidden)),
        ErrorKind::CursorStale
    );
    // The second source is republished at a new generation with the same rows.
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id, state,
         complete, records, info, created_ms) SELECT source_id, membership, scope, snapshot_id,
         versions, job_id, state, complete, records, info, created_ms FROM generations WHERE gen = ?1",
        [srcs[1].2],
    )
    .unwrap();
    let new = conn.last_insert_rowid();
    conn.execute(
        "UPDATE published SET gen = ?1 WHERE source_id = ?2",
        params![new, srcs[1].1],
    )
    .unwrap();
    assert_eq!(
        kind_of_error(library(&storage, &select, None, &reference("k", cursor))),
        ErrorKind::CursorStale
    );
    // Once the paged generation is collected, the same cursor has expired.
    for table in ["edges", "records", "generations"] {
        conn.execute(&format!("DELETE FROM {table} WHERE gen = ?1"), [srcs[1].2])
            .unwrap();
    }
    assert_eq!(
        kind_of_error(library(&storage, &select, None, &reference("k", cursor))),
        ErrorKind::CursorExpired
    );
}

fn kind_of_error(r: Result<Json>) -> ErrorKind {
    r.unwrap_err().downcast_ref::<FigError>().unwrap().kind()
}

/// Makes the first visible record of `gen_id` a component and returns its exact ref.
fn component_ref(
    conn: &Connection,
    source: &str,
    gen_id: i64,
) -> crate::workspace::library::DefinitionRef {
    let node_id: String = conn
        .query_row(
            "UPDATE records SET kind = 'COMPONENT' WHERE gen = ?1 AND rid = (SELECT min(rid)
             FROM records WHERE gen = ?1 AND hidden = 0) RETURNING node_id",
            [gen_id],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot_id = conn
        .query_row(
            "SELECT snapshot_id FROM generations WHERE gen = ?1",
            [gen_id],
            |r| r.get(0),
        )
        .unwrap();
    crate::workspace::library::DefinitionRef {
        source_id: source.into(),
        snapshot_id,
        node_id,
    }
}

#[test]
fn a_selected_definition_is_checked_on_its_exact_ref_and_never_resolved() {
    use crate::workspace::library::{DefinitionRef, Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    let ask = |storage: &Storage, source: Option<&str>, kind: &str, def: DefinitionRef| {
        let req = Request {
            kind: kind.into(),
            definition: Some(def),
            ..reference("k", None)
        };
        library(storage, &select, source, &req).unwrap()
    };
    let with = |f: fn(&mut DefinitionRef)| {
        let mut d = def.clone();
        f(&mut d);
        d
    };
    let selected = ask(&storage, None, "component", def.clone());
    assert_eq!(
        selected["definition"],
        json!({"state": "selected", "basis": "caller_ref", "keyEvidence": "unverified",
            "ref": {"sourceId": def.source_id, "snapshotId": def.snapshot_id, "nodeId": def.node_id}})
    );
    // Narrowed to another source, the definition's own source is still checked.
    let narrowed = ask(&storage, Some(&srcs[0].1), "component", def.clone());
    assert_eq!(
        (library_ids(&narrowed), &narrowed["definition"]["state"]),
        (vec![srcs[0].1.clone()], &json!("selected"))
    );
    // Only the first file is under the caller's roots now: the second is neither named nor read,
    // and reads as an unknown source, so its existence is not told either.
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let hidden = ask(&narrow, None, "component", def.clone());
    let text = hidden.to_string();
    assert!(!text.contains(&srcs[1].1) && !text.contains(srcs[1].0.to_str().unwrap()));
    assert!(!text.contains("definitionMatch") && !text.contains("definition_not_compared"));
    let reasons: Vec<Json> = [
        ask(&storage, None, "variable", def.clone()),
        ask(
            &storage,
            None,
            "component",
            with(|d| d.snapshot_id = "sha256:old".into()),
        ),
        ask(
            &storage,
            None,
            "component",
            with(|d| d.node_id = "9:9".into()),
        ),
        ask(
            &storage,
            None,
            "component",
            with(|d| d.source_id = "src_none".into()),
        ),
        hidden,
    ]
    .iter()
    .map(|out| out["definition"].clone())
    .collect();
    let unresolved = |r: &str| json!({"state": "unresolved", "reason": r});
    assert_eq!(
        reasons,
        [
            unresolved("definition_kind_mismatch"),
            unresolved("definition_snapshot_stale"),
            unresolved("definition_not_found"),
            unresolved("definition_not_found"),
            unresolved("definition_not_found"),
        ]
    );
    // A changed ref cannot continue a cursor paged under another one.
    let paged = ask(&storage, None, "component", def.clone());
    let other = Request {
        definition: Some(with(|d| d.node_id = "9:9".into())),
        ..reference("k", Some(&paged["nextCursor"]))
    };
    assert_eq!(
        kind_of_error(library(&storage, &select, None, &other)),
        ErrorKind::CursorStale
    );
    // A variable is checked as one; a style adds that its kind is unverified.
    conn.execute(
        "UPDATE records SET kind = 'VARIABLE' WHERE gen = ?1 AND node_id = ?2",
        params![srcs[1].2, def.node_id],
    )
    .unwrap();
    let kinds: Vec<Json> = ["variable", "style"]
        .iter()
        .map(|k| ask(&storage, None, k, def.clone())["definition"].clone())
        .collect();
    assert_eq!(
        (&kinds[0]["state"], &kinds[1]["kindEvidence"]),
        (&json!("selected"), &json!("unverified"))
    );
}

#[test]
fn a_key_only_library_question_lists_copies_naming_no_namespace_and_is_never_complete() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    uses(&conn, srcs[1].2, r#"["component",null,"k"]"#);
    let key_only = Request {
        namespace: None,
        limit: 10,
        ..reference("k", None)
    };
    let out = library(&storage, &select, None, &key_only).unwrap();
    assert_eq!(
        (
            library_ids(&out),
            &out["definition"],
            &out["coverage"]["complete"],
            &out["coverage"]["reasons"]
        ),
        (
            vec![srcs[1].1.clone()],
            &json!({"state": "unresolved", "reason": "namespace_required"}),
            &json!(false),
            &json!([{"code": "namespace_required"}])
        )
    );
}

#[test]
fn a_library_cursor_past_its_lease_is_expired_before_it_is_stale() {
    use crate::workspace::library::library;
    let d = TestDir::new();
    let (storage, select, _conn, _) = library_workspace(&d, 2);
    let first = library(&storage, &select, None, &reference("k", None)).unwrap();
    let cursor = first["nextCursor"].as_str().expect("a second page");
    // The issue time is the cursor's last field; rewrite it instead of sleeping.
    let (rest, issued) = cursor.rsplit_once('.').unwrap();
    let issued: i64 = issued.parse().unwrap();
    let at = |ms: i64| format!("{rest}.{ms}");
    let page = |c: &str, hidden: bool| {
        let request = crate::workspace::library::Request {
            include_hidden: hidden,
            cursor: Some(c.to_owned()),
            ..reference("k", None)
        };
        library(&storage, &select, None, &request)
    };
    let fresh = at(issued - crate::search::CURSOR_LEASE_MS + 1000);
    assert!(page(&fresh, false).is_ok());
    let old = at(issued - crate::search::CURSOR_LEASE_MS - 1000);
    assert_eq!(kind_of_error(page(&old, false)), ErrorKind::CursorExpired);
    // The lease is checked before the bindings.
    assert_eq!(kind_of_error(page(&old, true)), ErrorKind::CursorExpired);
    assert_eq!(kind_of_error(page(&fresh, true)), ErrorKind::CursorStale);
}

#[test]
fn a_cursor_ends_when_the_selected_definition_s_source_is_reindexed_or_collected() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    // Two uses in the first source, so its page of one has a cursor; the definition is the
    // second source's, whose uses this narrowed question does not list.
    uses(&conn, srcs[0].2, r#"["component","ns","k"]"#);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    let ask = |cursor: Option<&Json>| Request {
        definition: Some(def.clone()),
        ..reference("k", cursor)
    };
    let first = library(&storage, &select, Some(&srcs[0].1), &ask(None)).unwrap();
    let cursor = Some(&first["nextCursor"]);
    assert!(
        library(&storage, &select, Some(&srcs[0].1), &ask(cursor)).is_ok(),
        "{first}"
    );
    // The definition's source is republished at a new generation with the same records.
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id, state,
         complete, records, info, created_ms) SELECT source_id, membership, scope, snapshot_id,
         versions, job_id, state, complete, records, info, created_ms FROM generations WHERE gen = ?1",
        [srcs[1].2],
    )
    .unwrap();
    let new = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO records SELECT ?1, rid, node_id, name, kind, text, norm0, norm1, len0, len1,
         parent, source, text_source, hidden, page, root FROM records WHERE gen = ?2",
        params![new, srcs[1].2],
    )
    .unwrap();
    conn.execute(
        "UPDATE published SET gen = ?1 WHERE source_id = ?2",
        params![new, srcs[1].1],
    )
    .unwrap();
    let again = || library(&storage, &select, Some(&srcs[0].1), &ask(cursor));
    assert_eq!(kind_of_error(again()), ErrorKind::CursorStale);
    for table in ["edges", "records", "generations"] {
        conn.execute(&format!("DELETE FROM {table} WHERE gen = ?1"), [srcs[1].2])
            .unwrap();
    }
    assert_eq!(kind_of_error(again()), ErrorKind::CursorExpired);
}

/// Sets the fingerprint of `node` in `gen_id` to `digest` on every facet, as a component.
fn fingerprinted(conn: &Connection, gen_id: i64, node: &str, digest: &[u8]) {
    conn.execute(
        "UPDATE records SET kind = 'COMPONENT' WHERE gen = ?1 AND node_id = ?2",
        params![gen_id, node],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO fingerprints (gen, rid, facet, state, digest)
         SELECT ?1, rid, f.value, 'read', ?3 FROM records,
         json_each('[\"text\",\"layout\",\"appearance\",\"properties\"]') f
         WHERE gen = ?1 AND node_id = ?2",
        params![gen_id, node, digest],
    )
    .unwrap();
}

fn matches(out: &Json) -> Vec<&Json> {
    let results = out["results"].as_array().unwrap();
    results.iter().map(|r| &r["definitionMatch"]).collect()
}

#[test]
fn each_use_compares_its_cached_copy_with_the_selected_definition() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    // Each source's use names a cached copy at the definition's node id, fingerprinted alike.
    conn.execute(
        "UPDATE edges SET evidence = json_object('copy', ?1) WHERE relation = 'uses_library'",
        [&def.node_id],
    )
    .unwrap();
    for (_, _, gen_id) in &srcs {
        fingerprinted(&conn, *gen_id, &def.node_id, &[1; 32]);
    }
    let ask = Request {
        definition: Some(def.clone()),
        limit: 10,
        ..reference("k", None)
    };
    let same = library(&storage, &select, None, &ask).unwrap();
    assert_eq!(
        (matches(&same), &same["coverage"]["complete"]),
        (vec![&json!("equal"); 2], &json!(true))
    );
    // The first source's copy now differs: a finding, not incompleteness.
    fingerprinted(&conn, srcs[0].2, &def.node_id, &[2; 32]);
    let differ = library(&storage, &select, None, &ask).unwrap();
    assert_eq!(
        (
            matches(&differ),
            &differ["coverage"]["reasons"],
            &differ["coverage"]["complete"]
        ),
        (
            vec![&json!("definition_mismatch"), &json!("equal")],
            &json!([{"code": "definition_mismatch", "count": 1}]),
            &json!(true)
        )
    );
}

#[test]
fn an_oversized_tampered_copy_kind_is_refused_before_it_is_copied() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    conn.execute(
        "UPDATE edges SET evidence = json_object('copy', ?1) WHERE relation = 'uses_library'",
        [&def.node_id],
    )
    .unwrap();
    for (_, _, gen_id) in &srcs {
        fingerprinted(&conn, *gen_id, &def.node_id, &[1; 32]);
    }
    conn.execute(
        "UPDATE records SET kind = ?3 WHERE gen = ?1 AND node_id = ?2",
        params![srcs[0].2, def.node_id, "C".repeat(1 << 20)],
    )
    .unwrap();
    let ask = Request {
        definition: Some(def),
        limit: 10,
        ..reference("k", None)
    };
    let err = library(&storage, &select, None, &ask)
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid stored index"), "{err}");
}

#[test]
fn a_schema_7_workspace_is_refused_read_only_with_a_rebuild_not_misread() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    // As the release before fingerprints left it: no such table, which a read never adds.
    conn.execute_batch("DROP TABLE fingerprints; PRAGMA user_version = 7;")
        .unwrap();
    let ask = Request {
        definition: Some(def.clone()),
        limit: 10,
        ..reference("k", None)
    };
    let err = library(&storage, &select, None, &ask)
        .unwrap_err()
        .downcast::<FigError>()
        .unwrap();
    assert_eq!(
        (err.kind(), &err.recovery().unwrap()[0]["action"]),
        (ErrorKind::IndexIncompatible, &json!("rebuild"))
    );
}

#[test]
fn a_definition_its_build_did_not_fingerprint_compares_no_use_and_asks_to_index_it() {
    use crate::workspace::library::{Request, library};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    conn.execute("DELETE FROM fingerprints WHERE gen = ?1", [srcs[1].2])
        .unwrap();
    let ask = Request {
        definition: Some(def.clone()),
        limit: 10,
        ..reference("k", None)
    };
    let out = library(&storage, &select, None, &ask).unwrap();
    let ws = select.id().unwrap();
    assert_eq!(
        (
            matches(&out),
            &out["coverage"]["reasons"],
            &out["coverage"]["complete"]
        ),
        (
            vec![&json!("unknown"); 2],
            &json!([{"code": "definition_not_compared", "count": 2,
                "nextSteps": [{"tool": "index", "arguments": {"action": "refresh",
                    "workspaceId": ws, "sourceId": def.source_id, "scope": "0:0"}}]}]),
            &json!(false)
        )
    ); // A build its budgets stopped, or one with fingerprints but none for this node, would
    // not gain one by rebuilding.
    conn.execute(
        "UPDATE generations SET stop = 'work_limit' WHERE gen = ?1",
        [srcs[1].2],
    )
    .unwrap();
    let stopped = library(&storage, &select, None, &ask).unwrap();
    conn.execute(
        "UPDATE generations SET stop = NULL WHERE gen = ?1",
        [srcs[1].2],
    )
    .unwrap();
    // Reported partial by its stop, after which the definition's reason offers no refresh.
    assert_eq!(
        stopped["coverage"]["reasons"][1],
        json!({"code": "definition_not_compared", "count": 2})
    );
    conn.execute(
        "INSERT INTO fingerprints (gen, rid, facet, state) VALUES (?1, -1, 'text', 'unread')",
        [srcs[1].2],
    )
    .unwrap();
    let out = library(&storage, &select, None, &ask).unwrap();
    assert_eq!(
        out["coverage"]["reasons"],
        json!([{"code": "definition_not_compared", "count": 2}])
    );
}

/// Saves `query` (or `library`) over `sources` for `consumer`, replying as subscribe does.
fn subscribe_set(
    storage: &Storage,
    select: &Select,
    consumer: &str,
    sources: &str,
    query: Option<&str>,
    library: Option<crate::workspace::library::Request>,
) -> Result<Json> {
    crate::workspace::changes::subscriptions::subscribe(
        storage,
        select,
        &crate::workspace::changes::subscriptions::Subscribe {
            consumer: Some(consumer),
            source: None,
            scope: None,
            node: None,
            owner: None,
            query,
            types: Vec::new(),
            sources: Some(sources),
            library,
        },
    )
}

/// The newest stored evaluations of `sub`, as changes pages them.
fn evaluations(storage: &Storage, select: &Select, sub: &str) -> Json {
    let req = crate::workspace::changes::Request {
        subscription: Some(sub),
        limit: Some(50),
        ..Default::default()
    };
    crate::workspace::changes::changes(storage, select, &req).unwrap()
}

/// One runner turn on `source`'s `0:0`: (searches computed, subscriptions recorded) and their
/// evaluations.
fn runner(storage: &Storage, select: &Select, source: &str) -> (u64, Vec<Json>) {
    let go = std::sync::atomic::AtomicBool::new(false);
    let t =
        crate::workspace::changes::evaluate_published(storage, select, source, "0:0", None, &go)
            .unwrap();
    let states = t.report["subscriptions"].as_array().unwrap();
    (
        t.report["computed"].as_u64().unwrap(),
        states.iter().map(|s| s["evaluation"].clone()).collect(),
    )
}

#[test]
fn a_saved_search_over_a_source_set_is_fixed_when_subscribed_and_evaluated_by_each_member() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let mut ids: Vec<String> = srcs.iter().map(|s| s.1.clone()).collect();
    ids.sort();
    let saved = subscribe_set(
        &storage,
        &select,
        "a",
        "visible",
        Some("First success"),
        None,
    )
    .unwrap();
    let sub = saved["subscription"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned();
    // The same set named in another order and twice is the same subscription.
    let listed = format!("{},{},{}", ids[1], ids[0], ids[1]);
    let again =
        subscribe_set(&storage, &select, "a", &listed, Some("First success"), None).unwrap();
    let first = &evaluations(&storage, &select, &sub)["evaluations"][0];
    let entered = first["delta"]["enteredTopK"].as_array().unwrap();
    let hit_sources: BTreeSet<&str> = entered
        .iter()
        .map(|r| r["sourceId"].as_str().unwrap())
        .collect();
    // A source added later is never enrolled.
    let late = d.join("project").join("late.fig");
    std::fs::copy(FIXTURE, &late).unwrap();
    crate::workspace::registry::add(&storage, &select, &late).unwrap();
    let held =
        crate::workspace::changes::subscriptions::list(&storage, &select, Some("a"), None, None)
            .unwrap();
    assert_eq!(
        (
            &saved["subscription"]["sources"],
            &saved["subscription"]["sourceId"],
            &saved["evaluation"]["causes"],
            again["created"].clone(),
            hit_sources.len(),
            first["sources"].as_array().unwrap().len(),
            &held["subscriptions"][0]["sources"],
        ),
        (
            &json!(ids),
            &Json::Null,
            &json!(["initial"]),
            json!(false),
            2,
            2,
            &json!(ids),
        )
    );
    // Relocating one member is a corpus change seen only by that member's runner turn.
    conn.execute_batch(&format!(
        "UPDATE sources SET membership = 'moved' WHERE source_id = '{0}';
         UPDATE generations SET membership = 'moved' WHERE source_id = '{0}'",
        ids[0]
    ))
    .unwrap();
    let other = runner(&storage, &select, &ids[1]);
    let moved = runner(&storage, &select, &ids[0]);
    assert_eq!(
        (other.0, moved.0, &moved.1[0]["causes"]),
        (0, 1, &json!(["corpus_changed"]))
    );
}

#[test]
fn a_source_set_names_only_visible_sources_and_never_the_sentinel() {
    let d = TestDir::new();
    let (storage, select, _conn, srcs) = library_workspace(&d, 2);
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let kind = |r: Result<Json>| r.unwrap_err().downcast_ref::<FigError>().unwrap().kind();
    let hidden = format!("{},{}", srcs[0].1, srcs[1].1);
    let refused = [
        subscribe_set(&storage, &select, "a", "*", Some("badge"), None),
        subscribe_set(&narrow, &select, "a", &hidden, Some("badge"), None),
    ]
    .map(kind);
    // Visible is resolved to what this caller sees now, nothing more.
    let mine = subscribe_set(&narrow, &select, "a", "visible", Some("badge"), None).unwrap();
    assert_eq!(
        (refused, &mine["subscription"]["sources"]),
        (
            [ErrorKind::InvalidArguments; 2],
            &json!([srcs[0].1.clone()])
        )
    );
}

/// Publishes a copy of `gen_id` under snapshot `snapshot`, as a build of new bytes would; its
/// records, edges and fingerprints are the old ones until a test changes them.
fn republish(conn: &Connection, gen_id: i64, snapshot: &str) -> i64 {
    conn.execute(
        "INSERT INTO snapshots SELECT n.source_id, ?2, n.bytes, n.mtime_ns, n.verified_ms
         FROM generations g JOIN snapshots n ON n.source_id = g.source_id
         AND n.snapshot_id = g.snapshot_id WHERE g.gen = ?1",
        params![gen_id, snapshot],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id,
         state, complete, records, info, created_ms) SELECT source_id, membership, scope, ?2,
         versions, job_id, state, complete, records, info, created_ms FROM generations WHERE gen = ?1",
        params![gen_id, snapshot],
    )
    .unwrap();
    let new = conn.last_insert_rowid();
    for table in ["records", "edges", "fingerprints"] {
        let columns: Vec<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{table}') WHERE name <> 'gen'"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let columns = columns.join(", ");
        conn.execute(
            &format!("INSERT INTO {table} (gen, {columns}) SELECT ?1, {columns} FROM {table} WHERE gen = ?2"),
            params![new, gen_id],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE published SET gen = ?1 WHERE gen = ?2",
        params![new, gen_id],
    )
    .unwrap();
    new
}

/// The library question of key `k` with the definition `def`, as a subscription saves it.
fn saved_library(
    def: crate::workspace::library::DefinitionRef,
) -> crate::workspace::library::Request {
    crate::workspace::library::Request {
        definition: Some(def),
        limit: 50,
        ..reference("k", None)
    }
}

/// A -> B -> C over `srcs` (three sources): A consumes library B's component k, whose definition
/// itself uses library C's component c, so C's change reaches A only through B. Returns B's
/// definition.
fn transitive_chain(
    conn: &Connection,
    srcs: &[(PathBuf, String, i64)],
) -> crate::workspace::library::DefinitionRef {
    let [(_, _, a_gen), (_, b, b_gen), (_, _, c_gen)] = [0, 1, 2].map(|i| srcs[i].clone());
    conn.execute(
        "DELETE FROM edges WHERE relation = 'uses_library' AND gen IN (?1, ?2)",
        [b_gen, c_gen],
    )
    .unwrap();
    uses(conn, b_gen, r#"["component","ns","c"]"#);
    let def = component_ref(conn, &b, b_gen);
    conn.execute(
        "UPDATE edges SET evidence = json_object('copy', ?1) WHERE gen = ?2",
        params![def.node_id, a_gen],
    )
    .unwrap();
    fingerprinted(conn, a_gen, &def.node_id, &[1; 32]);
    fingerprinted(conn, b_gen, &def.node_id, &[1; 32]);
    def
}

#[test]
fn a_saved_library_question_reports_definition_moves_and_never_follows_a_transitive_change() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 3);
    let def = transitive_chain(&conn, &srcs);
    let [(_, a, a_gen), (_, b, b_gen), (_, c, c_gen)] = [0, 1, 2].map(|i| srcs[i].clone());
    let saved = subscribe_set(
        &storage,
        &select,
        "a",
        "visible",
        None,
        Some(saved_library(def.clone())),
    )
    .unwrap();
    let sub = saved["subscription"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let newest = || {
        let page = evaluations(&storage, &select, &sub);
        page["evaluations"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone()
    };
    let initial = newest();
    // C's new bytes reach A only through B's copy, which this question never follows.
    republish(&conn, c_gen, "sha256:c2");
    let after_c = (runner(&storage, &select, &c).0, newest());
    // B's definition changes: A's cached copy no longer matches it.
    let b2 = republish(&conn, b_gen, "sha256:b2");
    fingerprinted(&conn, b2, &def.node_id, &[2; 32]);
    runner(&storage, &select, &b);
    let after_b = newest();
    // A overrides its use, which never masks the definition mismatch of its cached copy.
    let a2 = republish(&conn, a_gen, "sha256:a2");
    conn.execute(
        "UPDATE records SET name = 'Overridden' WHERE gen = ?1 AND rid = (SELECT rid FROM edges
         WHERE gen = ?1 AND relation = 'uses_library')",
        [a2],
    )
    .unwrap();
    runner(&storage, &select, &a);
    let masked = newest();
    // A drops the use: it leaves, named in A's old snapshot.
    let a3 = republish(&conn, a2, "sha256:a3");
    conn.execute("DELETE FROM edges WHERE gen = ?1", [a3])
        .unwrap();
    runner(&storage, &select, &a);
    let dropped = newest();
    let node = |e: &Json, k: &str| e["delta"][k].clone();
    assert_eq!(
        (
            &initial["coverage"]["transitive"],
            &initial["coverage"]["definition"]["state"],
            initial["delta"]["enteredTopK"][0]["sourceId"].as_str(),
        ),
        (&json!("not_followed"), &json!("selected"), Some(a.as_str()))
    );
    assert_eq!(
        (
            after_c.0,
            &after_c.1["causes"],
            node(&after_c.1, "changedSourceIds"),
            node(&after_c.1, "definitionMatchChanged"),
            &after_c.1["coverage"]["transitive"],
            // C holds no use and no definition: what its change feeds was not followed.
            node(&after_c.1, "notFollowedSourceIds"),
        ),
        (
            1,
            &json!(["source_changed"]),
            json!([c]),
            json!([]),
            &json!("not_followed"),
            json!([c]),
        )
    );
    assert_eq!(
        (
            node(&after_b, "changedSourceIds"),
            node(&after_b, "definitionMatchChanged"),
            node(&after_b, "notFollowedSourceIds"),
        ),
        (
            json!([b]),
            json!([{"sourceId": a, "nodeId": def.node_id, "from": "equal", "to": "definition_mismatch"}]),
            json!([]),
        )
    );
    assert_eq!(
        (
            node(&masked, "changedSourceIds"),
            node(&masked, "definitionMatchChanged"),
            masked["coverage"]["reasons"]
                .as_array()
                .unwrap()
                .contains(&json!("definition_mismatch")),
            node(&masked, "notFollowedSourceIds"),
        ),
        (json!([a]), json!([]), true, json!([]))
    );
    assert_eq!(
        node(&dropped, "leftTopK"),
        json!([{"sourceId": a, "snapshotId": "sha256:a2", "nodeId": def.node_id}])
    );
}

/// The newest stored evaluation of `sub` as `storage` reads it, and the whole reply.
fn newest(storage: &Storage, select: &Select, sub: &str) -> (Json, String) {
    let page = evaluations(storage, select, sub);
    let last = page["evaluations"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    (last, page.to_string())
}

fn subscription_id(reply: &Json) -> String {
    reply["subscription"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_not_followed_member_outside_a_readers_roots_is_never_shown_to_that_reader() {
    use crate::workspace::changes::{self, subscriptions};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 3);
    let def = transitive_chain(&conn, &srcs);
    let (c, c_gen) = (srcs[2].1.clone(), srcs[2].2);
    // C is registered, but outside the narrower caller's roots.
    let narrow = storage
        .clone()
        .confined(&[srcs[0].0.clone(), srcs[1].0.clone()]);
    let ask = || Some(saved_library(def.clone()));
    let wide =
        subscription_id(&subscribe_set(&storage, &select, "w", "visible", None, ask()).unwrap());
    let mine =
        subscription_id(&subscribe_set(&narrow, &select, "n", "visible", None, ask()).unwrap());
    republish(&conn, c_gen, "sha256:c2");
    // Only the set holding C is due; the wider reader is shown C as not followed.
    let turned = runner(&storage, &select, &c);
    let shown = newest(&storage, &select, &wide).0["delta"]["notFollowedSourceIds"].clone();
    let req = changes::Request {
        subscription: Some(&wide),
        ..Default::default()
    };
    let refused = [
        changes::changes(&narrow, &select, &req)
            .unwrap_err()
            .to_string(),
        changes::evaluate(&narrow, &select, Some(&wide))
            .unwrap_err()
            .to_string(),
        subscriptions::unsubscribe(&narrow, &select, Some(&wide), None, None)
            .unwrap_err()
            .to_string(),
    ];
    let listed = subscriptions::list(&narrow, &select, None, None, None).unwrap();
    let (own, page) = newest(&narrow, &select, &mine);
    let text = format!("{refused:?}{listed}{page}");
    assert_eq!(
        (
            (turned.0, turned.1.len()),
            shown,
            listed["subscriptions"].as_array().unwrap().len(),
            &own["causes"],
            &own["coverage"]["transitive"],
            text.contains(&c),
        ),
        (
            (1, 1),
            json!([c]),
            1,
            &json!(["initial"]),
            &json!("not_followed"),
            false
        )
    );
}

#[test]
fn a_visible_source_outside_the_set_reaches_no_evaluation_and_the_gap_stays_marked() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 3);
    let def = transitive_chain(&conn, &srcs);
    let (c, c_gen) = (srcs[2].1.clone(), srcs[2].2);
    let set = format!("{},{}", srcs[0].1, srcs[1].1);
    let sub = subscription_id(
        &subscribe_set(&storage, &select, "a", &set, None, Some(saved_library(def))).unwrap(),
    );
    republish(&conn, c_gen, "sha256:c2");
    let turned = runner(&storage, &select, &c).0;
    let (last, page) = newest(&storage, &select, &sub);
    assert_eq!(
        (
            turned,
            &last["causes"],
            &last["coverage"]["transitive"],
            &last["delta"]["notFollowedSourceIds"],
            page.contains(&c),
        ),
        (
            0,
            &json!(["initial"]),
            &json!("not_followed"),
            &json!([]),
            false
        )
    );
}

#[test]
fn a_missing_not_followed_member_leaves_no_stored_id_for_a_reader_who_never_saw_it() {
    use crate::workspace::changes::{self, subscriptions};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 3);
    let def = transitive_chain(&conn, &srcs);
    let (c_path, c, c_gen) = srcs[2].clone();
    let rest = storage
        .clone()
        .confined(&[srcs[0].0.clone(), srcs[1].0.clone()]);
    let sub = subscription_id(
        &subscribe_set(
            &storage,
            &select,
            "a",
            "visible",
            None,
            Some(saved_library(def)),
        )
        .unwrap(),
    );
    republish(&conn, c_gen, "sha256:c2");
    runner(&storage, &select, &c);
    let req = changes::Request {
        subscription: Some(&sub),
        ..Default::default()
    };
    // Unreadable: C's file is gone but C stays registered, so the set stays whole and hidden.
    std::fs::remove_file(&c_path).unwrap();
    let unreadable = (
        evaluations(&storage, &select, &sub)["evaluation"]["state"].clone(),
        changes::changes(&rest, &select, &req).is_err(),
    );
    // Removed: the set is disabled and drops every evaluation that named C before a reader who
    // sees the remaining members can read it.
    crate::workspace::registry::remove(&storage, &select, &c).unwrap();
    let stored: i64 = conn
        .query_row(
            "SELECT count(*) FROM query_evaluations WHERE sub = ?1",
            [&sub],
            |r| r.get(0),
        )
        .unwrap();
    let listed = subscriptions::list(&rest, &select, None, None, None).unwrap();
    let page = changes::changes(&rest, &select, &req).unwrap();
    let evaluated = changes::evaluate(&rest, &select, Some(&sub)).unwrap();
    assert_eq!(
        (
            unreadable,
            stored,
            &page["evaluation"]["state"],
            &page["subscription"]["disabled"],
            format!("{listed}{page}{evaluated}").contains(&c),
        ),
        (
            (json!("current_for_index"), true),
            0,
            &json!("disabled"),
            &json!({"reason": "source_removed"}),
            false
        )
    );
}

#[test]
fn list_by_subscription_reads_only_that_row_under_the_set_visibility_rule() {
    use crate::workspace::changes::subscriptions;
    let d = TestDir::new();
    let (storage, select, _conn, srcs) = library_workspace(&d, 2);
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let one = subscription_id(
        &subscribe_set(&storage, &select, "a", &srcs[0].1, Some("badge"), None).unwrap(),
    );
    let set = subscription_id(
        &subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap(),
    );
    let only = |storage: &Storage, id: &str| {
        subscriptions::list(storage, &select, None, None, Some(id)).unwrap()["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["subscriptionId"].clone(), s["sources"].clone()))
            .collect::<Vec<_>>()
    };
    let mut ids = vec![srcs[0].1.clone(), srcs[1].1.clone()];
    ids.sort();
    assert_eq!(
        [
            only(&narrow, &one),
            only(&storage, &set),
            only(&narrow, &set),
            only(&storage, "sub_unknown"),
        ],
        [
            vec![(json!(one), json!([srcs[0].1]))],
            vec![(json!(set), json!(ids))],
            vec![],
            vec![],
        ]
    );
}

#[test]
fn a_set_is_unknown_to_a_narrower_caller_and_disabled_only_when_a_member_is_removed() {
    use crate::workspace::changes::{self, subscriptions};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    // The definition's source is outside the narrower caller's set, which reads as unknown.
    let hidden_definition = subscribe_set(
        &narrow,
        &select,
        "n",
        "visible",
        None,
        Some(saved_library(def.clone())),
    )
    .unwrap_err()
    .to_string();
    let saved = subscribe_set(
        &storage,
        &select,
        "a",
        "visible",
        None,
        Some(saved_library(def.clone())),
    )
    .unwrap();
    let sub = saved["subscription"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned();
    // A narrower caller's reads see nothing of it and change nothing.
    let req = changes::Request {
        subscription: Some(&sub),
        ..Default::default()
    };
    let unseen = (
        changes::changes(&narrow, &select, &req).is_err(),
        subscriptions::list(&narrow, &select, None, None, None).unwrap()["subscriptions"].clone(),
        changes::evaluate(&narrow, &select, Some(&sub)).is_err(),
    );
    let stored = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let kept = stored("SELECT count(*) FROM query_evaluations")
        + stored("SELECT count(*) FROM subscriptions WHERE disabled IS NOT NULL");
    // Removing a member disables the set and deletes its evaluations; no read revives it.
    crate::workspace::registry::remove(&storage, &select, &srcs[1].1).unwrap();
    let page = changes::changes(&storage, &select, &req).unwrap();
    let evaluated = changes::evaluate(&storage, &select, Some(&sub)).unwrap();
    let left = stored("SELECT count(*) FROM query_evaluations");
    let removed = subscriptions::unsubscribe(&storage, &select, Some(&sub), None, None).unwrap();
    assert!(
        hidden_definition.contains("definition_not_found")
            && !hidden_definition.contains(&srcs[1].1),
        "{hidden_definition}"
    );
    assert_eq!(
        (
            unseen,
            kept,
            &page["evaluations"],
            &page["evaluation"]["state"],
            &page["subscription"]["disabled"],
            &evaluated["evaluation"]["state"],
            left,
            &removed["removed"],
        ),
        (
            (true, json!([]), true),
            1,
            &json!([]),
            &json!("disabled"),
            &json!({"reason": "source_removed"}),
            &json!("disabled"),
            0,
            &json!(1),
        )
    );
}

#[test]
fn a_disabled_set_names_no_removed_member_and_is_seen_by_whoever_sees_every_remaining_one() {
    use crate::workspace::changes::{self, subscriptions};
    let d = TestDir::new();
    let (storage, select, _conn, srcs) = library_workspace(&d, 3);
    let saved = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let sub = saved["subscription"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned();
    crate::workspace::registry::remove(&storage, &select, &srcs[1].1).unwrap();
    let rest = storage
        .clone()
        .confined(&[srcs[0].0.clone(), srcs[2].0.clone()]);
    let one = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let req = changes::Request {
        subscription: Some(&sub),
        ..Default::default()
    };
    // A remaining member outside the caller's roots still hides the set.
    let hidden = (
        subscriptions::list(&one, &select, None, None, None).unwrap()["subscriptions"].clone(),
        changes::changes(&one, &select, &req).is_err(),
        subscriptions::unsubscribe(&one, &select, Some(&sub), None, None).is_err(),
    );
    let listed = subscriptions::list(&rest, &select, None, None, None).unwrap();
    let page = changes::changes(&rest, &select, &req).unwrap();
    let removed = subscriptions::unsubscribe(&rest, &select, Some(&sub), None, None).unwrap();
    let mut remaining = vec![srcs[0].1.clone(), srcs[2].1.clone()];
    remaining.sort();
    assert_eq!(
        (
            hidden,
            &listed["subscriptions"][0]["sources"],
            &page["subscription"]["disabled"],
            format!("{listed}{page}").contains(&srcs[1].1),
            &removed["removed"],
        ),
        (
            (json!([]), true, true),
            &json!(remaining),
            &json!({"reason": "source_removed"}),
            false,
            &json!(1),
        )
    );
}

#[test]
fn a_disabled_library_set_never_names_its_removed_definition_source() {
    use crate::workspace::changes::{self, subscriptions};
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let def = component_ref(&conn, &srcs[1].1, srcs[1].2);
    let saved = subscribe_set(
        &storage,
        &select,
        "a",
        "visible",
        None,
        Some(saved_library(def)),
    )
    .unwrap();
    let sub = subscription_id(&saved);
    crate::workspace::registry::remove(&storage, &select, &srcs[1].1).unwrap();
    let rest = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let req = changes::Request {
        subscription: Some(&sub),
        ..Default::default()
    };
    let listed = subscriptions::list(&rest, &select, None, None, None).unwrap();
    let page = changes::changes(&rest, &select, &req).unwrap();
    let evaluated = changes::evaluate(&rest, &select, Some(&sub)).unwrap();
    assert_eq!(
        (
            &listed["subscriptions"][0]["library"]["key"],
            format!("{listed}{page}{evaluated}").contains(&srcs[1].1),
        ),
        (&json!("k"), false)
    );
}

#[test]
fn an_unreadable_set_reads_as_disabled_and_only_a_caller_seeing_every_source_removes_it() {
    use crate::workspace::changes::subscriptions;
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    // Not JSON, not an array, more members than a set holds, more bytes than it takes, and a
    // lone surrogate SQLite's JSON accepts but serde refuses.
    let damaged = [
        "not json".to_owned(),
        json!({"m": srcs[0].1}).to_string(),
        json!(vec![srcs[0].1.clone(); 11]).to_string(),
        json!(["s".repeat(5000)]).to_string(),
        r#"["\ud800"]"#.to_owned(),
    ];
    let subs: Vec<String> = damaged
        .iter()
        .enumerate()
        .map(|(i, stored)| {
            let saved = subscribe_set(
                &storage,
                &select,
                &format!("c{i}"),
                "visible",
                Some("badge"),
                None,
            )
            .unwrap();
            let sub = saved["subscription"]["subscriptionId"]
                .as_str()
                .unwrap()
                .to_owned();
            conn.execute(
                "UPDATE subscriptions SET sources = ?2 WHERE id = ?1",
                params![sub, stored],
            )
            .unwrap();
            sub
        })
        .collect();
    let unseen = (
        subscriptions::list(&narrow, &select, None, None, None).unwrap()["subscriptions"].clone(),
        subscriptions::unsubscribe(&narrow, &select, Some(&subs[0]), None, None).is_err(),
    );
    let listed: Vec<Json> =
        subscriptions::list(&storage, &select, None, None, None).unwrap()["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| json!([s["state"], s["sources"], s["disabled"]]))
            .collect();
    let removed: Vec<Json> = subs
        .iter()
        .map(|s| {
            subscriptions::unsubscribe(&storage, &select, Some(s), None, None).unwrap()["removed"]
                .clone()
        })
        .collect();
    assert_eq!(
        (unseen, listed, removed),
        (
            (json!([]), true),
            vec![json!(["disabled", null, {"reason": "sources_unreadable"}]); 5],
            vec![json!(1); 5]
        )
    );
}

#[test]
fn an_unpublished_set_names_each_member_to_refresh_and_never_the_sentinel() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let saved = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let sub = saved["subscription"]["subscriptionId"].as_str().unwrap();
    conn.execute("DELETE FROM published", []).unwrap();
    let out = crate::workspace::changes::evaluate(&storage, &select, Some(sub)).unwrap();
    let steps = out["evaluation"]["nextSteps"].as_array().unwrap();
    let mut named: Vec<&str> = steps
        .iter()
        .map(|s| s["arguments"]["sourceId"].as_str().unwrap())
        .collect();
    named.sort();
    let mut ids: Vec<&str> = srcs.iter().map(|s| s.1.as_str()).collect();
    ids.sort();
    assert_eq!(
        (&out["evaluation"]["reason"], named),
        (&json!("scope_not_published"), ids)
    );
}

#[test]
fn subscribe_refuses_each_conflicting_or_missing_target_before_opening_the_workspace() {
    use crate::workspace::changes::subscriptions::{Subscribe, subscribe};
    let d = TestDir::new();
    let (storage, select, _) = workspace(&d, 1);
    let base = || Subscribe {
        consumer: Some("a"),
        source: Some("s"),
        scope: None,
        node: None,
        owner: None,
        query: None,
        types: Vec::new(),
        sources: None,
        library: None,
    };
    let cases = [
        (
            Subscribe {
                query: Some("badge"),
                library: Some(reference("k", None)),
                ..base()
            },
            "pass query or library, not both",
        ),
        (
            Subscribe {
                source: None,
                sources: Some("visible"),
                ..base()
            },
            "sources applies to a saved query",
        ),
        (
            Subscribe {
                sources: Some("visible"),
                query: Some("badge"),
                ..base()
            },
            "pass sourceId or sources, not both",
        ),
        (
            Subscribe {
                source: None,
                ..base()
            },
            "pass sourceId",
        ),
        (
            Subscribe {
                query: Some("badge"),
                node: Some("1:2"),
                ..base()
            },
            "or node, not both",
        ),
        (
            Subscribe {
                library: Some(reference("k", None)),
                node: Some("1:2"),
                ..base()
            },
            "or node, not both",
        ),
        (
            Subscribe {
                types: vec!["FRAME".into()],
                ..base()
            },
            "type filters a saved query",
        ),
    ];
    for (req, expected) in cases {
        let err = subscribe(&storage, &select, &req).unwrap_err().to_string();
        assert!(err.contains(expected), "{expected}: {err}");
    }
}

#[test]
fn a_listed_set_is_bounded_before_any_member_is_resolved() {
    let d = TestDir::new();
    let (storage, select, _conn, srcs) = library_workspace(&d, 1);
    // Eleven names of one visible source, and one name past the byte cap.
    let refused = [[srcs[0].1.as_str(); 11].join(","), "x".repeat(5000)].map(|list| {
        subscribe_set(&storage, &select, "a", &list, Some("badge"), None)
            .unwrap_err()
            .to_string()
    });
    assert!(refused.iter().all(|e| e.contains("at most")), "{refused:?}");
}

/// One runner turn of `storage` on `source`'s `0:0`: searches computed and whether more is due.
fn turn(storage: &Storage, select: &Select, source: &str) -> (u64, bool) {
    let go = std::sync::atomic::AtomicBool::new(false);
    let t =
        crate::workspace::changes::evaluate_published(storage, select, source, "0:0", None, &go)
            .unwrap();
    (t.report["computed"].as_u64().unwrap(), t.next.is_some())
}

#[test]
fn a_non_member_publication_leaves_a_set_untouched() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    subscribe_set(&storage, &select, "a", &srcs[0].1, Some("badge"), None).unwrap();
    republish(&conn, srcs[1].2, "sha256:other");
    let stored: i64 = conn
        .query_row("SELECT count(*) FROM query_evaluations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (turn(&storage, &select, &srcs[1].1), stored),
        ((0, false), 1)
    );
}

#[test]
fn a_set_whose_newest_membership_is_oversized_is_due_again_unparsed() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let current = turn(&storage, &select, &srcs[0].1);
    // Still naming this publication, but past what any written evaluation takes.
    conn.execute(
        "UPDATE query_evaluations SET membership = json_insert(membership, '$[#]', ?1)",
        ["x".repeat(1 << 20)],
    )
    .unwrap();
    assert_eq!(
        (current, turn(&storage, &select, &srcs[0].1).0),
        ((0, false), 1)
    );
}

#[test]
fn a_runner_that_cannot_see_every_member_never_evaluates_the_set() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    republish(&conn, srcs[0].2, "sha256:new");
    let narrow = storage.clone().confined(std::slice::from_ref(&srcs[0].0));
    let narrower = turn(&narrow, &select, &srcs[0].1);
    // The same publication is due for a runner that sees the whole set.
    assert_eq!(
        (narrower, turn(&storage, &select, &srcs[0].1).0),
        ((0, false), 1)
    );
}

#[test]
fn a_removed_member_added_again_never_revives_a_disabled_set() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 2);
    let saved = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let sub = saved["subscription"]["subscriptionId"].as_str().unwrap();
    crate::workspace::registry::remove(&storage, &select, &srcs[1].1).unwrap();
    crate::workspace::registry::add(&storage, &select, &srcs[1].0).unwrap();
    let back: i64 = conn
        .query_row(
            "SELECT count(*) FROM sources WHERE source_id = ?1",
            [&srcs[1].1],
            |r| r.get(0),
        )
        .unwrap();
    republish(&conn, srcs[0].2, "sha256:new");
    let after = turn(&storage, &select, &srcs[0].1);
    assert_eq!(
        (
            back,
            after,
            evaluations(&storage, &select, sub)["evaluation"]["state"].clone()
        ),
        (1, (0, false), json!("disabled"))
    );
}

#[test]
fn ack_on_a_disabled_subscription_is_refused() {
    let d = TestDir::new();
    let (storage, select, conn, _) = library_workspace(&d, 2);
    let saved = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let sub = saved["subscription"]["subscriptionId"].as_str().unwrap();
    let cursor = evaluations(&storage, &select, sub)["nextCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    conn.execute(
        "UPDATE subscriptions SET disabled = '{\"reason\":\"source_removed\"}' WHERE id = ?1",
        [sub],
    )
    .unwrap();
    let req = crate::workspace::changes::Request {
        subscription: Some(sub),
        cursor: Some(&cursor),
        ..Default::default()
    };
    let err = crate::workspace::changes::ack(&storage, &select, &req)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is disabled"), "{err}");
}

#[test]
fn a_subscribe_that_meets_its_disabled_subscription_asks_to_unsubscribe_first() {
    let d = TestDir::new();
    let (storage, select, conn, _) = library_workspace(&d, 2);
    let first = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None).unwrap();
    let sub = first["subscription"]["subscriptionId"].as_str().unwrap();
    conn.execute(
        "UPDATE subscriptions SET disabled = '{\"reason\":\"source_removed\"}' WHERE id = ?1",
        [sub],
    )
    .unwrap();
    let err = subscribe_set(&storage, &select, "a", "visible", Some("badge"), None)
        .unwrap_err()
        .downcast::<FigError>()
        .unwrap();
    assert_eq!(
        (
            err.kind(),
            &err.recovery().unwrap()[0]["arguments"]["subscriptionId"]
        ),
        (ErrorKind::InvalidArguments, &json!(sub))
    );
}

#[test]
fn a_submit_never_joins_a_build_that_is_being_cancelled() {
    let d = TestDir::new();
    let (storage, select, _) = workspace(&d, 1);
    queued(&storage, &select, "0:0");
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    // As a closing runner's cancel leaves its own build running until its worker sees it.
    ws.conn.execute("UPDATE jobs SET cancel = 1", []).unwrap();
    let source = sources(&ws.conn)[0].clone();
    let Err(error) = submit(&storage, &mut ws, "refresh", &source, "0:0", None) else {
        panic!("joined a cancelled build");
    };
    assert_eq!(
        error.downcast_ref::<FigError>().unwrap().kind(),
        ErrorKind::WorkspaceBusy
    );
}

/// Republishes `source`'s generation `gen_id` as a new one whose term statistics differ.
fn republish_with_other_statistics(conn: &Connection, source: &str, gen_id: i64) {
    conn.execute(
        "INSERT INTO generations (source_id, membership, scope, snapshot_id, versions, job_id, state,
         complete, records, info, created_ms) SELECT source_id, membership, scope, snapshot_id,
         versions, job_id, state, complete, records, info, created_ms FROM generations WHERE gen = ?1",
        [gen_id],
    )
    .unwrap();
    let new = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO records SELECT ?1, rid, node_id, name, kind, text, norm0, norm1, len0, len1,
         parent, source, text_source, hidden, page, root FROM records WHERE gen = ?2",
        params![new, gen_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO terms SELECT ?1, term, df + 1, postings FROM terms WHERE gen = ?2",
        params![new, gen_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE published SET gen = ?1 WHERE source_id = ?2",
        params![new, source],
    )
    .unwrap();
}

#[test]
fn a_search_cursor_is_stale_when_a_selected_source_s_statistics_change_not_an_unrelated_one() {
    let d = TestDir::new();
    let (storage, select, conn, srcs) = library_workspace(&d, 3);
    let only = [srcs[0].1.clone(), srcs[1].1.clone()];
    let ask = |cursor: Option<&str>| crate::search::Request {
        query: "first".into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit: 1,
        cursor: cursor.map(str::to_owned),
    };
    let first = crate::workspace::query::search(&storage, &select, &only, &ask(None)).unwrap();
    let cursor = first["nextCursor"]
        .as_str()
        .expect("a second page")
        .to_owned();
    let again = || crate::workspace::query::search(&storage, &select, &only, &ask(Some(&cursor)));
    republish_with_other_statistics(&conn, &srcs[2].1, srcs[2].2);
    assert!(again().is_ok(), "an unselected source changed");
    republish_with_other_statistics(&conn, &srcs[1].1, srcs[1].2);
    assert_eq!(kind_of_error(again()), ErrorKind::CursorStale);
}

#[test]
fn a_search_cursor_past_its_lease_is_expired_not_stale() {
    let d = TestDir::new();
    let (storage, select, _conn, _) = library_workspace(&d, 2);
    let ask = |cursor: Option<&str>| crate::search::Request {
        query: "first".into(),
        scope: "0:0".into(),
        types: Vec::new(),
        include_hidden: false,
        include_internal: false,
        limit: 1,
        cursor: cursor.map(str::to_owned),
    };
    let search = |cursor: Option<&str>| {
        crate::workspace::query::search(&storage, &select, &[], &ask(cursor))
    };
    let first = search(None).unwrap();
    let cursor = first["nextCursor"].as_str().expect("a second page");
    // The issue time is the cursor's last field; one lease earlier, it has ended.
    let (rest, issued) = cursor.rsplit_once('.').unwrap();
    let issued: i64 = issued.parse().unwrap();
    let at = |ms: i64| format!("{rest}.{ms}");
    assert!(search(Some(&at(issued - crate::search::CURSOR_LEASE_MS + 1000))).is_ok());
    let old = at(issued - crate::search::CURSOR_LEASE_MS - 1000);
    assert_eq!(kind_of_error(search(Some(&old))), ErrorKind::CursorExpired);
}
