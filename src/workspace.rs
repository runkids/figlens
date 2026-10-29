//! Local workspaces: the figlens application roots, where each workspace's storage lives,
//! workspace identity and ownership, and the SQLite registry of a workspace's sources. Load
//! the `workspace` wiki topic before changing it.

use std::error::Error;
use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

use crate::{ErrorKind, FigError};

/// The registry schema this binary reads and writes; any other version is left untouched.
pub const SCHEMA: i64 = 1;
/// The version of a remembered workspace location record.
const LOCATION: i64 = 1;
/// The longest wait for another process's workspace lock.
#[cfg(not(test))]
const LOCK_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const LOCK_WAIT: Duration = Duration::from_secs(2);
const MARKER: &str = "workspace.json";
const SMALL_FILE: u64 = 64 * 1024;
const DB: &str = "index.sqlite";
/// The database and every file SQLite keeps beside it; reset removes exactly these.
const DB_FILES: [&str; 4] = [
    DB,
    "index.sqlite-wal",
    "index.sqlite-shm",
    "index.sqlite-journal",
];

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn fail(kind: ErrorKind, message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(kind, message).into()
}

/// A storage write failure names its path and never moves on to another directory.
fn storage_error(path: &Path, error: std::io::Error) -> Box<dyn Error> {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            ErrorKind::PermissionDenied
        }
        _ => ErrorKind::OperationFailed,
    };
    fail(
        kind,
        format!(
            "cannot use figlens storage at {}: {error}; fix its permissions or pass a writable --storage <dir>; nothing was written elsewhere",
            path.display()
        ),
    )
}

fn preserved(path: &Path, why: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::OperationFailed,
        format!(
            "{} {why}; figlens does not own it and left it as found",
            path.display()
        ),
    )
}

fn utf8(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        fail(
            ErrorKind::InvalidArguments,
            format!("{} is not valid UTF-8", path.display()),
        )
    })
}

/// Whether `path` exists as a plain directory (`dir`) or plain file, never following a
/// symlink. Another kind of entry is refused and preserved.
fn plain(path: &Path, dir: bool) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if (dir && meta.is_dir()) || (!dir && meta.is_file()) => Ok(true),
        Ok(_) => Err(preserved(
            path,
            if dir {
                "is not a plain directory"
            } else {
                "is not a plain file"
            },
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(storage_error(path, e)),
    }
}

/// Creates a plain directory unless it exists; true when this call created it.
fn mkdir(path: &Path) -> Result<bool> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            plain(path, true)?;
            Ok(false)
        }
        Err(e) => Err(storage_error(path, e)),
    }
}

fn read_small(path: &Path) -> Result<Json> {
    let mut text = String::new();
    File::open(path)
        .and_then(|f| f.take(SMALL_FILE).read_to_string(&mut text))
        .map_err(|e| storage_error(path, e))?;
    serde_json::from_str(&text).map_err(|_| preserved(path, "is not a figlens record"))
}

