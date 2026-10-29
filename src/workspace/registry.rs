//! Workspace actions on the registry: prepare (including recreation elsewhere), status, add,
//! remove, relocate and reset.

use super::*;

/// What `prepare`, `refresh` and `rebuild` did, and the jobs this process must now run (it
/// owns them); jobs joined from another live process are only reported.
pub struct Outcome {
    pub report: Json,
    pub run: Vec<String>,
}

/// `prepare` choices beyond the file and project.
#[derive(Default, Clone, Copy)]
pub struct PrepareOptions<'a> {
    /// The stored scope to index now; without it only the registry is prepared.
    pub scope: Option<&'a str>,
    /// Makes a retried request return its first job instead of starting another.
    pub request_id: Option<&'a str>,
    /// Recreate the workspace at `--storage` because its current storage cannot be used.
    pub replace_unavailable: bool,
}

/// A fresh 128-bit hex nonce from SQLite's random source.
pub(super) fn nonce() -> Result<String> {
    Connection::open_in_memory()
        .and_then(|c| c.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0)))
        .map_err(sql)
}

/// The location lock of workspace `id`, exclusive: one prepare, replacement or relocation of
/// this workspace at a time across every storage root. Publications take it shared.
pub(super) fn location_exclusive(storage: &Storage, id: &str) -> Result<(PathBuf, File)> {
    let locations = locations(&storage.defaults.config.path, true)?.expect("created");
    // Its own name: config and state may be one root, and two opens of one lock file conflict.
    let path = locations.join(format!("{id}.location.lock"));
    let file = open_lock(&path, true)?.expect("created");
    lock(&file, &path, true, id)?;
    Ok((locations, file))
}

/// Opens the workspace for writing, initializing or upgrading it first when needed. An
/// initialized current database is reused under the shared lock, so a prepare never waits
/// behind a running build; only initialization and upgrade take the lock exclusively.
pub(super) fn open_for_prepare(
    place: &Place,
    id: &str,
    locator: &Locator,
) -> Result<(Opened, bool)> {
    let state = &place.roots.state.path;
    // A shared lock suffices for an existing current database; anything else, including an
    // unmarked directory, takes the exclusive path that classifies it.
    if let Ok(Some(held)) = acquire(state, &Select::Locator(locator.clone()), false)
        && let Some(ws) = open_held(held, true, place)?
        && ws.version == SCHEMA
    {
        return Ok((ws, false));
    }
    let held = acquire_new(state, id, locator)?;
    let path = held.dir.join(DB);
    let flags = if plain(&path, false)? {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    let mut conn = open(&path, flags)?;
    let found = contents(&conn, &path, &held).map_err(|e| damaged(&path, &held.id, e))?;
    if let Some(found) = &found
        && found.binding != place.binding
    {
        return Err(fail(
            ErrorKind::OperationFailed,
            format!(
                "{} is detached: its location record names another database; figlens left it untouched",
                path.display()
            ),
        ));
    }
    writable(&conn)?;
    let (incarnation, initialized) = match found {
        Some(found) => {
            upgrade(&mut conn)?;
            (found.incarnation, false)
        }
        None => (
            initialize(&mut conn, &held, place.binding.as_deref())?,
            true,
        ),
    };
    Ok((
        Opened {
            incarnation,
            version: SCHEMA,
            conn,
            held,
        },
        initialized,
    ))
}

/// The coverage of a prepare that indexed nothing yet: only the registry exists.
pub(super) fn not_requested() -> Json {
    json!({"layers": {"text": {"state": "not_requested"}, "relations": {"state": "not_requested"}},
        "reasons": [{"code": "scope_not_requested"}]})
}

/// Why workspace `id` cannot be used where it lives now: `storage_unavailable`,
/// `permission_denied` or `ownership_unreadable`; `None` when it works or has no database.
pub(super) fn unusable_reason(
    current: &Storage,
    id: &str,
    locator: &Locator,
) -> Result<Option<&'static str>> {
    let classify = |e: Box<dyn Error>| -> Result<Option<&'static str>> {
        match e.downcast_ref::<FigError>().map(FigError::kind) {
            Some(ErrorKind::StorageUnavailable) => Ok(Some("storage_unavailable")),
            Some(ErrorKind::PermissionDenied) => Ok(Some("permission_denied")),
            _ => Err(e),
        }
    };
    let place = match current.place(id, Some(locator)) {
        Ok(place) => place,
        Err(e) => return classify(e),
    };
    let held = match acquire(
        &place.roots.state.path,
        &Select::Locator(locator.clone()),
        false,
    ) {
        Ok(Some(held)) => held,
        Ok(None) => return Ok(None),
        Err(e) => return classify(e),
    };
    let path = held.dir.join(DB);
    match plain(&path, false) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => return classify(e),
    }
    match open_held(held, false, &place) {
        Ok(_) => Ok(None),
        Err(e) if owner_of(&path, id).is_none() => match classify(e) {
            Ok(reason) => Ok(reason),
            Err(_) => Ok(Some("ownership_unreadable")),
        },
        Err(e) => classify(e),
    }
}

