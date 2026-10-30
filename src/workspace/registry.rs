//! Workspace actions on the registry: prepare (including recreation elsewhere), status, add,
//! remove, relocate and reset.

use super::changes::subscriptions::SET;
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
    initialize_held(acquire_new(state, id, locator)?, place)
}

/// Opens, initializing or upgrading, the database of the workspace `held` locks exclusively.
pub(super) fn initialize_held(held: Held, place: &Place) -> Result<(Opened, bool)> {
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

/// The `index prepare` call that (re)creates the workspace `locator` names, as a next step:
/// its own file for a source-scoped workspace, any of its files for a project one.
pub(super) fn prepare_step(locator: Option<&Locator>, why: &str) -> Json {
    let arguments = match locator {
        Some(Locator::Source(file)) => json!({"action": "prepare", "file": file}),
        Some(Locator::Project(dir)) => {
            json!({"action": "prepare", "file": "<a .fig of this project>", "workspace": dir})
        }
        None => json!({"action": "prepare", "file": "<file.fig>"}),
    };
    json!([{"tool": "index", "arguments": arguments, "why": why}])
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
    let in_use = || {
        fail(
            ErrorKind::InvalidArguments,
            format!(
                "--storage {} is the workspace's current storage; choose a new directory",
                new.display()
            ),
        )
    };
    let locations = locations(&current.defaults.config.path, true)?.expect("created");
    let record_path = locations.join(format!("{id}.json"));
    // A replacement whose record already names `new` but whose database was never created
    // there (its initialization failed) is finished in place by the same call again.
    let reserved = previous
        .as_ref()
        .is_some_and(|r| same_dir(&r.storage, &new));
    let gone = if reserved {
        let gone = read_small(&record_path)?["previous"].clone();
        if !gone.is_object() {
            return Err(in_use());
        }
        gone
    } else {
        if same_dir(&old_root, &new.join("state")) {
            return Err(in_use());
        }
        let Some(reason) = unusable_reason(&current, id, locator)? else {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!(
                    "workspace {id} can be used where it is; --replace-unavailable only recreates a workspace whose storage is unavailable, denied or has unreadable ownership"
                ),
            ));
        };
        json!({"storage": previous.as_ref().map(|r| &r.storage).unwrap_or(&current.defaults.state.path),
            "binding": previous.as_ref().and_then(|r| r.binding.clone()), "reason": reason, "atMs": now_ms()})
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
    // The new root's workspace lock is held from the emptiness check through the record and
    // initialization, under the location lock: an existing database there is refused, never
    // merged or overwritten, and no other process can create one in between.
    let held = acquire_new(&target.roots.state.path, id, locator)?;
    let db = held.dir.join(DB);
    if plain(&db, false)? {
        return Err(if reserved {
            in_use()
        } else {
            preserved(
                &db,
                "already exists for this workspace; choose an empty directory for the recreated workspace",
            )
        });
    }
    if !reserved {
        let record = json!({"version": LOCATION, "workspaceId": id, "locator": locator.report()?,
            "storage": utf8(&canonical)?, "binding": nonce()?, "previous": gone});
        write_small(&record_path, &record)?;
    }
    let place = current.place(id, Some(locator))?;
    let (mut ws, initialized) = initialize_held(held, &place)?;
    let (source_id, _) = register(&mut ws.conn, source)?;
    let mut extra = json!({"initialized": initialized, "replaced": true,
        "source": {"sourceId": source_id, "path": source, "added": true},
        "previous": {"storage": gone["storage"], "binding": gone["binding"], "reason": gone["reason"],
            "state": "retained, not restored", "sources": "unknown", "metadata": "unknown"}});
    let mut run = Vec::new();
    match options.scope {
        Some(scope) => {
            let job = jobs::submit(
                storage,
                &mut ws,
                "prepare",
                &source_id,
                scope,
                options.request_id,
            )?;
            if job.owned {
                run.push(job.job_id.clone());
            }
            match jobs::report_job(&ws.conn, &ws.held.dir, &job.job_id) {
                Ok(job) => extra["job"] = job.unwrap_or(Json::Null),
                Err(e) => return Err(jobs::release(&ws, &run, e)),
            }
        }
        None => extra["coverage"] = not_requested(),
    }
    match opened_report(storage, &place, &ws, extra) {
        Ok(report) => Ok(Outcome { report, run }),
        Err(e) => Err(jobs::release(&ws, &run, e)),
    }
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
    // A relocated source-scoped file prepares the workspace that holds it. An alias whose
    // workspace no longer registers this path (reset, or moved on) is stale and is dropped.
    if project.is_none()
        && !options.replace_unavailable
        && let Some(old) = storage.alias(&id)?
    {
        match opened(storage, &Select::Id(old), true) {
            Ok((place, ws)) => {
                let holds = ws
                    .conn
                    .query_row(
                        "SELECT 1 FROM sources WHERE path = ?1",
                        [utf8(&source)?],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(sql)?;
                if holds.is_some() {
                    return registered(storage, place, ws, false, &source, options);
                }
            }
            // Only a workspace that is gone makes the alias stale; a busy or unavailable one
            // still holds the source.
            Err(e) if !gone(&*e) => return Err(e),
            Err(_) => {}
        }
        let record = locations.join(format!("{id}.json"));
        std::fs::remove_file(&record).map_err(|e| storage_error(&record, e))?;
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
    if project.is_none() {
        // A source-scoped workspace whose source moved away keeps it; a new file at the old
        // path is not added beside it.
        let moved: Option<String> = ws
            .conn
            .query_row(
                "SELECT path FROM sources WHERE path <> ?1",
                [utf8(&source)?],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        if let Some(moved) = moved {
            return Err(fail(
                ErrorKind::InvalidArguments,
                format!(
                    "workspace {id} now holds {moved}, relocated from {}; prepare that file, or reset this workspace to index the new file here",
                    source.display()
                ),
            ));
        }
    }
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
            let job = jobs::submit(
                storage,
                &mut ws,
                "prepare",
                &source_id,
                scope,
                options.request_id,
            )?;
            if job.owned {
                run.push(job.job_id.clone());
            }
            match jobs::report_job(&ws.conn, &ws.held.dir, &job.job_id) {
                Ok(report) => extra["job"] = report.unwrap_or(Json::Null),
                Err(e) => return Err(jobs::release(&ws, &run, e)),
            }
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
    match opened_report(storage, &place, &ws, extra) {
        Ok(report) => Ok(Outcome { report, run }),
        Err(e) => Err(jobs::release(&ws, &run, e)),
    }
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
            json!({"nextSteps": prepare_step(select.locator(), "the workspace does not exist; prepare creates it")}),
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
            json!({"locator": locator, "nextSteps": prepare_step(select.locator(),
                "prepare initializes it with a new incarnation")}),
        )),
    }
}