/// Writes `value` to `path` through a sibling temporary file and a rename.
/// Callers hold the lock that serializes writers of `path`, so a leftover `.part` is stale.
fn write_small(path: &Path, value: &Json) -> Result<()> {
    let part = path.with_extension("json.part");
    if plain(&part, false)? {
        std::fs::remove_file(&part).map_err(|e| storage_error(&part, e))?;
    }
    // create_new never follows a link planted at `part`.
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{value}\n").as_bytes()))
        .and_then(|()| std::fs::rename(&part, path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&part);
        storage_error(path, e)
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

// ---- Application roots ----

/// One resolved application root and the rule that chose it.
#[derive(Debug, Clone, PartialEq)]
pub struct Root {
    pub path: PathBuf,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Roots {
    pub config: Root,
    pub state: Root,
    pub cache: Root,
    pub data: Root,
}

impl Roots {
    fn report(&self) -> Json {
        let root = |r: &Root| json!({"path": r.path, "rule": r.rule});
        json!({"config": root(&self.config), "state": root(&self.state),
            "cache": root(&self.cache), "data": root(&self.data)})
    }

    /// State, cache and data under the application root `dir`; config never moves, so the
    /// remembered locations stay findable.
    fn under(&self, dir: &Path, rule: &str) -> Self {
        let root = |kind: &str| Root {
            path: dir.join(kind),
            rule: rule.into(),
        };
        Self {
            config: self.config.clone(),
            state: root("state"),
            cache: root("cache"),
            data: root("data"),
        }
    }
}

/// The default roots and the command's explicit `--storage` application root, if any.
#[derive(Debug, Clone)]
pub struct Storage {
    defaults: Roots,
    explicit: Option<PathBuf>,
}

/// (name, XDG variable, default under HOME) for each root.
const KINDS: [(&str, &str, &str); 4] = [
    ("config", "XDG_CONFIG_HOME", ".config"),
    ("state", "XDG_STATE_HOME", ".local/state"),
    ("cache", "XDG_CACHE_HOME", ".cache"),
    ("data", "XDG_DATA_HOME", ".local/share"),
];

/// Resolves the default roots from the environment and validates `explicit`. Nothing is
/// created here.
pub fn storage(explicit: Option<&Path>) -> Result<Storage> {
    resolve(
        explicit,
        &|name| std::env::var_os(name),
        cfg!(windows),
        &local_app_data,
    )
}

fn resolve(
    explicit: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
    windows: bool,
    known: &dyn Fn() -> Option<PathBuf>,
) -> Result<Storage> {
    if let Some(dir) = explicit
        && (!dir.is_absolute() || (dir.exists() && !dir.is_dir()))
    {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "--storage {} must be an absolute directory path",
                dir.display()
            ),
        ));
    }
    if let Some(dir) = explicit {
        // Checked now: a prepare must not build a database it cannot remember.
        utf8(dir)?;
    }
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let default = |(kind, xdg, home): (&str, &str, &str)| -> Result<Root> {
        if let Some(dir) = absolute(xdg) {
            return Ok(Root {
                path: dir.join("figlens"),
                rule: xdg.into(),
            });
        }
        if windows {
            return windows_root(kind, absolute("LOCALAPPDATA"), known);
        }
        let base = absolute("HOME").ok_or_else(|| {
            fail(
                ErrorKind::InvalidArguments,
                format!("no {kind} directory: HOME is not an absolute path; set HOME or {xdg} (config is needed even with --storage)"),
            )
        })?;
        Ok(Root {
            path: base.join(home).join("figlens"),
            rule: "HOME".into(),
        })
    };
    let [config, state, cache, data] = KINDS;
    Ok(Storage {
        defaults: Roots {
            config: default(config)?,
            state: default(state)?,
            cache: default(cache)?,
            data: default(data)?,
        },
        explicit: explicit.map(Path::to_path_buf),
    })
}

/// Windows has no HOME requirement: %LOCALAPPDATA%, then its Known Folder.
fn windows_root(
    kind: &str,
    local: Option<PathBuf>,
    known: &dyn Fn() -> Option<PathBuf>,
) -> Result<Root> {
    let (dir, rule) = match local {
        Some(dir) => (dir, "LOCALAPPDATA"),
        None => (
            known().filter(|p| p.is_absolute()).ok_or_else(|| {
                fail(ErrorKind::InvalidArguments, format!(
                    "no {kind} directory: %LOCALAPPDATA% and the Local AppData known folder are unavailable; set %LOCALAPPDATA%"
                ))
            })?,
            "FOLDERID_LocalAppData",
        ),
    };
    Ok(Root {
        path: dir.join("figlens").join(kind),
        rule: rule.into(),
    })
}

