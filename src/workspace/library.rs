//! Uses of one library component, style or variable (#93) across every visible source's
//! published index of one scope, as each file's cached copies record the reference. Reads only,
//! like usages: one SQLite read transaction, no parse, no build. The definition is never
//! resolved from the key, since a file's own definitions carry none; a caller may select one by
//! its exact ref, which is checked but stays unverified as the key's definition.

use sha2::{Digest, Sha256};

use rusqlite::params;

use super::*;
use crate::search::usages::{LIBRARY, MAX_LIMIT, SCAN, scan_limit};
use crate::search::{fingerprint, store};

pub const KINDS: [&str; 3] = ["component", "style", "variable"];
/// Serialized bytes of one page's results; a page past it ends early. MCP sends a reply as text
/// and as structuredContent, so two pages must fit its 8 MiB cap.
const PAGE_BYTES: usize = 3 * 1024 * 1024;

/// Which layers use the library reference `(kind, namespace, key)`.
#[derive(Clone, Debug)]
pub struct Request {
    pub kind: String,
    /// `None` asks for the copies that record no namespace, and never implies one.
    pub namespace: Option<String>,
    pub key: String,
    /// The caller's exact ref of the definition to compare against, checked on every page.
    pub definition: Option<DefinitionRef>,
    pub scope: String,
    pub include_hidden: bool,
    pub limit: usize,
    pub cursor: Option<String>,
}

/// A definition the caller selects: one node of one source's snapshot.
#[derive(Clone, Debug)]
pub struct DefinitionRef {
    pub source_id: String,
    pub snapshot_id: String,
    pub node_id: String,
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
        if self.scope.is_empty() || self.scope.len() > store::ID_BYTES {
            return Err(invalid(format!(
                "usages scope takes 1 to {} bytes",
                store::ID_BYTES
            )));
        }
        if self.key.is_empty() || self.namespace.as_deref() == Some("") {
            return Err(invalid(
                "library needs a key, and a namespace when one is given".into(),
            ));
        }
        if let Some(d) = &self.definition
            && [&d.source_id, &d.snapshot_id, &d.node_id]
                .iter()
                .any(|v| v.is_empty() || v.len() > store::ID_BYTES)
        {
            return Err(invalid(format!(
                "library definition takes sourceId, snapshotId and nodeId of 1 to {} bytes",
                store::ID_BYTES
            )));
        }
        if self.target(self.namespace.as_deref()).len() > store::ID_BYTES {
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
        let mut asked = json!({"library": {"kind": self.kind, "key": self.key},
            "scope": self.scope, "limit": self.limit});
        if let Some(ns) = &self.namespace {
            asked["library"]["namespace"] = json!(ns);
        }
        if let Some(d) = &self.definition {
            asked["library"]["definition"] = json!({"sourceId": d.source_id,
                "snapshotId": d.snapshot_id, "nodeId": d.node_id});
        }
        if self.include_hidden {
            asked["includeHidden"] = json!(true);
        }
        asked
    }
}

/// A stored page: id, name and whether it is internal.
type Page = (String, String, bool);

/// One visible source and where its published scope stands.
struct Source {
    id: String,
    path: String,
    snapshot: Option<String>,
    freshness: Option<Json>,
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
    ask(storage, select, source, None, req)
}

/// The same question over the fixed set `only`, as a saved library question reads it: a member
/// outside the caller's roots is left out unnamed (`authorization_restricted`), never a source
/// outside the set.
pub(crate) fn library_in(
    storage: &Storage,
    select: &Select,
    only: &[String],
    req: &Request,
) -> Result<Json> {
    ask(storage, select, None, Some(only), req)
}

