use super::*;
use crate::test_support::TestDir;
use crate::workspace::{PrepareOptions, prepare, resolve};
use std::ffi::OsString;

fn served(rows: &[(&str, Option<i64>)]) -> Served {
    rows.iter()
        .map(|(scope, g)| ("src".to_owned(), (*scope).to_owned(), "f".to_owned(), *g))
        .collect()
}

fn key(scope: &str) -> Scope {
    ("src".to_owned(), scope.to_owned())
}

#[test]
fn a_renewal_pass_forgets_a_workspace_only_if_no_lease_started_in_it_since_its_read() {
    let dir = TestDir::new();
    let home = std::path::absolute(dir.join("home")).unwrap();
    let vars = move |name: &str| (name == "HOME").then(|| home.clone().into_os_string());
    let storage = resolve(None, &vars, false, &|| None).unwrap();
    let mut held = vec![
        (storage.clone(), "a".to_owned(), 1),
        (storage, "b".to_owned(), 3),
    ];
    // The pass read both with no lease left; a start in "b" moved its mark to 3 meanwhile.
    forget(&mut held, &[("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let left: Vec<&str> = held.iter().map(|(_, id, _)| id.as_str()).collect();
    assert_eq!(left, ["b"]);
}

#[test]
fn saved_queries_are_due_once_per_appearance_or_publication() {
    let (mut q, now) = (Saved::<u32>::new(), Instant::now());
    q.observe(&served(&[("0:1", Some(1)), ("0:2", None)]), now);
    q.observe(&served(&[("0:1", Some(1)), ("0:2", None)]), now);
    assert_eq!(
        q.due.len(),
        2,
        "an unchanged union read queues nothing more"
    );
    let first = q.take(now).unwrap();
    q.finished(first.key, Outcome::Next(Some(7)), now);
    q.observe(&served(&[("0:1", Some(2)), ("0:2", None)]), now);
    let again: Vec<_> = q.due.iter().map(|d| (d.key.1.clone(), d.from)).collect();
    assert_eq!(again, [("0:2".to_owned(), None), ("0:1".to_owned(), None)]);
}

#[test]
fn a_scope_no_longer_served_is_dropped_even_mid_turn() {
    let (mut q, now) = (Saved::<u32>::new(), Instant::now());
    q.observe(&served(&[("0:1", Some(1))]), now);
    let due = q.take(now).unwrap();
    q.observe(&served(&[]), now);
    q.finished(due.key, Outcome::Again(Some(3)), now);
    assert!(q.due.is_empty() && q.seen.is_empty());
}

#[test]
fn a_busy_turn_waits_out_the_backoff_and_an_interrupted_one_resumes_at_once() {
    let (mut q, now) = (Saved::<u32>::new(), Instant::now());
    q.observe(&served(&[("0:1", Some(1)), ("0:2", Some(1))]), now);
    let a = q.take(now).unwrap();
    q.finished(a.key, Outcome::Retry(Some(4)), now);
    let b = q.take(now).unwrap();
    assert_eq!(b.key, key("0:2"));
    q.finished(b.key, Outcome::Again(Some(5)), now);
    assert_eq!(
        q.take(now).map(|d| (d.key, d.from)),
        Some((key("0:2"), Some(5)))
    );
    assert!(q.take(now).is_none(), "the busy scope waits");
    assert_eq!(q.take(now + BUSY).map(|d| d.from), Some(Some(4)));
}

#[test]
fn scopes_take_turns_and_a_finished_queue_stops_until_the_next_publication() {
    let (mut q, now) = (Saved::<u32>::new(), Instant::now());
    q.observe(&served(&[("0:1", Some(1)), ("0:2", Some(1))]), now);
    let a = q.take(now).unwrap();
    q.finished(a.key, Outcome::Next(Some(1)), now);
    assert_eq!(q.take(now).unwrap().key, key("0:2"), "round-robin");
    let a = q.take(now).unwrap();
    q.finished(a.key, Outcome::Next(None), now);
    assert!(q.take(now).is_none());
}

#[test]
fn a_turn_of_an_older_publication_does_not_overwrite_the_fresh_start() {
    let (mut q, now) = (Saved::<u32>::new(), Instant::now());
    q.observe(&served(&[("0:1", Some(1))]), now);
    let due = q.take(now).unwrap();
    q.observe(&served(&[("0:1", Some(2))]), now);
    q.finished(due.key, Outcome::Next(Some(9)), now);
    assert_eq!(q.due.iter().map(|d| d.from).collect::<Vec<_>>(), [None]);
}

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
        let report = prepare(&storage, &fig, Some(&project), PrepareOptions::default()).unwrap();
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
    let first = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    let again = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    assert_eq!(
        (again["leaseId"].clone(), again["state"].clone()),
        (first["leaseId"].clone(), json!("active"))
    );
    stop(&storage, &select, first["leaseId"].as_str().unwrap()).unwrap();
    let ended = lease(&storage, &select, Some(&ids[0]), "0:1", Some("r1"))
        .unwrap()
        .0;
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
    lease(&storage, &select, None, "0:1", Some("r1")).unwrap();
    assert!(lease(&storage, &select, None, "0:2", Some("r1")).is_err());
}

#[test]
fn an_untracked_scope_is_refused_with_the_track_step() {
    let dir = TestDir::new();
    let (storage, select, ..) = tracked(&dir, 1);
    let e = lease(&storage, &select, None, "0:9", None).unwrap_err();
    let fig = e.downcast_ref::<FigError>().unwrap();
    assert_eq!(fig.recovery().unwrap()[0]["action"], "track");
}

#[test]
fn a_holder_stops_only_its_own_lease() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let lease = lease(&storage, &select, None, "0:1", None).unwrap().0;
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
    let a = lease(&storage, &select, Some(&ids[0]), "0:1", None)
        .unwrap()
        .0;
    lease(&storage, &select, Some(&ids[1]), "0:1", None).unwrap();
    stop(&storage, &select, a["leaseId"].as_str().unwrap()).unwrap();
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute("UPDATE watch_leases SET expires_ms = expires_ms - 1", [])
        .unwrap();
    assert_eq!(renew_one(&storage, &select_id(&select)).unwrap().0, 1);
}

#[test]
fn a_start_where_no_source_is_in_view_says_so() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let elsewhere = std::path::absolute(dir.join("elsewhere")).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let confined = storage.confined(&[elsewhere.canonicalize().unwrap()]);
    let refused = lease(&confined, &select, None, "0:1", None).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("no source this process can see"),
        "{refused}"
    );
}

