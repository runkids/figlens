//! Uses of one library component, style or variable (#93) across every visible source's
//! published index of one scope, as each file's cached copies record the reference. Reads only,
//! like usages: one SQLite read transaction, no parse, no build. The definition is never
//! resolved to a source, since a file's own definitions carry no key.

use sha2::{Digest, Sha256};

use rusqlite::params;

use super::*;
use crate::search::store;
use crate::search::usages::{LIBRARY, MAX_LIMIT, SCAN, scan_limit};

pub const KINDS: [&str; 3] = ["component", "style", "variable"];
/// Bytes of names, ids and evidence one page copies; a page past it ends early.
const PAGE_BYTES: usize = 4 * 1024 * 1024;

/// Which layers use the library reference `(kind, namespace, key)`.
#[derive(Clone, Debug)]
pub struct Request {
    pub kind: String,
    pub namespace: String,
    pub key: String,
    pub scope: String,
    pub include_hidden: bool,
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Request {
    /// The stored edge target, as extraction writes it; `None` for the namespace stays unknown.
    fn target(&self, namespace: Option<&str>) -> String {
        json!([self.kind, namespace, self.key]).to_string()
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |m: String| fail(ErrorKind::InvalidArguments, m);
        if !KINDS.contains(&self.kind.as_str()) {
            return Err(invalid(format!("library kind takes {}", KINDS.join(", "))));
        }
        if self.namespace.is_empty() || self.key.is_empty() {
            return Err(invalid("library needs a namespace and a key".into()));
        }
        if self.target(Some(&self.namespace)).len() > store::ID_BYTES {
            return Err(invalid(format!(
                "library namespace and key take at most {} bytes together",
                store::ID_BYTES
            )));
        }
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(invalid(format!("usages limit takes 1 to {MAX_LIMIT}")));
        }
        Ok(())
    }

    /// The question as tool arguments, for a recovery that repeats it.
    pub(crate) fn arguments(&self) -> Json {
        let mut asked = json!({"library": {"kind": self.kind, "namespace": self.namespace,
            "key": self.key}, "scope": self.scope, "limit": self.limit});
        if self.include_hidden {
            asked["includeHidden"] = json!(true);
        }
        asked
    }
}

/// One visible source and where its published scope stands.
struct Source {
    id: String,
    snapshot: Option<String>,
    state: &'static str,
    gen_id: Option<i64>,
    step: Json,
}

