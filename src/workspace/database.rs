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
        let count = settled(name, &path, Instant::now() + LOCK_WAIT, || {
            plain(&path, false).and_then(|p| if p { links(&path) } else { Ok(1) })
        })?;
        if count != 1 {
            return Err(preserved(
                &path,
                &format!(
                    "has {count} hard links, through which SQLite would write; figlens leaves it as found: remove the other link, then retry"
                ),
            ));
        }
    }
    Ok(())
}

/// The link count `probe` reads for `name`, probed again (path and all, each probe's handle
/// closed) within `deadline` while a sidecar may be mid-deletion: on Windows its probe fails as
/// permission denied (a file pending deletion refuses the read open), or it reads 0 links once
/// its name is deleted under the probe's open handle. Error 5 alone may also be a real denial,
/// which the deadline still reports. A sidecar still reading 0
/// at the deadline answers busy. `index.sqlite`, which SQLite never deletes, is read once.
pub(super) fn settled(
    name: &str,
    path: &Path,
    deadline: Instant,
    mut probe: impl FnMut() -> Result<u64>,
) -> Result<u64> {
    loop {
        let checked = probe();
        let again = name != DB
            && Instant::now() < deadline
            && match &checked {
                Ok(count) => *count == 0,
                Err(e) => cfg!(windows) && kind_of(&**e) == Some(ErrorKind::PermissionDenied),
            };
        if !again {
            return match checked {
                Ok(0) if name != DB => Err(fail(
                    ErrorKind::WorkspaceBusy,
                    format!(
                        "{} is still being deleted by another SQLite connection; retry when it finishes",
                        path.display()
                    ),
                )),
                checked => checked,
            };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn kind_of(error: &(dyn Error + 'static)) -> Option<ErrorKind> {
    error.downcast_ref::<FigError>().map(|f| f.kind())
}

/// The hard links of `path`; a file another connection removed since it was listed (SQLite
/// deletes `-shm` and `-journal` on close) has none to write through, so it counts as one.
#[cfg(unix)]
pub(super) fn links(path: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.nlink()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(1),
        Err(e) => Err(storage_error(path, e)),
    }
}

#[cfg(windows)]
pub(super) fn links(path: &Path) -> Result<u64> {
    match File::open(path) {
        Ok(file) => handle_links(&file, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(1),
        Err(e) => Err(storage_error(path, e)),
    }
}

#[cfg(windows)]
pub(super) fn handle_links(file: &File, path: &Path) -> Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
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
        return Err(incompatible(path, version, held));
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
        Some(true) => json!([reset_action(id)]),
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

/// The explicit reset of workspace `id`, after the user confirms it.
pub(super) fn reset_action(id: &str) -> Json {
    json!({"action": "reset", "requires": "user_confirmation",
        "tool": "index", "arguments": {"action": "reset", "workspaceId": id},
        "effects": {"removes": "this workspace's database: registry, jobs and indexes",
            "keeps": ".fig sources, .figlens/, exports and other workspaces"}})
}

/// A newer schema: only the figlens that wrote it can read it, so nothing is offered as safe
/// here; an explicit reset is the confirmed alternative when its ownership record names `held`.
pub(super) fn incompatible(path: &Path, version: i64, held: &Held) -> Box<dyn Error> {
    FigError::new(
        ErrorKind::IndexIncompatible,
        format!(
            "{} has workspace schema {version}; this figlens reads schemas 1-{SCHEMA} and left it untouched",
            path.display()
        ),
    )
    .with_recovery({
        let mut actions = vec![json!({"action": "use_writer_version", "requires": "none",
            "detail": format!("run the figlens version that wrote schema {version}")})];
        if owner_of(path, &held.id) == Some(true) {
            actions.push(placed(reset_action(&held.id), &held.dir));
        }
        json!(actions)
    })
    .into()
}

/// Schema 2 beside the schema 1 registry: membership tokens on sources, snapshots, index
/// generations with their records, terms and edges, the published pointer and jobs.
pub(crate) const V2: &str = "
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
        heartbeat_ms INTEGER NOT NULL, finished_ms INTEGER, evicted TEXT,
        seq INTEGER NOT NULL DEFAULT 0) STRICT;
    -- Further requestIds that joined or took over a job, so each replays it; '' for a track
    -- that found its scope already tracked and started none.
    CREATE TABLE requests (request_id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL,
        job_id TEXT NOT NULL) STRICT;
    -- One active build per source and scope, whatever its kind: a second request joins it.
    CREATE UNIQUE INDEX jobs_active ON jobs (source_id, scope) WHERE state IN ('queued', 'running');
    PRAGMA user_version = 2;";

/// Schema 3: relation edges keyed by location, relation and sequence, with their resolution
/// state and a target index for usages; the definitions they point at; record lookups by node
/// and parent. Generations built before carry other versions, so they read as incompatible.
pub(crate) const V3: &str = "
    DROP INDEX edges_target;
    DROP TABLE edges;
    CREATE TABLE edges (
        gen INTEGER NOT NULL, rid INTEGER NOT NULL, relation TEXT NOT NULL, seq INTEGER NOT NULL,
        target TEXT, state TEXT NOT NULL, basis TEXT NOT NULL, evidence TEXT NOT NULL,
        PRIMARY KEY (gen, rid, relation, seq)) STRICT, WITHOUT ROWID;
    CREATE INDEX edges_target ON edges (gen, target, rid, relation, seq);
    CREATE TABLE definitions (
        gen INTEGER NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
        set_id TEXT, page TEXT, PRIMARY KEY (gen, id)) STRICT, WITHOUT ROWID;
    CREATE INDEX definitions_set ON definitions (gen, set_id);
    CREATE INDEX records_node ON records (gen, node_id);
    CREATE INDEX records_parent ON records (gen, parent, rid);
    PRAGMA user_version = 3;";

/// Schema 4: explicit change tracking per source and scope, each tracked generation's facet
/// layout, selected-node context and record facets, published revisions with their bounded change rows, consumer checkpoints, and the
/// secret cursor MACs are keyed with. `seq` orders revisions across the workspace and is never
/// exposed; `id` is the random public revision id.
pub(crate) const V4: &str = "
    CREATE TABLE tracking (
        source_id TEXT NOT NULL, scope TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('baseline_pending', 'active', 'disabled')),
        epoch INTEGER NOT NULL, facet_version TEXT NOT NULL, enabled_ms INTEGER NOT NULL,
        enabled_seq INTEGER NOT NULL DEFAULT 0,
        baseline_gen INTEGER, PRIMARY KEY (source_id, scope)) STRICT;
    CREATE TABLE facet_sets (
        gen INTEGER PRIMARY KEY, facet_version TEXT NOT NULL, present INTEGER NOT NULL,
        origin TEXT NOT NULL) STRICT;
    CREATE TABLE facets (
        gen INTEGER NOT NULL, rid INTEGER NOT NULL, blob BLOB NOT NULL,
        PRIMARY KEY (gen, rid)) STRICT, WITHOUT ROWID;
    CREATE TABLE revisions (
        id BLOB PRIMARY KEY, seq INTEGER NOT NULL UNIQUE, source_id TEXT NOT NULL,
        scope TEXT NOT NULL, epoch INTEGER NOT NULL, gen INTEGER NOT NULL, from_gen INTEGER,
        snapshot_id TEXT, kind TEXT NOT NULL, coverage TEXT NOT NULL, summary TEXT NOT NULL,
        detail TEXT NOT NULL CHECK (detail IN ('complete', 'truncated', 'expired')),
        detail_rows INTEGER NOT NULL, detail_bytes INTEGER NOT NULL,
        created_ms INTEGER NOT NULL) STRICT;
    CREATE INDEX revisions_scope ON revisions (source_id, scope, seq);
    CREATE TABLE revision_changes (
        rev BLOB NOT NULL, ord INTEGER NOT NULL, frame TEXT, node TEXT NOT NULL,
        kind TEXT NOT NULL, change TEXT NOT NULL, facets INTEGER NOT NULL,
        descendants INTEGER NOT NULL, PRIMARY KEY (rev, ord)) STRICT, WITHOUT ROWID;
    CREATE TABLE consumers (
        consumer TEXT NOT NULL, filter TEXT NOT NULL, after_seq INTEGER NOT NULL,
        floor INTEGER NOT NULL, updated_ms INTEGER NOT NULL, PRIMARY KEY (consumer, filter)) STRICT;
    INSERT INTO meta (key, value) VALUES ('cursor_secret', lower(hex(randomblob(32))));
    PRAGMA user_version = 4;";

/// Schema 5: temporary watch leases (#92), each a holder process's interest in one tracked
/// source and scope; the single runner that keeps them fresh, fenced by `attempt`; and each
/// watched source's last persisted monitor state. An ended lease keeps its row (with no path)
/// until the runner's GC, so its requestId replays as ended instead of starting it again.
/// `jobs.shared` marks an attempt another caller joined (or replayed, or took over), so the
/// runner cancels only attempts nobody else waits for.
pub(crate) const V5: &str = "
    CREATE TABLE watch_leases (
        lease_id TEXT PRIMARY KEY, holder TEXT NOT NULL, source_id TEXT NOT NULL,
        scope TEXT NOT NULL, incarnation TEXT NOT NULL, epoch INTEGER NOT NULL, authorized_path TEXT,
        request_id TEXT UNIQUE, fingerprint TEXT NOT NULL, renewed_ms INTEGER NOT NULL,
        expires_ms INTEGER NOT NULL, ended TEXT) STRICT;
    CREATE INDEX watch_leases_holder ON watch_leases (holder, expires_ms);
    CREATE TABLE watch_runner (
        one INTEGER PRIMARY KEY CHECK (one = 1), token TEXT NOT NULL, attempt INTEGER NOT NULL,
        placement TEXT NOT NULL, started_ms INTEGER NOT NULL, beat_ms INTEGER NOT NULL,
        closing INTEGER NOT NULL DEFAULT 0) STRICT;
    CREATE TABLE watch_monitor (
        source_id TEXT PRIMARY KEY, state TEXT NOT NULL, stamp_checked_ms INTEGER,
        content_verified_ms INTEGER, last_job_id TEXT, last_error TEXT,
        written_ms INTEGER NOT NULL) STRICT;
    ALTER TABLE jobs ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
    PRAGMA user_version = 5;";

/// Schema 6: durable change subscriptions, each its own checkpoint, and the visible sources a
/// raw consumer checkpoint was written under, so `watch forget` removes only rows it may see.
pub(crate) const SUBSCRIPTIONS: &str = "
    CREATE TABLE subscriptions (
        id TEXT PRIMARY KEY, consumer TEXT NOT NULL, source_id TEXT NOT NULL,
        scope TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('source', 'scope', 'node', 'query')),
        target TEXT NOT NULL, anchor TEXT NOT NULL,
        lifetime TEXT NOT NULL CHECK (lifetime IN ('durable', 'temporary')), owner TEXT,
        filter_version INTEGER NOT NULL, after_seq INTEGER NOT NULL, floor INTEGER NOT NULL,
        created_ms INTEGER NOT NULL, read_ms INTEGER NOT NULL,
        pruned_seq INTEGER NOT NULL DEFAULT 0,
        eval_failure TEXT,
        UNIQUE (consumer, source_id, scope, kind, target)) STRICT;
    CREATE TABLE query_evaluations (
        id BLOB PRIMARY KEY CHECK (length(id) = 16),
        sub TEXT NOT NULL REFERENCES subscriptions (id) ON DELETE CASCADE, seq INTEGER NOT NULL,
        gen INTEGER NOT NULL, snapshot_id TEXT NOT NULL, membership TEXT NOT NULL,
        ranking TEXT NOT NULL, coverage TEXT NOT NULL, causes TEXT NOT NULL, refs TEXT NOT NULL,
        total INTEGER, delta TEXT NOT NULL, bytes INTEGER NOT NULL, created_ms INTEGER NOT NULL,
        UNIQUE (sub, seq)) STRICT;
    CREATE INDEX subscriptions_owner ON subscriptions (owner) WHERE owner IS NOT NULL;
    CREATE INDEX subscriptions_query ON subscriptions (source_id, scope, kind, target, id);
    ALTER TABLE consumers ADD COLUMN sources TEXT;
    PRAGMA user_version = 6;";

/// Schema 7: a revision's impact (#92), the layers its changes reach. `revisions.impact` is its
/// summary (NULL for a baseline or a revision recorded before schema 7, never an empty
/// complete impact) and `revision_impacts` the reached layers it listed, kept and expired with
/// the revision's change rows.
pub(crate) const IMPACTS: &str = "
    ALTER TABLE revisions ADD COLUMN impact TEXT;
    CREATE TABLE revision_impacts (
        rev BLOB NOT NULL, ord INTEGER NOT NULL, node TEXT NOT NULL, class TEXT NOT NULL,
        side INTEGER NOT NULL, changed INTEGER NOT NULL, unknown INTEGER NOT NULL,
        equal INTEGER NOT NULL, relation TEXT, target TEXT, old_frame TEXT, new_frame TEXT,
        PRIMARY KEY (rev, ord)) STRICT, WITHOUT ROWID;
    PRAGMA user_version = 7;";

/// Schema 8: each generation's definition and cached-copy fingerprints (#93), added beside the
/// rest and collected with them; and a saved question's fixed source set (`source_id` '*', its
/// members in `sources`) and why an authoritative event disabled it.
pub(crate) const V8: &str = "
    CREATE TABLE fingerprints (
        gen INTEGER NOT NULL, rid INTEGER NOT NULL, facet TEXT NOT NULL, state TEXT NOT NULL,
        digest BLOB, PRIMARY KEY (gen, rid, facet)) STRICT, WITHOUT ROWID;
    ALTER TABLE subscriptions ADD COLUMN sources TEXT;
    ALTER TABLE subscriptions ADD COLUMN disabled TEXT;
    PRAGMA user_version = 8;";

/// Upgrades schemas 1 to 7 to the current one in one transaction, keeping every source, index,
/// job and the incarnation. The caller holds the workspace lock exclusively.
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
    if version <= 2 {
        tx.execute_batch(V3).map_err(sql)?;
    }
    if version <= 3 {
        tx.execute_batch(V4).map_err(sql)?;
    }
    if version <= 4 {
        tx.execute_batch(V5).map_err(sql)?;
    }
    if version <= 5 {
        tx.execute_batch(SUBSCRIPTIONS).map_err(sql)?;
    }
    if version <= 6 {
        tx.execute_batch(IMPACTS).map_err(sql)?;
    }
    if version <= 7 {
        tx.execute_batch(V8).map_err(sql)?;
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
    tx.execute_batch(V3).map_err(sql)?;
    tx.execute_batch(V4).map_err(sql)?;
    tx.execute_batch(V5).map_err(sql)?;
    tx.execute_batch(SUBSCRIPTIONS).map_err(sql)?;
    tx.execute_batch(IMPACTS).map_err(sql)?;
    tx.execute_batch(V8).map_err(sql)?;
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

/// Whether opening a workspace failed as [`missing`] does, the workspace being absent, rather
/// than busy, unavailable or unreadable.
pub(super) fn gone(e: &(dyn Error + 'static)) -> bool {
    e.downcast_ref::<FigError>()
        .is_some_and(|f| f.kind() == ErrorKind::InvalidArguments)
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
        "paths": dir.map(|d| json!({"workspace": d, "database": d.join(DB),
            "lock": super::lock_beside(d)}))});
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
    let select = &storage.follow(select)?;
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