#[test]
fn a_lease_on_a_source_outside_the_roots_is_unauthorized_at_renewal() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", None).unwrap();
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
fn a_monitor_is_partial_while_one_leased_scope_is_unauthorized() {
    let dir = TestDir::new();
    let (storage, select, dir, ids) = tracked(&dir, 1);
    let conn = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    conn.execute(
        "INSERT INTO tracking (source_id, scope, state, epoch, facet_version, enabled_ms)
             VALUES (?1, '0:2', 'active', 1, 'test', 0)",
        [&ids[0]],
    )
    .unwrap();
    lease(&storage, &select, None, "0:1", None).unwrap();
    lease(&storage, &select, None, "0:2", None).unwrap();
    conn.execute_batch(&format!(
        "UPDATE watch_leases SET authorized_path = NULL WHERE scope = '0:2';
             INSERT INTO watch_runner (one, token, attempt, placement, started_ms, beat_ms)
             VALUES (1, 't', 1, 'test', 0, {now})",
        now = now_ms()
    ))
    .unwrap();
    std::fs::create_dir_all(dir.join("watch")).unwrap();
    let held = File::create(runner_lock(&dir)).unwrap();
    held.lock().unwrap();
    assert_eq!(
        monitor(&conn, &dir, &ids, None).unwrap()["state"],
        "partial"
    );
}

#[test]
fn status_writes_nothing() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", None).unwrap();
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
fn status_keeps_a_visible_live_lease_past_a_hidden_sources_ended_history() {
    let dir = TestDir::new();
    let (storage, select, dir, ids) = tracked(&dir, 2);
    let a = lease(&storage, &select, Some(&ids[0]), "0:1", None)
        .unwrap()
        .0;
    // A hidden source's newer ended leases, more than the bound, within the retention.
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?2)
                 INSERT INTO watch_leases (lease_id, holder, source_id, scope, incarnation, epoch,
                 fingerprint, renewed_ms, expires_ms, ended)
                 SELECT 'old' || i, holder, ?1, scope, incarnation, epoch, fingerprint,
                 renewed_ms + i, expires_ms, 'stopped' FROM watch_leases, n",
            params![ids[1], MAX_LEASES * 4],
        )
        .unwrap();
    let path: String = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row(
            "SELECT path FROM sources WHERE source_id = ?1",
            [&ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    let confined = storage.clone().confined(&[PathBuf::from(path)]);
    let report = status(&confined, &select).unwrap();
    assert_eq!(
        (report["leases"].clone(), report.get("truncated")),
        (
            json!([{"leaseId": a["leaseId"], "sourceId": ids[0], "scope": "0:1",
                "state": "active", "expiresAtMs": a["expiresAtMs"], "mine": true}]),
            None
        )
    );
    let full = status(&storage, &select).unwrap();
    assert_eq!(
        (
            full["leases"][0]["leaseId"].clone(),
            full["leases"].as_array().unwrap().len(),
            full["truncated"].clone()
        ),
        (a["leaseId"].clone(), (MAX_LEASES * 4) as usize, json!(true))
    );
}

#[test]
fn a_replay_after_a_retrack_ends_the_old_lease_and_a_new_request_id_starts_one() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let first = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute("UPDATE tracking SET epoch = epoch + 1", [])
        .unwrap();
    let again = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    assert_eq!(
        (again["state"].clone(), again["reason"].clone()),
        (json!("ended"), json!("untracked"))
    );
    let fresh = lease(&storage, &select, None, "0:1", Some("r2")).unwrap().0;
    assert_eq!(fresh["state"], "active");
    assert_ne!(fresh["leaseId"], first["leaseId"]);
}