#[cfg(windows)]
fn local_app_data() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };
    let mut raw = std::ptr::null_mut();
    // SAFETY: valid GUID and out pointer; the shell allocates `raw`, freed below in every case.
    let ok = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut raw,
        )
    } >= 0;
    let path = (ok && !raw.is_null()).then(|| {
        // SAFETY: on success `raw` is a NUL-terminated UTF-16 string.
        let len = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
        PathBuf::from(OsString::from_wide(unsafe {
            std::slice::from_raw_parts(raw, len)
        }))
    });
    // SAFETY: `raw` came from SHGetKnownFolderPath (or is null, which CoTaskMemFree accepts).
    unsafe { CoTaskMemFree(raw as *const _) };
    path
}

#[cfg(not(windows))]
fn local_app_data() -> Option<PathBuf> {
    None
}

// ---- Identity ----

/// What a workspace is for: a selected project directory, or one source when there is none.
#[derive(Debug, Clone, PartialEq)]
pub enum Locator {
    Project(PathBuf),
    Source(PathBuf),
}

impl Locator {
    /// The project directory `dir`, by its canonical path.
    pub fn project(dir: &Path) -> Result<Self> {
        let canonical = std::fs::canonicalize(dir)
            .ok()
            .filter(|p| p.is_dir())
            .ok_or_else(|| {
                fail(
                    ErrorKind::InvalidArguments,
                    format!("--workspace {} is not a directory", dir.display()),
                )
            })?;
        Ok(Self::Project(canonical))
    }

    /// The source-scoped workspace of the `.fig` at `file`.
    pub fn source(file: &Path) -> Result<Self> {
        Ok(Self::Source(readable(file)?))
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Project(_) => "project",
            Self::Source(_) => "source",
        }
    }

    fn path(&self) -> Result<&str> {
        let (Self::Project(p) | Self::Source(p)) = self;
        utf8(p)
    }

    /// `ws_` and 64 bits of the SHA-256 of kind and canonical path. The marker, location record
    /// and database keep the full locator, so a colliding id is refused instead of reused.
    pub fn id(&self) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(self.kind());
        hash.update([0]);
        hash.update(self.path()?);
        Ok(format!(
            "ws_{}",
            &crate::search::hex(&hash.finalize())[..16]
        ))
    }

    fn report(&self) -> Result<Json> {
        Ok(json!({"kind": self.kind(), "path": self.path()?}))
    }

    fn parse(value: &Json) -> Option<Self> {
        let path = PathBuf::from(value["path"].as_str()?);
        match value["kind"].as_str()? {
            "project" => Some(Self::Project(path)),
            "source" => Some(Self::Source(path)),
            _ => None,
        }
    }
}

/// How a command names its workspace: by locator, or by the id an earlier call returned.
#[derive(Debug, Clone)]
pub enum Select {
    Locator(Locator),
    Id(String),
}

impl Select {
    /// At most one of a project directory, a `.fig` (its source-scoped workspace) and an id.
    pub fn from_choice(
        workspace: Option<&Path>,
        file: Option<&Path>,
        id: Option<&str>,
    ) -> Result<Option<Self>> {
        Ok(match (workspace, file, id) {
            (None, None, None) => None,
            (Some(dir), None, None) => Some(Self::Locator(Locator::project(dir)?)),
            (None, Some(file), None) => Some(Self::Locator(Locator::source(file)?)),
            (None, None, Some(id)) => Some(Self::Id(id.to_owned())),
            _ => {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    "choose one of --workspace, --file and --id",
                ));
            }
        })
    }

    fn id(&self) -> Result<String> {
        match self {
            Self::Locator(l) => l.id(),
            Self::Id(id) => {
                let hex = id.strip_prefix("ws_").unwrap_or("");
                if hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    Ok(id.clone())
                } else {
                    Err(fail(
                        ErrorKind::InvalidArguments,
                        format!(
                            "{id:?} is not a workspace id; ids look like ws_ and 16 hex digits"
                        ),
                    ))
                }
            }
        }
    }

    fn locator(&self) -> Option<&Locator> {
        match self {
            Self::Locator(l) => Some(l),
            Self::Id(_) => None,
        }
    }
}