/// Recreates workspace `id` at `--storage` because its current storage cannot be used: missing
/// or offline, access denied, or a database whose ownership cannot be read. Under the location
/// lock, the new root's workspace directory is claimed (absent or empty, never merged), the
/// location record is rewritten with a fresh binding, and a new incarnation is initialized
/// there that knows only `source`. The old database and files are never touched; a builder or
/// reader of the old database is fenced by the binding. A failed initialization is retried by
/// the next prepare at the same place and binding.
pub(super) fn recreate(
    storage: &Storage,
    id: &str,
    locator: &Locator,
    source: &Path,
    options: PrepareOptions,
) -> Result<Outcome> {
    let new = storage.explicit.clone().ok_or_else(|| {
        fail(
            ErrorKind::InvalidArguments,
            "--replace-unavailable needs --storage <new absolute directory>",
        )
    })?;
    storage.authorized(&new)?;
    let current = Storage {
        explicit: None,
        ..storage.clone()
    };
    let previous = current.remembered(id).ok().flatten();
    let old_root = previous
        .as_ref()
        .map_or(current.defaults.state.path.clone(), |r| {
            r.storage.join("state")
        });
    if previous
        .as_ref()
        .is_some_and(|r| same_dir(&r.storage, &new))
        || same_dir(&old_root, &new.join("state"))
    {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "--storage {} is the workspace's current storage; choose a new directory",
                new.display()
            ),
        ));
    }
    let Some(reason) = unusable_reason(&current, id, locator)? else {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "workspace {id} can be used where it is; --replace-unavailable only recreates a workspace whose storage is unavailable, denied or has unreadable ownership"
            ),
        ));
    };
    create_durable(&new)?;
    let canonical = std::fs::canonicalize(&new).map_err(|e| storage_error(&new, e))?;
    let target = Place {
        roots: current.defaults.under(&canonical, "remembered"),
        rule: "remembered",
        remembered: Some(canonical.clone()),
        binding: None,
    };
    outside_storage(source, storage, &target)?;
    // Claimed under the new root's workspace lock before the record names it: an existing
    // database there is refused, never merged or overwritten.
    let held = acquire_new(&target.roots.state.path, id, locator)?;
    let db = held.dir.join(DB);
    if plain(&db, false)? {
        return Err(preserved(
            &db,
            "already exists for this workspace; choose an empty directory for the recreated workspace",
        ));
    }
    drop(held);
    let binding = nonce()?;
    let locations = locations(&current.defaults.config.path, true)?.expect("created");
    let gone = json!({"storage": previous.as_ref().map(|r| &r.storage).unwrap_or(&current.defaults.state.path),
        "binding": previous.as_ref().and_then(|r| r.binding.clone()), "reason": reason, "atMs": now_ms()});
    let record = json!({"version": LOCATION, "workspaceId": id, "locator": locator.report()?,
        "storage": utf8(&canonical)?, "binding": binding, "previous": gone});
    write_small(&locations.join(format!("{id}.json")), &record)?;
    let place = current.place(id, Some(locator))?;
    let (mut ws, initialized) = open_for_prepare(&place, id, locator)?;
    let (source_id, _) = register(&mut ws.conn, source)?;
    let mut extra = json!({"initialized": initialized, "replaced": true,
        "source": {"sourceId": source_id, "path": source, "added": true},
        "previous": {"storage": gone["storage"], "binding": gone["binding"], "reason": reason,
            "state": "retained, not restored", "sources": "unknown", "metadata": "unknown"}});
    let mut run = Vec::new();
    match options.scope {
        Some(scope) => {
            let job = jobs::submit(&mut ws, "prepare", &source_id, scope, options.request_id)?;
            if job.owned {
                run.push(job.job_id.clone());
            }
            extra["job"] =
                jobs::report_job(&ws.conn, &ws.held.dir, &job.job_id)?.unwrap_or(Json::Null);
        }
        None => extra["coverage"] = not_requested(),
    }
    Ok(Outcome {
        report: opened_report(storage, &place, &ws, extra)?,
        run,
    })
}

