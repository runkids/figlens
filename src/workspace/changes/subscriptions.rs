//! Durable change subscriptions: a source, scope or exact node a consumer follows, each with
//! its own checkpoint. Removing one never untracks, deletes history or touches other consumers.

use super::*;

/// The filter semantics a subscription's cursors are bound to; a change invalidates them.
pub(super) const FILTER_VERSION: i64 = 1;
/// Subscriptions and raw checkpoint views one consumer may hold, and the workspace in all.
const PER_CONSUMER: i64 = 64;
const PER_WORKSPACE: i64 = 4096;
/// Most subscriptions in one `watch list` page.
const LIST_PAGE: usize = 64;

pub struct Subscribe<'a> {
    pub consumer: Option<&'a str>,
    pub source: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub node: Option<&'a str>,
    /// A task-owned subscription its owner removes by owner; otherwise it is durable.
    pub owner: Option<&'a str>,
    /// A saved workspace search over the source's scope, with its node types.
    pub query: Option<&'a str>,
    pub types: Vec<String>,
    /// A saved question's fixed source set in place of `source`: `visible` (resolved once, now)
    /// or comma-separated sourceIds.
    pub sources: Option<&'a str>,
    /// A saved library question (#93) over that set; its scope, limit and cursor are unused.
    pub library: Option<crate::workspace::library::Request>,
}

pub(super) struct Sub {
    pub(super) id: String,
    pub(super) consumer: String,
    pub(super) source: String,
    pub(super) scope: Option<String>,
    kind: String,
    target: String,
    anchor: String,
    owner: Option<String>,
    pub(super) after_seq: i64,
    pub(super) floor: i64,
    /// A saved question's fixed source set (`source` is then [`SET`]); `None` when unreadable.
    pub(super) members: Option<Vec<String>>,
    /// Why an authoritative event disabled it, as stored.
    pub(super) disabled: Option<String>,
}

impl Sub {
    pub(super) fn node(&self) -> Option<&str> {
        (self.kind == "node").then_some(self.target.as_str())
    }

    pub(super) fn is_query(&self) -> bool {
        self.kind == "query"
    }

    pub(super) fn is_set(&self) -> bool {
        self.source == SET
    }

    /// The saved search of a query subscription.
    pub(super) fn saved(&self) -> Result<Option<queries::Saved>> {
        (self.kind == "query")
            .then(|| queries::Saved::parse(&self.anchor, self.members.clone()))
            .transpose()
    }
}

/// The `source_id` of a saved question over a fixed source set; never a real sourceId.
pub(crate) const SET: &str = "*";

/// The most a stored set takes; past it, or as anything but an array of at most
/// `query::QUERY_SOURCES` sourceIds, it is unreadable.
const SET_BYTES: usize = 4096;
/// The disabled reason an unreadable set reads with: nothing of its members is emitted.
const UNREADABLE: &str = r#"{"reason":"sources_unreadable"}"#;

/// A stored set as read: `None` when it is unreadable.
pub(super) fn members(text: Option<String>) -> Option<Vec<String>> {
    text.filter(|t| t.len() <= SET_BYTES)
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .filter(|m| m.len() <= query::QUERY_SOURCES)
}

/// SQL: whether column `col` holds a set [`members`] reads.
pub(super) fn readable(col: &str) -> String {
    format!(
        "CASE WHEN length(CAST({col} AS BLOB)) <= {SET_BYTES} AND json_valid({col})
         THEN json_type({col}) = 'array' AND json_array_length({col}) <= {}
         AND NOT EXISTS (SELECT 1 FROM json_each({col}) WHERE type <> 'text') ELSE 0 END",
        query::QUERY_SOURCES
    )
}

