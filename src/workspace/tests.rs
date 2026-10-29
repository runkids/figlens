use super::*;
use crate::test_support::TestDir;

/// `prepare` of the registry alone, as schema 1 tests call it.
fn prepare(storage: &Storage, file: &Path, project: Option<&Path>) -> Result<Json> {
    super::prepare(storage, file, project, PrepareOptions::default()).map(|o| o.report)
}

struct Fixture {
    dir: TestDir,
    home: PathBuf,
    storage: Storage,
}

impl Fixture {
    fn new() -> Self {
        let dir = TestDir::new();
        let home = std::path::absolute(dir.join("home")).unwrap();
        let storage = Self::at(&home, None);
        Self { dir, home, storage }
    }

    /// Storage with HOME at `home` and no XDG variables, plus an optional `--storage`.
    fn at(home: &Path, explicit: Option<&Path>) -> Storage {
        let home = home.to_str().unwrap().to_owned();
        resolve(explicit, &env(&[("HOME", &home)]), false, &|| None).unwrap()
    }

    fn custom(&self, name: &str) -> (PathBuf, Storage) {
        let root = std::path::absolute(self.dir.join(name)).unwrap();
        let storage = Self::at(&self.home, Some(&root));
        (root, storage)
    }

    fn fig(&self, name: &str) -> PathBuf {
        let path = std::path::absolute(self.dir.join(name)).unwrap();
        std::fs::write(&path, format!("design {name}")).unwrap();
        path
    }

    fn workspace(&self, id: &str) -> PathBuf {
        self.storage.defaults.state.path.join("workspaces").join(id)
    }
}

fn id_of(report: &Json) -> String {
    report["workspaceId"].as_str().unwrap().to_owned()
}

fn kind(error: &(dyn Error + 'static)) -> ErrorKind {
    error.downcast_ref::<FigError>().unwrap().kind()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| {
        vars.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

fn source(fig: &Path) -> Select {
    Select::Locator(Locator::source(fig).unwrap())
}

fn root(path: &str, rule: &str) -> Root {
    Root {
        path: path.into(),
        rule: rule.into(),
    }
}

#[cfg(unix)]
#[test]
fn roots_follow_absolute_xdg_then_home_and_storage_never_moves_config() {
    let none = || None;
    let vars = env(&[
        ("HOME", "/h"),
        ("XDG_STATE_HOME", "/x"),
        ("XDG_CACHE_HOME", "relative"),
        ("XDG_CONFIG_HOME", ""),
    ]);
    let roots = resolve(None, &vars, false, &none).unwrap().defaults;
    assert_eq!(roots.state, root("/x/figlens", "XDG_STATE_HOME"));
    assert_eq!(roots.cache, root("/h/.cache/figlens", "HOME"));
    assert_eq!(roots.config, root("/h/.config/figlens", "HOME"));
    assert_eq!(roots.data, root("/h/.local/share/figlens", "HOME"));
    let custom = resolve(Some(Path::new("/s")), &vars, false, &none).unwrap();
    let under = custom.defaults.under(Path::new("/s"), "--storage");
    assert_eq!(under.state, root("/s/state", "--storage"));
    assert_eq!(under.data, root("/s/data", "--storage"));
    assert_eq!(under.config, roots.config);
}

#[cfg(unix)]
#[test]
fn windows_roots_use_local_app_data_then_its_known_folder_without_home() {
    let known = || Some(PathBuf::from("/k"));
    let roots = resolve(None, &env(&[("LOCALAPPDATA", "/l")]), true, &known)
        .unwrap()
        .defaults;
    assert_eq!(roots.state, root("/l/figlens/state", "LOCALAPPDATA"));
    assert_eq!(roots.config, root("/l/figlens/config", "LOCALAPPDATA"));
    let roots = resolve(None, &env(&[]), true, &known).unwrap().defaults;
    assert_eq!(
        roots.cache,
        root("/k/figlens/cache", "FOLDERID_LocalAppData")
    );
    assert!(resolve(None, &env(&[("HOME", "/h")]), true, &|| None).is_err());
}

#[test]
fn roots_refuse_a_relative_storage_or_missing_home() {
    let none = || None;
    let error = resolve(
        Some(Path::new("store")),
        &env(&[("HOME", "/h")]),
        false,
        &none,
    )
    .unwrap_err();
    assert_eq!(kind(&*error), ErrorKind::InvalidArguments);
    assert!(resolve(None, &env(&[("HOME", "home")]), false, &none).is_err());
}

#[cfg(windows)]
#[test]
fn a_sidecar_being_deleted_is_waited_out_not_reported_denied() {
    let dir = TestDir::new();
    std::fs::write(dir.join("index.sqlite"), b"").unwrap();
    let shm = dir.join("index.sqlite-shm");
    std::fs::write(&shm, b"").unwrap();
    let held = mark_deleted(&shm, false);
    assert!(
        sidecar_denied(&shm),
        "{:?}",
        std::fs::symlink_metadata(&shm)
    );
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
    });
    unaliased(&dir.join("")).unwrap();
    closer.join().unwrap();
    assert!(!shm.exists());
}

/// Marks `path` deleted through a new handle: `posix` removes its name at once, else the name
/// stays, pending, until every handle to it closes. Returns that handle, still open.
#[cfg(windows)]
fn mark_deleted(path: &Path, posix: bool) -> File {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FileDispositionInfo,
        FileDispositionInfoEx, SetFileInformationByHandle,
    };
    let handle = OpenOptions::new()
        .read(true)
        .access_mode(DELETE | 0x8000_0000)
        .open(path)
        .unwrap();
    let plain = FILE_DISPOSITION_INFO { DeleteFile: true };
    let ex = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    // SAFETY: a valid open handle with DELETE access and a matching info struct and size.
    let set = unsafe {
        if posix {
            SetFileInformationByHandle(
                handle.as_raw_handle() as _,
                FileDispositionInfoEx,
                &ex as *const _ as _,
                std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        } else {
            SetFileInformationByHandle(
                handle.as_raw_handle() as _,
                FileDispositionInfo,
                &plain as *const _ as _,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        }
    };
    assert_ne!(set, 0, "{}", std::io::Error::last_os_error());
    handle
}

/// Platform behaviour only: an open handle reads 0 links once its name is deleted with POSIX
/// semantics. The alias check still refuses any count but 1.
#[cfg(windows)]
#[test]
fn a_file_deleted_under_an_open_handle_reports_no_links() {
    let dir = TestDir::new();
    let shm = dir.join("index.sqlite-shm");
    std::fs::write(&shm, b"").unwrap();
    let probe = File::open(&shm).unwrap();
    drop(mark_deleted(&shm, true));
    assert!(!shm.exists());
    assert_eq!(handle_links(&probe, &shm).unwrap(), 0);
}

#[cfg(windows)]
#[test]
fn a_sidecar_denied_past_the_lock_wait_is_still_reported_denied() {
    let dir = TestDir::new();
    std::fs::write(dir.join("index.sqlite"), b"").unwrap();
    let shm = dir.join("index.sqlite-shm");
    std::fs::write(&shm, b"").unwrap();
    let held = mark_deleted(&shm, false);
    let start = Instant::now();
    let error = unaliased(&dir.join("")).unwrap_err();
    assert!(start.elapsed() >= LOCK_WAIT, "{:?}", start.elapsed());
    assert_eq!(kind(&*error), ErrorKind::PermissionDenied, "{error}");
    drop(held);
}

#[cfg(windows)]
#[test]
fn windows_known_folder_resolves_to_an_absolute_path() {
    assert!(local_app_data().is_some_and(|p| p.is_absolute()));
}

#[test]
fn prepare_creates_one_registry_and_reuses_it() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let first = prepare(&f.storage, &fig, None).unwrap();
    assert_eq!(first["initialized"], true);
    assert_eq!(first["source"]["added"], true);
    assert_eq!(first["storage"]["rule"], "default");
    let id = id_of(&first);
    assert_eq!(
        listing(&f.workspace(&id)),
        ["index.sqlite", "workspace.json"]
    );
    let again = prepare(&f.storage, &fig, None).unwrap();
    assert_eq!(again["initialized"], false);
    assert_eq!(again["source"]["added"], false);
    assert_eq!(again["incarnation"], first["incarnation"]);
    assert_eq!(
        again["source"]["sourceId"],
        crate::search::source_id(&std::fs::canonicalize(&fig).unwrap())
    );
}

#[test]
fn concurrent_prepares_of_one_workspace_create_one_database() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let reports: Vec<Json> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| prepare(&f.storage, &fig, None).unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        reports.iter().filter(|r| r["initialized"] == true).count(),
        1
    );
    assert!(
        reports
            .iter()
            .all(|r| r["incarnation"] == reports[0]["incarnation"])
    );
    let status = status(&f.storage, &source(&fig)).unwrap();
    assert_eq!(status["sources"].as_array().unwrap().len(), 1);
}