/// Creates or reuses the workspace for `file` (source-scoped, or the `project` directory's)
/// and registers the file. Concurrent calls for one workspace create one database. With
/// a new `--storage`, the workspace's location is remembered before its database is created.
/// With a scope, it starts or joins that scope's build; without one, it prepares the registry
/// only and indexes nothing, never the whole file implicitly.
pub fn prepare(
    storage: &Storage,
    file: &Path,
    project: Option<&Path>,
    options: PrepareOptions,
) -> Result<Outcome> {
    let source = readable(file)?;
    let locator = match project {
        Some(dir) => Locator::project(dir)?,
        None => Locator::Source(source.clone()),
    };
    let id = locator.id()?;
    let (locations, _location) = location_exclusive(storage, &id)?;
    // A relocated source-scoped file prepares the workspace that already holds it.
    if project.is_none()
        && !options.replace_unavailable
        && let Some(old) = storage.alias(&id)?
    {
        let (place, ws) = opened(storage, &Select::Id(old), true)?;
        return registered(storage, place, ws, false, &source, options);
    }
    if options.replace_unavailable {
        return recreate(storage, &id, &locator, &source, options);
    }
    let mut place = storage.place(&id, Some(&locator))?;
    outside_storage(&source, storage, &place)?;
    // Remember a new --storage before creating anything there: a database normal discovery
    // cannot find is never left behind, and a failed initialization is retried in that place.
    if let Some(explicit) = &storage.explicit
        && !place
            .remembered
            .as_ref()
            .is_some_and(|r| same_dir(r, explicit))
    {
        create_durable(explicit)?;
        let canonical = std::fs::canonicalize(explicit).map_err(|e| storage_error(explicit, e))?;
        let binding = nonce()?;
        let record = json!({"version": LOCATION, "workspaceId": id,
            "locator": locator.report()?, "storage": utf8(&canonical)?, "binding": binding});
        write_small(&locations.join(format!("{id}.json")), &record)?;
        place.binding = Some(binding);
    }
    let (ws, initialized) = open_for_prepare(&place, &id, &locator)?;
    registered(storage, place, ws, initialized, &source, options)
}

/// Registers `source` in the open workspace and submits the requested scope's build.
fn registered(
    storage: &Storage,
    place: Place,
    mut ws: Opened,
    initialized: bool,
    source: &Path,
    options: PrepareOptions,
) -> Result<Outcome> {
    let (source_id, added) = register(&mut ws.conn, source)?;
    let mut extra = json!({"initialized": initialized,
        "source": {"sourceId": source_id, "path": source, "added": added}});
    let mut run = Vec::new();
    match options.scope {
        Some(scope) => {
            let job = jobs::submit(&mut ws, "prepare", &source_id, scope, options.request_id)?;
            if job.owned {
                run.push(job.job_id.clone());
            }
            extra["job"] =
                jobs::report_job(&ws.conn, &ws.held.dir, &job.job_id)?.unwrap_or(Json::Null);
            extra["job"]["joined"] = json!(job.joined);
        }
        None => {
            extra["coverage"] = not_requested();
            extra["nextSteps"] = json!([{"tool": "index", "arguments": {"action": "prepare",
                "file": source, "scope": "<stored node id>"},
                "why": "only the registry was prepared; choose the scope to search (find or tree), or 0:0 for the whole file"}]);
        }
    }
    let place = Place {
        remembered: storage.explicit.clone().or(place.remembered.clone()),
        ..place
    };
    Ok(Outcome {
        report: opened_report(storage, &place, &ws, extra)?,
        run,
    })
}