/// Whether the caller sees a set: every member still registered (one removed disabled the set
/// and left with its data, so it guards nothing), and an unreadable one (`None`) only when the
/// caller sees every source. [`seen`] is the same rule in SQL.
pub(super) fn sees_set(
    storage: &Storage,
    conn: &Connection,
    members: Option<&[String]>,
) -> Result<bool> {
    // Only the members' rows are read; an unreadable set (null) reads every source.
    let mut stmt = conn
        .prepare(
            "SELECT path FROM sources WHERE ?1 IS NULL
             OR source_id IN (SELECT value FROM json_each(?1))",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map([members.map(|m| json!(m).to_string())], |r| {
            r.get::<_, String>(0)
        })
        .map_err(sql)?;
    for path in rows {
        if !storage.sees(Path::new(&path.map_err(sql)?)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the caller sees a subscription row of `subscriptions`: its source, or a set as
/// [`sees_set`] decides. `?N` is the caller's visible sources as JSON. Before schema 8 no row
/// is a set, and a read never upgrades.
pub(super) fn seen(n: usize, version: i64) -> String {
    let one = format!("subscriptions.source_id IN (SELECT value FROM json_each(?{n}))");
    if version < 8 {
        return format!("({one})");
    }
    format!(
        "({one} OR (subscriptions.source_id = '{SET}' AND NOT EXISTS (SELECT 1 FROM sources x
           WHERE x.source_id NOT IN (SELECT value FROM json_each(?{n}))
           AND CASE WHEN {} THEN x.source_id IN
             (SELECT value FROM json_each(subscriptions.sources)) ELSE 1 END)))",
        readable("subscriptions.sources")
    )
}

fn name(what: &str, value: Option<&str>) -> Result<String> {
    value
        .filter(|c| !c.is_empty() && c.len() <= 128)
        .map(str::to_owned)
        .ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("pass {what}: a name of 1 to 128 bytes"),
            )
        })
}

pub(super) fn unknown(id: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::InvalidArguments,
        format!("no subscription {id} in this workspace"),
    )
}

/// The columns `row` reads; schema 8 added source sets and `disabled`, which an older
/// workspace reads as null, never upgrading.
fn columns(version: i64) -> String {
    let added = match version >= 8 {
        true => format!(
            "CASE WHEN length(CAST(sources AS BLOB)) <= {SET_BYTES} THEN sources END, disabled"
        ),
        false => "NULL, NULL".to_owned(),
    };
    format!(
        "id, consumer, source_id, scope, kind, target, anchor, owner, after_seq, floor, {added}"
    )
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Sub> {
    let scope: String = r.get(3)?;
    let source: String = r.get(2)?;
    let members = members(r.get(10)?);
    // An unreadable set is disabled as read: nothing of it is evaluated, acked or emitted.
    let disabled = r
        .get::<_, Option<String>>(11)?
        .or_else(|| (source == SET && members.is_none()).then(|| UNREADABLE.to_owned()));
    Ok(Sub {
        id: r.get(0)?,
        consumer: r.get(1)?,
        source,
        scope: (!scope.is_empty()).then_some(scope),
        kind: r.get(4)?,
        target: r.get(5)?,
        anchor: r.get(6)?,
        owner: r.get(7)?,
        after_seq: r.get(8)?,
        floor: r.get(9)?,
        members,
        disabled,
    })
}

/// The subscription, reauthorized against the sources this caller may see before anything of
/// it is read out: one on a hidden or removed source is reported as unknown.
pub(super) fn visible(storage: &Storage, ws: &Opened, id: &str) -> Result<Sub> {
    // Schema 6 introduced subscriptions; a read never upgrades.
    if ws.version < 6 {
        return Err(unknown(id));
    }
    let sub = ws
        .conn
        .query_row(
            &format!(
                "SELECT {} FROM subscriptions WHERE id = ?1",
                columns(ws.version)
            ),
            [id],
            row,
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| unknown(id))?;
    if !sub.is_set() {
        return match registry::visible_path(storage, &ws.conn, &sub.source)? {
            Some(_) => Ok(sub),
            None => Err(unknown(id)),
        };
    }
    // A set is seen only whole: nothing of its evaluations is read otherwise.
    match sees_set(storage, &ws.conn, sub.members.as_deref())? {
        true => Ok(sub),
        false => Err(unknown(id)),
    }
}

/// The node's kind in the source's published generations, or why it cannot be followed: never
/// a same-name replacement.
fn target(
    conn: &Connection,
    source: &str,
    scope: &str,
    node: &str,
) -> Result<std::result::Result<String, &'static str>> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT r.kind FROM published p JOIN records r ON r.gen = p.gen
             AND r.node_id = ?2 WHERE p.source_id = ?1 AND (?3 = '' OR p.scope = ?3) LIMIT 2",
        )
        .map_err(sql)?;
    let kinds = stmt
        .query_map([source, node, scope], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(match kinds.as_slice() {
        [] => Err("target_missing"),
        [kind] => Ok(kind.clone()),
        _ => Err("identity_uncertain"),
    })
}