/// The canonical path of a readable regular file.
fn readable(file: &Path) -> Result<PathBuf> {
    let source = |e: std::io::Error| {
        fail(
            ErrorKind::SourceUnreadable,
            format!("{}: {e}", file.display()),
        )
    };
    let path = std::fs::canonicalize(file).map_err(source)?;
    if !File::open(&path)
        .and_then(|f| f.metadata())
        .map_err(source)?
        .is_file()
    {
        return Err(fail(
            ErrorKind::SourceUnreadable,
            format!("{} is not a regular .fig file", file.display()),
        ));
    }
    Ok(path)
}

/// Fails when `source` lies inside any figlens root, default or placed, which an index must
/// never alias.
fn outside_storage(source: &Path, storage: &Storage, place: &Place) -> Result<()> {
    let (d, p) = (&storage.defaults, &place.roots);
    for root in [d, p]
        .map(|r| [&r.config.path, &r.state.path, &r.cache.path, &r.data.path])
        .concat()
    {
        if let Ok(root) = std::fs::canonicalize(root)
            && source.starts_with(&root)
        {
            return Err(fail(
                ErrorKind::PermissionDenied,
                format!(
                    "{} is inside figlens storage {}; keep designs outside the storage roots",
                    source.display(),
                    root.display()
                ),
            ));
        }
    }
    Ok(())
}

// ---- Remembered locations ----

/// Where one workspace's storage lives, and why.
struct Place {
    roots: Roots,
    /// `--storage`, `remembered` or `default`.
    rule: &'static str,
    /// The remembered application root, when there is one.
    remembered: Option<PathBuf>,
}

impl Place {
    fn report(&self) -> Json {
        json!({"rule": self.rule, "remembered": self.remembered})
    }
}

impl Storage {
    fn locations(&self) -> PathBuf {
        self.defaults.config.path.join("workspaces")
    }