/// A source-scoped workspace is its one file: it may only add that file.
pub(super) fn project_only(ws: &Opened, file: &Path) -> Result<()> {
    match &ws.held.locator {
        Locator::Source(own) if file != own.as_path() => Err(fail(
            ErrorKind::InvalidArguments,
            "a source-scoped workspace holds only its own file; prepare with --workspace <dir> to manage several sources, or reset this one",
        )),
        _ => Ok(()),
    }
}

/// Adds `file` to a project workspace, or the `.fig` files under it when it is a directory.
pub fn add(storage: &Storage, select: &Select, file: &Path) -> Result<Json> {
    if std::fs::metadata(file).is_ok_and(|m| m.is_dir()) {
        return add_dir(storage, select, file, None);
    }
    let source = readable(file)?;
    let (place, mut ws) = opened(storage, select, true)?;
    outside_storage(&source, storage, &place)?;
    project_only(&ws, &source)?;
    let (source_id, added) = register(&mut ws.conn, &source)?;
    opened_report(
        storage,
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": source, "added": added}}),
    )
}

/// Registers `local`, the download of the `gs://` object `uri` pinned to `generation`, in a
/// project workspace. Its sourceId comes from the URI, so an add that downloaded a newer
/// generation keeps the id and, like relocate, moves the source to that download. Nothing here
/// reaches the network; the caller downloaded it.
pub fn add_remote(
    storage: &Storage,
    select: &Select,
    uri: &str,
    local: &Path,
    generation: &str,
) -> Result<Json> {
    // Not `outside_storage`: the download is figlens' own cache copy, never a database file.
    let local = readable(local)?;
    let (place, mut ws) = opened(storage, select, true)?;
    if let Locator::Source(_) = ws.held.locator {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "a source-scoped workspace holds only its own file; prepare with --workspace <dir> to add a gs:// object",
        ));
    }
    let path = utf8(&local)?;
    let tx = ws
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    let found: Option<(String, String)> = tx
        .query_row(
            "SELECT source_id, path FROM sources WHERE remote = ?1",
            [uri],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let (source_id, added) = match found {
        Some((id, old)) if old == path => (id, false),
        Some((id, _)) => {
            // A new membership and stopped builds, as relocate does: the old download's index
            // is stale until a refresh.
            tx.execute(
                "UPDATE sources SET path = ?1, generation = ?2,
                 membership = lower(hex(randomblob(16))) WHERE source_id = ?3",
                [path, generation, &id],
            )
            .map_err(sql)?;
            tx.execute(
                "UPDATE jobs SET cancel = 1 WHERE source_id = ?1 AND state IN ('queued', 'running')",
                [&id],
            )
            .map_err(sql)?;
            (id, false)
        }
        None => {
            let id = crate::search::source_id(Path::new(uri));
            tx.execute(
                "INSERT INTO sources (source_id, path, added_at_ms, membership, remote, generation)
                 VALUES (?1, ?2, ?3, lower(hex(randomblob(16))), ?4, ?5)",
                rusqlite::params![id, path, now_ms(), uri, generation],
            )
            .map_err(sql)?;
            (id, true)
        }
    };
    tx.commit().map_err(sql)?;
    opened_report(
        storage,
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": local, "added": added,
            "remote": {"uri": uri, "generation": generation}}}),
    )
}

