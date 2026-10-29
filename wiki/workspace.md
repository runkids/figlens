# Local workspaces

`src/workspace.rs` owns the figlens application roots, workspace identity and ownership, and each workspace's SQLite registry. CLI `figlens index` is its only caller. This is a partial #90 milestone: a workspace holds the source registry, not an index. `search` still builds its in-process index from the `.fig` (see [search](search.md)), MCP has no `index` tool, and Pi, OpenCode and OMP reach no workspace code. Connecting, listing tools, `find`, `inspect`, `search` and every other command create no storage.

## Roots and storage location

`workspace::storage` resolves the default `config`, `state`, `cache` and `data` roots, and each report names the path with the rule that chose it:

1. The matching `XDG_*_HOME` when it is an absolute path (judged by the running OS), plus `/figlens`. Empty or relative values are ignored.
2. Linux and macOS: `$HOME/.config`, `.local/state`, `.cache` and `.local/share`, each plus `/figlens`. Windows: `%LOCALAPPDATA%\figlens\<root>`, then the `FOLDERID_LocalAppData` Known Folder, never `HOME`.

Each action then places one workspace's `state`, `cache` and `data` (`storage.rule` in the report):

1. `--storage <dir>` (`--storage`): an absolute application root used as `<dir>/{state,cache,data}`. A relative path or a non-directory is refused, never replaced.
2. The remembered location (`remembered`): `<config>/workspaces/<workspaceId>.json`, version 1, holding the id, full locator and absolute `storage`. A `prepare` with a new `--storage` creates that root and writes the record atomically before any database exists there, and asks the OS to flush it first. On every platform the temporary record is synced before it replaces the old one, and a record over 64 KiB, which a later lookup could not read back, is refused before anything is written. On Unix, the record's directory and every config, state, `--storage`, `workspaces/` or workspace directory that prepare creates are synced into their parents, from the first missing one down (a config, state or `--storage` directory found just created by a concurrent prepare is synced too); directories that already existed are left alone, and a failed sync stops prepare before the database is created. On Windows the record is moved with a same-volume write-through `MoveFileExW`, and new directories are not flushed explicitly. This survives a crash only as far as the filesystem and hardware honour these flush requests; no crash or power-loss test backs it. A process that stops mid-prepare has already published its record, so normal discovery finds that storage and the next prepare retries an interrupted initialization in the remembered place. It does so under `<config>/workspaces/<workspaceId>.location.lock` (named apart from the workspace lock, since config and state may be one root), which every prepare holds so concurrent prepares never initialize two roots. A remembered root that is missing, not a directory or unreachable is an error for every action, also with a different `--storage`, never a second database; repeating the same `--storage`, also in an equivalent path spelling resolved through an existing ancestor (`same_dir` canonicalizes the longest existing prefix and applies the rest lexically, so a removed root still matches), recreates it there, and deleting the record forgets it. The record stores the canonical root. The record only locates the workspace: ownership is still the marker and the database `meta`, and it authorizes no source.
3. The defaults (`default`).

Config never moves with `--storage`, so the record stays findable. Any action whose `--storage` names another root while the current root still has this workspace's database is refused with both paths; after `reset` there, the new root takes over and the record is rewritten. Reset keeps the record. Only `prepare` writes config; status and reads create nothing.

The `gs://` download cache (`remote::cache_root`), the preview render cache and project `.figlens/` keep their own locations.

## Layout and ownership

A workspace is `<state>/workspaces/<workspaceId>/` holding `workspace.json` (the ownership marker) and `index.sqlite` with its `-wal` and `-shm` files, beside its lock file `<state>/workspaces/<workspaceId>.lock`.

- `workspaceId` is `ws_` plus 16 hex digits of SHA-256 over the locator kind and canonical path: a `--workspace` project directory, or the `.fig` itself for a source-scoped workspace. The marker, the remembered location and the database `meta` table keep the full locator, so an id collision is refused instead of reused.
- `sourceId` starts as `search::source_id` of the canonical source path, the id stateless `search` derives. `relocate` keeps the registered id, while stateless `search` still derives one from the current path, so after a relocate the two differ until search reads the workspace.
- A source (prepare, add, relocate, `--file`) must be a readable regular file, checked by metadata before it is opened, so a FIFO or device is refused instead of blocking.
- Writes go through the canonical root and real directories only: a symlinked `workspaces/` (under state or config, so location records and their lock are never read or written through a link; the root itself may be an alias) or workspace directory, a source inside any figlens root (default or placed), or an unwritable root fails with the path named. Nothing falls back to another directory. Only a missing root reads as absent; any other error keeps its path and kind.
- Every open, read-only included, first refuses an `index.sqlite`, `-wal`, `-shm` or `-journal` that is not a plain file or has another hard link (`nlink`, or `GetFileInformationByHandle` on Windows): SQLite writes through such a link, so a `.fig` linked there would be overwritten. Reset refuses it too; remove the other link to recover.
- A directory is figlens-owned only when its marker names its id. `prepare` takes the lock beside the directory first, then creates the directory and writes its marker, so a concurrent prepare never sees an unmarked one. An empty directory, or one holding only an unfinished `workspace.json.part`, is claimed, so an interrupted prepare can be retried. Any other directory without the marker, or a database without this workspace's `meta`, is left as found and reported. A missing lock file beside an owned directory is an error until `prepare` or `reset` recreates it.