    /// The remembered application root of workspace `id` and its locator, read from the fixed
    /// config directory. It only locates the workspace; ownership is checked at the workspace.
    fn remembered(&self, id: &str) -> Result<Option<(PathBuf, Locator)>> {
        let path = self.locations().join(format!("{id}.json"));
        if !plain(&path, false)? {
            return Ok(None);
        }
        let record = read_small(&path)?;
        if record["version"] != LOCATION {
            return Err(fail(
                ErrorKind::IndexIncompatible,
                format!(
                    "{} has location version {}; this figlens reads version {LOCATION} and left it untouched",
                    path.display(),
                    record["version"]
                ),
            ));
        }
        let storage = record["storage"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        match (
            record["workspaceId"] == id,
            storage,
            Locator::parse(&record["locator"]),
        ) {
            (true, Some(storage), Some(locator)) => Ok(Some((storage, locator))),
            _ => Err(preserved(&path, "is not this workspace's location record")),
        }
    }

    /// Precedence: this command's `--storage`, then the workspace's remembered root, then the
    /// default roots. A remembered root that is gone is an error, never a second database.
    fn place(&self, id: &str, wanted: Option<&Locator>) -> Result<Place> {
        let remembered = self.remembered(id)?;
        if let (Some((_, found)), Some(wanted)) = (&remembered, wanted)
            && found != wanted
        {
            return Err(preserved(
                &self.locations().join(format!("{id}.json")),
                "records a different workspace with the same id",
            ));
        }
        let remembered = remembered.map(|(root, _)| root);
        // Its database may still be there, so even --storage must not start another one;
        // retrying the same --storage recreates its own place, which is not a second database.
        if let Some(dir) = &remembered
            && !self.explicit.as_ref().is_some_and(|e| same_dir(e, dir))
        {
            let cause = match std::fs::metadata(dir) {
                Ok(meta) if meta.is_dir() => None,
                Ok(_) => Some("it is not a directory".to_owned()),
                Err(e) => Some(e.to_string()),
            };
            if let Some(cause) = cause {
                return Err(fail(
                    ErrorKind::OperationFailed,
                    format!(
                        "workspace {id} is remembered at {}, which is unavailable ({cause}); restore it, or delete {} to forget it; no other database was created",
                        dir.display(),
                        self.locations().join(format!("{id}.json")).display()
                    ),
                ));
            }
        }
        if let Some(explicit) = &self.explicit {
            // The workspace lives where it would without --storage; another root must not shadow
            // or duplicate a database there.
            let home = remembered.as_ref().map_or(self.defaults.clone(), |r| {
                self.defaults.under(r, "remembered")
            });
            let current = home.state.path.join("workspaces").join(id).join(DB);
            if !same_dir(&home.state.path, &explicit.join("state")) && current.exists() {
                return Err(fail(
                    ErrorKind::InvalidArguments,
                    format!(
                        "workspace {id} has its database at {}; omit --storage to use it, or reset it there before using --storage {}",
                        current.display(),
                        explicit.display()
                    ),
                ));
            }
        }
        Ok(match (&self.explicit, &remembered) {
            (Some(dir), _) => Place {
                roots: self.defaults.under(dir, "--storage"),
                rule: "--storage",
                remembered,
            },
            (None, Some(dir)) => Place {
                roots: self.defaults.under(dir, "remembered"),
                rule: "remembered",
                remembered: remembered.clone(),
            },
            (None, None) => Place {
                roots: self.defaults.clone(),
                rule: "default",
                remembered: None,
            },
        })
    }
}

// ---- Owned directories and locks ----

/// `<state>/workspaces`, through the canonical root and a real directory, so a symlinked
/// leaf cannot redirect writes or cleanup. Created only when `create`.
fn workspaces(state: &Path, create: bool) -> Result<Option<PathBuf>> {
    if create {
        std::fs::create_dir_all(state).map_err(|e| storage_error(state, e))?;
    }
    let root = match std::fs::canonicalize(state) {
        Ok(root) => root,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(storage_error(state, e)),
    };
    let dir = root.join("workspaces");
    if create {
        mkdir(&dir)?;
        return Ok(Some(dir));
    }
    Ok(plain(&dir, true)?.then_some(dir))
}

/// Opens the lock file at `path`, creating it when `create`. Every database connection holds
/// it shared; prepare and reset hold it exclusive. It is never removed, so all processes
/// lock the same file.
fn open_lock(path: &Path, create: bool) -> Result<Option<File>> {
    if !plain(path, false)? && !create {
        return Ok(None);
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .open(path)
        .map(Some)
        .map_err(|e| storage_error(path, e))
}

/// Takes the lock within `LOCK_WAIT`, or reports the workspace busy. std has no timed lock.
fn lock(file: &File, path: &Path, exclusive: bool, id: &str) -> Result<()> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let attempt = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        match attempt {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(TryLockError::WouldBlock) => {
                return Err(fail(
                    ErrorKind::WorkspaceBusy,
                    format!(
                        "workspace {id} is in use by another figlens process; retry when it finishes"
                    ),
                ));
            }
            Err(TryLockError::Error(e)) => return Err(storage_error(path, e)),
        }
    }
}

fn marker(id: &str, locator: &Locator) -> Result<Json> {
    Ok(
        json!({"owner": "figlens", "kind": "workspace", "workspaceId": id,
        "locator": locator.report()?}),
    )
}

/// The marker's locator when `dir` holds a figlens marker for `id`; any other marker is refused.
fn read_marker(dir: &Path, id: &str) -> Result<Option<Locator>> {
    let path = dir.join(MARKER);
    if !plain(&path, false)? {
        return Ok(None);
    }
    let value = read_small(&path)?;
    (value["owner"] == "figlens" && value["kind"] == "workspace" && value["workspaceId"] == id)
        .then(|| Locator::parse(&value["locator"]))
        .flatten()
        .map(Some)
        .ok_or_else(|| preserved(&path, "is not this workspace's figlens marker"))
}

/// A workspace directory under its held lock.
struct Held {
    id: String,
    dir: PathBuf,
    locator: Locator,
    _lock: File,
}