/// What a subscription follows and whether its target still holds.
pub(super) fn describe(conn: &Connection, sub: &Sub) -> Result<Json> {
    let scope = sub.scope.as_deref().unwrap_or("");
    let state = match sub.node() {
        None if sub.disabled.is_some() => "disabled",
        None => "active",
        Some(node) => match target(conn, &sub.source, scope, node)? {
            Ok(kind) if kind == sub.anchor => "active",
            // The id now names another kind of layer: never followed as the old one.
            Ok(_) => "identity_uncertain",
            Err(state) => state,
        },
    };
    let mut out = json!({"subscriptionId": sub.id, "consumer": sub.consumer,
        "sourceId": sub.source, "kind": sub.kind, "state": state,
        "lifetime": if sub.owner.is_some() { "temporary" } else { "durable" },
        "filterVersion": FILTER_VERSION});
    if let Some(scope) = &sub.scope {
        out["scope"] = json!(scope);
    }
    if let Some(node) = sub.node() {
        out["node"] = json!(node);
        out["nodeKind"] = json!(sub.anchor);
    }
    if let Some(saved) = sub.saved()? {
        match &saved.library {
            Some(l) => {
                out["library"] = json!({"kind": l.kind, "namespace": l.namespace, "key": l.key});
                out["definition"] = l.definition.as_ref().map_or(Json::Null, |d| {
                    json!({"sourceId": d.source_id, "nodeId": d.node_id,
                        "baselineSnapshotId": d.snapshot_id})
                });
            }
            None => {
                out["query"] = json!(saved.query);
                out["types"] = json!(saved.types);
            }
        }
        out["topK"] = json!(queries::TOP_K);
    }
    if sub.is_set() {
        out["sourceId"] = Json::Null;
        // Only members still registered, which the caller sees: a removed one's id is not
        // the caller's to learn.
        out["sources"] = match &sub.members {
            None => Json::Null,
            Some(m) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT value FROM json_each(?1)
                         WHERE value IN (SELECT source_id FROM sources) ORDER BY key",
                    )
                    .map_err(sql)?;
                let kept = stmt
                    .query_map([json!(m).to_string()], |r| r.get::<_, String>(0))
                    .map_err(sql)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(sql)?;
                json!(kept)
            }
        };
    }
    if let Some(why) = &sub.disabled {
        out["disabled"] = serde_json::from_str(why).unwrap_or(Json::Null);
    }
    Ok(out)
}

