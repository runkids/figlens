# Durable index and jobs

`src/workspace/jobs.rs` builds and publishes a workspace's search indexes in a child process; `src/workspace/query.rs` searches them; `src/search/store.rs` writes and reads one generation. Roots, identity, locks and the registry are in [workspace](workspace.md); tokenizer, ranking and coverage are in [search](search.md).

## Generations

Schema 2 adds `snapshots` (verified size and mtime per content hash), `generations`, `published` (one generation per source and scope), `records`, `terms` (delta-varint postings per term and field), `edges` and `jobs`. A v1 registry is upgraded in place under the exclusive workspace lock, keeping every source.

- A generation is one source, one exact scope node and one membership. Search never widens or narrows the scope and never parses the `.fig`. Prepare without `--scope` only registers.
- `versions` records ranking, tokenizer and budgets. A generation from other versions is `incompatible` and `rebuild` is the recovery.
- `edges` keep only evidence-correct relations: an instance's `instance_of` points at its component only when the expansion followed that component (symbolData, definition or swap) and it is a COMPONENT. A swap to a missing main is stored unresolved (no target), never the definition's main. `contains` is each record's stored `parent`. Coverage reports `layers.relations` with both kinds' state and lists `variant_of`, `binds_variable`, `uses_style`, `uses_image`, `prototype_target` and `library_identity` as `relation_unavailable`, so the layer is never `ready`. There is no usages tool (#91).
- Each source keeps at most `MAX_SCOPES` = 8 published scopes; the least recently published is retired. Retired and abandoned generations go at the next publish or `clear_cache`.

## Jobs

`prepare --scope`, `refresh` and `rebuild` submit jobs. One row per job; a unique partial index allows one queued or running job per source and scope across every process, so a second submit joins it (`joined: true`). A `requestId` with the same arguments replays the same job; with different arguments it is refused. A workspace accepts `MAX_ACTIVE` = 16 active jobs and keeps the last `KEPT` = 50 finished ones. A refresh or rebuild that cannot admit every job replies `admission: partial` with `deferred` items (sourceId, scope and the `nextStep` index call that builds it later) and `deferredBecause`; otherwise `admission: complete`. A job that waits over 600 s for its build turn fails `workspace_busy`; a queued job's cancel is accepted at once. A publish that retires a scope past `MAX_SCOPES` names it in the job's `evictedScopes`.

- The submitting process owns the job: it holds `<workspace>/owners/<token>.lock` for its lifetime and runs `figlens __index-worker <dir> <jobId>`. The child exits when its stdin closes, so it never outlives its owner. A job whose owner lock is free is `interrupted`; the next submit for that scope takes it over as a new attempt.
- Stages: `stage` (owner), `hash`, `parse`, `write`, `verify` (child), `publish` (owner). The child writes an unpublished generation in batches of 2,000 rows; a build that would pass `DISK_CAP` = 2 GiB of database plus WAL fails, and before each batch it waits for the WAL to drop under `WAL_CAP` = 64 MiB.
- Publication is one IMMEDIATE transaction fenced by the workspace incarnation and binding, the owner and attempt, the source's current membership, the cancel flag and the expected previous generation. Any mismatch publishes nothing and the prior generation keeps serving. Remove plus add of the same path gets a new membership, so a build started before cannot publish (ABA).
- An unchanged snapshot (same hash and versions, complete) is reused instead of rebuilt, except by `rebuild`.
- `jobs cancel` sets the flag; the owner polls it every 100 ms, kills the child, waits for its exit, drops its unpublished generation and only then reports `cancelled`. `cancel` replies `accepted` with the terminal state, or `pending` after `CANCEL_WAIT` = 2 s.
- A CLI interrupted job is retried as its own kind: an interrupted rebuild stays a rebuild, never a refresh that reuses an unchanged generation.
- `FIGLENS_TEST_HOLD=<stage>:<file>` holds a debug-build worker in a stage while the file exists; `tests/index_jobs.rs` uses it.

## Actions

| Action | Effect |
|---|---|
| `refresh <select> [--source ID] [--scope NODE]` | Builds the chosen published scopes again when their bytes changed |
| `rebuild <select> [--source ID] [--scope NODE]` | Builds the chosen published scopes again even when unchanged (a corrupt or deferred one included); the previous generations serve until each replacement publishes |
| `clear_cache <select>` | Deletes retired generations, unpublished ones no live job writes, and unreferenced snapshots; keeps the registry, published indexes, jobs and location |
| `jobs status <jobId>` / `jobs cancel <jobId>` | A job's state, stage, progress, attempt, generation and next steps |

The CLI runs owned jobs in the foreground with stderr progress, then exits 1 if any did not succeed. MCP runs them on its build lane and returns at once; see [mcp](mcp.md).

## Workspace search

`search --workspace DIR|--file F|--id WS [--source ID]...` (MCP: `workspaceId`, `sourceId`) re-resolves location and binding per request and reads one transaction:

- Sources outside the caller's visibility (MCP `--root`) are dropped before any row, count or path is read. A reply over a corpus that lost sources this way has `coverage.complete: false`, `totalRecords: null` and reason `authorization_restricted`, naming no source and no count.
- Each source is `ready`, `pending`, `not_prepared`, `stale` (size or mtime differs from the verified snapshot, or relocated since), `incompatible` or `unreadable`. Only ready sources are searched; the rest are listed with their next step and make coverage partial (`sources_not_ready`). A `not_prepared` source whose scope has no published generation carries `reason: scope_not_indexed`; with nothing ready the reason is `index_pending` while a job runs, else `scope_not_indexed`. When every source is stale the reply is `source_stale` with refresh recovery.
- Loading reads only the query terms' postings, intersects them, and fetches the shown page's records and their ancestors. Corpus statistics, phrase tiers and ranking cover every survivor of every source before the global top-k; independently ranked pages are never merged.
- A search that cannot use the workspace (storage unavailable, busy, incompatible) and named a `.fig` the caller may read adds a `stateless_search` recovery with only the arguments the request gave.
- A cursor binds the generations it paged: `cursor_stale` when one was replaced, `cursor_expired` when one was deleted.

Results equal stateless search of the same scope; only `ref.sourceId` differs after a relocate, by design.

## Capacity

Hard refusals, checked before the work they bound:

| Limit | Value | When crossed |
|---|---|---|
| Ready sources one workspace search reads | 10 | `query_work_limit` before any index loads; recovery lists one search per source |
| Records of those generations in total | 100,000 (not 10 times a per-source cap) | the same, plus `index_narrower_scope` when one source alone exceeds it |
| Record text one load copies | 512 MiB | `query_work_limit`, never `index_incompatible` |
| One scope extraction (build) | `WORK` 10M units | the generation is partial with `work_limit` |
| Database plus WAL | `DISK_CAP` 2 GiB, WAL `WAL_CAP` 64 MiB | the build fails; the prior generation serves |
| Published scopes per source, active jobs per workspace | 8, 16 | oldest scope retired; submit refused |

The registry and every published index stay when a search exceeds the envelope. Measured so far: one source of 36,171 records and the two-source and eleven-source test corpora; a corpus near 100,000 records over 10 sources is not measured yet.

## Measurements

Docker on Apple silicon, release build, a public 60 MB Community file:

| Case | Result |
|---|---|
| Scope `0:0`, 36,171 records: build | p50 0.85 to 0.95 s; worker max RSS about 600 MiB; database 4 to 12 MiB; WAL 0 after checkpoint |
| Page scope, 2,263 records: build x20 | p50 432 ms, p95 526 ms; RSS 534 MiB; database 780 KiB |
| Warm workspace search x100 (CLI process) | p95 1.7 to 4.9 ms; stateless search 347 to 620 ms |
| Cancel x20 per stage, request to terminal state with no worker left | max 130 (stage), 121 (hash), 102 (parse), 230 (write), 136 (verify) ms |
| Parity, 36 queries and filters including page 2 | identical to stateless search |

Worker memory is the parse of the whole `.fig`, as in stateless search; a database is bounded by `DISK_CAP`. A cancelled or failed build leaves the previous generation serving. Native Windows and macOS runs are not measured yet.

## Tests

`src/workspace/tests.rs` covers the v1 upgrade and registry. `tests/index_jobs.rs` drives the real binary: parity and rebuild, two sources ranked as one corpus, a stale cursor after rebuild, per-stage cancel that stops the worker first and keeps the prior index, a killed owner whose worker exits and whose job a refresh retries, concurrent prepares joining one job, remove plus add during a build not publishing, source-scoped relocate, MCP background builds with responsive status, search and cancel, MCP `--root` hiding sources and jobs, and no database from listing or stateless calls.