/// Locks workspace `id` exclusively and returns it, creating its directory and marker first
/// when missing. The lock sits beside the directory, so a concurrent prepare waits here and
/// never sees a directory without its marker. An existing unmarked directory is preserved.
fn acquire_new(state: &Path, id: &str, locator: &Locator) -> Result<Held> {
    let parent = workspaces(state, true)?.expect("created");
    let lock_path = parent.join(format!("{id}.lock"));
    let lock_file = open_lock(&lock_path, true)?.expect("created");
    lock(&lock_file, &lock_path, true, id)?;
    let dir = parent.join(id);
    mkdir(&dir)?;
    match read_marker(&dir, id)? {
        Some(found) if found == *locator => {}
        Some(_) => return Err(preserved(&dir, SAME_ID)),
        // Empty, or left by a prepare interrupted before its marker: claiming it loses nothing.
        None if unclaimed(&dir)? => write_small(&dir.join(MARKER), &marker(id, locator)?)?,
        None => return Err(preserved(&dir, "has no figlens workspace marker")),
    }
    Ok(Held {
        id: id.to_owned(),
        dir,
        locator: locator.clone(),
        _lock: lock_file,
    })
}

const SAME_ID: &str = "belongs to a different workspace with the same id";

/// Whether `dir` holds nothing but an unfinished marker write.
fn unclaimed(dir: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(dir).map_err(|e| storage_error(dir, e))? {
        let entry = entry.map_err(|e| storage_error(dir, e))?;
        if entry.file_name() != "workspace.json.part"
            || !entry.file_type().is_ok_and(|t| t.is_file())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Locks an existing workspace, or `None` when it is absent. Reading (`!exclusive`) creates no
/// file; reset may recreate a lost lock file because the marker proves ownership.
fn acquire(state: &Path, select: &Select, exclusive: bool) -> Result<Option<Held>> {
    let id = select.id()?;
    let Some(parent) = workspaces(state, false)? else {
        return Ok(None);
    };
    let dir = parent.join(&id);
    if !plain(&dir, true)? {
        return Ok(None);
    }
    let lock_path = parent.join(format!("{id}.lock"));
    let lock_file = open_lock(&lock_path, exclusive)?.ok_or_else(|| {
        fail(
            ErrorKind::OperationFailed,
            format!(
                "{} is missing; `figlens index prepare` or `figlens index reset` recreates it",
                lock_path.display()
            ),
        )
    })?;
    lock(&lock_file, &lock_path, exclusive, &id)?;
    let locator = read_marker(&dir, &id)?
        .ok_or_else(|| preserved(&dir, "has no figlens workspace marker"))?;
    if select.locator().is_some_and(|wanted| *wanted != locator) {
        return Err(preserved(&dir, SAME_ID));
    }
    Ok(Some(Held {
        id,
        dir,
        locator,
        _lock: lock_file,
    }))
}

// ---- Database ----

fn sql(error: rusqlite::Error) -> Box<dyn Error> {
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
fn unaliased(dir: &Path) -> Result<()> {
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
fn links(path: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::symlink_metadata(path)
        .map_err(|e| storage_error(path, e))?
        .nlink())
}

#[cfg(windows)]
fn links(path: &Path) -> Result<u64> {
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

fn open(path: &Path, flags: OpenFlags) -> Result<Connection> {
    unaliased(path.parent().expect("a workspace file"))?;
    let conn =
        Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sql)?;
    conn.busy_timeout(LOCK_WAIT).map_err(sql)?;
    Ok(conn)
}

fn is_empty(conn: &Connection) -> Result<bool> {
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(sql)?;
    Ok(tables == 0)
}

fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

/// The incarnation of this workspace's registry, or `None` for an empty database. Schema and
/// identity are read before any write: another schema or workspace is refused untouched.
fn contents(conn: &Connection, path: &Path, held: &Held) -> Result<Option<String>> {
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
    if version != SCHEMA {
        return Err(fail(
            ErrorKind::IndexIncompatible,
            format!(
                "{} has workspace schema {version}; this figlens reads schema {SCHEMA} and left it untouched",
                path.display()
            ),
        ));
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
    Ok(Some(
        meta(conn, "incarnation").map_err(sql)?.unwrap_or_default(),
    ))
}

/// Durable WAL settings for a writing connection.
fn writable(conn: &Connection) -> Result<()> {
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
fn initialize(conn: &mut Connection, held: &Held) -> Result<String> {
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
    tx.execute_batch(&format!("PRAGMA user_version = {SCHEMA};"))
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(incarnation)
}

/// An initialized workspace opened for one command under its held lock.
struct Opened {
    incarnation: String,
    // Declared before `held`, so the connection closes before the lock is released.
    conn: Connection,
    held: Held,
}

/// Opens the database of a held workspace, or `None` when it has none (after reset).
fn open_held(held: Held, write: bool) -> Result<Option<Opened>> {
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
    let Some(incarnation) = contents(&conn, &path, &held)? else {
        return Ok(None);
    };
    if write {
        writable(&conn)?;
    }
    Ok(Some(Opened {
        incarnation,
        conn,
        held,
    }))
}

fn missing(id: &str) -> Box<dyn Error> {
    fail(
        ErrorKind::InvalidArguments,
        format!("workspace {id} is not prepared; run `figlens index prepare <file.fig>` first"),
    )
}

/// The `sourceId` registered for `source`, adding it when new; true when added.
fn register(conn: &mut Connection, source: &Path) -> Result<(String, bool)> {
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
        "INSERT INTO sources (source_id, path, added_at_ms) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, path, now_ms()],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok((id, true))
}

fn sources(conn: &Connection) -> Result<Vec<Json>> {
    let mut stmt = conn
        .prepare("SELECT source_id, path FROM sources ORDER BY added_at_ms, source_id")
        .map_err(sql)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?;
    rows.map(|row| {
        let (id, path) = row.map_err(sql)?;
        let present = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
        Ok(json!({"sourceId": id, "path": path, "present": present}))
    })
    .collect()
}

// ---- Reports ----

/// Every report's common fields, plus `extra`.
fn report(place: &Place, id: &str, dir: Option<&Path>, state: &str, extra: Json) -> Json {
    let mut out = json!({"workspaceId": id, "state": state, "schemaVersion": SCHEMA,
        "storage": place.report(), "roots": place.roots.report(),
        "paths": dir.map(|d| json!({"workspace": d, "database": d.join(DB)}))});
    if let (Some(out), Json::Object(extra)) = (out.as_object_mut(), extra) {
        out.extend(extra);
    }
    out
}

/// The report of an initialized workspace: its incarnation, locator and sources, plus `extra`.
fn opened_report(place: &Place, ws: &Opened, extra: Json) -> Result<Json> {
    let mut extra = extra;
    extra["incarnation"] = json!(ws.incarnation);
    extra["locator"] = ws.held.locator.report()?;
    extra["sources"] = json!(sources(&ws.conn)?);
    extra["nextSteps"] = json!([NEXT_SEARCH]);
    Ok(report(
        place,
        &ws.held.id,
        Some(&ws.held.dir),
        "initialized",
        extra,
    ))
}

/// The milestone note every initialized report carries: the registry exists, its index does not.
const NEXT_SEARCH: &str = "search still reads the .fig directly: `figlens search <file.fig> <query>`; this workspace holds the source registry only, no index yet";

// ---- Actions ----

/// Opens the selected, initialized workspace with its storage place.
fn opened(storage: &Storage, select: &Select, write: bool) -> Result<(Place, Opened)> {
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
    let held = acquire(&place.roots.state.path, select, false)?.ok_or_else(|| missing(&id))?;
    let ws = open_held(held, write)?.ok_or_else(|| missing(&id))?;
    Ok((place, ws))
}

/// Creates or reuses the workspace for `file` (source-scoped, or the `project` directory's)
/// and registers the file. Concurrent calls for one workspace create one database. With
/// a new `--storage`, the workspace's location is remembered before its database is created.
pub fn prepare(storage: &Storage, file: &Path, project: Option<&Path>) -> Result<Json> {
    let source = readable(file)?;
    let locator = match project {
        Some(dir) => Locator::project(dir)?,
        None => Locator::Source(source.clone()),
    };
    let id = locator.id()?;
    // One prepare per workspace at a time across every storage root, so two roots never both
    // initialize it and its location record is never written concurrently.
    let locations = storage.locations();
    std::fs::create_dir_all(&locations).map_err(|e| storage_error(&locations, e))?;
    let config_path = locations.join(format!("{id}.lock"));
    let config_lock = open_lock(&config_path, true)?.expect("created");
    lock(&config_lock, &config_path, true, &id)?;
    let place = storage.place(&id, Some(&locator))?;
    outside_storage(&source, storage, &place)?;
    // Remember a new --storage before creating anything there: a database normal discovery
    // cannot find is never left behind, and a failed initialization is retried in that place.
    if let Some(explicit) = &storage.explicit
        && !place
            .remembered
            .as_ref()
            .is_some_and(|r| same_dir(r, explicit))
    {
        std::fs::create_dir_all(explicit).map_err(|e| storage_error(explicit, e))?;
        let canonical = std::fs::canonicalize(explicit).map_err(|e| storage_error(explicit, e))?;
        let record = json!({"version": LOCATION, "workspaceId": id,
            "locator": locator.report()?, "storage": utf8(&canonical)?});
        write_small(&locations.join(format!("{id}.json")), &record)?;
    }
    let held = acquire_new(&place.roots.state.path, &id, &locator)?;
    let path = held.dir.join(DB);
    let flags = if plain(&path, false)? {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    let mut conn = open(&path, flags)?;
    let found = contents(&conn, &path, &held)?;
    writable(&conn)?;
    let (incarnation, initialized) = match found {
        Some(incarnation) => (incarnation, false),
        None => (initialize(&mut conn, &held)?, true),
    };
    let (source_id, added) = register(&mut conn, &source)?;
    let ws = Opened {
        incarnation,
        conn,
        held,
    };
    opened_report(
        &Place {
            remembered: storage.explicit.clone().or(place.remembered.clone()),
            ..place
        },
        &ws,
        json!({"initialized": initialized,
            "source": {"sourceId": source_id, "path": source, "added": added}}),
    )
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .is_some_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b))
}

/// The workspace's state and sources. A missing workspace creates nothing at all; an existing
/// one is read through a read-only connection.
pub fn status(storage: &Storage, select: &Select) -> Result<Json> {
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
    match open_held(held, false)? {
        Some(ws) => opened_report(&place, &ws, json!({})),
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
fn project_only(ws: &Opened, file: Option<&Path>) -> Result<()> {
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
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": source, "added": added}}),
    )
}

/// Removes `source_id` from the workspace's registry. The `.fig` itself is never touched.
pub fn remove(storage: &Storage, select: &Select, source_id: &str) -> Result<Json> {
    let (place, ws) = opened(storage, select, true)?;
    let removed = ws
        .conn
        .execute("DELETE FROM sources WHERE source_id = ?1", [source_id])
        .map_err(sql)?
        > 0;
    opened_report(
        &place,
        &ws,
        json!({"sourceId": source_id, "removed": removed}),
    )
}

/// Points `source_id` at `file`, keeping its id.
pub fn relocate(storage: &Storage, select: &Select, source_id: &str, file: &Path) -> Result<Json> {
    let source = readable(file)?;
    let (place, ws) = opened(storage, select, true)?;
    project_only(&ws, None)?;
    outside_storage(&source, storage, &place)?;
    let changed = match ws.conn.execute(
        "UPDATE sources SET path = ?1 WHERE source_id = ?2",
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
        &place,
        &ws,
        json!({"source": {"sourceId": source_id, "path": source}}),
    )
}

/// Confirms a database is this workspace's, whatever its schema, before reset deletes it.
fn resettable(conn: &Connection, path: &Path, id: &str) -> Result<()> {
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
    let id = select.id()?;
    let place = storage.place(&id, select.locator())?;
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

#[cfg(test)]
mod tests;
