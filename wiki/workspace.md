# Local workspaces

`src/workspace.rs` is the facade over `storage.rs` (roots, placement, location records, directory, marker and locks), `database.rs` (schema, open, upgrade, registry reads) and `registry.rs` (prepare, status, add, remove, relocate, reset). Durable indexes, jobs and workspace search are in [index](index-jobs.md). CLI `figlens index`, `jobs` and `search --workspace|--file|--id`, and the MCP `index`, `jobs` and `search` (`workspaceId`) tools call it; Pi, OpenCode and OMP reach it only through those. Connecting, listing tools, `find`, `inspect`, stateless `search` and every other command create no storage.

## Roots and storage location

`workspace::storage` resolves the default `config`, `state`, `cache` and `data` roots, and each report names the path with the rule that chose it:

1. The matching `XDG_*_HOME` when it is an absolute path (judged by the running OS), plus `/figlens`. Empty or relative values are ignored.
2. Linux and macOS: `$HOME/.config`, `.local/state`, `.cache` and `.local/share`, each plus `/figlens`. Windows: `%LOCALAPPDATA%\figlens\<root>`, then the `FOLDERID_LocalAppData` Known Folder, never `HOME`.

Each action then places one workspace's `state`, `cache` and `data` (`storage.rule` in the report):

1. `--storage <dir>` (`--storage`): an absolute application root used as `<dir>/{state,cache,data}`. A relative path or a non-directory is refused, never replaced.
2. The remembered location (`remembered`): `<config>/workspaces/<workspaceId>.json`, version 2 (version 1, without `binding`, is still read), holding the id, full locator, absolute `storage` and a random `binding` that the database's `meta` must repeat, so a database the record no longer names is detached rather than reused. A `prepare` with a new `--storage` creates that root and writes the record atomically before any database exists there, and asks the OS to flush it first. On every platform the temporary record is synced before it replaces the old one, and a record over 64 KiB, which a later lookup could not read back, is refused before anything is written. On Unix, the record's directory and every config, state, `--storage`, `workspaces/` or workspace directory that prepare creates are synced into their parents, from the first missing one down (a config, state or `--storage` directory found just created by a concurrent prepare is synced too); directories that already existed are left alone, and a failed sync stops prepare before the database is created. On Windows the record is moved with a same-volume write-through `MoveFileExW`, and new directories are not flushed explicitly. This survives a crash only as far as the filesystem and hardware honour these flush requests; no crash or power-loss test backs it. A process that stops mid-prepare has already published its record, so normal discovery finds that storage and the next prepare retries an interrupted initialization in the remembered place. It does so under `<config>/workspaces/<workspaceId>.location.lock` (named apart from the workspace lock, since config and state may be one root), which every prepare holds so concurrent prepares never initialize two roots. A remembered root that is missing, not a directory or unreachable is an error for every action, also with a different `--storage`, never a second database; repeating the same `--storage`, also in an equivalent path spelling resolved through an existing ancestor (`same_dir` canonicalizes the longest existing prefix and applies the rest lexically, so a removed root still matches), recreates it there, and deleting the record forgets it. The record stores the canonical root. The record only locates the workspace: ownership is still the marker and the database `meta`, and it authorizes no source.
3. The defaults (`default`).

Config never moves with `--storage`, so the record stays findable. Any action whose `--storage` names another root while the current root still has this workspace's database is refused with both paths; after `reset` there, the new root takes over and the record is rewritten. Reset keeps the record. Only `prepare` writes config; status and reads create nothing.

The `gs://` download cache (`remote::cache_root`), the preview render cache and project `.figlens/` keep their own locations.

## Layout and ownership

A workspace is `<state>/workspaces/<workspaceId>/` holding `workspace.json` (the ownership marker) and `index.sqlite` with its `-wal` and `-shm` files, beside its lock file `<state>/workspaces/<workspaceId>.lock`.

- `workspaceId` is `ws_` plus 16 hex digits of SHA-256 over the locator kind and canonical path: a `--workspace` project directory, or the `.fig` itself for a source-scoped workspace. The marker, the remembered location and the database `meta` table keep the full locator, so an id collision is refused instead of reused.
- `sourceId` starts as `search::source_id` of the canonical source path, the id stateless `search` derives. `relocate` keeps the registered id, so after a relocate workspace search reports the kept id while stateless search derives a new one.
- A source (prepare, add, relocate, `--file`) must be a readable regular file, checked by metadata before it is opened, so a FIFO or device is refused instead of blocking.
- Writes go through the canonical root and real directories only: a symlinked `workspaces/` (under state or config, so location records and their lock are never read or written through a link; the root itself may be an alias) or workspace directory, a source inside any figlens root (default or placed), or an unwritable root fails with the path named. Nothing falls back to another directory. Only a missing root reads as absent; any other error keeps its path and kind.
- Every open, read-only included, first refuses an `index.sqlite`, `-wal`, `-shm` or `-journal` that is not a plain file or has another hard link (`nlink`, or `GetFileInformationByHandle` on Windows): SQLite writes through such a link, so a `.fig` linked there would be overwritten. Reset refuses it too; remove the other link to recover.
- A directory is figlens-owned only when its marker names its id. `prepare` takes the lock beside the directory first, then creates the directory and writes its marker, so a concurrent prepare never sees an unmarked one. An empty directory, or one holding only an unfinished `workspace.json.part`, is claimed, so an interrupted prepare can be retried. Any other directory without the marker, or a database without this workspace's `meta`, is left as found and reported. A missing lock file beside an owned directory is an error until `prepare` or `reset` recreates it.

