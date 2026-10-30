# Directory add

`index add <dir> <select>` (MCP `index` `add` with `file` naming a directory) registers the `.fig` files under one explicitly given directory in a project workspace (#93). Code: `list_dir`, `add_dir` and `list_add` in `src/workspace/registry.rs`. Registry, identity and the single-file `add` are in [workspace](workspace.md).

## Enumeration

- The root is the given directory, canonicalized. MCP admits it like any `file`: it must lie under the server's `--root`s, checked again when the call runs, so a caller confined to narrower roots cannot name a wider directory.
- The walk enters real subdirectories only, in name order. Under a canonical root, an entry whose canonical path differs is a link: one whose target leaves the root is skipped as `escaping_link` and never followed; a link to a directory inside it is `linked_directory`, since its real path is walked on its own; a link to a `.fig` inside it registers the target once.
- Each file passes the checks of a single add: a `.fig` extension on the target (others only count in `notFig`), `readable` (a regular file, so a FIFO never blocks), outside every figlens root (`inside_storage`) and visible to the caller (`outside_roots`). A failure is `unreadable`; an unreadable root fails the call.
- `skipped` names each entry relative to the root, never where a link leads.

## Bounds

Checked before the work they bound; the reply's `truncated` names the one that stopped it, else `null`:

| Bound | Value | When crossed |
|---|---|---|
| `DIR_SOURCES` | 10 files registered or found registered per call, `query::QUERY_SOURCES` | the walk stops before the 11th: `{reason: source_cap, limit: 10}`; add subdirectories separately |
| `DIR_ENTRIES` | 4096 entries read, links and subdirectories included | the read stops before the next entry: `{reason: entry_budget, limit: 4096}` |

The cap counts existing sources too, so a repeat lists the same files whatever the registry holds.

## Writes

- One IMMEDIATE transaction registers every listed file through `register_in`, the single add's code; any error rolls all of it back.
- A path already registered keeps its `sourceId` and is listed in `existing`; a repeat adds nothing. Nothing is relocated: a file moved inside the directory is a new source, and only `relocate` keeps an id.
- A source-scoped workspace refuses a directory. Nothing is built: each file gets its scope from `prepare --scope`.
- `--dry-run` (CLI only) prints `directory.files` with the same `skipped`, `notFig` and `truncated`, reading no database and creating nothing.

## Hosts

MCP replies `directory {path, added, existing, skipped, notFig, truncated}` under the index output schema. Pi and OMP call that MCP tool and need no code. The OpenCode CLI path asks `read` on the directory, then on every file `--dry-run` lists, before the status and `edit` asks of any add; see [integrations](integrations.md).

## Tests

`src/workspace/tests.rs`: two `.fig` files, a non-`.fig`, an escaping file link, an escaping directory link, in-root file and directory links, a repeat with the same ids, a move inside the directory, a source-scoped refusal, and both bounds. `tests/index_jobs.rs`: a dry run that creates no state, and an MCP caller whose roots are narrower than the directory. `opencode/adapter.test.mjs`: the read asks before any write.
