//! A workspace's SQLite database: schema 1 and 2, opening with binding and ownership checks,
//! upgrade, initialization and registry reads shared by every action.

use super::*;

pub(super) fn sql(error: rusqlite::Error) -> Box<dyn Error> {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => fail(
            ErrorKind::WorkspaceBusy,
            format!("the workspace database is busy: {error}"),
        ),
        Some(rusqlite::ErrorCode::ReadOnly | rusqlite::ErrorCode::CannotOpen) => fail(
            ErrorKind::PermissionDenied,
            format!("cannot write the workspace database: {error}"),
        ),
        _ => error.into(),
    }
}

/// Refuses a database whose main or sidecar file is a link or has another hard link: SQLite
/// writes through it, so a `.fig` linked there would be overwritten.
pub(super) fn unaliased(dir: &Path) -> Result<()> {
    for name in DB_FILES {
        let path = dir.join(name);
        if plain(&path, false)? && links(&path)? != 1 {
            return Err(preserved(
                &path,
                "has other hard links, through which SQLite would write; figlens leaves it as found: remove the other link, then retry",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn links(path: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::symlink_metadata(path)
        .map_err(|e| storage_error(path, e))?
        .nlink())
}

#[cfg(windows)]
pub(super) fn links(path: &Path) -> Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = File::open(path).map_err(|e| storage_error(path, e))?;
    // SAFETY: a valid open handle and an out struct the call fills on success.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(storage_error(path, std::io::Error::last_os_error()));
    }
    Ok(u64::from(info.nNumberOfLinks))
}

pub(super) fn open(path: &Path, flags: OpenFlags) -> Result<Connection> {
    unaliased(path.parent().expect("a workspace file"))?;
    let conn =
        Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sql)?;
    conn.busy_timeout(LOCK_WAIT).map_err(sql)?;
    Ok(conn)
}

pub(super) fn is_empty(conn: &Connection) -> Result<bool> {
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(sql)?;
    Ok(tables == 0)
}

pub(super) fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

/// What an initialized database holds: its schema version, incarnation and binding.
pub(super) struct Contents {
    pub(super) version: i64,
    pub(super) incarnation: String,
    pub(super) binding: Option<String>,
}

/// The contents of this workspace's database, or `None` for an empty one. Schema and identity
/// are read before any write: another schema or workspace is refused untouched.
pub(super) fn contents(conn: &Connection, path: &Path, held: &Held) -> Result<Option<Contents>> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql)?;
    if version == 0 {
        return if is_empty(conn)? {
            Ok(None)
        } else {
            Err(preserved(path, "is not a figlens workspace database"))
        };
    }
    if !(1..=SCHEMA).contains(&version) {
        return Err(incompatible(path, version));
    }
    let found = Locator::parse(&json!({
        "kind": meta(conn, "locator_kind").map_err(sql)?,
        "path": meta(conn, "locator_path").map_err(sql)?,
    }));
    if meta(conn, "workspace_id").map_err(sql)?.as_deref() != Some(held.id.as_str())
        || found.as_ref() != Some(&held.locator)
    {
        return Err(preserved(path, "belongs to a different workspace"));
    }
    Ok(Some(Contents {
        version,
        incarnation: meta(conn, "incarnation").map_err(sql)?.unwrap_or_default(),
        binding: meta(conn, "binding").map_err(sql)?,
    }))
}

/// Whether the database at `path` still names its owner: `Some(true)` for workspace `id`,
/// `Some(false)` for another one, `None` when its ownership metadata cannot be read at all.
pub(super) fn owner_of(path: &Path, id: &str) -> Option<bool> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    meta(&conn, "workspace_id")
        .ok()
        .flatten()
        .map(|owner| owner == id)
}