#[test]
fn status_of_a_missing_workspace_creates_no_storage() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let report = status(&f.storage, &source(&fig)).unwrap();
    assert_eq!(report["state"], "absent");
    assert!(!f.home.exists());
}

#[test]
fn status_reads_a_prepared_workspace_without_changing_it() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &fig, None).unwrap());
    let dir = f.workspace(&id);
    let before = std::fs::read(dir.join(DB)).unwrap();
    let report = status(&f.storage, &Select::Id(id.clone())).unwrap();
    assert_eq!(report["state"], "initialized");
    assert_eq!(report["sources"][0]["present"], true);
    assert_eq!(std::fs::read(dir.join(DB)).unwrap(), before);
    // Only SQLite's own WAL sidecars may appear beside the database.
    assert!(
        listing(&dir)
            .iter()
            .all(|n| n == "workspace.json" || n.starts_with(DB))
    );
}

#[test]
fn reset_waits_for_an_active_reader_then_reports_busy_and_keeps_every_file() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &fig, None).unwrap());
    let select = Select::Id(id.clone());
    let (_, reader) = opened(&f.storage, &select, false).unwrap();
    let dir = f.workspace(&id);
    let before = listing(&dir);
    let error = reset(&f.storage, &select).unwrap_err();
    assert_eq!(kind(&*error), ErrorKind::WorkspaceBusy);
    assert_eq!(listing(&dir), before);
    assert_eq!(sources(&f.storage, &reader).unwrap().len(), 1);
}

#[test]
fn reset_removes_only_the_database_and_prepare_starts_a_new_incarnation() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let sibling = f.fig("b.fig");
    let first = prepare(&f.storage, &fig, None).unwrap();
    let id = id_of(&first);
    let dir = std::fs::canonicalize(f.workspace(&id)).unwrap();
    let report = reset(&f.storage, &Select::Id(id.clone())).unwrap();
    assert_eq!(report["state"], "reset");
    assert_eq!(report["removed"][0], json!(dir.join(DB)));
    assert_eq!(listing(&dir), ["workspace.json"]);
    assert!(dir.with_extension("lock").is_file());
    assert_eq!(
        status(&f.storage, &Select::Id(id.clone())).unwrap()["state"],
        "uninitialized"
    );
    assert_eq!(std::fs::read_to_string(&fig).unwrap(), "design a.fig");
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "design b.fig");
    let again = prepare(&f.storage, &fig, None).unwrap();
    assert_eq!(again["initialized"], true);
    assert_ne!(again["incarnation"], first["incarnation"]);
}

#[test]
fn config_and_state_may_share_one_root() {
    let f = Fixture::new();
    let shared = f.home.join("shared").to_str().unwrap().to_owned();
    let home = f.home.to_str().unwrap().to_owned();
    let vars = [
        ("HOME", home.as_str()),
        ("XDG_CONFIG_HOME", shared.as_str()),
        ("XDG_STATE_HOME", shared.as_str()),
    ];
    let storage = resolve(None, &env(&vars), false, &|| None).unwrap();
    assert_eq!(storage.defaults.config.path, storage.defaults.state.path);
    let a = f.fig("a.fig");
    let first = prepare(&storage, &a, None).unwrap();
    assert_eq!(id_of(&prepare(&storage, &a, None).unwrap()), id_of(&first));
    reset(&storage, &Select::Id(id_of(&first))).unwrap();
    prepare(&storage, &a, None).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_workspaces_directory_is_refused() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (_, custom) = f.custom("custom");
    let id = id_of(&prepare(&custom, &a, None).unwrap());
    let locations = custom.defaults.config.path.join("workspaces");
    let outside = f.dir.join("outside");
    std::fs::rename(&locations, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &locations).unwrap();
    let before = std::fs::read_dir(&outside).unwrap().count();
    // Neither the record lookup nor a prepare follows it, so nothing is read or written there.
    let error = status(&f.storage, &source(&a)).unwrap_err();
    assert!(
        error.to_string().contains("not a plain directory"),
        "{error}"
    );
    let (_, other) = f.custom("other");
    let error = prepare(&other, &f.fig("b.fig"), None).unwrap_err();
    assert!(
        error.to_string().contains("not a plain directory"),
        "{error}"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), before);
    assert!(outside.join(format!("{id}.json")).exists());
}