## Database and locks

- SQLite is `rusqlite` with `bundled`: every release target links SQLite statically, so users install nothing.
- Writing connections use WAL, `synchronous = FULL` and a 5 s busy timeout. A read-only `status` of an existing workspace may let SQLite create its own `-wal`/`-shm`; an absent workspace gets no file at all.
- Schema 1 is `meta` (workspace id, locator, incarnation, creation time) and `sources` (`source_id`, unique `path`, `added_at_ms`). Schema 2 adds a per-source `membership`, the `binding` and the index tables ([index](index-jobs.md)); a v1 database is upgraded in place under the exclusive lock. Schema and identity are read before any write pragma; any newer version is `index_incompatible` and stays untouched.
- Every connection holds the lock file shared for its lifetime; `prepare` and `reset` hold it exclusive. Locks are `std::fs::File` advisory locks (flock, `LockFileEx`) taken with `try_lock` until a 5 s deadline, then `workspace_busy`. The lock file is never removed, so every process locks the same file.
- `incarnation` is `hex(randomblob(16))`, new for every initialization. Later cursors and jobs must bind it so a handle from before a reset never matches the recreated workspace.

## Errors and recovery

Every storage error names the workspace or path and one way on; none suggests another `--storage`, which would meet the same blocked storage. `workspace::recovery` maps the I/O error:

| Kind | Cause | Next step |
|---|---|---|
| `permission_denied` | Access denied or read-only | Restore access; figlens never changes permissions |
| `storage_unavailable` | Missing, unmounted or offline (`NotFound`, `NotConnected`, `TimedOut`, stale network handle) | Reconnect or restore it, then retry; a missing remembered root may be forgotten by deleting its record only if its database is gone for good |
| `workspace_busy` | Another process holds the lock | Retry after it finishes |
| `index_incompatible` | Another schema, or an index built by other versions | Use that figlens version or reset; for an index, `rebuild` |
| `operation_failed` | Anything else | Fix the storage at the named path |

When the current storage cannot be checked, a different `--storage` is refused with that error (`blocked`), because the database may still exist behind it. Reset and rebuild need the old database and stay blocked the same way. Stateless search of a readable `.fig` is unaffected and is the fallback.

Errors carry structured `recovery` (CLI: a JSON line on stderr; MCP: `_meta.figlens.recovery`) listing only actions that apply: `restore_access` or `restore_storage`, and `recreate_elsewhere`, which is `prepare <file.fig> --storage <new dir> --replace-unavailable` (MCP `replaceUnavailable`), and `stateless_search` when a search named a `.fig` the caller may read and it is readable now. Recreation needs the user's confirmation; it leaves every old file in place and starts a new incarnation and binding that know only the given source. A database whose ownership `meta` is unreadable is never reset; recreation elsewhere is its recovery. Config-root errors are reported as config: `--storage` does not repair them.

## Actions

| Action | Effect |
|---|---|
| `prepare <file.fig> [--workspace DIR]` | Creates or reuses the workspace and registers the file; concurrent calls create one database |
| `status <select>` | `absent`, `uninitialized` (after reset) or `initialized` with sources and their presence; creates nothing |
| `add <file.fig> <select>` | Registers another source in a project workspace; a source-scoped workspace keeps only its file |
| `remove <sourceId> <select>` | Deletes the registry row; a source-scoped workspace may unregister its own file and stays valid and empty. The `.fig` is never touched |
| `relocate <sourceId> <new.fig> <select>` | Keeps the id, changes the path and gives the source a new membership, so its old index is `stale` until a refresh. In a source-scoped workspace it first writes `<config>/workspaces/<new id>.json` `{"aliasOf": <workspaceId>}`, so the new path selects and prepares the same workspace; a path with its own workspace is refused. `--file` of a moved-away file still selects through its canonical directory |
| `reset <select>` | Asks active jobs to stop, then exclusive lock, identity check, `wal_checkpoint(TRUNCATE)` that must finish, close, then removes exactly `index.sqlite`, `-wal`, `-shm` and `-journal` |
| `refresh`, `rebuild`, `clear_cache` | See [index](index-jobs.md) |

`<select>` is `--workspace DIR`, `--file <file.fig>` (its source-scoped workspace) or `--id ws_...`. `reset` keeps the marker, lock and remembered location and lists `removed`, `retained` and `failed`; when anything failed, the CLI prints that report and exits 1. Designs, `.figlens/`, exports, global models and other workspaces are never touched.

Remove retires the source's published generations and cancels its jobs in the same transaction. Yolo cleanup never removes a workspace, database or registry; see `skills/figlens/references/yolo.md`.

## MCP authorization

`figlens mcp` uses the default roots without configuration. A custom `storage` in a request must lie under a repeatable `--storage-root <dir>` (separate from design `--root`s), judged where it really leads (`..` and links resolved) before anything there is read or created. A remembered root is the user's earlier choice from config and is reused by every host without a flag. With `--root`, sources and jobs outside the roots are invisible: they are filtered before statistics and serialization, and read as unknown.

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