/// Adds the recovery that applies to a database this binary cannot read: a readable ownership
/// record allows an explicit reset; an unreadable one only a recreation elsewhere, since reset
/// deletes only a database that proves it is this workspace's. A foreign database, a newer
/// schema or a storage error keeps its own error.
pub(super) fn damaged(path: &Path, id: &str, error: Box<dyn Error>) -> Box<dyn Error> {
    let kind = error.downcast_ref::<FigError>().map(FigError::kind);
    if matches!(
        kind,
        Some(
            ErrorKind::IndexIncompatible
                | ErrorKind::WorkspaceBusy
                | ErrorKind::PermissionDenied
                | ErrorKind::StorageUnavailable
        )
    ) {
        return error;
    }
    let recovery = match owner_of(path, id) {
        Some(false) => return error,
        Some(true) => json!([{"action": "reset", "requires": "user_confirmation",
            "tool": "index", "arguments": {"action": "reset", "workspaceId": id},
            "effects": {"removes": "this workspace's database: registry, jobs and indexes",
                "keeps": ".fig sources, .figlens/, exports and other workspaces"}}]),
        None => json!([recreate_elsewhere()]),
    };
    FigError::new(
        ErrorKind::IndexIncompatible,
        format!(
            "{} cannot be read as this workspace's database: {error}",
            path.display()
        ),
    )
    .with_recovery(recovery)
    .into()
}

/// A newer schema: only the figlens that wrote it can read it, so nothing is offered as safe here.
pub(super) fn incompatible(path: &Path, version: i64) -> Box<dyn Error> {
    FigError::new(
        ErrorKind::IndexIncompatible,
        format!(
            "{} has workspace schema {version}; this figlens reads schemas 1-{SCHEMA} and left it untouched",
            path.display()
        ),
    )
    .with_recovery(json!([{"action": "use_writer_version", "requires": "none",
        "detail": format!("run the figlens version that wrote schema {version}")}]))
    .into()
}

/// Schema 2 beside the schema 1 registry: membership tokens on sources, snapshots, index
/// generations with their records, terms and edges, the published pointer and jobs.
pub(super) const V2: &str = "
    ALTER TABLE sources ADD COLUMN membership TEXT NOT NULL DEFAULT '';
    UPDATE sources SET membership = lower(hex(randomblob(16)));
    CREATE TABLE snapshots (
        source_id TEXT NOT NULL, snapshot_id TEXT NOT NULL, bytes INTEGER NOT NULL,
        mtime_ns INTEGER NOT NULL, verified_ms INTEGER NOT NULL,
        PRIMARY KEY (source_id, snapshot_id)) STRICT;
    CREATE TABLE generations (
        gen INTEGER PRIMARY KEY AUTOINCREMENT, source_id TEXT NOT NULL,
        membership TEXT NOT NULL, scope TEXT NOT NULL, snapshot_id TEXT, versions TEXT NOT NULL,
        job_id TEXT NOT NULL, state TEXT NOT NULL, complete INTEGER NOT NULL DEFAULT 0,
        stop TEXT, records INTEGER NOT NULL DEFAULT 0, info TEXT, created_ms INTEGER NOT NULL,
        retired_ms INTEGER) STRICT;
    CREATE INDEX generations_job ON generations (job_id);
    CREATE TABLE published (
        source_id TEXT NOT NULL, scope TEXT NOT NULL, gen INTEGER NOT NULL,
        published_ms INTEGER NOT NULL, PRIMARY KEY (source_id, scope)) STRICT;
    CREATE TABLE records (
        gen INTEGER NOT NULL, rid INTEGER NOT NULL, node_id TEXT NOT NULL, name TEXT NOT NULL,
        kind TEXT NOT NULL, text TEXT, norm0 TEXT NOT NULL, norm1 TEXT NOT NULL,
        len0 INTEGER NOT NULL, len1 INTEGER NOT NULL, parent INTEGER, source TEXT,
        text_source TEXT, hidden INTEGER NOT NULL, page INTEGER, root INTEGER,
        PRIMARY KEY (gen, rid)) STRICT, WITHOUT ROWID;
    CREATE TABLE terms (
        gen INTEGER NOT NULL, term TEXT NOT NULL, df INTEGER NOT NULL, postings BLOB NOT NULL,
        PRIMARY KEY (gen, term)) STRICT, WITHOUT ROWID;
    CREATE TABLE edges (
        gen INTEGER NOT NULL, rid INTEGER NOT NULL, relation TEXT NOT NULL, target TEXT,
        basis TEXT NOT NULL, evidence TEXT NOT NULL,
        PRIMARY KEY (gen, rid, relation)) STRICT, WITHOUT ROWID;
    CREATE INDEX edges_target ON edges (gen, relation, target);
    CREATE TABLE jobs (
        job_id TEXT PRIMARY KEY, request_id TEXT UNIQUE, fingerprint TEXT NOT NULL,
        kind TEXT NOT NULL, source_id TEXT NOT NULL, membership TEXT NOT NULL,
        scope TEXT NOT NULL, incarnation TEXT NOT NULL, expected_gen INTEGER,
        state TEXT NOT NULL, stage TEXT, done INTEGER NOT NULL DEFAULT 0, total INTEGER,
        error TEXT, owner TEXT NOT NULL, attempt INTEGER NOT NULL DEFAULT 1,
        cancel INTEGER NOT NULL DEFAULT 0, gen INTEGER, created_ms INTEGER NOT NULL,
        heartbeat_ms INTEGER NOT NULL, finished_ms INTEGER) STRICT;
    -- One active build per source and scope, whatever its kind: a second request joins it.
    CREATE UNIQUE INDEX jobs_active ON jobs (source_id, scope) WHERE state IN ('queued', 'running');
    PRAGMA user_version = 2;";