/// Refuses a new checkpoint row past the per-consumer or workspace allowance. Nothing is ever
/// evicted: the reply names how to remove rows instead.
pub(super) fn admit(conn: &Connection, ws: &str, consumer: &str) -> Result<()> {
    let (mine, all): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM consumers WHERE consumer = ?1)
             + (SELECT COUNT(*) FROM subscriptions WHERE consumer = ?1),
             (SELECT COUNT(*) FROM consumers) + (SELECT COUNT(*) FROM subscriptions)",
            [consumer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    let reason = match () {
        _ if mine >= PER_CONSUMER => format!(
            "consumer {consumer} already holds {PER_CONSUMER} subscriptions and checkpoints"
        ),
        _ if all >= PER_WORKSPACE => {
            format!("workspace {ws} already holds {PER_WORKSPACE} subscriptions and checkpoints")
        }
        _ => return Ok(()),
    };
    Err(FigError::new(
        ErrorKind::WorkspaceBusy,
        format!("{reason}; unsubscribe or forget ones no longer read"),
    )
    .with_recovery(
        json!([{"action": "list", "requires": "none", "tool": "watch",
        "arguments": {"action": "list", "workspaceId": ws, "consumer": consumer}},
        {"action": "unsubscribe", "requires": "write", "tool": "watch",
        "arguments": {"action": "unsubscribe", "workspaceId": ws, "subscriptionId": "<id>"}},
        {"action": "forget", "requires": "write", "tool": "watch",
        "arguments": {"action": "forget", "workspaceId": ws, "consumer": consumer}}]),
    )
    .into())
}