/// The workspace's state and sources. A missing workspace creates nothing at all; an existing
/// one is read through a read-only connection.
pub fn status(storage: &Storage, select: &Select) -> Result<Json> {
    let select = &storage.follow(select)?;
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
    let Some(held) = acquire(&place.roots.state.path, select, false)? else {
        return Ok(report(
            &place,
            &id,
            None,
            "absent",
            json!({"nextSteps": ["`figlens index prepare <file.fig>` creates it"]}),
        ));
    };
    let (dir, locator) = (held.dir.clone(), held.locator.report()?);
    match open_held(held, false, &place)? {
        Some(ws) => opened_report(storage, &place, &ws, json!({})),
        None => Ok(report(
            &place,
            &id,
            Some(&dir),
            "uninitialized",
            json!({"locator": locator,
                "nextSteps": ["`figlens index prepare <file.fig>` initializes it with a new incarnation"]}),
        )),
    }
}

/// A source-scoped workspace is its one file: it may only add that file (`Some` of it), and
/// relocating it (`None`) is refused. shortcut: source-scoped relocate needs a locator move,
/// deferred to the durable-index milestone of #90.
pub(super) fn project_only(ws: &Opened, file: Option<&Path>) -> Result<()> {
    match &ws.held.locator {
        Locator::Source(own) if file != Some(own.as_path()) => Err(fail(
            ErrorKind::InvalidArguments,
            "a source-scoped workspace holds only its own file; prepare with --workspace <dir> to manage several sources, or reset this one",
        )),
        _ => Ok(()),
    }
}

/// Adds `file` to a project workspace.
pub fn add(storage: &Storage, select: &Select, file: &Path) -> Result<Json> {
    let source = readable(file)?;
    let (place, mut ws) = opened(storage, select, true)?;
    outside_storage(&source, storage, &place)?;
    project_only(&ws, Some(&source))?;
    let (source_id, added) = register(&mut ws.conn, &source)?;
    opened_report(
        storage,
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": source, "added": added}}),
    )
}

/// Removes `source_id` from the workspace's registry. The `.fig` itself is never touched.
/// The path of `source_id` when this caller may see it; an invisible source reads as unknown.
pub(super) fn visible_path(
    storage: &Storage,
    conn: &Connection,
    source_id: &str,
) -> Result<Option<String>> {
    let path: Option<String> = conn
        .query_row(
            "SELECT path FROM sources WHERE source_id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    Ok(path.filter(|p| storage.sees(Path::new(p))))
}

/// Removes `source_id` from the workspace's registry and, in the same transaction, withdraws
/// its published indexes from search and asks its active jobs to stop. The `.fig` itself is
/// never touched.
pub fn remove(storage: &Storage, select: &Select, source_id: &str) -> Result<Json> {
    let (place, mut ws) = opened(storage, select, true)?;
    let tx = ws
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    let removed = visible_path(storage, &tx, source_id)?.is_some()
        && tx
            .execute("DELETE FROM sources WHERE source_id = ?1", [source_id])
            .map_err(sql)?
            > 0;
    if removed {
        tx.execute(
            "UPDATE generations SET state = 'retired', retired_ms = ?2 WHERE gen IN
             (SELECT gen FROM published WHERE source_id = ?1)",
            rusqlite::params![source_id, now_ms()],
        )
        .map_err(sql)?;
        tx.execute("DELETE FROM published WHERE source_id = ?1", [source_id])
            .map_err(sql)?;
        tx.execute(
            "UPDATE jobs SET cancel = 1 WHERE source_id = ?1 AND state IN ('queued', 'running')",
            [source_id],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    opened_report(
        storage,
        &place,
        &ws,
        json!({"sourceId": source_id, "removed": removed}),
    )
}

/// Points `source_id` at `file`, keeping its id.
pub fn relocate(storage: &Storage, select: &Select, source_id: &str, file: &Path) -> Result<Json> {
    let source = readable(file)?;
    let (place, ws) = opened(storage, select, true)?;
    outside_storage(&source, storage, &place)?;
    if visible_path(storage, &ws.conn, source_id)?.is_none() || !storage.sees(&source) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {} has no source {source_id}", ws.held.id),
        ));
    }
    // A source-scoped workspace keeps its id: the new path's id becomes an alias of it, written
    // first, so an interruption leaves the alias pointing at the workspace still holding the
    // old path, and a retry completes the move.
    let _alias_lock;
    if let Locator::Source(_) = ws.held.locator {
        let new_id = Locator::Source(source.clone()).id()?;
        if new_id != ws.held.id {
            let (locations, lock) = location_exclusive(storage, &new_id)?;
            _alias_lock = lock;
            let record = locations.join(format!("{new_id}.json"));
            let own =
                plain(&record, false)? && storage.alias(&new_id)?.as_deref() != Some(&ws.held.id);
            let state = storage.place(&new_id, None)?.roots.state.path;
            if own || acquire(&state, &Select::Id(new_id.clone()), false)?.is_some() {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!(
                        "{} has its own workspace {new_id}; reset that workspace first, or relocate into a project workspace",
                        source.display()
                    ),
                ));
            }
            write_small(
                &record,
                &json!({"version": LOCATION, "aliasOf": ws.held.id,
                    "locator": Locator::Source(source.clone()).report()?}),
            )?;
        }
    }
    // A new membership: indexes of the old path stop being served until a refresh, and a
    // build still running for the old path cannot publish.
    let changed = match ws.conn.execute(
        "UPDATE sources SET path = ?1, membership = lower(hex(randomblob(16))) WHERE source_id = ?2",
        [utf8(&source)?, source_id],
    ) {
        Ok(n) => n,
        Err(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) => {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!(
                    "{} is already registered under another sourceId",
                    source.display()
                ),
            ));
        }
        Err(e) => return Err(sql(e)),
    };
    if changed == 0 {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {} has no source {source_id}", ws.held.id),
        ));
    }
    opened_report(
        storage,
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": source}}),
    )
}