#[test]
fn a_renewal_ends_an_old_epoch_lease_instead_of_renewing_it() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", None).unwrap();
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute("UPDATE tracking SET epoch = epoch + 1", [])
        .unwrap();
    assert_eq!(renew_one(&storage, &select_id(&select)).unwrap().0, 0);
    assert_eq!(
        rows(
            &dir,
            "SELECT count(*) FROM watch_leases WHERE ended = 'untracked'"
        ),
        1
    );
}

#[test]
fn a_request_id_of_another_live_holders_lease_is_refused_and_leaves_it_active() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", Some("r1")).unwrap();
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute("UPDATE watch_leases SET holder = 'another'", [])
        .unwrap();
    let refused = lease(&storage, &select, None, "0:1", Some("r1")).unwrap_err();
    assert!(
        refused.to_string().contains("another running process"),
        "{refused}"
    );
    assert_eq!(
        rows(
            &dir,
            "SELECT count(*) FROM watch_leases WHERE ended IS NULL"
        ),
        1
    );
}

#[test]
fn collection_bounds_ended_rows_and_a_collected_request_id_starts_a_new_lease() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let first = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    let ws_id = first["workspaceId"].as_str().unwrap();
    stop(&storage, &select, first["leaseId"].as_str().unwrap()).unwrap();
    let db = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    db.execute_batch(&format!(
        "UPDATE watch_leases SET renewed_ms = renewed_ms - {KEEP_ENDED_MS} - 1;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {extra})
             INSERT INTO watch_leases (lease_id, holder, source_id, scope, incarnation, epoch,
             fingerprint, renewed_ms, expires_ms, ended)
             SELECT 'old' || i, holder, source_id, scope, incarnation, epoch, fingerprint,
             renewed_ms + {KEEP_ENDED_MS} + i, expires_ms, 'stopped' FROM watch_leases, n;
             INSERT INTO watch_runner (one, token, attempt, placement, started_ms, beat_ms)
             VALUES (1, 'gc', 1, 'independent', 0, 0);",
        extra = KEEP_ENDED_ROWS + 8
    ))
    .unwrap();
    assert!(persist(&storage, ws_id, "gc", 1, &std::collections::BTreeMap::new()).unwrap());
    assert_eq!(
        rows(&dir, "SELECT count(*) FROM watch_leases"),
        KEEP_ENDED_ROWS
    );
    let again = lease(&storage, &select, None, "0:1", Some("r1")).unwrap().0;
    assert_eq!(again["state"], "active");
    assert_ne!(again["leaseId"], first["leaseId"]);
}

#[test]
fn removing_a_source_ends_its_leases() {
    let dir = TestDir::new();
    let (storage, select, dir, ids) = tracked(&dir, 2);
    lease(&storage, &select, Some(&ids[0]), "0:1", None).unwrap();
    crate::workspace::remove(&storage, &select, &ids[0]).unwrap();
    assert_eq!(
        rows(
            &dir,
            "SELECT count(*) FROM watch_leases WHERE ended = 'source_removed'"
        ),
        1
    );
}