fn ask(
    storage: &Storage,
    select: &Select,
    source: Option<&str>,
    only: Option<&[String]>,
    req: &Request,
) -> Result<Json> {
    req.validate()?;
    let (_, ws) = opened(storage, select, false)?;
    let ws_id = ws.held.id.clone();
    // Schema 8 introduced fingerprints; later migrations must not refuse it.
    if ws.version < 8 {
        return Err(FigError::new(
            ErrorKind::IndexIncompatible,
            format!("workspace {ws_id} has no relations index of this figlens yet; rebuild it, then query usages"),
        )
        .with_recovery(json!([{"action": "rebuild", "requires": "none", "tool": "index",
            "arguments": {"action": "rebuild", "workspaceId": ws_id}}]))
        .into());
    }
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let mut rows = super::readiness::rows(&tx, &req.scope)?;
    if let Some(only) = only {
        rows.retain(|r| only.contains(&r.id));
    }
    // A file whose source moved no longer names this workspace; a library question reads every
    // source, so it is asked again by workspaceId, never of the moved file.
    // Only the relocation error carries a recovery; any other one stands as it is.
    if let Err(e) = super::query::relocated(
        storage,
        select,
        rows.iter().map(|r| r.path.as_str()),
        "usages",
        json!({}),
    ) {
        if e.downcast_ref::<FigError>()
            .and_then(FigError::recovery)
            .is_none()
        {
            return Err(e);
        }
        let mut arguments = req.arguments();
        arguments["workspaceId"] = json!(ws_id);
        if let Some(source) = source {
            arguments["sourceId"] = json!(source);
        }
        return Err(FigError::new(
            ErrorKind::InvalidArguments,
            "that file is no longer this workspace's source: it was relocated; ask by workspaceId",
        )
        .with_recovery(json!([{"action": "usages_workspace", "requires": "none",
            "tool": "usages", "arguments": arguments}]))
        .into());
    }
    // Invisible sources are dropped before anything about them is read or named; visibility is
    // decided again on every page, and the cursor binds the sources it saw.
    let total = rows.len();
    // A selected definition's source this caller may not see reads as unknown: nothing about
    // it, not even that it exists, is told.
    let wanted = req.definition.as_ref().map(|d| d.source_id.as_str());
    let mut visible: Vec<_> = rows
        .into_iter()
        .filter(|r| storage.sees(Path::new(&r.path)))
        .collect();
    // Sources this caller may not see are left out unnamed, so the reply cannot be complete.
    let restricted = source.is_none() && visible.len() < total;
    if let Some(id) = source {
        if !visible.iter().any(|r| r.id == id) {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!("workspace {ws_id} has no source {id}"),
            ));
        }
        // The definition's source keeps its readiness check, though its uses are not listed.
        visible.retain(|r| r.id == id || Some(r.id.as_str()) == wanted);
    }
    let versions = store::versions();
    let mut sources = Vec::new();
    let now = now_ms();
    for row in visible {
        let refresh = json!({"tool": "index", "arguments": {"action": "refresh",
            "workspaceId": ws_id, "sourceId": row.id, "scope": req.scope}});
        use super::readiness::State;
        let checked = row.check(&tx, &ws.held.dir, &req.scope, &versions, false)?;
        let current = row.current();
        let state = checked.state.label();
        let step = match checked.state {
            State::Pending => json!({"tool": "jobs", "arguments": {"action": "status",
                "jobId": checked.job.expect("pending job")["jobId"]}}),
            State::NotPrepared | State::StaleMembership | State::StaleStamp => refresh,
            State::Incompatible => {
                json!({"tool": "index", "arguments": {"action": "rebuild", "workspaceId": ws_id}})
            }
            State::Unreadable | State::Ready => Json::Null,
        };
        // As search: the snapshot was verified then, and its stamp was checked now; no hash now.
        let freshness = row.snapshot.as_ref().map(|_| {
            json!({"verifiedAtMs": row.verified_ms, "stampCheckedAtMs": now,
                "basis": "size and modification time equal to the verified snapshot"})
        });
        sources.push(Source {
            id: row.id,
            path: row.path,
            snapshot: row.snapshot.filter(|_| state == "ready"),
            freshness: freshness.filter(|_| state == "ready"),
            state,
            gen_id: current.filter(|_| state == "ready"),
            step,
        });
    }
    let (definition, definition_gen) = selected(&tx, req, &sources)?;
    if let Some(id) = source {
        sources.retain(|s| s.id == id);
    }
    // Each paged generation with its source: every listed one, then the definition's.
    let mut generations: Vec<(&str, Option<i64>)> =
        sources.iter().map(|s| (s.id.as_str(), s.gen_id)).collect();
    if let Some(d) = &req.definition {
        generations.push((d.source_id.as_str(), definition_gen));
    }
    // The cursor binds the question and every source it covered, so a changed reference, scope,
    // filter, figlens, visibility or readiness cannot continue it, nor a definition now selected
    // otherwise. It also carries each source's generation and the definition's, so a reindexed
    // source is stale and a deleted generation expired.
    let fingerprint = {
        let bound: Vec<Json> = sources.iter().map(|s| json!([s.id, s.state])).collect();
        let asked = json!([
            "l2",
            req.target(req.namespace.as_deref()),
            req.arguments()["library"]["definition"],
            definition,
            req.scope,
            req.include_hidden,
            source,
            only,
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
        // One generation id per source at most; checked before splitting it.
        if c.len() > 80 + 21 * generations.len() {
            return Err(bad());
        }
        let parts: Vec<&str> = c.splitn(7, '.').collect();
        let ["l2", fp, at, rid, seq, paged, issued] = parts[..] else {
            return Err(bad());
        };
        // The lease ends before any binding is compared, as for a search cursor.
        if crate::search::cursor_lease_ended(issued.parse().map_err(|_| bad())?, now_ms()) {
            return Err(FigError::new(
                ErrorKind::CursorExpired,
                "this usages cursor's lease ended; query usages again without cursor",
            )
            .into());
        }
        // Changed filters, visibility or readiness are stale before any generation is looked up.
        if fp != fingerprint {
            return Err(FigError::new(
                ErrorKind::CursorStale,
                "the reference, its scope or the workspace's sources or indexes changed since this cursor; query usages again without cursor",
            )
            .into());
        }
        let paged: Vec<&str> = paged.split('-').collect();
        if paged.len() != generations.len() {
            return Err(bad());
        }
        let (mut changed, mut gone) = (false, false);
        for (&(id, gen_id), g) in generations.iter().zip(paged) {
            let old = match g {
                "n" => None,
                g => Some(g.parse::<i64>().map_err(|_| bad())?),
            };
            if old == gen_id {
                continue;
            }
            changed = true;
            // Only this source's own generation is looked up, never another source's.
            if let Some(old) = old {
                gone |= tx
                    .query_row(
                        "SELECT 1 FROM generations WHERE gen = ?1 AND source_id = ?2",
                        params![old, id],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(sql)?
                    .is_none();
            }
        }
        if changed {
            let (kind, why) = if gone {
                (
                    ErrorKind::CursorExpired,
                    "an index this cursor paged was deleted",
                )
            } else {
                (
                    ErrorKind::CursorStale,
                    "a source this cursor paged was reindexed",
                )
            };
            return Err(
                FigError::new(kind, format!("{why}; query usages again without cursor")).into(),
            );
        }
        after = (
            at.parse().map_err(|_| bad())?,
            rid.parse().map_err(|_| bad())?,
            seq.parse().map_err(|_| bad())?,
        );
    }
    // Coverage is sent on every page, and reasons repeat source ids at most four times; both are
    // charged before any use, so a workspace of many sources cannot push a page past its budget.
    let coverage_sources: Vec<Json> = sources
        .iter()
        .map(|s| {
            let mut c = json!({"sourceId": s.id, "path": s.path, "state": s.state, "snapshotId": s.snapshot});
            if let Some(f) = &s.freshness {
                c["freshness"] = f.clone();
            }
            if !s.step.is_null() {
                c["nextSteps"] = json!([s.step]);
            }
            c
        })
        .collect();
    let ids: usize = sources.iter().map(|s| s.id.len() + 3).sum();
    let mut copied = serde_json::to_vec(&coverage_sources)
        .map_err(|e| e.to_string())?
        .len()
        + 4 * ids;
    if copied > PAGE_BYTES {
        return Err(fail(
            ErrorKind::ResponseBudget,
            format!(
                "the {} visible sources' coverage exceeds the {PAGE_BYTES}-byte page; ask with sourceId",
                sources.len()
            ),
        ));
    }
    // A selected definition's fingerprint, captured by its source's build; `Some(None)` when that
    // generation holds none, so no use is compared. Its reasons are charged with the coverage.
    let baseline = match (&req.definition, definition_gen, &definition["state"]) {
        (Some(d), Some(gen_id), state) if state == "selected" => {
            copied += 256 + d.source_id.len() + req.scope.len() + ws_id.len();
            Some(store::print(&tx, gen_id, &d.node_id)?)
        }
        _ => None,
    };
    let (mut mismatched, mut uncompared) = (0usize, 0usize);
    let exact = req.target(req.namespace.as_deref());
    let unknown = req.target(None);
    let mut results = Vec::new();
    let mut scanned = 0usize;
    let (mut more, mut last) = (false, None);
    let (mut hidden, mut partial, mut unnamed) = (Vec::new(), Vec::new(), Vec::new());
    let mut uses = tx
        .prepare(
            "SELECT e.rid, e.seq, e.state, e.evidence, r.node_id, r.name, r.kind, r.hidden,
             length(CAST(e.state AS BLOB)) + length(CAST(e.evidence AS BLOB))
             + length(CAST(r.node_id AS BLOB))
             + length(CAST(r.name AS BLOB)) + length(CAST(r.kind AS BLOB)), r.page, r.root
             FROM edges e INDEXED BY edges_target JOIN records r ON r.gen = e.gen AND r.rid = e.rid
             WHERE e.gen = ?1 AND e.target = ?2 AND e.relation = ?3 AND (e.rid, e.seq) > (?4, ?5)
             ORDER BY e.rid, e.seq",
        )
        .map_err(sql)?;
    for (i, s) in sources.iter().enumerate() {
        let Some(gen_id) = s.gen_id else { continue };
        // A stopped build, binding fields too large to read, or a reference too long to keep can
        // hide uses; a copy with no namespace may name this one. Each is an indexed probe, made
        // on every page.
        let (complete, unread, nameless, unkept, need): (bool, i64, bool, bool, i64) = tx
            .query_row(
                "SELECT complete AND stop IS NULL, coalesce(json_extract(info, '$.unreadFields'), 0),
                 EXISTS (SELECT 1 FROM edges INDEXED BY edges_target WHERE gen = ?1 AND target = ?2),
                 EXISTS (SELECT 1 FROM edges INDEXED BY edges_target
                         WHERE gen = ?1 AND target IS NULL AND relation = ?3),
                 coalesce(length(CAST(json_extract(info, '$.pages') AS BLOB)), 0)
                 + coalesce(length(CAST(json_extract(info, '$.roots') AS BLOB)), 0)
                 FROM generations WHERE gen = ?1",
                params![gen_id, unknown, LIBRARY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(sql)?;
        if !complete || unread > 0 || unkept {
            partial.push(s.id.clone());
        }
        if nameless {
            unnamed.push(s.id.clone());
        }
        if i < after.0 || more {
            continue;
        }
        // Its pages and scope roots, read once the source shows a use to place.
        let mut places: Option<(Vec<Page>, Vec<String>)> = None;
        // Uses of one key share few cached copies: each copy's match is read once per page.
        let mut prints: std::collections::HashMap<String, &'static str> = Default::default();
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
            let node: String = row.get(4).map_err(sql)?;
            // The same question narrowed to this source, and the use itself, which inspect reads.
            let too_large = || {
                let mut steps = Vec::new();
                if source.is_none() {
                    let mut arguments = req.arguments();
                    arguments["workspaceId"] = json!(ws_id);
                    arguments["sourceId"] = json!(s.id);
                    steps.push(json!({"action": "usages_source", "requires": "none",
                        "tool": "usages", "arguments": arguments}));
                }
                steps.push(
                    json!({"action": "inspect_use", "requires": "none", "tool": "inspect",
                    "arguments": {"file": s.path, "node": node}}),
                );
                FigError::new(
                    ErrorKind::ResponseBudget,
                    format!(
                        "one library use in source {} exceeds the {PAGE_BYTES}-byte page",
                        s.id
                    ),
                )
                .with_recovery(json!(steps))
            };
            if places.is_none() {
                // The lists' UTF-8 bytes are charged to the page before they are copied or parsed.
                if copied + need as usize > PAGE_BYTES {
                    if results.is_empty() {
                        return Err(too_large().into());
                    }
                    more = true;
                    break;
                }
                copied += need as usize;
                let (pages, roots): (Option<String>, Option<String>) = tx
                    .query_row(
                        "SELECT json_extract(info, '$.pages'), json_extract(info, '$.roots')
                         FROM generations WHERE gen = ?1",
                        [gen_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(sql)?;
                let pages = serde_json::from_str(pages.as_deref().unwrap_or("[]"));
                let roots = serde_json::from_str(roots.as_deref().unwrap_or("[]"));
                places = Some((
                    pages.map_err(|_| store::bad())?,
                    roots.map_err(|_| store::bad())?,
                ));
            }
            let (pages, roots) = places.as_ref().unwrap();
            let at = |col| -> Result<Option<usize>> {
                Ok(row
                    .get::<_, Option<i64>>(col)
                    .map_err(sql)?
                    .map(|v| v as usize))
            };
            let page = match at(9)? {
                Some(p) => Some(pages.get(p).ok_or_else(store::bad)?),
                None => None,
            };
            let root = match at(10)? {
                Some(r) => Some(roots.get(r).ok_or_else(store::bad)?),
                None => None,
            };
            // Raw bytes first, so an oversized row is never copied; then its serialized size.
            let raw = row.get::<_, i64>(8).map_err(sql)? as usize
                + page.map_or(0, |p| p.0.len() + p.1.len())
                + root.map_or(0, String::len);
            // An empty page that cannot take this use refuses it; it never ends without a cursor.
            if results.is_empty() && copied + raw > PAGE_BYTES {
                return Err(too_large().into());
            }
            if results.len() == req.limit || copied + raw > PAGE_BYTES {
                more = true;
                break;
            }
            let (r, q): (i64, i64) = (row.get(0).map_err(sql)?, row.get(1).map_err(sql)?);
            let evidence: String = row.get(3).map_err(sql)?;
            let evidence =
                serde_json::from_str::<Json>(&evidence).unwrap_or_else(|_| json!(evidence));
            // The use's cached copy as this source's build fingerprinted it, never the use's own
            // overrides; only a component use names one copy.
            let matched = match &baseline {
                None => None,
                Some(None) => Some("unknown"),
                Some(Some(d)) => match evidence["copy"]
                    .as_str()
                    .filter(|_| req.kind == "component")
                {
                    Some(copy) => Some(match prints.get(copy) {
                        Some(m) => *m,
                        None => {
                            let m = store::print(&tx, gen_id, copy)?
                                .map_or("unknown", |c| fingerprint::compare(d, &c));
                            prints.insert(copy.to_owned(), m);
                            m
                        }
                    }),
                    None => Some("unknown"),
                },
            };
            let mut item = json!({
                "ref": {"sourceId": s.id, "snapshotId": s.snapshot,
                    "nodeId": node},
                "name": row.get::<_, String>(5).map_err(sql)?,
                "type": row.get::<_, String>(6).map_err(sql)?,
                "relation": LIBRARY, "basis": "cached_copy",
                "state": row.get::<_, String>(2).map_err(sql)?,
                "evidence": evidence,
                "hidden": row.get::<_, bool>(7).map_err(sql)?,
                "internal": page.is_some_and(|p| p.2),
                "page": page.map(|p| json!({"id": p.0, "name": p.1})),
                "scopeRoot": root,
            });
            if let Some(m) = matched {
                item["definitionMatch"] = json!(m);
            }
            let size = serde_json::to_vec(&item).map_err(|e| e.to_string())?.len();
            if copied + size > PAGE_BYTES {
                if results.is_empty() {
                    return Err(too_large().into());
                }
                more = true;
                break;
            }
            copied += size;
            mismatched += usize::from(matched == Some("definition_mismatch"));
            uncompared += usize::from(matches!(baseline, Some(None)));
            results.push(item);
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
        // Asked without a namespace, the copies naming none are the answer, not a gap.
        (
            "namespace_unknown",
            if req.namespace.is_some() {
                unnamed
            } else {
                Vec::new()
            },
        ),
        ("hidden_excluded", hidden),
    ] {
        if !ids.is_empty() {
            reasons.push(json!({"code": code, "sourceIds": ids}));
        }
    }
    if let (Some(None), Some(d), Some(gen_id)) = (&baseline, &req.definition, definition_gen) {
        let mut reason = json!({"code": "definition_not_compared", "count": uncompared});
        // Only a generation built before fingerprints gains one by rebuilding; in a newer one
        // the selected node is not a kind that takes one, and a build its budgets stopped
        // stops again.
        let built: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM fingerprints WHERE gen = ?1)
             OR EXISTS(SELECT 1 FROM generations WHERE gen = ?1 AND stop IS NOT NULL)",
            params![gen_id],
            |r| r.get(0),
        )?;
        if !built {
            reason["nextSteps"] = json!([{"tool": "index", "arguments": {"action": "refresh",
                "workspaceId": ws_id, "sourceId": d.source_id, "scope": req.scope}}]);
        }
        reasons.push(reason);
    }
    if mismatched > 0 {
        reasons.push(json!({"code": "definition_mismatch", "count": mismatched}));
    }
    if restricted {
        reasons.push(json!({"code": "authorization_restricted"}));
    }
    // Copies with no namespace may name any library: they are never this one's complete uses.
    if req.namespace.is_none() {
        reasons.push(json!({"code": "namespace_required"}));
    }
    // Hidden matches are left out as asked, which is not incompleteness.
    // So are differences from a selected definition, which are findings.
    let complete = reasons
        .iter()
        .all(|r| r["code"] == "hidden_excluded" || r["code"] == "definition_mismatch");
    Ok(json!({
        "workspaceId": ws_id,
        "library": {"kind": req.kind, "namespace": req.namespace, "key": req.key},
        "definition": definition,
        "scope": {"nodeId": req.scope},
        "results": results,
        "coverage": {"complete": complete, "sources": coverage_sources, "reasons": reasons},
        "hasMore": more,
        "nextCursor": match (more, last) {
            (true, Some((i, r, q))) => {
                let paged: Vec<String> = generations
                    .iter()
                    .map(|(_, g)| g.map_or("n".into(), |g| g.to_string()))
                    .collect();
                json!(format!("l2.{fingerprint}.{i}.{r}.{q}.{}.{}", paged.join("-"), now_ms()))
            }
            _ => Json::Null,
        },
    }))
}

/// The caller's definition ref as this read sees it, with the generation it was checked in. It
/// is `selected` only when its source is visible and ready at exactly that snapshot and the node
/// is a record of the published scope of the asked kind; otherwise unresolved, never another
/// node by name or id. A selection names the caller's choice; the key's evidence stays unverified.
fn selected(
    tx: &rusqlite::Transaction,
    req: &Request,
    sources: &[Source],
) -> Result<(Json, Option<i64>)> {
    let unresolved = |reason: &str| json!({"state": "unresolved", "reason": reason});
    let Some(d) = &req.definition else {
        let reason = match req.namespace {
            Some(_) => "library_source_unproven",
            None => "namespace_required",
        };
        return Ok((unresolved(reason), None));
    };
    let Some(s) = sources.iter().find(|s| s.id == d.source_id) else {
        return Ok((unresolved("definition_not_found"), None));
    };
    let Some(gen_id) = s.gen_id else {
        return Ok((unresolved("source_not_ready"), None));
    };
    if s.snapshot.as_deref() != Some(d.snapshot_id.as_str()) {
        return Ok((unresolved("definition_snapshot_stale"), Some(gen_id)));
    }
    // The kind is compared in SQLite's buffer, never copied: a stored one may be any size.
    let fits: Option<bool> = tx
        .query_row(
            "SELECT kind FROM records INDEXED BY records_node WHERE gen = ?1 AND node_id = ?2
             ORDER BY rid LIMIT 1",
            params![gen_id, d.node_id],
            |r| {
                let k = r.get_ref(0)?.as_str().unwrap_or("");
                Ok(match req.kind.as_str() {
                    "component" => matches!(k, "COMPONENT" | "COMPONENT_SET"),
                    "variable" => k == "VARIABLE",
                    // A record does not keep whether its node is a style, so that is left
                    // unverified.
                    _ => true,
                })
            },
        )
        .optional()
        .map_err(sql)?;
    let Some(fits) = fits else {
        return Ok((unresolved("definition_not_found"), Some(gen_id)));
    };
    if !fits {
        return Ok((unresolved("definition_kind_mismatch"), Some(gen_id)));
    }
    let mut out = json!({"state": "selected", "basis": "caller_ref",
        "ref": {"sourceId": d.source_id, "snapshotId": d.snapshot_id, "nodeId": d.node_id},
        "keyEvidence": "unverified"});
    if req.kind == "style" {
        out["kindEvidence"] = json!("unverified");
    }
    Ok((out, Some(gen_id)))
}