/// New sources one directory add registers: the sources one workspace search reads together.
pub(super) const DIR_SOURCES: usize = super::query::QUERY_SOURCES;
/// Directory entries one directory add visits, links and subdirectories included.
pub(super) const DIR_ENTRIES: usize = 4096;

/// What a directory add would register under its root, and why every other entry was left.
struct Listing {
    root: PathBuf,
    files: Vec<PathBuf>,
    skipped: Vec<Json>,
    not_fig: usize,
    truncated: Json,
}

/// The `.fig` files under `dir` that pass the checks a single add makes, in name order. Only
/// real subdirectories are entered: a link whose target lies outside the root is skipped as
/// `escaping_link`, and a link to a directory inside it as `linked_directory` (its real path is
/// listed on its own). Skipped entries are named relative to the root, so a reply never names
/// where a link leads.
fn list_dir(storage: &Storage, place: &Place, dir: &Path) -> Result<Listing> {
    let unreadable = |e: std::io::Error| {
        fail(
            ErrorKind::SourceUnreadable,
            format!("{}: {e}", dir.display()),
        )
    };
    let root = std::fs::canonicalize(dir).map_err(unreadable)?;
    let mut l = Listing {
        root: root.clone(),
        files: Vec::new(),
        skipped: Vec::new(),
        not_fig: 0,
        truncated: Json::Null,
    };
    let skip = |l: &mut Listing, path: &Path, reason: &str| {
        let shown = path.strip_prefix(&root).unwrap_or(path);
        l.skipped
            .push(json!({"path": shown.to_string_lossy(), "reason": reason}));
    };
    let (mut visited, mut stack) = (0, vec![root.clone()]);
    'walk: while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == root => return Err(unreadable(e)),
            Err(_) => {
                skip(&mut l, &dir, "unreadable");
                continue;
            }
        };
        // The budget is checked before each entry is read, so a huge directory costs no more.
        let mut names = Vec::new();
        let mut out = false;
        for entry in entries {
            if visited == DIR_ENTRIES {
                l.truncated = json!({"reason": "entry_budget", "limit": DIR_ENTRIES});
                out = true;
                break;
            }
            visited += 1;
            match entry {
                Ok(entry) => names.push(entry.path()),
                Err(_) => skip(&mut l, &dir, "unreadable"),
            }
        }
        names.sort();
        let mut subdirs = Vec::new();
        for path in names {
            // Under a canonical root, an entry whose canonical path differs is a link.
            let Ok(real) = std::fs::canonicalize(&path) else {
                skip(&mut l, &path, "unreadable");
                continue;
            };
            if !real.starts_with(&root) {
                skip(&mut l, &path, "escaping_link");
                continue;
            }
            if std::fs::metadata(&real).is_ok_and(|m| m.is_dir()) {
                if real == path {
                    subdirs.push(path);
                } else {
                    skip(&mut l, &path, "linked_directory");
                }
                continue;
            }
            if !real
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("fig"))
            {
                l.not_fig += 1;
                continue;
            }
            let reason = match readable(&real) {
                Err(_) => Some("unreadable"),
                Ok(_) if real.to_str().is_none() => Some("unreadable"),
                Ok(_) if outside_storage(&real, storage, place).is_err() => Some("inside_storage"),
                Ok(_) if !storage.sees(&real) => Some("outside_roots"),
                // An in-root link to a file already listed registers nothing more.
                Ok(_) if l.files.contains(&real) => continue,
                Ok(_) => None,
            };
            if let Some(reason) = reason {
                skip(&mut l, &path, reason);
            } else if l.files.len() == DIR_SOURCES {
                l.truncated = json!({"reason": "source_cap", "limit": DIR_SOURCES});
                break 'walk;
            } else {
                l.files.push(real);
            }
        }
        if out {
            break;
        }
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(l)
}