/// Confirms a database is this workspace's, whatever its schema, before reset deletes it.
pub(super) fn resettable(conn: &Connection, path: &Path, id: &str) -> Result<()> {
    if is_empty(conn)? {
        return Ok(());
    }
    match meta(conn, "workspace_id") {
        Ok(Some(owner)) if owner == id => Ok(()),
        _ => Err(preserved(path, "is not this workspace's database")),
    }
}

/// Removes the workspace's database files once no other process holds the workspace, leaving
/// its marker, lock and remembered location so it can be prepared again in the same place with
/// a new incarnation. Sources, project mappings, exports and other workspaces are never
/// touched. `failed` lists what remains.
pub fn reset(storage: &Storage, select: &Select) -> Result<Json> {
    let select = &storage.follow(select)?;
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
    // Active jobs are asked to stop first; their owners stop the child and release the
    // workspace lock, which the exclusive lock below then waits for.
    if let Ok((_, ws)) = opened(storage, select, false)
        && ws.version >= 2
        && let Ok((_, ws)) = (drop(ws), opened(storage, select, true)).1
    {
        ws.conn
            .execute(
                "UPDATE jobs SET cancel = 1 WHERE state IN ('queued', 'running')",
                [],
            )
            .map_err(sql)?;
    }
    let Some(held) = acquire(&place.roots.state.path, select, true)? else {
        return Ok(report(
            &place,
            &id,
            None,
            "absent",
            json!({"removed": [], "retained": [], "failed": []}),
        ));
    };
    let path = held.dir.join(DB);
    if plain(&path, false)? {
        let conn = open(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        resettable(&conn, &path, &id)?;
        let (busy, log, done): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(sql)?;
        if busy != 0 || log != done {
            return Err(fail(
                ErrorKind::WorkspaceBusy,
                format!(
                    "workspace {id} could not checkpoint its database; retry when other figlens processes finish"
                ),
            ));
        }
        conn.close().map_err(|(_, e)| sql(e))?;
    }
    let (mut removed, mut failed) = (Vec::new(), Vec::new());
    for name in DB_FILES {
        let path = held.dir.join(name);
        let outcome = plain(&path, false).and_then(|exists| {
            if exists {
                std::fs::remove_file(&path)?;
            }
            Ok(exists)
        });
        match outcome {
            Ok(true) => removed.push(path),
            Ok(false) => {}
            Err(e) => failed.push(json!({"path": path, "error": e.to_string()})),
        }
    }
    let mut retained: Vec<PathBuf> = std::fs::read_dir(&held.dir)
        .map_err(|e| storage_error(&held.dir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    retained.sort();
    let dir = held.dir.clone();
    drop(held);
    Ok(report(
        &place,
        &id,
        Some(&dir),
        if failed.is_empty() {
            "reset"
        } else {
            "partial"
        },
        json!({"removed": removed, "retained": retained, "failed": failed,
            "nextSteps": ["`figlens index prepare <file.fig>` initializes it again in the same storage with a new incarnation; earlier sourceIds must be added again"]}),
    ))
}