#[test]
fn a_schema_4_workspace_reads_its_history_and_watch_status_without_migrating() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .execute_batch(
            "DROP TABLE query_evaluations; DROP TABLE subscriptions;
                 ALTER TABLE consumers DROP COLUMN sources;
                 DROP TABLE watch_leases; DROP TABLE watch_runner; DROP TABLE watch_monitor;
                 PRAGMA user_version = 4;",
        )
        .unwrap();
    let request = super::super::changes::Request {
        source: None,
        scope: Some("0:1"),
        consumer: None,
        cursor: None,
        revision: None,
        limit: None,
        subscription: None,
    };
    let reply = super::super::changes::changes(&storage, &select, &request).unwrap();
    assert_eq!(reply["tracking"][0]["state"], "active");
    assert_eq!(status(&storage, &select).unwrap()["leases"], json!([]));
    assert_eq!(rows(&dir, "PRAGMA user_version"), 4);
}

fn runner_row(storage: &Storage, select: &Select) -> (String, i64) {
    let ws_id = select_id(select);
    let (_, ws) = opened(storage, select, true).unwrap();
    ws.conn
        .execute(
            "INSERT INTO watch_runner (one, token, attempt, placement, started_ms, beat_ms)
                 VALUES (1, 't', 1, 'independent', 0, 0)",
            [],
        )
        .unwrap();
    (ws_id, 1)
}

#[test]
fn untracking_ends_a_lease_for_the_runner_and_retracking_never_revives_it() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", None).unwrap();
    let (_, attempt) = runner_row(&storage, &select);
    let served = || {
        let (_, ws) = opened(&storage, &select, false).unwrap();
        union(&ws, "t", attempt).unwrap().unwrap().len()
    };
    assert_eq!(served(), 1);
    let db = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    db.execute("UPDATE tracking SET state = 'baseline_pending'", [])
        .unwrap();
    assert_eq!(
        served(),
        0,
        "nothing is refreshed while the baseline is pending"
    );
    // Untrack, then track and baseline again: a new epoch.
    db.execute(
        "UPDATE tracking SET state = 'active', epoch = epoch + 2",
        [],
    )
    .unwrap();
    assert_eq!(served(), 0);
    assert_eq!(
        status(&storage, &select).unwrap()["leases"][0]["state"],
        "untracked"
    );
}

#[test]
fn a_runner_closes_only_with_no_live_lease_and_only_while_it_is_the_runner() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let (ws_id, attempt) = runner_row(&storage, &select);
    let lease = lease(&storage, &select, None, "0:1", None).unwrap().0;
    assert_eq!(close(&storage, &ws_id, "t", attempt).unwrap(), Some(false));
    stop(&storage, &select, lease["leaseId"].as_str().unwrap()).unwrap();
    assert_eq!(close(&storage, &ws_id, "other", attempt).unwrap(), None);
    assert_eq!(close(&storage, &ws_id, "t", attempt).unwrap(), Some(true));
    assert_eq!(rows(&dir, "SELECT closing FROM watch_runner"), 1);
}

#[test]
fn a_relocation_committed_while_the_runner_waits_to_admit_admits_and_joins_nothing() {
    let dir = TestDir::new();
    let (storage, select, dir, ids) = tracked(&dir, 1);
    lease(&storage, &select, None, "0:1", None).unwrap();
    let mut db = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    let path: String = db
        .query_row("SELECT path FROM sources", [], |r| r.get(0))
        .unwrap();
    let moved = format!("{path}.moved.fig");
    std::fs::copy(&path, &moved).unwrap();
    // The relocation holds the write lock while the runner reads its lease state, and
    // commits while the runner waits to admit.
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute("UPDATE sources SET path = ?1", [&moved])
        .unwrap();
    let runner = std::thread::spawn({
        let (storage, ws_id) = (storage.clone(), select_id(&select));
        let job = Refresh {
            source: ids[0].clone(),
            path: path.into(),
            scopes: vec!["0:1".into()],
        };
        move || {
            let stop = AtomicBool::new(false);
            let done = refresh(&storage, &ws_id, &job, &stop, &stop);
            (done.job, done.error, Instant::now())
        }
    });
    std::thread::sleep(Duration::from_millis(300));
    let committed = Instant::now();
    tx.commit().unwrap();
    let (job, error, ended) = runner.join().unwrap();
    // It ended after the commit with no error: it waited on the admission's write lock and was
    // refused by the fence, not by a busy workspace or a timeout.
    assert_eq!(
        (
            job,
            error,
            ended > committed,
            rows(&dir, "SELECT count(*) FROM jobs")
        ),
        (None, None, true, 0)
    );
}