/// Starts following from now: the checkpoint is the newest revision this view can see, never
/// a hidden source's. A repeated request returns the existing subscription.
pub fn subscribe(storage: &Storage, select: &Select, req: &Subscribe) -> Result<Json> {
    let consumer = name("consumer", req.consumer)?;
    let invalid = |m: &str| fail(ErrorKind::InvalidArguments, m);
    if req.query.is_some() && req.library.is_some() {
        return Err(invalid("pass query or library, not both"));
    }
    if req.sources.is_some() && req.query.is_none() && req.library.is_none() {
        return Err(invalid(
            "sources applies to a saved query or library question; follow revisions by sourceId",
        ));
    }
    let source = match (req.source, req.sources) {
        (Some(_), Some(_)) => return Err(invalid("pass sourceId or sources, not both")),
        (None, None) => {
            return Err(invalid(
                "pass sourceId: subscriptions follow one source, or sources for a saved question",
            ));
        }
        (source, _) => source.unwrap_or(SET),
    };
    // A listed set is bounded before any of its names is resolved.
    if let Some(list) = req.sources.filter(|l| *l != "visible")
        && (list.len() > SET_BYTES || list.split(',').count() > query::QUERY_SOURCES)
    {
        return Err(invalid(&format!(
            "sources lists at most {} sourceIds in {SET_BYTES} bytes",
            query::QUERY_SOURCES
        )));
    }
    let owner = req.owner.map(|o| name("owner", Some(o))).transpose()?;
    let node = req.node.map(crate::normalize_id);
    if let Some(l) = &req.library {
        l.validate()?;
    }
    let saved = match (req.query, &req.library) {
        (Some(q), _) => {
            let mut types = req.types.clone();
            types.sort();
            types.dedup();
            Some(queries::Saved {
                query: q.to_owned(),
                types,
                library: None,
                members: None,
            })
        }
        (None, Some(l)) => Some(queries::Saved {
            query: String::new(),
            types: Vec::new(),
            library: Some(l.clone()),
            members: None,
        }),
        (None, None) => None,
    };
    // A saved query searches one published scope, the whole file unless named.
    let scope = match (&saved, req.scope) {
        (Some(_), None) => "0:0".to_owned(),
        (_, scope) => scope.map(crate::normalize_id).unwrap_or_default(),
    };
    if saved.is_some() && node.is_some() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "pass query or library, or node, not both",
        ));
    }
    if req.query.is_none() && !req.types.is_empty() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "type filters a saved query; pass query with it",
        ));
    }
    let (_, ws) = opened(storage, select, true)?;
    // The write lock is taken first, so visibility and the write share one snapshot.
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    let ws_id = ws.held.id.clone();
    // A library question always reads a set, one source's included; the set is fixed here and
    // never gains a source added later.
    let members = match (req.sources, &req.library) {
        (None, None) => {
            jobs::chosen(storage, &ws, Some(source))?;
            None
        }
        (None, Some(_)) => Some(jobs::chosen(storage, &ws, Some(source))?),
        (Some("visible"), _) => Some(jobs::chosen(storage, &ws, None)?),
        (Some(list), _) => {
            let mut out = Vec::new();
            for id in list.split(',').map(str::trim) {
                out.extend(jobs::chosen(storage, &ws, Some(id))?);
            }
            Some(out)
        }
    };
    let saved = saved.map(|mut s| {
        s.members = members.map(|mut m| {
            m.sort();
            m.dedup();
            m
        });
        s
    });
    let members = saved.as_ref().and_then(|s| s.members.clone());
    let source = if members.is_some() { SET } else { source };
    match members.as_ref().map(Vec::len) {
        Some(0) => return Err(invalid("sources names no visible source")),
        Some(n) if n > query::QUERY_SOURCES => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!(
                    "sources names {n} sources; a saved question reads at most {}, so pass them as a list",
                    query::QUERY_SOURCES
                ),
            ));
        }
        _ => {}
    }
    if let Some(saved) = &saved {
        queries::definition_holds(&tx, saved, &scope)?;
    }
    let canonical = saved.as_ref().map(|s| s.canonical(&scope)).transpose()?;
    let (kind, target_id, anchor) = match &node {
        None if let (Some(saved), Some(canonical)) = (&saved, &canonical) => {
            ("query", canonical.clone(), saved.anchor())
        }
        Some(node) => match target(&tx, source, &scope, node)? {
            Ok(kind) => ("node", node.clone(), kind),
            Err(state) => {
                return Err(fail(
                    ErrorKind::NodeNotFound,
                    format!(
                        "{state}: node {node} is not one exact layer in source {source}'s published index"
                    ),
                ));
            }
        },
        None if scope.is_empty() => ("source", String::new(), String::new()),
        None => ("scope", String::new(), String::new()),
    };
    let existing: Option<(String, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT id, owner, disabled FROM subscriptions WHERE consumer = ?1 AND source_id = ?2
             AND scope = ?3 AND kind = ?4 AND target = ?5",
            params![consumer, source, scope, kind, target_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    // A disabled subscription never turns back into a fresh baseline, nor its old cursors.
    if let Some((id, _, Some(_))) = &existing {
        return Err(FigError::new(
            ErrorKind::InvalidArguments,
            format!("subscription {id} is disabled; unsubscribe it, then subscribe again"),
        )
        .with_recovery(
            json!([{"action": "unsubscribe", "requires": "write", "tool": "watch",
            "arguments": {"action": "unsubscribe", "workspaceId": ws_id, "subscriptionId": id}}]),
        )
        .into());
    }
    // Returning another owner's row would let either owner's cleanup remove the other's follow.
    if existing.as_ref().is_some_and(|(_, o, _)| *o != owner) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "consumer {consumer} already follows this target under another owner or lifetime; use another consumer name"
            ),
        ));
    }
    let existing = existing.map(|(id, ..)| id);
    let created = existing.is_none();
    let id =
        match existing {
            Some(id) => id,
            None => {
                admit(&tx, &ws_id, &consumer)?;
                let id: String = tx
                    .query_row("SELECT 'sub_' || lower(hex(randomblob(8)))", [], |r| {
                        r.get(0)
                    })
                    .map_err(sql)?;
                tx.execute(
                "INSERT INTO subscriptions (id, consumer, source_id, scope, kind, target, anchor,
                 lifetime, owner, filter_version, after_seq, floor, created_ms, read_ms, sources)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 CASE WHEN ?5 = 'query' THEN 0 ELSE (SELECT IFNULL(MAX(seq), 0) FROM revisions
                 WHERE source_id = ?3 AND (?4 = '' OR scope = ?4)) END, ?11, ?12, ?12, ?13)",
                params![id, consumer, source, scope, kind, target_id, anchor,
                    if owner.is_some() { "temporary" } else { "durable" }, owner,
                    FILTER_VERSION, floor(&tx)?, now_ms(),
                    members.as_ref().map(|m| json!(m).to_string())],
            )
            .map_err(sql)?;
                id
            }
        };
    // The reply is read inside the write, so it describes only what this caller was
    // authorized to see when it subscribed.
    jobs::hold("subscribe");
    let v = view(
        storage,
        &ws,
        &Request {
            subscription: Some(&id),
            ..Default::default()
        },
    )?;
    let sub = v.sub.as_ref().ok_or_else(|| unknown(&id))?;
    let (tracked, _) = tracking(&tx, &v)?;
    // A saved query's checkpoint sits before its first evaluation, which `changes` then reads.
    let checkpoint = match kind {
        "query" => queries::held(&tx, &v, &sub.id, sub.after_seq)?,
        _ => held_cursor(
            &tx,
            &v,
            sub.after_seq,
            &marks(&tx, &v, Some(sub.floor))?.receipt,
        )?,
    };
    let mut reply = json!({"workspaceId": v.workspace, "subscription": describe(&tx, sub)?,
        "created": created,
        // An existing subscription resumes at its own checkpoint, not the tail.
        "initialPosition": match (created, kind) {
            (false, _) => "existing",
            (true, "query") => "initial_evaluation",
            _ => "tail",
        },
        "tracking": tracked, "checkpoint": checkpoint});
    tx.commit().map_err(sql)?;
    // Only a new one is evaluated here, outside that write and under its own fences; a repeat
    // reads its stored evaluations with `changes`.
    if created && kind == "query" {
        drop(ws);
        // The subscription is already written: a failed first evaluation is reported, and
        // `watch evaluate` retries it.
        reply["evaluation"] = match queries::evaluate(storage, select, Some(&id)) {
            Ok(mut out) => out["evaluation"].take(),
            Err(e) => {
                let mut out = queries::pending("evaluation_failed", &v.workspace, &id);
                out["state"] = json!("unavailable");
                out["message"] = json!(e.to_string());
                out
            }
        };
    }
    Ok(reply)
}