/// Uses of `req`'s library reference in every visible source (only `source` when given).
pub fn library(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    req: &Request,
) -> Result<Json> {
    req.validate()?;
    let (_, ws) = opened(storage, select, false)?;
    let ws_id = ws.held.id.clone();
    if ws.version < SCHEMA {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!("workspace {ws_id} has no relations index of this figlens yet; rebuild it, then query usages"),
        )
        .with_recovery(json!([{"action": "rebuild", "requires": "none", "tool": "index",
            "arguments": {"action": "rebuild", "workspaceId": ws_id}}]))
        .into());
    }
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    type Row = (
        String,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = tx
        .prepare(
            "SELECT s.source_id, s.path, s.membership, p.gen, g.membership, g.snapshot_id
             FROM sources s
             LEFT JOIN published p ON p.source_id = s.source_id AND p.scope = ?1
             LEFT JOIN generations g ON g.gen = p.gen
             ORDER BY s.added_at_ms, s.source_id",
        )
        .map_err(sql)?
        .query_map([&req.scope], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql)?;
    super::query::relocated(
        storage,
        select,
        rows.iter().map(|r| r.1.as_str()),
        "usages",
        req.arguments(),
    )?;
    // Invisible sources are dropped before anything about them is read or named; visibility is
    // decided again on every page, and the cursor binds the sources it saw.
    let total = rows.len();
    let mut visible: Vec<Row> = rows
        .into_iter()
        .filter(|r| storage.sees(Path::new(&r.1)))
        .collect();
    // Sources this caller may not see are left out unnamed, so the reply cannot be complete.
    let restricted = source.is_none() && visible.len() < total;
    if let Some(id) = source {
        visible.retain(|r| r.0 == id);
        if visible.is_empty() {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!("workspace {ws_id} has no source {id}"),
            ));
        }
    }
    let versions = store::versions();
    let mut sources = Vec::new();
    for (id, path, membership, gen_id, gen_membership, snapshot) in visible {
        let refresh = json!({"tool": "index", "arguments": {"action": "refresh",
            "workspaceId": ws_id, "sourceId": id, "scope": req.scope}});
        let current = gen_id.filter(|_| gen_membership.as_deref() == Some(membership.as_str()));
        let (state, step) = match current {
            None => {
                let job: Option<(String, String, i64)> = tx
                    .query_row(
                        "SELECT job_id, owner, attempt FROM jobs WHERE source_id = ?1
                         AND scope = ?2 AND state IN ('queued', 'running')",
                        [&id, &req.scope],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(sql)?;
                let job = job.filter(|(job, owner, attempt)| {
                    jobs::job_alive(&ws.held.dir, owner, job, *attempt)
                });
                match (job, gen_id) {
                    (Some((job, ..)), _) => (
                        "pending",
                        json!({"tool": "jobs", "arguments": {"action": "status", "jobId": job}}),
                    ),
                    (None, None) => ("not_prepared", refresh),
                    (None, Some(_)) => ("stale", refresh),
                }
            }
            Some(gen_id) => {
                let (built, bytes, mtime): (String, Option<i64>, Option<i64>) = tx
                    .query_row(
                        "SELECT g.versions, n.bytes, n.mtime_ns FROM generations g
                         LEFT JOIN snapshots n ON n.source_id = g.source_id
                         AND n.snapshot_id = g.snapshot_id WHERE g.gen = ?1",
                        [gen_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .map_err(sql)?;
                let stamp = readable(Path::new(&path))
                    .and_then(|p| crate::search::stamp(&p).map_err(Into::into));
                match stamp {
                    Err(_) => ("unreadable", Json::Null),
                    Ok(_) if built != versions => (
                        "incompatible",
                        json!({"tool": "index", "arguments": {"action": "rebuild",
                            "workspaceId": ws_id}}),
                    ),
                    Ok((mtime_at, len)) => {
                        let ns = mtime_at
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map_or(0, |d| d.as_nanos() as i64);
                        if Some(len as i64) != bytes || Some(ns) != mtime {
                            ("stale", refresh)
                        } else {
                            ("ready", Json::Null)
                        }
                    }
                }
            }
        };
        sources.push(Source {
            id,
            snapshot: snapshot.filter(|_| state == "ready"),
            state,
            gen_id: current.filter(|_| state == "ready"),
            step,
        });
    }
    // The cursor binds the question and every source it covered, so a changed reference, scope,
    // filter, figlens, visibility or generation cannot continue it.
    let fingerprint = {
        let bound: Vec<Json> = sources
            .iter()
            .map(|s| json!([s.id, s.state, s.gen_id]))
            .collect();
        let asked = json!([
            "l1",
            req.target(Some(&req.namespace)),
            req.scope,
            req.include_hidden,
            source,
            versions,
            restricted,
            bound
        ]);
        crate::search::hex(&Sha256::digest(asked.to_string().as_bytes()))[..16].to_owned()
    };
    let mut after = (0usize, -1i64, -1i64);
    if let Some(c) = &req.cursor {
        let bad = || {
            fail(
                ErrorKind::InvalidArguments,
                "invalid usages cursor; pass nextCursor from the previous reply",
            )
        };
        let parts: Vec<&str> = c.split('.').collect();
        let ["l1", fp, at, rid, seq] = parts[..] else {
            return Err(bad());
        };
        if fp != fingerprint {
            return Err(FigError::new(
                ErrorKind::CursorStale,
                "the reference, its scope or the workspace's sources or indexes changed since this cursor; query usages again without cursor",
            )
            .into());
        }
        after = (
            at.parse().map_err(|_| bad())?,
            rid.parse().map_err(|_| bad())?,
            seq.parse().map_err(|_| bad())?,
        );
    }
    let exact = req.target(Some(&req.namespace));
    let unknown = req.target(None);
    let mut results = Vec::new();
    let (mut scanned, mut copied) = (0usize, 0usize);
    let (mut more, mut last) = (false, None);
    let (mut hidden, mut partial, mut unnamed) = (Vec::new(), Vec::new(), Vec::new());
    let mut uses = tx
        .prepare(
            "SELECT e.rid, e.seq, e.state, e.evidence, r.node_id, r.name, r.kind, r.hidden,
             length(e.evidence) + length(r.node_id) + length(r.name) + length(r.kind)
             FROM edges e INDEXED BY edges_target JOIN records r ON r.gen = e.gen AND r.rid = e.rid
             WHERE e.gen = ?1 AND e.target = ?2 AND e.relation = ?3 AND (e.rid, e.seq) > (?4, ?5)
             ORDER BY e.rid, e.seq",
        )
        .map_err(sql)?;
    for (i, s) in sources.iter().enumerate() {
        let Some(gen_id) = s.gen_id else { continue };
        // A stopped build, or binding fields too large to read, can hide uses; a copy with no
        // namespace may name this one. Each is an indexed probe, made on every page.
        let (complete, unread, nameless): (bool, i64, bool) = tx
            .query_row(
                "SELECT complete AND stop IS NULL, coalesce(json_extract(info, '$.unreadFields'), 0),
                 EXISTS (SELECT 1 FROM edges INDEXED BY edges_target WHERE gen = ?1 AND target = ?2)
                 FROM generations WHERE gen = ?1",
                params![gen_id, unknown],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(sql)?;
        if !complete || unread > 0 {
            partial.push(s.id.clone());
        }
        if nameless {
            unnamed.push(s.id.clone());
        }
        if i < after.0 || more {
            continue;
        }
        let (rid, seq) = if i == after.0 {
            (after.1, after.2)
        } else {
            (-1, -1)
        };
        let mut rows = uses
            .query(params![gen_id, exact, LIBRARY, rid, seq])
            .map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            scanned += 1;
            if scanned > SCAN {
                return Err(scan_limit());
            }
            if row.get::<_, bool>(7).map_err(sql)? && !req.include_hidden {
                if hidden.last() != Some(&s.id) {
                    hidden.push(s.id.clone());
                }
                continue;
            }
            let size = 128 + row.get::<_, i64>(8).map_err(sql)? as usize;
            if results.len() == req.limit || copied + size > PAGE_BYTES {
                more = true;
                break;
            }
            copied += size;
            let (r, q): (i64, i64) = (row.get(0).map_err(sql)?, row.get(1).map_err(sql)?);
            let evidence: String = row.get(3).map_err(sql)?;
            results.push(json!({
                "ref": {"sourceId": s.id, "snapshotId": s.snapshot,
                    "nodeId": row.get::<_, String>(4).map_err(sql)?},
                "name": row.get::<_, String>(5).map_err(sql)?,
                "type": row.get::<_, String>(6).map_err(sql)?,
                "relation": LIBRARY, "basis": "cached_copy",
                "state": row.get::<_, String>(2).map_err(sql)?,
                "evidence": serde_json::from_str::<Json>(&evidence).unwrap_or(json!(evidence)),
            }));
            last = Some((i, r, q));
        }
    }
    drop(uses);
    let waiting: Vec<&str> = sources
        .iter()
        .filter(|s| s.state != "ready")
        .map(|s| s.id.as_str())
        .collect();
    let mut reasons = Vec::new();
    for (code, ids) in [
        (
            "source_not_ready",
            waiting.iter().map(|s| s.to_string()).collect(),
        ),
        ("index_partial", partial),
        ("namespace_unknown", unnamed),
        ("hidden_excluded", hidden),
    ] {
        if !ids.is_empty() {
            reasons.push(json!({"code": code, "sourceIds": ids}));
        }
    }
    if restricted {
        reasons.push(json!({"code": "authorization_restricted"}));
    }
    // Hidden matches are left out as asked, which is not incompleteness.
    let complete = reasons.iter().all(|r| r["code"] == "hidden_excluded");
    let coverage_sources: Vec<Json> = sources
        .iter()
        .map(|s| {
            let mut c = json!({"sourceId": s.id, "state": s.state, "snapshotId": s.snapshot});
            if !s.step.is_null() {
                c["nextSteps"] = json!([s.step]);
            }
            c
        })
        .collect();
    Ok(json!({
        "workspaceId": ws_id,
        "library": {"kind": req.kind, "namespace": req.namespace, "key": req.key},
        "definition": {"state": "unresolved", "reason": "library_source_unproven"},
        "scope": {"nodeId": req.scope},
        "results": results,
        "coverage": {"complete": complete, "sources": coverage_sources, "reasons": reasons},
        "hasMore": more,
        "nextCursor": match (more, last) {
            (true, Some((i, r, q))) => json!(format!("l1.{fingerprint}.{i}.{r}.{q}")),
            _ => Json::Null,
        },
    }))
}