#[test]
fn a_removed_remembered_storage_is_recreated_by_another_spelling_of_it() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (root, custom) = f.custom("custom");
    prepare(&custom, &a, None).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let respelled = root.join("..").join("custom");
    let again = Fixture::at(&f.home, Some(&respelled));
    let report = prepare(&again, &a, None).unwrap();
    assert!(root.join("state").is_dir(), "{report}");
}

#[cfg(unix)]
#[test]
fn a_fifo_source_is_refused_without_blocking() {
    let f = Fixture::new();
    let fifo = f.dir.join("pipe.fig");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    assert!(
        made.is_ok_and(|s| s.success()),
        "mkfifo is needed for this test"
    );
    let storage = Fixture::at(&f.home, None);
    let (sent, received) = std::sync::mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let _ = sent.send(prepare(&storage, &path, None).map_err(|e| e.to_string()));
    });
    let result = received
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("prepare blocked on a FIFO source");
    let error = result.unwrap_err();
    assert!(error.contains("not a regular .fig file"), "{error}");
    assert!(!f.storage.defaults.state.path.exists());
}

#[cfg(unix)]
#[test]
fn an_unsyncable_new_config_directory_stops_prepare_before_any_database() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let a = f.fig("a.fig");
    // Entries can be created in `locked` but it cannot be opened to sync them.
    let locked = std::path::absolute(f.dir.join("locked")).unwrap();
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o300)).unwrap();
    let unprivileged = File::open(&locked).is_err();
    let config = locked.join("config");
    let home = f.home.to_str().unwrap().to_owned();
    let vars = [
        ("HOME", home.as_str()),
        ("XDG_CONFIG_HOME", config.to_str().unwrap()),
    ];
    let storage = resolve(None, &env(&vars), false, &|| None).unwrap();
    let result = prepare(&storage, &a, None);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root opens any directory; there is nothing to observe then.
    if unprivileged {
        let error = result.unwrap_err();
        assert_eq!(kind(&*error), ErrorKind::PermissionDenied, "{error}");
        assert!(
            !storage.defaults.state.path.exists(),
            "a database root was created"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_unsyncable_workspace_entry_stops_prepare_before_its_database() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let state = std::path::absolute(f.dir.join("state")).unwrap();
    let workspaces = state.join("figlens").join("workspaces");
    std::fs::create_dir_all(&workspaces).unwrap();
    // Entries can be created in `workspaces` but it cannot be opened to sync them.
    std::fs::set_permissions(&workspaces, std::fs::Permissions::from_mode(0o300)).unwrap();
    let unprivileged = File::open(&workspaces).is_err();
    let home = f.home.to_str().unwrap().to_owned();
    let vars = [
        ("HOME", home.as_str()),
        ("XDG_STATE_HOME", state.to_str().unwrap()),
    ];
    let storage = resolve(None, &env(&vars), false, &|| None).unwrap();
    let result = prepare(&storage, &a, None);
    std::fs::set_permissions(&workspaces, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root opens any directory; there is nothing to observe then.
    if unprivileged {
        let error = result.unwrap_err();
        assert_eq!(kind(&*error), ErrorKind::PermissionDenied, "{error}");
        let databases = std::fs::read_dir(&workspaces)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().join("index.sqlite").exists())
            .count();
        assert_eq!(databases, 0, "a database was created");
    }
}

#[test]
fn an_oversized_record_is_refused_and_leaves_the_old_one() {
    let f = Fixture::new();
    let path = f.dir.join("record.json");
    write_small(&path, &json!({"root": "old"})).unwrap();
    let long = "x".repeat(SMALL_FILE as usize);
    let error = write_small(&path, &json!({ "root": long })).unwrap_err();
    assert_eq!(kind(&*error), ErrorKind::InvalidArguments, "{error}");
    assert_eq!(read_small(&path).unwrap(), json!({"root": "old"}));
    assert!(!path.with_extension("json.part").exists());
}

#[test]
fn custom_storage_is_remembered_for_that_workspace_only() {
    let f = Fixture::new();
    let (a, b) = (f.fig("a.fig"), f.fig("b.fig"));
    let (custom_root, custom) = f.custom("custom");
    let first = prepare(&custom, &a, None).unwrap();
    assert_eq!(first["storage"]["rule"], "--storage");
    let id = id_of(&first);
    assert!(
        custom_root
            .join("state/workspaces")
            .join(&id)
            .join(DB)
            .is_file()
    );
    assert!(!f.workspace(&id).exists());
    // A later process without --storage finds the same database.
    let again = prepare(&f.storage, &a, None).unwrap();
    assert_eq!(again["storage"]["rule"], "remembered");
    assert_eq!(again["incarnation"], first["incarnation"]);
    let status = status(&f.storage, &Select::Id(id.clone())).unwrap();
    assert_eq!(status["state"], "initialized");
    // The record keeps the canonical root (\\?\ on Windows), not the supplied spelling.
    let canonical = std::fs::canonicalize(&custom_root).unwrap();
    assert_eq!(status["storage"]["remembered"], json!(canonical));
    // Another workspace keeps the defaults.
    let other = prepare(&f.storage, &b, None).unwrap();
    assert_eq!(other["storage"]["rule"], "default");
    // Reset keeps the remembered location, so the workspace is prepared there again.
    reset(&f.storage, &Select::Id(id.clone())).unwrap();
    let recreated = prepare(&f.storage, &a, None).unwrap();
    assert_eq!(recreated["storage"]["rule"], "remembered");
    assert_ne!(recreated["incarnation"], first["incarnation"]);
}

#[test]
fn another_storage_for_a_workspace_with_a_database_is_refused() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &a, None).unwrap());
    let (custom_root, custom) = f.custom("custom");
    for error in [
        prepare(&custom, &a, None).unwrap_err(),
        status(&custom, &Select::Id(id.clone())).unwrap_err(),
        reset(&custom, &Select::Id(id.clone())).unwrap_err(),
    ] {
        assert_eq!(kind(&*error), ErrorKind::InvalidArguments);
    }
    assert!(!custom_root.exists());
    // Once the default database is reset, the other storage may take the workspace.
    reset(&f.storage, &Select::Id(id.clone())).unwrap();
    assert_eq!(prepare(&custom, &a, None).unwrap()["initialized"], true);
    assert_eq!(
        status(&f.storage, &Select::Id(id)).unwrap()["storage"]["rule"],
        "remembered"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_current_storage_does_not_let_another_root_take_over() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &a, None).unwrap());
    let (custom_root, custom) = f.custom("custom");
    let closed = f.storage.defaults.state.path.join("workspaces");
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).unwrap();
    let visible = std::fs::symlink_metadata(closed.join(&id).join(DB)).is_ok();
    let error = prepare(&custom, &a, None).unwrap_err();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A visible database is a conflict (root, or a mount that ignores the mode); a hidden one
    // is an access error. Neither may let the other root take over.
    let expected = if visible {
        ErrorKind::InvalidArguments
    } else {
        ErrorKind::PermissionDenied
    };
    assert_eq!(kind(&*error), expected, "{error}");
    if !visible {
        // The only way on is access to the old storage, never another --storage.
        assert!(error.to_string().contains("restore access"), "{error}");
        assert!(error.to_string().contains(&id), "{error}");
    }
    assert!(!custom_root.exists());
    assert!(!custom.locations().join(format!("{id}.json")).exists());
}

