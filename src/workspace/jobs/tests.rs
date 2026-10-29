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
        Some(child),
        held,
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
    let _ = abandon(&mut read_only, &job_id, &me, 1, None, held, "lost".into());
    drop(read_only);
    assert_eq!(state(&conn, &job_id), "running");
    let report = report_job(&conn, &dir, &job_id).unwrap().unwrap();
    assert_eq!(report["state"], "interrupted", "{report}");
    // Storage is back: a refresh takes the job over as the next attempt.
    let (_, mut ws) = opened(&storage, &select, true).unwrap();
    let again = submit(&mut ws, "refresh", &source, "0:0", None).unwrap();
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