/// Upgrades a schema 1 registry to schema 2 in one transaction, keeping every source and the
/// incarnation. The caller holds the workspace lock exclusively.
pub(super) fn upgrade(conn: &mut Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql)?;
    if version == 1 {
        tx.execute_batch(V2).map_err(sql)?;
    }
    tx.commit().map_err(sql)
}

/// Durable WAL settings for a writing connection.
pub(super) fn writable(conn: &Connection) -> Result<()> {
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(sql)?;
    if !mode.eq_ignore_ascii_case("wal") {
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .map_err(sql)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(fail(
                ErrorKind::OperationFailed,
                format!("the workspace database cannot use WAL (journal mode {mode})"),
            ));
        }
    }
    conn.execute_batch("PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON;")
        .map_err(sql)
}

/// Creates the registry in one transaction. The incarnation is new for every initialization,
/// so ids handed out before a reset never match the recreated workspace.
pub(super) fn initialize(
    conn: &mut Connection,
    held: &Held,
    binding: Option<&str>,
) -> Result<String> {
    let tx = conn.transaction().map_err(sql)?;
    tx.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
         CREATE TABLE sources (
             source_id TEXT PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             added_at_ms INTEGER NOT NULL
         ) STRICT;",
    )
    .map_err(sql)?;
    tx.execute_batch(V2).map_err(sql)?;
    if let Some(binding) = binding {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('binding', ?1)",
            [binding],
        )
        .map_err(sql)?;
    }
    let incarnation: String = tx
        .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
        .map_err(sql)?;
    for (key, value) in [
        ("workspace_id", held.id.as_str()),
        ("locator_kind", held.locator.kind()),
        ("locator_path", held.locator.path()?),
        ("incarnation", &incarnation),
        ("created_at_ms", &now_ms().to_string()),
    ] {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)",
            [key, value],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(incarnation)
}

/// An initialized workspace opened for one command under its held lock.
pub(crate) struct Opened {
    pub(super) incarnation: String,
    pub(super) version: i64,
    // Declared before `held`, so the connection closes before the lock is released.
    pub(super) conn: Connection,
    pub(super) held: Held,
}

/// Opens the database of a held workspace, or `None` when it has none (after reset). A
/// database whose binding differs from its location record's is detached: another database
/// replaced it, so it is never served.
pub(super) fn open_held(held: Held, write: bool, place: &Place) -> Result<Option<Opened>> {
    let path = held.dir.join(DB);
    if !plain(&path, false)? {
        return Ok(None);
    }
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = open(&path, flags)?;
    let Some(found) = contents(&conn, &path, &held).map_err(|e| damaged(&path, &held.id, e))?
    else {
        return Ok(None);
    };
    if found.binding != place.binding {
        return Err(fail(
            ErrorKind::OperationFailed,
            format!(
                "{} is detached: its location record names another database; figlens left it untouched",
                path.display()
            ),
        ));
    }
    if write {
        writable(&conn)?;
    }
    Ok(Some(Opened {
        incarnation: found.incarnation,
        version: found.version,
        conn,
        held,
    }))
}

pub(super) fn missing(id: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::InvalidArguments,
        format!("workspace {id} is not prepared; run `figlens index prepare <file.fig>` first"),
    )
}

/// The `sourceId` registered for `source`, adding it when new; true when added.
pub(super) fn register(conn: &mut Connection, source: &Path) -> Result<(String, bool)> {
    let path = utf8(source)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT source_id FROM sources WHERE path = ?1",
            [path],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(id) = existing {
        return Ok((id, false));
    }
    let id = crate::search::source_id(source);
    let other: Option<String> = tx
        .query_row(
            "SELECT path FROM sources WHERE source_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(other) = other {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "{id} was relocated to {other}; relocate it back or remove it before adding {path}"
            ),
        ));
    }
    tx.execute(
        "INSERT INTO sources (source_id, path, added_at_ms, membership)
         VALUES (?1, ?2, ?3, lower(hex(randomblob(16))))",
        rusqlite::params![id, path, now_ms()],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok((id, true))
}