#[test]
fn an_unavailable_remembered_storage_is_an_error_not_a_second_database() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (custom_root, custom) = f.custom("custom");
    let id = id_of(&prepare(&custom, &a, None).unwrap());
    std::fs::rename(&custom_root, f.dir.join("moved")).unwrap();
    for error in [
        status(&f.storage, &Select::Id(id.clone())).unwrap_err(),
        prepare(&f.storage, &a, None).unwrap_err(),
        prepare(&f.custom("other").1, &a, None).unwrap_err(),
    ] {
        assert_eq!(kind(&*error), ErrorKind::StorageUnavailable);
        assert!(!error.to_string().contains("another --storage"), "{error}");
    }
    assert!(!f.dir.join("other").exists());
    assert!(!f.storage.defaults.state.path.exists());
}

#[cfg(unix)]
#[test]
fn an_unwritable_remembered_storage_fails_without_falling_back() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (custom_root, custom) = f.custom("custom");
    let id = id_of(&prepare(&custom, &a, None).unwrap());
    reset(&custom, &Select::Id(id.clone())).unwrap();
    let workspaces = custom_root.join("state/workspaces").join(&id);
    std::fs::set_permissions(&workspaces, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = workspaces.join("probe");
    let writable = std::fs::write(&probe, "").is_ok();
    let result = prepare(&f.storage, &a, None);
    std::fs::set_permissions(&workspaces, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root ignores permissions; the database is recreated there, which is also not a fallback.
    if !writable {
        assert!(result.is_err());
    }
    assert!(!f.storage.defaults.state.path.exists());
}

#[test]
fn a_source_hard_linked_as_a_database_file_is_refused_and_unchanged() {
    let f = Fixture::new();
    let fig = std::path::absolute(f.dir.join("design.fig")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/first-success/design.fig"),
        &fig,
    )
    .unwrap();
    let bytes = std::fs::read(&fig).unwrap();
    let project = std::path::absolute(f.dir.join("project")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let id = id_of(&prepare(&f.storage, &fig, Some(&project)).unwrap());
    let dir = f.workspace(&id);
    // Without the guard, SQLite writes its WAL through this link: adding x.fig grew the source.
    std::fs::hard_link(&fig, dir.join("index.sqlite-wal")).unwrap();
    let select = Select::Id(id.clone());
    let other = f.fig("x.fig");
    for error in [
        prepare(&f.storage, &other, Some(&project)).unwrap_err(),
        status(&f.storage, &select).unwrap_err(),
        add(&f.storage, &select, &other).unwrap_err(),
        reset(&f.storage, &select).unwrap_err(),
    ] {
        assert!(error.to_string().contains("hard links"), "{error}");
    }
    assert_eq!(std::fs::read(&fig).unwrap(), bytes);
    // A linked main database is refused the same way.
    std::fs::remove_file(dir.join("index.sqlite-wal")).unwrap();
    std::fs::hard_link(dir.join(DB), f.dir.join("copy.sqlite")).unwrap();
    assert!(status(&f.storage, &select).is_err());
    std::fs::remove_file(f.dir.join("copy.sqlite")).unwrap();
    assert_eq!(status(&f.storage, &select).unwrap()["state"], "initialized");
}

#[test]
fn a_failed_location_write_creates_no_database() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (custom_root, custom) = f.custom("custom");
    let id = Locator::source(&a).unwrap().id().unwrap();
    // A directory where the record's temporary file goes makes the write fail.
    let blocker = custom.locations().join(format!("{id}.json.part"));
    std::fs::create_dir_all(&blocker).unwrap();
    assert!(prepare(&custom, &a, None).is_err());
    assert!(!custom_root.join("state").exists());
    assert!(!f.storage.defaults.state.path.exists());
    std::fs::remove_dir(&blocker).unwrap();
    assert_eq!(
        prepare(&custom, &a, None).unwrap()["storage"]["rule"],
        "--storage"
    );
}

#[test]
fn an_interrupted_initialization_is_retried_in_the_remembered_place() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (custom_root, custom) = f.custom("custom");
    // A directory where the workspace lock goes fails the prepare after its place is remembered.
    let id = Locator::source(&a).unwrap().id().unwrap();
    let blocker = custom_root
        .join("state/workspaces")
        .join(format!("{id}.lock"));
    std::fs::create_dir_all(&blocker).unwrap();
    assert!(prepare(&custom, &a, None).is_err());
    assert!(custom.locations().join(format!("{id}.json")).is_file());
    let report = status(&f.storage, &Select::Id(id.clone())).unwrap();
    assert_eq!(report["state"], "absent");
    assert_eq!(report["storage"]["rule"], "remembered");
    std::fs::remove_dir(&blocker).unwrap();
    let again = prepare(&f.storage, &a, None).unwrap();
    assert_eq!(again["initialized"], true);
    assert_eq!(again["storage"]["rule"], "remembered");
    assert!(
        custom_root
            .join("state/workspaces")
            .join(&id)
            .join(DB)
            .is_file()
    );
    assert!(!f.storage.defaults.state.path.exists());
}