## Database and locks

- SQLite is `rusqlite` with `bundled`: every release target links SQLite statically, so users install nothing.
- Writing connections use WAL, `synchronous = FULL` and a 5 s busy timeout. A read-only `status` of an existing workspace may let SQLite create its own `-wal`/`-shm`; an absent workspace gets no file at all.
- Schema 1 is `meta` (workspace id, locator, incarnation, creation time) and `sources` (`source_id`, unique `path`, `added_at_ms`), with `PRAGMA user_version = 1`. Schema and identity are read before any write pragma; any other version is `index_incompatible` and stays untouched. There is no migration yet.
- Every connection holds the lock file shared for its lifetime; `prepare` and `reset` hold it exclusive. Locks are `std::fs::File` advisory locks (flock, `LockFileEx`) taken with `try_lock` until a 5 s deadline, then `workspace_busy`. The lock file is never removed, so every process locks the same file.
- `incarnation` is `hex(randomblob(16))`, new for every initialization. Later cursors and jobs must bind it so a handle from before a reset never matches the recreated workspace.

## Errors and recovery

Every storage error names the workspace or path and one way on; none suggests another `--storage`, which would meet the same blocked storage. `workspace::recovery` maps the I/O error:

| Kind | Cause | Next step |
|---|---|---|
| `permission_denied` | Access denied or read-only | Restore access; figlens never changes permissions |
| `storage_unavailable` | Missing, unmounted or offline (`NotFound`, `NotConnected`, `TimedOut`, stale network handle) | Reconnect or restore it, then retry; a missing remembered root may be forgotten by deleting its record only if its database is gone for good |
| `workspace_busy` | Another process holds the lock | Retry after it finishes |
| `index_incompatible` | Another schema | Use that figlens version, or reset |
| `operation_failed` | Anything else | Fix the storage at the named path |

When the current storage cannot be checked, a different `--storage` is refused with that error (`blocked`), because the database may still exist behind it. Reset and later rebuilds need the old database and stay blocked the same way. Stateless search of a readable `.fig` (CLI `figlens search` or the MCP `search` tool) is unaffected and is the fallback. In this milestone no automatic workaround (another `--storage`, reset, deleting files) is offered; explicit recovery and rebuild arrive with the durable index. Structured MCP recovery fields arrive with the MCP `index` tools.

## Actions

| Action | Effect |
|---|---|
| `prepare <file.fig> [--workspace DIR]` | Creates or reuses the workspace and registers the file; concurrent calls create one database |
| `status <select>` | `absent`, `uninitialized` (after reset) or `initialized` with sources and their presence; creates nothing |
| `add <file.fig> <select>` | Registers another source in a project workspace; a source-scoped workspace keeps only its file |
| `remove <sourceId> <select>` | Deletes the registry row; a source-scoped workspace may unregister its own file and stays valid and empty. The `.fig` is never touched |
| `relocate <sourceId> <new.fig> <select>` | Keeps the id and changes the path in a project workspace; a path registered under another id is refused. Source-scoped relocate is not available yet: it moves the locator itself and arrives with the durable index |
| `reset <select>` | Exclusive lock, identity check, `wal_checkpoint(TRUNCATE)` that must finish, close, then removes exactly `index.sqlite`, `-wal`, `-shm` and `-journal` |

`<select>` is `--workspace DIR`, `--file <file.fig>` (its source-scoped workspace) or `--id ws_...`. `reset` keeps the marker, lock and remembered location and lists `removed`, `retained` and `failed`; when anything failed, the CLI prints that report and exits 1. Designs, `.figlens/`, exports, global models and other workspaces are never touched.

Not exposed yet: `rebuild`, `refresh` and `clear_cache` (no derived index or cache exists), `jobs`, and the MCP `index` and `jobs` tools. They arrive with the durable search index. Yolo cleanup never removes a workspace, database or registry; see `skills/figlens/references/yolo.md`.

## Tests

`src/workspace/tests.rs` uses real directories under `samples/`, covering:

- root precedence, including the Windows rules on every OS and a native Known Folder test on Windows
- a remembered `--storage` found by later calls without it, another workspace unaffected, an explicit root refused while another holds the database, a missing remembered root, and the location kept after reset
- concurrent prepares creating one database
- status creating nothing
- reset refused while a reader holds the lock, then a reset and re-prepare with a new incarnation
- unmarked, colliding, foreign and newer-schema directories preserved
- a FIFO source refused promptly, with no state created
- a new config directory that cannot be synced stops prepare before any database root exists (unprivileged only)
- a new workspace directory whose entry cannot be synced stops prepare before its database (unprivileged only)
- an oversized record refused with the old record and no `.part` left behind
- a public-fixture source hard-linked as the WAL, refused with its bytes unchanged (without the guard, an add rewrote it)
- an injected location-write failure that leaves no database, and a prepare that fails after remembering its place, then succeeds there without `--storage`
- symlinked roots, state and config `workspaces/` and workspace directories, config and state sharing one root, a source inside storage, unwritable storage and a claimed empty directory

The read-only-root test returns early under root (Docker) and runs on a native non-root run.