#[test]
fn a_runner_cancels_only_attempts_nobody_joined_replayed_or_took_over() {
    let dir = TestDir::new();
    let (storage, select, dir, _) = tracked(&dir, 1);
    let ws_id = select_id(&select);
    let me = owner(&dir).unwrap();
    let refresh =
        |request| super::super::jobs::refresh(&storage, &select, None, Some("0:1"), request);
    let shared = || rows(&dir, "SELECT max(shared) FROM jobs WHERE state = 'queued'");
    refresh(None).unwrap();
    assert_eq!(shared(), 0);
    refresh(None).unwrap();
    assert_eq!(shared(), 1, "a join without a requestId");
    assert_eq!(cancel_exclusive(&storage, &ws_id, &me).unwrap(), 0);
    let db = open(&dir.join(DB), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    db.execute("DELETE FROM jobs", []).unwrap();
    refresh(Some("r")).unwrap();
    refresh(Some("r")).unwrap();
    assert_eq!(shared(), 1, "a requestId replay");
    db.execute("UPDATE jobs SET shared = 0, owner = 'gone'", [])
        .unwrap();
    refresh(None).unwrap();
    assert_eq!(shared(), 1, "a takeover from a dead owner");
    db.execute("DELETE FROM jobs", []).unwrap();
    refresh(None).unwrap();
    assert_eq!(cancel_exclusive(&storage, &ws_id, &me).unwrap(), 1);
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
fn refreshes_failing_twice_running_stay_reported_until_one_succeeds() {
    let t0 = Instant::now();
    let at = |ms| t0 + Duration::from_millis(ms);
    let mut d = Detector::new(t0);
    d.observe(at(0), Probe::Stamp(1));
    assert!(d.observe(at(300), Probe::Stamp(1)).submit);
    // One failure alone is a save caught mid-write.
    assert_eq!(d.finished(at(400), true).state, State::Watching);
    d.observe(at(500), Probe::Stamp(1));
    assert!(d.observe(at(800), Probe::Stamp(1)).submit);
    assert_eq!(d.finished(at(900), true).state, State::RefreshFailed);
    // A good stat of the same file does not clear it.
    for ms in (1000..3000).step_by(250) {
        assert_eq!(
            d.observe(at(ms), Probe::Stamp(1)).state,
            State::RefreshFailed
        );
    }
    assert_eq!(d.finished(at(3000), false).state, State::Watching);
    assert_eq!(d.observe(at(3250), Probe::Stamp(1)).state, State::Watching);
}

#[test]
fn an_invalidated_detector_refreshes_its_unchanged_stamp_once_it_holds() {
    let t0 = Instant::now();
    let mut d = Detector::new(t0);
    run(&mut d, t0, &ticks(0, 1000, Some(1)));
    d.invalidate();
    assert_eq!(run(&mut d, t0, &ticks(1000, 3000, Some(1))), [1500]);
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

/// Probes where each step says to, from `from` ms after `t0`, until a submit; returns every
/// probe time (ms after `t0`), the submitting one last.
fn follow(d: &mut Detector<u32>, t0: Instant, from: u64, stamp: impl Fn(u64) -> u32) -> Vec<u64> {
    let mut at = t0 + Duration::from_millis(from);
    let mut probes = Vec::new();
    loop {
        let ms = (at - t0).as_millis() as u64;
        probes.push(ms);
        let step = d.observe(at, Probe::Stamp(stamp(ms)));
        if step.submit {
            d.finished(at, false);
            return probes;
        }
        at = step.next;
    }
}

#[test]
fn a_held_stamp_is_probed_again_exactly_when_it_settles() {
    let t0 = Instant::now();
    let mut d = Detector::new(t0);
    assert_eq!(follow(&mut d, t0, 0, |_| 1), [0, 250, 300]);
    run(&mut d, t0, &ticks(1000, 2000, Some(1)));
    assert_eq!(follow(&mut d, t0, 2000, |_| 2), [2000, 2250, 2300]);
}

#[test]
fn churn_is_probed_again_exactly_at_its_cap() {
    let t0 = Instant::now();
    let mut d = Detector::new(t0);
    run(&mut d, t0, &ticks(0, 1000, Some(1)));
    let probes = follow(&mut d, t0, 1000, |ms| ms as u32);
    assert_eq!(probes, [1000, 1250, 1500, 1750, 2000, 2200]);
}

#[test]
fn the_runner_sleeps_until_the_earliest_probe_but_never_past_a_poll() {
    let now = Instant::now();
    let ms = Duration::from_millis;
    assert_eq!(pause(now, std::iter::empty()), POLL);
    assert_eq!(
        pause(now, [now + ms(400), now + ms(50)].into_iter()),
        ms(50)
    );
    assert_eq!(pause(now, [now + ms(5000)].into_iter()), POLL);
    assert_eq!(pause(now + ms(10), [now].into_iter()), Duration::ZERO);
}
