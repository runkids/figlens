use super::*;
use crate::test_support::TestDir;

fn row() -> Row {
    Row {
        id: "source".into(),
        path: "unused".into(),
        membership: "current".into(),
        gen_id: Some(1),
        gen_membership: Some("current".into()),
        snapshot: Some("snapshot".into()),
        versions: Some("versions".into()),
        bytes: Some(10),
        mtime_ns: Some(0),
        verified_ms: Some(0),
    }
}

#[test]
fn readiness_precedence_matrix() {
    // Every combination checks precedence, not just one example of each state.
    for readable in [false, true] {
        for generation in [false, true] {
            for membership in [false, true] {
                for compatible in [false, true] {
                    for unchanged in [false, true] {
                        for job_state in
                            [None, Some("queued"), Some("running"), Some("interrupted")]
                        {
                            let mut row = row();
                            row.gen_id = generation.then_some(1);
                            row.gen_membership = membership.then(|| "current".into());
                            row.versions = compatible.then(|| "versions".into());
                            let stamp = if readable {
                                Ok((SystemTime::UNIX_EPOCH, if unchanged { 10 } else { 11 }))
                            } else {
                                Err("unreadable".into())
                            };
                            let job = job_state.map(|s| json!({"jobId": "job", "state": s}));
                            let expected = if !readable {
                                State::Unreadable
                            } else if !(generation && membership) {
                                if matches!(job_state, Some("queued" | "running")) {
                                    State::Pending
                                } else if generation {
                                    State::StaleMembership
                                } else {
                                    State::NotPrepared
                                }
                            } else if !compatible {
                                State::Incompatible
                            } else if !unchanged {
                                State::StaleStamp
                            } else {
                                State::Ready
                            };
                            assert_eq!(row.state(stamp, "versions", job.as_ref()), expected);
                        }
                    }
                }
            }
        }
    }
    let mut row = row();
    row.mtime_ns = Some(1);
    assert_eq!(
        row.state(Ok((SystemTime::UNIX_EPOCH, 10)), "versions", None),
        State::StaleStamp
    );
}

#[test]
fn check_preserves_reader_job_policy_and_regular_file_guard() {
    let dir = TestDir::new();
    let source = dir.join("source.fig");
    std::fs::write(&source, "source").unwrap();
    let mut row = row();
    row.path = source.to_str().unwrap().into();
    let stamp = crate::search::stamp(&source).unwrap();
    row.bytes = Some(stamp.1 as i64);
    row.mtime_ns = Some(mtime_ns(stamp.0));
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE jobs (job_id TEXT, source_id TEXT, scope TEXT, state TEXT, owner TEXT, attempt INTEGER, kind TEXT);
        INSERT INTO jobs VALUES ('job', 'source', '0:0', 'running', 'dead', 1, 'refresh');").unwrap();
    let tx = conn.transaction().unwrap();
    let checked = row
        .check(&tx, &dir.join("workspace"), "0:0", "versions", true)
        .unwrap();
    assert_eq!(checked.state, State::Ready);
    assert_eq!(checked.job.unwrap()["state"], "interrupted");
    assert!(
        row.check(&tx, &dir.join("workspace"), "0:0", "versions", false)
            .unwrap()
            .job
            .is_none()
    );
    row.gen_membership = None;
    assert_eq!(
        row.check(&tx, &dir.join("workspace"), "0:0", "versions", false)
            .unwrap()
            .state,
        State::StaleMembership
    );
    std::fs::create_dir(dir.join("workspace")).unwrap();
    tx.execute(
        "UPDATE jobs SET owner = ?1",
        [jobs::owner(&dir.join("workspace")).unwrap()],
    )
    .unwrap();
    assert_eq!(
        row.check(&tx, &dir.join("workspace"), "0:0", "versions", false)
            .unwrap()
            .state,
        State::Pending
    );
    row.path = dir.join("").to_str().unwrap().into();
    assert_eq!(
        row.check(&tx, &dir.join("workspace"), "0:0", "versions", false)
            .unwrap()
            .state,
        State::Unreadable
    );
}