/// The caller's visible subscriptions, one page at a time in id order.
pub fn list(
    storage: &Storage,
    select: &Select,
    consumer: Option<&str>,
    after: Option<&str>,
) -> Result<Json> {
    let (_, ws) = opened(storage, select, false)?;
    // Schema 6 introduced subscriptions; a read never upgrades.
    if ws.version < 6 {
        return Ok(json!({"workspaceId": ws.held.id, "subscriptions": [], "next": null}));
    }
    // Visible sources and the rows come from one snapshot.
    let tx = ws.conn.unchecked_transaction().map_err(sql)?;
    let sources = jobs::chosen(storage, &ws, None)?;
    let mut stmt = tx
        .prepare(&format!(
            "SELECT {} FROM subscriptions WHERE (?1 IS NULL OR consumer = ?1)
             AND id > ?2 AND {} ORDER BY id LIMIT ?4",
            columns(ws.version),
            seen(3, ws.version)
        ))
        .map_err(sql)?;
    let subs = stmt
        .query_map(
            params![
                consumer,
                after.unwrap_or(""),
                json!(sources).to_string(),
                LIST_PAGE as i64 + 1
            ],
            row,
        )
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let page = subs
        .iter()
        .take(LIST_PAGE)
        .map(|s| describe(&tx, s))
        .collect::<Result<Vec<_>>>()?;
    let next = (subs.len() > LIST_PAGE).then(|| subs[LIST_PAGE - 1].id.clone());
    Ok(json!({"workspaceId": ws.held.id, "subscriptions": page, "next": next}))
}