#[cfg(unix)]
#[test]
fn an_unreadable_state_root_is_an_error_not_absent() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    prepare(&f.storage, &fig, None).unwrap();
    // An unsearchable parent makes canonicalizing the state root itself fail.
    let parent = f
        .storage
        .defaults
        .state
        .path
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).unwrap();
    let searchable = std::fs::canonicalize(&f.storage.defaults.state.path).is_ok();
    let result = status(&f.storage, &source(&fig));
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root searches through any mode; there is nothing to observe then.
    if !searchable {
        assert_eq!(kind(&*result.unwrap_err()), ErrorKind::PermissionDenied);
    }
}

#[test]
fn an_unmarked_directory_is_preserved_and_refused() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = Locator::source(&fig).unwrap().id().unwrap();
    let dir = f.workspace(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "keep").unwrap();
    assert!(prepare(&f.storage, &fig, None).is_err());
    assert!(reset(&f.storage, &Select::Id(id.clone())).is_err());
    assert_eq!(listing(&dir), ["notes.txt"]);
    // An empty one, as an interrupted prepare leaves it, is claimed.
    std::fs::remove_file(dir.join("notes.txt")).unwrap();
    std::fs::write(dir.join("workspace.json.part"), "{").unwrap();
    assert_eq!(
        prepare(&f.storage, &fig, None).unwrap()["initialized"],
        true
    );
}

#[test]
fn a_marker_or_location_for_another_locator_with_the_same_id_is_refused() {
    let f = Fixture::new();
    let (fig, other) = (f.fig("a.fig"), f.fig("b.fig"));
    let id = Locator::source(&fig).unwrap().id().unwrap();
    let dir = f.workspace(&id);
    std::fs::create_dir_all(&dir).unwrap();
    let foreign = marker(&id, &Locator::source(&other).unwrap()).unwrap();
    write_small(&dir.join(MARKER), &foreign).unwrap();
    assert!(prepare(&f.storage, &fig, None).is_err());
    assert_eq!(listing(&dir), ["workspace.json"]);

    let g = Fixture::new();
    let fig = g.fig("a.fig");
    let id = Locator::source(&fig).unwrap().id().unwrap();
    let locations = g.storage.locations();
    std::fs::create_dir_all(&locations).unwrap();
    let record = json!({"version": LOCATION, "workspaceId": id,
        "locator": Locator::source(&other).unwrap().report().unwrap(), "storage": "/elsewhere"});
    write_small(&locations.join(format!("{id}.json")), &record).unwrap();
    assert!(prepare(&g.storage, &fig, None).is_err());
    assert!(status(&g.storage, &source(&fig)).is_err());
}

#[test]
fn a_foreign_or_newer_database_is_refused_untouched() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &fig, None).unwrap());
    let db = f.workspace(&id).join(DB);
    Connection::open(&db)
        .unwrap()
        .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA + 1))
        .unwrap();
    let bytes = std::fs::read(&db).unwrap();
    let select = Select::Id(id.clone());
    for error in [
        prepare(&f.storage, &fig, None).unwrap_err(),
        status(&f.storage, &select).unwrap_err(),
        add(&f.storage, &select, &fig).unwrap_err(),
    ] {
        assert_eq!(kind(&*error), ErrorKind::IndexIncompatible);
    }
    assert_eq!(std::fs::read(&db).unwrap(), bytes);
    // Reset still removes it: the marker and the database's own meta name this workspace.
    assert_eq!(reset(&f.storage, &select).unwrap()["state"], "reset");

    let other = f.fig("c.fig");
    let id = Locator::source(&other).unwrap().id().unwrap();
    let dir = f.workspace(&id);
    std::fs::create_dir_all(&dir).unwrap();
    let own = marker(&id, &Locator::source(&other).unwrap()).unwrap();
    write_small(&dir.join(MARKER), &own).unwrap();
    Connection::open(dir.join(DB))
        .unwrap()
        .execute_batch("CREATE TABLE notes (x);")
        .unwrap();
    assert!(prepare(&f.storage, &other, None).is_err());
    assert!(reset(&f.storage, &Select::Id(id)).is_err());
    assert!(dir.join(DB).exists());
}

#[cfg(unix)]
#[test]
fn symlinked_workspace_directories_are_refused() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = Locator::source(&fig).unwrap().id().unwrap();
    let elsewhere = std::path::absolute(f.dir.join("elsewhere")).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let parent = f.storage.defaults.state.path.join("workspaces");
    std::fs::create_dir_all(&parent).unwrap();
    std::os::unix::fs::symlink(&elsewhere, parent.join(&id)).unwrap();
    assert!(prepare(&f.storage, &fig, None).is_err());
    assert!(status(&f.storage, &Select::Id(id.clone())).is_err());
    assert!(listing(&elsewhere).is_empty());

    let g = Fixture::new();
    let state = &g.storage.defaults.state.path;
    std::fs::create_dir_all(state).unwrap();
    std::os::unix::fs::symlink(&elsewhere, state.join("workspaces")).unwrap();
    assert!(prepare(&g.storage, &fig, None).is_err());
    assert!(listing(&elsewhere).is_empty());
}

#[test]
fn a_source_inside_storage_is_refused() {
    let f = Fixture::new();
    let state = &f.storage.defaults.state.path;
    std::fs::create_dir_all(state).unwrap();
    let inside = state.join("a.fig");
    std::fs::write(&inside, "design").unwrap();
    let error = prepare(&f.storage, &inside, None).unwrap_err();
    assert_eq!(kind(&*error), ErrorKind::PermissionDenied);
    assert!(!state.join("workspaces").exists());
}

#[test]
fn unwritable_storage_fails_without_writing_elsewhere() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let blocker = std::path::absolute(f.dir.join("blocker")).unwrap();
    std::fs::write(&blocker, "a file, not a directory").unwrap();
    let custom = Fixture::at(&f.home, Some(&blocker.join("store")));
    assert!(prepare(&custom, &fig, None).is_err());
    assert!(!f.storage.defaults.state.path.exists());
    let id = Locator::source(&fig).unwrap().id().unwrap();
    assert!(!custom.locations().join(format!("{id}.json")).exists());
}