/// What `index add <dir> --dry-run` would register, for a host to authorize first. Reads no
/// database and creates nothing.
pub fn list_add(storage: &Storage, select: &Select, dir: &Path) -> Result<Json> {
    let select = &storage.follow(select)?;
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
    let l = list_dir(storage, &place, dir)?;
    Ok(
        json!({"workspaceId": id, "directory": {"path": l.root, "files": l.files,
        "skipped": l.skipped, "notFig": l.not_fig, "truncated": l.truncated,
        "manifest": manifest(&l)}}),
    )
}

/// The sha256 of the sorted canonical files a listing would register and its truncation, so an
/// add can prove it registers exactly the set a host authorized from the dry run.
fn manifest(l: &Listing) -> String {
    let mut files: Vec<_> = l
        .files
        .iter()
        .map(|f| f.as_os_str().as_encoded_bytes())
        .collect();
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file);
        hash.update([0]);
    }
    hash.update(l.truncated.to_string());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// [`add`] of a directory bound to the `manifest` its dry run printed: when the files under it
/// changed since, nothing registers.
pub fn add_listed(storage: &Storage, select: &Select, dir: &Path, manifest: &str) -> Result<Json> {
    add_dir(storage, select, dir, Some(manifest))
}

/// Registers the `.fig` files under `dir` (see [`list_dir`]) in one write transaction: either
/// all of them or, on an error, none. Already registered paths keep their ids; nothing is
/// relocated, so a file moved inside the directory is a new source.
fn add_dir(storage: &Storage, select: &Select, dir: &Path, planned: Option<&str>) -> Result<Json> {
    let (place, mut ws) = opened(storage, select, true)?;
    if let Locator::Source(_) = ws.held.locator {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "a source-scoped workspace holds only its own file; prepare with --workspace <dir> to register a directory",
        ));
    }
    let tx = ws
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(sql)?;
    // Walked under the write lock, so the files checked against the manifest are those registered.
    let l = list_dir(storage, &place, dir)?;
    if planned.is_some_and(|m| m != manifest(&l)) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            "the .fig files under the directory changed since its dry run; run `index add <dir> --dry-run` again and add with its new manifest",
        ));
    }
    let (mut added, mut existing) = (Vec::new(), Vec::new());
    for file in &l.files {
        let (source_id, new) = register_in(&tx, file)?;
        let list = if new { &mut added } else { &mut existing };
        list.push(json!({"sourceId": source_id, "path": file}));
    }
    tx.commit().map_err(sql)?;
    opened_report(
        storage,
        &place,
        &ws,
        json!({"directory": {"path": l.root, "added": added, "existing": existing,
            "skipped": l.skipped, "notFig": l.not_fig, "truncated": l.truncated}}),
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
/// its published indexes from search, deletes its change tracking and history, and asks its
/// active jobs to stop. The `.fig` itself is never touched.
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
        // Its change history goes with it; a relocate keeps the id and so keeps the history.
        for q in [
            "DELETE FROM revision_changes WHERE rev IN (SELECT id FROM revisions WHERE source_id = ?1)",
            "DELETE FROM revision_impacts WHERE rev IN (SELECT id FROM revisions WHERE source_id = ?1)",
            "DELETE FROM revisions WHERE source_id = ?1",
            "DELETE FROM tracking WHERE source_id = ?1",
            "DELETE FROM consumers WHERE substr(filter, 1, length(?1) + 1) = ?1 || char(31)",
            "DELETE FROM subscriptions WHERE source_id = ?1",
            // A saved question over a set is disabled, never narrowed, and keeps no evaluation;
            // its reason names no member, which a caller seeing the rest may never have seen.
            &format!("UPDATE subscriptions SET disabled = json_object('reason', 'source_removed'),
             eval_failure = NULL WHERE source_id = '{SET}' AND disabled IS NULL
             AND json_valid(sources) AND EXISTS (SELECT 1 FROM json_each(sources) WHERE value = ?1)"),
            &format!("DELETE FROM query_evaluations WHERE sub IN (SELECT id FROM subscriptions
             WHERE source_id = '{SET}' AND json_valid(sources)
             AND EXISTS (SELECT 1 FROM json_each(sources) WHERE value = ?1))"),
            "DELETE FROM meta WHERE substr(key, 1, length(?1) + 8) = 'pruned' || char(31) || ?1 || char(31)",
            // Its watch leases end at once, kept until GC so their requestIds replay as ended.
            "UPDATE watch_leases SET ended = 'source_removed', authorized_path = NULL
             WHERE source_id = ?1 AND ended IS NULL",
            "DELETE FROM watch_monitor WHERE source_id = ?1",
        ] {
            tx.execute(q, [source_id]).map_err(sql)?;
        }
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
    // The new path's location lock comes before the workspace lock, the order prepare takes.
    let new_id = Locator::Source(source.clone()).id()?;
    let (locations, alias_lock) = location_exclusive(storage, &new_id)?;
    let (place, ws) = opened(storage, select, true)?;
    outside_storage(&source, storage, &place)?;
    if visible_path(storage, &ws.conn, source_id)?.is_none() || !storage.sees(&source) {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!("workspace {} has no source {source_id}", ws.held.id),
        ));
    }
    let remote: Option<String> = ws
        .conn
        .query_row(
            "SELECT remote FROM sources WHERE source_id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if remote.is_some() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "{source_id} is a gs:// object's download; add the object again to fetch a new generation, or remove it"
            ),
        ));
    }
    // A source-scoped workspace keeps its id: the new path's id becomes an alias of it, written
    // first, so an interruption leaves the alias pointing at the workspace still holding the
    // old path, and a retry completes the move.
    if let Locator::Source(_) = ws.held.locator
        && new_id != ws.held.id
    {
        let record = locations.join(format!("{new_id}.json"));
        let own = plain(&record, false)? && storage.alias(&new_id)?.as_deref() != Some(&ws.held.id);
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
    // Builds of the old path cannot publish any more; stop them.
    ws.conn
        .execute(
            "UPDATE jobs SET cancel = 1 WHERE source_id = ?1 AND state IN ('queued', 'running')",
            [source_id],
        )
        .map_err(sql)?;
    drop(alias_lock);
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
    // A caller confined to some roots may not delete sources it cannot see.
    if storage.visible.is_some()
        && let Some((_, ws)) = match opened(storage, select, false) {
            Ok(opened) => Some(opened),
            Err(e) if gone(&*e) => None,
            Err(e) => {
                return Err(fail(
                    ErrorKind::PermissionDenied,
                    format!(
                        "workspace {id}'s sources cannot be checked against this server's roots ({e}); reset it from the CLI"
                    ),
                ));
            }
        }
    {
        let mut stmt = ws.conn.prepare("SELECT path FROM sources").map_err(sql)?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql)?;
        if !paths.iter().all(|p| storage.sees(Path::new(p))) {
            return Err(fail(
                ErrorKind::PermissionDenied,
                format!(
                    "workspace {id} holds sources outside this server's roots; reset it from the CLI"
                ),
            ));
        }
    }
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
        // Watch leases end too, so the runner (which holds a shared lock) exits before the
        // exclusive lock below gives up; their holders forget the workspace at next renewal.
        if ws.version >= 5 {
            ws.conn
                .execute(
                    "UPDATE watch_leases SET ended = 'reset', authorized_path = NULL
                     WHERE ended IS NULL",
                    [],
                )
                .map_err(sql)?;
        }
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
            "nextSteps": prepare_step(select.locator(), "prepare initializes it again in the same storage with a new incarnation; earlier sourceIds must be added again")}),
    ))
}