/// Removes one subscription and its checkpoint, or every temporary one `owner` created for
/// `consumer`. Tracking, history and other consumers stay.
pub fn unsubscribe(
    storage: &Storage,
    select: &Select,
    id: Option<&str>,
    consumer: Option<&str>,
    owner: Option<&str>,
) -> Result<Json> {
    let (_, ws) = opened(storage, select, true)?;
    // The write lock is taken first, so visibility and the write share one snapshot.
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    let sources = jobs::chosen(storage, &ws, None)?;
    let ws_id = ws.held.id.clone();
    let removed = match (id, consumer, owner) {
        (Some(id), None, None) => {
            let n = tx
                .execute(
                    &format!(
                        "DELETE FROM subscriptions WHERE id = ?1 AND {}",
                        seen(2, ws.version)
                    ),
                    params![id, json!(sources).to_string()],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err(unknown(id));
            }
            n
        }
        (None, Some(consumer), Some(owner)) => tx
            .execute(
                &format!(
                    "DELETE FROM subscriptions WHERE consumer = ?1 AND owner = ?2 AND {}",
                    seen(3, ws.version)
                ),
                params![consumer, owner, json!(sources).to_string()],
            )
            .map_err(sql)?,
        _ => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                "pass subscriptionId, or consumer and owner to remove that owner's temporary subscriptions",
            ));
        }
    };
    tx.commit().map_err(sql)?;
    Ok(json!({"workspaceId": ws_id, "removed": removed}))
}

/// Removes a consumer's raw checkpoint views (not subscriptions), optionally for one source
/// and scope. A view is removed only when the caller sees every source it was written under;
/// one written before that was recorded is removed only by a caller who sees every source.
pub fn forget(
    storage: &Storage,
    select: &Select,
    consumer: Option<&str>,
    source: Option<&str>,
    scope: Option<&str>,
) -> Result<Json> {
    let consumer = name("consumer", consumer)?;
    let (_, ws) = opened(storage, select, true)?;
    // The write lock is taken first, so visibility and the write share one snapshot.
    let tx =
        rusqlite::Transaction::new_unchecked(&ws.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(sql)?;
    if let Some(source) = source {
        jobs::chosen(storage, &ws, Some(source))?;
    }
    let scope = scope.map(crate::normalize_id);
    let visible = jobs::chosen(storage, &ws, None)?;
    let all: i64 = tx
        .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
        .map_err(sql)?;
    let everything = all as usize == visible.len();
    let existing: Vec<String> = {
        let mut stmt = tx.prepare("SELECT source_id FROM sources").map_err(sql)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(sql)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql)?
    };
    let ws_id = ws.held.id.clone();
    let rows = {
        let mut stmt = tx
            .prepare("SELECT filter, sources FROM consumers WHERE consumer = ?1")
            .map_err(sql)?;
        stmt.query_map([&consumer], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?
    };
    let (mut removed, mut legacy) = (0, 0);
    for (filter, written) in rows {
        let mut parts = filter.split('\u{1f}');
        let (f_source, f_scope) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if source.is_some_and(|s| s != f_source) || scope.as_deref().is_some_and(|s| s != f_scope) {
            continue;
        }
        let allowed = match written.as_deref().map(serde_json::from_str::<Vec<String>>) {
            // A source removed since no longer guards the view: its data is gone.
            Some(Ok(written)) => written
                .iter()
                .all(|s| visible.contains(s) || !existing.contains(s)),
            // Unknown ownership: only a caller who sees every source may remove it.
            _ => {
                legacy += usize::from(!everything);
                everything
            }
        };
        if allowed {
            removed += tx
                .execute(
                    "DELETE FROM consumers WHERE consumer = ?1 AND filter = ?2",
                    [&consumer, &filter],
                )
                .map_err(sql)?;
        }
    }
    tx.commit().map_err(sql)?;
    Ok(
        json!({"workspaceId": ws_id, "consumer": consumer, "removed": removed,
        "keptUnknownOwnership": legacy}),
    )
}
