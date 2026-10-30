# Directory add

`index add <dir> <select>` (MCP `index` `add` with `file` naming a directory) registers the `.fig` files under one explicitly given directory in a project workspace (#93). Code: `list_dir`, `add_dir` and `list_add` in `src/workspace/registry.rs`. Registry, identity and the single-file `add` are in [workspace](workspace.md).

## Enumeration

- The root is the given directory, canonicalized. MCP admits it like any `file`: it must lie under the server's `--root`s, checked again when the call runs, so a caller confined to narrower roots cannot name a wider directory.
- The walk enters real subdirectories only, in name order. Under a canonical root, an entry whose canonical path differs is a link: one whose target leaves the root is skipped as `escaping_link` and never followed; a link to a directory inside it is `linked_directory`, since its real path is walked on its own; a link to a `.fig` inside it registers the target once.
- Each file passes the checks of a single add: a `.fig` extension on the target (others only count in `notFig`), `readable` (a regular file, so a FIFO never blocks), outside every figlens root (`inside_storage`) and visible to the caller (`outside_roots`). A failure is `unreadable`; an unreadable root fails the call.
- `skipped` names each entry relative to the root, never where a link leads.
- A call registers one page: the first 10 passing files in canonical path order (not walk order), after the cursor's file.

## Bounds

Checked before the work they bound; the reply's `truncated` names the one that stopped it, else `null`:

| Bound | Value | When crossed |
|---|---|---|
| `DIR_SOURCES` | 10 files registered or found registered per call, `query::QUERY_SOURCES` | the rest wait for the next page: `{reason: source_cap, limit: 10}` and `nextCursor` |
| `DIR_ENTRIES` | 4096 entries read, links and subdirectories included | the read stops before the next entry: `{reason: entry_budget, limit: 4096}` |

The cap counts existing sources too, so a repeat lists the same files whatever the registry holds.

Past the 4096-entry budget, the entries read follow the OS's order, so the pages and manifest of such a directory are not guaranteed stable. A mismatch fails closed, and the reply says `truncated` `entry_budget`.

## Continuation

`nextCursor` is `d1.<root digest>.<hex of the page's last canonical file>`, or `null` on the last page. `--cursor C` (MCP `cursor`) registers the next page after that file, so a file added or removed before it never shifts the page. A cursor of another root is `cursor_stale`. Every page walks again from the root, so continuation reaches only the files within the first 4096 entries: a wider flat directory still ends at `entry_budget` and is not fully registrable this way.

## Writes

- One IMMEDIATE transaction registers every listed file through `register_in`, the single add's code; any error rolls all of it back.
- A path already registered keeps its `sourceId` and is listed in `existing`; a repeat adds nothing. Nothing is relocated: a file moved inside the directory is a new source, and only `relocate` keeps an id.
- A source-scoped workspace refuses a directory. Nothing is built: each file gets its scope from `prepare --scope`.
- `--dry-run` (CLI only) prints `directory.files` with the same `skipped`, `notFig`, `truncated` and `nextCursor`, reading no database and creating nothing; with `--cursor` it lists that page.
- The dry run also prints `directory.manifest`, a sha256 of its page's canonical files and truncation. `--manifest <hex>` (CLI only, with the same `--cursor`) binds the add to that page: it walks again under the write transaction and, if the digest differs (a new file in the page), registers nothing and says to run the dry run again.

## Hosts

MCP replies `directory {path, added, existing, skipped, notFig, truncated, nextCursor}` under the index output schema and takes `cursor` for `add` only. Pi and OMP call that MCP tool and need no code. The OpenCode CLI path asks `read` on the directory, then on every file `--dry-run` lists, before the status and `edit` asks of any add, which carries the dry run's `--manifest`; a `cursor` goes to both, so each page is asked on its own; see [integrations](integrations.md).

## Tests

`src/workspace/tests.rs`: two `.fig` files, a non-`.fig`, an escaping file link, an escaping directory link, in-root file and directory links, a repeat with the same ids, a move inside the directory, a source-scoped refusal, both bounds, and a manifest add refusing a new file, and 23 files over three pages with a manifest per page and a stale cursor of another directory. `tests/index_jobs.rs`: a dry run that creates no state, and an MCP caller whose roots are narrower than the directory. `opencode/adapter.test.mjs`: the read asks before any write, the add's `--manifest`, and a forwarded `--cursor`. Installed hosts (Pi's MCP command, OMP's bridge, OpenCode managed MCP and its CLI path) follow a directory's `nextCursor` from one candidate; see [host acceptance](host-acceptance.md).