#[cfg(unix)]
#[test]
fn a_read_only_state_root_reports_permission_denied() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let state = &f.storage.defaults.state.path;
    std::fs::create_dir_all(state).unwrap();
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root ignores directory permissions; there is nothing to observe then.
    let probe = state.join("probe");
    if std::fs::create_dir(&probe).is_ok() {
        std::fs::remove_dir(&probe).unwrap();
        return;
    }
    let error = prepare(&f.storage, &fig, None).unwrap_err();
    assert_eq!(kind(&*error), ErrorKind::PermissionDenied);
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(listing(state).is_empty());
}

#[test]
fn project_sources_are_added_removed_and_relocated_without_touching_files() {
    let f = Fixture::new();
    let project = std::path::absolute(f.dir.join("project")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let (a, b, moved) = (f.fig("a.fig"), f.fig("b.fig"), f.fig("moved.fig"));
    let first = prepare(&f.storage, &a, Some(&project)).unwrap();
    let select = Select::from_choice(Some(&project), None, None)
        .unwrap()
        .unwrap();
    let added = add(&f.storage, &select, &b).unwrap();
    let b_id = added["source"]["sourceId"].as_str().unwrap().to_owned();
    assert_eq!(added["sources"].as_array().unwrap().len(), 2);
    let relocated = relocate(&f.storage, &select, &b_id, &moved).unwrap();
    assert_eq!(relocated["source"]["sourceId"], b_id.as_str());
    let a_id = first["source"]["sourceId"].as_str().unwrap();
    assert!(relocate(&f.storage, &select, a_id, &moved).is_err());
    let removed = remove(&f.storage, &select, &b_id).unwrap();
    assert_eq!(removed["removed"], true);
    assert_eq!(removed["sources"].as_array().unwrap().len(), 1);
    for (path, name) in [(&a, "a.fig"), (&b, "b.fig"), (&moved, "moved.fig")] {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            format!("design {name}")
        );
    }
    // A source-scoped workspace keeps only its own file.
    let solo = Select::Id(id_of(&prepare(&f.storage, &a, None).unwrap()));
    assert!(add(&f.storage, &solo, &b).is_err());
    let own = crate::search::source_id(&std::fs::canonicalize(&a).unwrap());
    // Relocating it keeps the workspace: the new path selects it through an alias.
    relocate(&f.storage, &solo, &own, &b).unwrap();
    let by_new_path = Select::Locator(Locator::source(&b).unwrap());
    assert_eq!(
        status(&f.storage, &by_new_path).unwrap()["workspaceId"],
        json!(match &solo {
            Select::Id(id) => id.clone(),
            _ => unreachable!(),
        })
    );
    // Removing its own source leaves a valid, empty workspace that may add the file again.
    assert_eq!(remove(&f.storage, &solo, &own).unwrap()["removed"], true);
    assert_eq!(status(&f.storage, &solo).unwrap()["sources"], json!([]));
    assert_eq!(add(&f.storage, &solo, &a).unwrap()["source"]["added"], true);
    assert!(Select::from_choice(Some(&project), Some(&a), None).is_err());
}

#[test]
fn an_unusable_config_root_is_named_as_config_not_storage() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.home.join(".config")).unwrap();
    std::fs::write(f.home.join(".config/figlens"), "not a directory").unwrap();
    let (_, custom) = f.custom("custom");
    let error = prepare(&custom, &f.fig("a.fig"), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("--storage does not change it"), "{error}");
}

#[test]
fn server_storage_roots_judge_where_a_new_storage_path_really_leads() {
    let f = Fixture::new();
    let allowed = std::path::absolute(f.dir.join("allowed")).unwrap();
    let outside = std::path::absolute(f.dir.join("outside")).unwrap();
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let fig = f.fig("a.fig");
    let at = |dir: &Path| Fixture::at(&f.home, Some(dir)).allow(std::slice::from_ref(&allowed));
    let escape = allowed.join("missing/../../outside/store");
    assert!(prepare(&at(&escape), &fig, None).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, allowed.join("link")).unwrap();
        assert!(prepare(&at(&allowed.join("link/store")), &fig, None).is_err());
    }
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    prepare(&at(&allowed.join("store")), &fig, None).unwrap();
    // The remembered root is reused by a server that allows no custom storage.
    let server = Fixture::at(&f.home, None).allow(&[]);
    let select = Select::Locator(Locator::source(&fig).unwrap());
    assert_eq!(
        status(&server, &select).unwrap()["storage"]["rule"],
        "remembered"
    );
}

#[test]
fn an_unavailable_storage_offers_recreation_elsewhere_that_keeps_the_old_files() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (old, custom) = f.custom("old");
    let first = prepare(&custom, &a, None).unwrap();
    let moved = f.dir.join("offline");
    std::fs::rename(&old, &moved).unwrap();
    let error = prepare(&Fixture::at(&f.home, None), &a, None).unwrap_err();
    let fig = error.downcast_ref::<FigError>().unwrap();
    assert_eq!(fig.kind(), ErrorKind::StorageUnavailable);
    let actions: Vec<&Json> = fig.recovery().unwrap().as_array().unwrap().iter().collect();
    assert_eq!(actions[1]["action"], "recreate_elsewhere");
    let (new, elsewhere) = f.custom("new");
    let options = PrepareOptions {
        replace_unavailable: true,
        ..PrepareOptions::default()
    };
    let recreated = super::prepare(&elsewhere, &a, None, options)
        .unwrap()
        .report;
    assert_ne!(recreated["incarnation"], first["incarnation"]);
    assert!(new.join("state").is_dir());
    assert!(moved.join("state/workspaces").read_dir().unwrap().count() > 0);
    let select = Select::Locator(Locator::source(&a).unwrap());
    assert_eq!(
        status(&Fixture::at(&f.home, None), &select).unwrap()["incarnation"],
        recreated["incarnation"]
    );
}