/// The registered sources this caller may see, in registry order, with their published scopes.
pub(super) fn sources(storage: &Storage, ws: &Opened) -> Result<Vec<Json>> {
    let conn = &ws.conn;
    let mut stmt = conn
        .prepare("SELECT source_id, path FROM sources ORDER BY added_at_ms, source_id")
        .map_err(sql)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, path) = row.map_err(sql)?;
        if !storage.sees(Path::new(&path)) {
            continue;
        }
        let present = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
        let mut source = json!({"sourceId": id, "path": path, "present": present});
        if ws.version >= 2 {
            let mut stmt = conn
                .prepare(
                    "SELECT p.scope, g.snapshot_id, g.complete, g.records, g.membership = s.membership
                     FROM published p JOIN generations g ON g.gen = p.gen
                     JOIN sources s ON s.source_id = p.source_id
                     WHERE p.source_id = ?1 ORDER BY p.published_ms DESC",
                )
                .map_err(sql)?;
            let scopes = stmt
                .query_map([&id], |r| {
                    Ok(json!({"scope": r.get::<_, String>(0)?, "snapshotId": r.get::<_, Option<String>>(1)?,
                        "complete": r.get::<_, bool>(2)?, "records": r.get::<_, i64>(3)?,
                        "current": r.get::<_, bool>(4)?}))
                })
                .map_err(sql)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql)?;
            source["indexed"] = json!(scopes);
        }
        out.push(source);
    }
    Ok(out)
}

// ---- Reports ----

/// Every report's common fields, plus `extra`.
pub(super) fn report(
    place: &Place,
    id: &str,
    dir: Option<&Path>,
    state: &str,
    extra: Json,
) -> Json {
    let mut out = json!({"workspaceId": id, "state": state, "schemaVersion": SCHEMA,
        "storage": place.report(), "roots": place.roots.report(),
        "paths": dir.map(|d| json!({"workspace": d, "database": d.join(DB)}))});
    if let (Some(out), Json::Object(extra)) = (out.as_object_mut(), extra) {
        out.extend(extra);
    }
    out
}

/// The report of an initialized workspace: its incarnation, locator and sources, plus `extra`.
pub(super) fn opened_report(
    storage: &Storage,
    place: &Place,
    ws: &Opened,
    extra: Json,
) -> Result<Json> {
    let mut extra = extra;
    extra["incarnation"] = json!(ws.incarnation);
    extra["schemaVersion"] = json!(ws.version);
    extra["locator"] = ws.held.locator.report()?;
    extra["sources"] = json!(sources(storage, ws)?);
    if extra["nextSteps"].is_null() {
        extra["nextSteps"] = json!([{"tool": "search", "arguments": {"workspaceId": ws.held.id,
            "node": "<an indexed scope>", "query": "<words>"}}]);
    }
    Ok(report(
        place,
        &ws.held.id,
        Some(&ws.held.dir),
        "initialized",
        extra,
    ))
}

// ---- Actions ----

/// Opens the selected, initialized workspace with its storage place. A writer upgrades a
/// schema 1 registry first, briefly holding the workspace lock exclusively.
pub(super) fn opened(storage: &Storage, select: &Select, write: bool) -> Result<(Place, Opened)> {
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
    let held = acquire(&place.roots.state.path, select, false)?.ok_or_else(|| missing(&id))?;
    let ws = open_held(held, write, &place)?.ok_or_else(|| missing(&id))?;
    if !write || ws.version == SCHEMA {
        return Ok((place, ws));
    }
    drop(ws);
    let held = acquire(&place.roots.state.path, select, true)?.ok_or_else(|| missing(&id))?;
    let mut ws = open_held(held, true, &place)?.ok_or_else(|| missing(&id))?;
    upgrade(&mut ws.conn)?;
    drop(ws);
    let held = acquire(&place.roots.state.path, select, false)?.ok_or_else(|| missing(&id))?;
    let ws = open_held(held, write, &place)?.ok_or_else(|| missing(&id))?;
    Ok((place, ws))
}
