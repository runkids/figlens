//! Published-generation facts and readiness shared by workspace readers.

use super::*;

/// One registered source as the query found it.
pub(super) struct Row {
    pub(super) id: String,
    pub(super) path: String,
    pub(super) membership: String,
    pub(super) gen_id: Option<i64>,
    pub(super) gen_membership: Option<String>,
    pub(super) snapshot: Option<String>,
    pub(super) versions: Option<String>,
    pub(super) bytes: Option<i64>,
    pub(super) mtime_ns: Option<i64>,
    pub(super) verified_ms: Option<i64>,
}

pub(super) fn mtime_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Every registered source with its published generation of `scope` and that snapshot's stamp,
/// in added order, from one read of `tx`.
pub(super) fn rows(tx: &rusqlite::Transaction, scope: &str) -> Result<Vec<Row>> {
    let mut stmt = tx
        .prepare(
            "SELECT s.source_id, s.path, s.membership, p.gen, g.membership, g.snapshot_id,
             g.versions, n.bytes, n.mtime_ns, n.verified_ms
             FROM sources s
             LEFT JOIN published p ON p.source_id = s.source_id AND p.scope = ?1
             LEFT JOIN generations g ON g.gen = p.gen
             LEFT JOIN snapshots n ON n.source_id = s.source_id AND n.snapshot_id = g.snapshot_id
             ORDER BY s.added_at_ms, s.source_id",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map([scope], |r| {
            Ok(Row {
                id: r.get(0)?,
                path: r.get(1)?,
                membership: r.get(2)?,
                gen_id: r.get(3)?,
                gen_membership: r.get(4)?,
                snapshot: r.get(5)?,
                versions: r.get(6)?,
                bytes: r.get(7)?,
                mtime_ns: r.get(8)?,
                verified_ms: r.get(9)?,
            })
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    drop(stmt);
    Ok(rows)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum State {
    Unreadable,
    Pending,
    StaleMembership,
    NotPrepared,
    Incompatible,
    StaleStamp,
    Ready,
}

impl State {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::Pending => "pending",
            Self::StaleMembership | Self::StaleStamp => "stale",
            Self::NotPrepared => "not_prepared",
            Self::Incompatible => "incompatible",
            Self::Ready => "ready",
        }
    }
}

pub(super) struct Checked {
    pub(super) state: State,
    pub(super) job: Option<Json>,
}

impl Row {
    pub(super) fn current(&self) -> Option<i64> {
        self.gen_id
            .filter(|_| self.gen_membership.as_deref() == Some(self.membership.as_str()))
    }

    /// Called only after visibility and source selection. Search also reports interrupted jobs
    /// beside a current generation; other readers only need a job when publication is missing.
    pub(super) fn check(
        &self,
        tx: &rusqlite::Transaction,
        dir: &Path,
        scope: &str,
        versions: &str,
        report_job: bool,
    ) -> Result<Checked> {
        let mut job = if report_job {
            self.job(tx, dir, scope)?
        } else {
            None
        };
        // Check regular files before opening them: a FIFO must never block a reader.
        let stamp = readable(Path::new(&self.path))
            .and_then(|p| crate::search::stamp(&p).map_err(Into::into));
        if !report_job && stamp.is_ok() && self.current().is_none() {
            job = self.job(tx, dir, scope)?;
        }
        Ok(Checked {
            state: self.state(stamp, versions, job.as_ref()),
            job,
        })
    }

    fn job(&self, tx: &rusqlite::Transaction, dir: &Path, scope: &str) -> Result<Option<Json>> {
        let job: Option<(String, String, String, i64)> = tx
            .query_row(
                "SELECT job_id, state, owner, attempt FROM jobs WHERE source_id = ?1 AND scope = ?2
             AND state IN ('queued', 'running') AND kind <> 'embed'",
                [&self.id, scope],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(sql)?;
        Ok(job.map(|(id, state, owner, attempt)| {
            let state = if jobs::job_alive(dir, &owner, &id, attempt) {
                state
            } else {
                "interrupted".into()
            };
            json!({"jobId": id, "state": state})
        }))
    }

    fn state(&self, stamp: Result<(SystemTime, u64)>, versions: &str, job: Option<&Json>) -> State {
        let Ok((mtime, bytes)) = stamp else {
            return State::Unreadable;
        };
        if self.current().is_none() {
            return if job.is_some_and(|j| j["state"] != "interrupted") {
                State::Pending
            } else if self.gen_id.is_some() {
                State::StaleMembership
            } else {
                State::NotPrepared
            };
        }
        if self.versions.as_deref() != Some(versions) {
            return State::Incompatible;
        }
        if Some(bytes as i64) != self.bytes || Some(mtime_ns(mtime)) != self.mtime_ns {
            return State::StaleStamp;
        }
        State::Ready
    }
}

#[cfg(test)]
mod tests;