#[test]
fn a_confined_caller_cannot_reset_a_workspace_holding_sources_it_cannot_see() {
    let f = Fixture::new();
    let (inside, outside) = (f.dir.join("in"), f.dir.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let project = std::fs::canonicalize(f.dir.join("")).unwrap();
    let seen = std::fs::canonicalize(&inside).unwrap().join("a.fig");
    let hidden = std::fs::canonicalize(&outside).unwrap().join("b.fig");
    std::fs::write(&seen, "a").unwrap();
    std::fs::write(&hidden, "b").unwrap();
    prepare(&f.storage, &seen, Some(&project)).unwrap();
    prepare(&f.storage, &hidden, Some(&project)).unwrap();
    let select = Select::Locator(Locator::project(&project).unwrap());
    let confined = f
        .storage
        .clone()
        .confined(&[std::fs::canonicalize(&inside).unwrap()]);
    assert_eq!(
        kind(&*reset(&confined, &select).unwrap_err()),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        status(&f.storage, &select).unwrap()["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // A link inside the roots to a file outside them is not visible either.
    #[cfg(unix)]
    {
        let link = std::fs::canonicalize(&inside).unwrap().join("link.fig");
        std::os::unix::fs::symlink(&hidden, &link).unwrap();
        assert!(!confined.sees(&link));
        assert!(confined.sees(&seen));
    }
}

#[test]
fn a_stale_alias_neither_blocks_nor_captures_a_new_prepare_of_its_path() {
    let f = Fixture::new();
    let a = std::fs::canonicalize(f.fig("a.fig")).unwrap();
    let ws = id_of(&prepare(&f.storage, &a, None).unwrap());
    let b = f.dir.join("b.fig");
    std::fs::rename(&a, &b).unwrap();
    let b = std::fs::canonicalize(b).unwrap();
    let own = crate::search::source_id(&a);
    relocate(&f.storage, &Select::Id(ws.clone()), &own, &b).unwrap();
    // Moved back: the alias of b no longer names a workspace holding b.
    std::fs::rename(&b, &a).unwrap();
    relocate(&f.storage, &Select::Id(ws.clone()), &own, &a).unwrap();
    std::fs::rename(&a, &b).unwrap();
    std::fs::write(&a, "a new design").unwrap();
    let fresh = prepare(&f.storage, &b, None).unwrap();
    assert_ne!(id_of(&fresh), ws);
    // After a reset, the relocated path prepares a workspace again instead of failing.
    reset(&f.storage, &Select::Id(id_of(&fresh))).unwrap();
    assert!(prepare(&f.storage, &b, None).is_ok());
}

#[cfg(unix)]
#[test]
fn server_storage_roots_refuse_parent_steps_that_reenter_a_link_out_of_them() {
    let f = Fixture::new();
    let allowed = std::path::absolute(f.dir.join("allowed")).unwrap();
    let outside = std::path::absolute(f.dir.join("outside")).unwrap();
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, allowed.join("link")).unwrap();
    let combined = allowed.join("missing/../link/store");
    let storage = Fixture::at(&f.home, Some(&combined)).allow(std::slice::from_ref(&allowed));
    assert_eq!(
        kind(&*prepare(&storage, &f.fig("a.fig"), None).unwrap_err()),
        ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn a_replacement_whose_database_was_never_created_finishes_where_it_was_reserved() {
    let f = Fixture::new();
    let a = f.fig("a.fig");
    let (old, custom) = f.custom("old");
    prepare(&custom, &a, None).unwrap();
    std::fs::rename(&old, f.dir.join("offline")).unwrap();
    let (new, elsewhere) = f.custom("new");
    let options = || PrepareOptions {
        replace_unavailable: true,
        ..PrepareOptions::default()
    };
    super::prepare(&elsewhere, &a, None, options()).unwrap();
    // As if initialization failed after the record named the new storage.
    let workspaces = new.join("state/workspaces");
    for dir in std::fs::read_dir(&workspaces).unwrap() {
        let dir = dir.unwrap().path();
        if dir.is_dir() {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    let retried = super::prepare(&elsewhere, &a, None, options())
        .unwrap()
        .report;
    assert_eq!(retried["initialized"], true);
    assert_eq!(retried["previous"]["reason"], "storage_unavailable");
    // Once its database exists there, the same call is refused instead of replacing it.
    let Err(again) = super::prepare(&elsewhere, &a, None, options()) else {
        panic!("a second replacement into the same storage was accepted");
    };
    assert!(again.to_string().contains("current storage"), "{again}");
}

#[cfg(unix)]
#[test]
fn an_alias_whose_workspace_is_busy_is_kept_and_the_prepare_reports_busy() {
    let f = Fixture::new();
    let a = std::fs::canonicalize(f.fig("a.fig")).unwrap();
    let ws = id_of(&prepare(&f.storage, &a, None).unwrap());
    let b = f.dir.join("b.fig");
    std::fs::rename(&a, &b).unwrap();
    let b = std::fs::canonicalize(b).unwrap();
    let own = crate::search::source_id(&a);
    relocate(&f.storage, &Select::Id(ws.clone()), &own, &b).unwrap();
    let select = Select::Id(ws.clone());
    let place = f.storage.place(&ws, None).unwrap();
    let held = acquire(&place.roots.state.path, &select, true).unwrap();
    let error = prepare(&f.storage, &b, None).unwrap_err();
    drop(held);
    assert_eq!(kind(&*error), ErrorKind::WorkspaceBusy);
    let alias = Locator::Source(b.clone()).id().unwrap();
    assert_eq!(f.storage.alias(&alias).unwrap(), Some(ws));
}

#[test]
fn a_confined_caller_cannot_reset_a_workspace_whose_sources_it_cannot_read() {
    let f = Fixture::new();
    let inside = f.dir.join("in");
    std::fs::create_dir_all(&inside).unwrap();
    let inside = std::fs::canonicalize(&inside).unwrap();
    let project = std::fs::canonicalize(f.dir.join("")).unwrap();
    let seen = inside.join("a.fig");
    std::fs::write(&seen, "a").unwrap();
    let id = id_of(&prepare(&f.storage, &seen, Some(&project)).unwrap());
    let db = f.workspace(&id).join(DB);
    std::fs::write(&db, "not a database").unwrap();
    let confined = f.storage.clone().confined(&[inside]);
    let select = Select::Locator(Locator::project(&project).unwrap());
    assert_eq!(
        kind(&*reset(&confined, &select).unwrap_err()),
        ErrorKind::PermissionDenied
    );
    assert!(db.is_file());
}

#[test]
fn a_relocate_interrupted_after_its_alias_completes_when_retried() {
    let f = Fixture::new();
    let a = std::fs::canonicalize(f.fig("a.fig")).unwrap();
    let ws = id_of(&prepare(&f.storage, &a, None).unwrap());
    let b = f.dir.join("b.fig");
    std::fs::rename(&a, &b).unwrap();
    let b = std::fs::canonicalize(b).unwrap();
    let own = crate::search::source_id(&a);
    let select = Select::Id(ws.clone());
    relocate(&f.storage, &select, &own, &b).unwrap();
    // Interrupted after the alias was written, before the source row moved.
    let (_, opened_ws) = opened(&f.storage, &select, true).unwrap();
    opened_ws
        .conn
        .execute("UPDATE sources SET path = ?1", [a.to_str().unwrap()])
        .unwrap();
    drop(opened_ws);
    relocate(&f.storage, &select, &own, &b).unwrap();
    assert_eq!(id_of(&prepare(&f.storage, &b, None).unwrap()), ws);
}

#[test]
fn a_denied_storage_offers_restore_access_naming_the_path_and_roots() {
    let f = Fixture::new();
    let path = f.storage.defaults.state.path.join("workspaces");
    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let error = rooted(&f.storage, storage_error(&path, denied));
    let fig = error.downcast_ref::<FigError>().unwrap();
    let restore = &fig.recovery().unwrap()[0];
    assert_eq!(
        (
            restore["action"].as_str(),
            restore["path"].as_str(),
            restore["roots"]["config"]["path"].as_str()
        ),
        (
            Some("restore_access"),
            path.to_str(),
            f.storage.defaults.config.path.to_str()
        )
    );
}

#[test]
fn a_missing_lock_or_newer_schema_offers_placed_prepare_and_reset_with_the_config_root() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &fig, None).unwrap());
    let dir = f.workspace(&id);
    let actions = |storage: &Storage, select: &Select| {
        let error = rooted(storage, status(storage, select).unwrap_err());
        let fig = error.downcast_ref::<FigError>().unwrap();
        fig.recovery()
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["action"].as_str().unwrap().to_owned(),
                    a["path"].as_str().map(PathBuf::from),
                    a["roots"]["config"]["path"].as_str().map(PathBuf::from),
                    a["lock"].as_str().map(PathBuf::from),
                )
            })
            .collect::<Vec<_>>()
    };
    // Status names the sibling lock a repair recreates, so hosts never derive it.
    let reported = status(&f.storage, &Select::Id(id.clone())).unwrap();
    let lock = PathBuf::from(reported["paths"]["lock"].as_str().unwrap());
    assert_eq!(
        (lock.exists(), lock.file_name().unwrap().to_str().unwrap()),
        (true, format!("{id}.lock").as_str())
    );
    // Recovery names the canonical workspace path (`\\?\D:\…` on Windows).
    let canonical = std::fs::canonicalize(&dir).unwrap();
    let placed = |action: &str| {
        (
            action.to_owned(),
            Some(canonical.clone()),
            Some(f.storage.defaults.config.path.clone()),
            Some(lock_beside(&canonical)),
        )
    };
    std::fs::remove_file(dir.with_extension("lock")).unwrap();
    let source = Select::Locator(Locator::source(&fig).unwrap());
    let missing = actions(&f.storage, &source);
    let confined = f.storage.clone().confined(&[f.dir.join("")]);
    let missing_confined = actions(&confined, &source);
    prepare(&f.storage, &fig, None).unwrap();
    Connection::open(dir.join(DB))
        .unwrap()
        .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA + 1))
        .unwrap();
    let newer = actions(&f.storage, &Select::Id(id.clone()));
    assert_eq!(
        (missing, missing_confined, newer),
        (
            vec![placed("prepare"), placed("reset")],
            vec![placed("prepare")],
            vec![
                ("use_writer_version".to_owned(), None, None, None),
                placed("reset")
            ],
        )
    );
}

#[test]
fn an_unproven_workspace_offers_no_placed_reset() {
    let f = Fixture::new();
    let fig = f.fig("a.fig");
    let id = id_of(&prepare(&f.storage, &fig, None).unwrap());
    let dir = f.workspace(&id);
    let select = Select::Id(id.clone());
    let offered = |storage: &Storage| {
        let error = rooted(storage, status(storage, &select).unwrap_err());
        let fig = error.downcast_ref::<FigError>().unwrap();
        fig.recovery()
            .and_then(Json::as_array)
            .is_some_and(|a| a.iter().any(|a| a["action"] == "reset"))
    };
    let conn = Connection::open(dir.join(DB)).unwrap();
    conn.execute(
        "UPDATE meta SET value = 'ws_other' WHERE key = 'workspace_id'",
        [],
    )
    .unwrap();
    conn.execute_batch(&format!("PRAGMA user_version = {};", SCHEMA + 1))
        .unwrap();
    drop(conn);
    let foreign_newer = offered(&f.storage);
    std::fs::remove_file(dir.with_extension("lock")).unwrap();
    std::fs::remove_file(dir.join(MARKER)).unwrap();
    let unmarked = offered(&f.storage);
    assert_eq!((foreign_newer, unmarked), (false, false));
}

#[test]
fn search_relocated_recovery_keeps_the_callers_filters() {
    let f = Fixture::new();
    let a = std::fs::canonicalize(f.fig("a.fig")).unwrap();
    let ws = id_of(&prepare(&f.storage, &a, None).unwrap());
    let b = f.dir.join("b.fig");
    std::fs::rename(&a, &b).unwrap();
    let b = std::fs::canonicalize(b).unwrap();
    relocate(
        &f.storage,
        &Select::Id(ws),
        &crate::search::source_id(&a),
        &b,
    )
    .unwrap();
    let req = crate::search::Request {
        query: "card".into(),
        scope: "0:1".into(),
        types: vec!["TEXT".into(), "FRAME".into()],
        include_hidden: true,
        include_internal: true,
        limit: 7,
        cursor: None,
    };
    let select = Select::Locator(Locator::source(&a).unwrap());
    let e = query::search(&f.storage, &select, &[], &req).unwrap_err();
    let recovery = e
        .downcast_ref::<FigError>()
        .and_then(FigError::recovery)
        .unwrap_or_else(|| panic!("{e}"))[0]
        .clone();
    assert_eq!(
        recovery["arguments"],
        json!({"file": b.to_str().unwrap(), "query": "card", "node": "0:1", "type": "TEXT,FRAME",
            "includeHidden": true, "includeInternal": true, "limit": 7})
    );
}
