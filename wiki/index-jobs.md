# Durable index and jobs

`src/workspace/jobs.rs` builds and publishes a workspace's search indexes in a child process; `src/workspace/query.rs` searches them; `src/search/store.rs` writes and reads one generation. Roots, identity, locks and the registry are in [workspace](workspace.md); tokenizer, ranking and coverage are in [search](search.md).

## Generations

Schema 2 adds `snapshots` (verified size and mtime per content hash), `generations`, `published` (one generation per source and scope), `records`, `terms` (delta-varint postings per term and field), `edges` and `jobs`; schema 3 reshapes `edges` and adds `definitions` ([usages](usages.md)); schema 4 adds change-tracking tables, empty until a scope is tracked. Tracking, revisions and their publication are in [changes](changes.md). A v1 registry is upgraded in place under the exclusive workspace lock, keeping every source.

- A generation is one source, one exact scope node and one membership. Search never widens or narrows the scope and never parses the `.fig`. Prepare without `--scope` only registers.
- `versions` records ranking, tokenizer and budgets. A generation from other versions is `incompatible` and `rebuild` is the recovery.
- `edges` and `definitions` hold the usages relations (schema V3), `fingerprints` the definition fingerprints (schema 8, [library usages](library-usages.md)), all collected with their generation; their kinds, states and paging are in [usages](usages.md). An `instance_of` edge points at the main the expansion actually followed (symbolData, definition or swap); a swap to a missing main keeps its id as `missing`, never the definition's main. Coverage `layers.relations` lists every kind with the relations version (stateless search marks the field kinds it did not build `not_built`, see [usages](usages.md#cost)) and stays `partial` because `library_identity` (#93) is unavailable.
- Each source keeps at most `MAX_SCOPES` = 8 published scopes; the least recently published is evicted, keeps no generation, and is named in the job's `evictedScopes` (stored in `jobs.evicted`). A published scope keeps at most one retired generation; older ones and abandoned ones go at the next publish or `clear_cache`.
- No lease protects a generation a query reads: a workspace search reads every generation in one SQLite read transaction, and a cursor over a generation no longer published is `cursor_stale`, or `cursor_expired` once it is deleted.

## Jobs

`prepare --scope`, `refresh` and `rebuild` submit jobs. One row per job; a unique partial index allows one queued or running job per source and scope across every process, so a second submit joins it (`joined: true`). A rebuild that joins a queued refresh makes it a rebuild; a running refresh already read its kind, so a rebuild is refused `workspace_busy` with a wait and then a rebuild, and its `requestId` is not recorded. Admission judges the source's visibility again inside its write, before any replay or admission. A `requestId` with the same arguments replays the same job, also one that only joined or took it over (or no job, for a `track` that found its scope already tracked); with different arguments it is refused, and `track` is its own operation, never a rebuild's replay. A join, replay or takeover marks the job `shared` (`jobs::share`), so a watch runner never cancels it as its own. A live build already cancelled, by its caller or a closing runner, is never joined: `workspace_busy` until it stops. A workspace accepts `MAX_ACTIVE` = 16 active jobs and keeps the last `KEPT` = 50 finished ones. A refresh or rebuild that cannot admit every job replies `admission: partial` with `deferred` items (sourceId, scope and the `nextStep` index call that builds it later) and `deferredBecause`; otherwise `admission: complete`. Builds run in admission order (`seq`, assigned in the submit transaction and again at takeover), so each owner's jobs queue in the order it runs them and no two owners wait on each other. A job that waits over 600 s for its build turn fails `workspace_busy`; a queued job's cancel is accepted at once. A publish that retires a scope past `MAX_SCOPES` names it in the job's `evictedScopes`.

- The submitting process owns the job: it holds `<workspace>/owners/<token>.lock` for its lifetime and runs `figlens __index-worker <dir> <jobId>`. The owner's exit closes the child's stdin, which asks it to stop; a stopped child can outlive it, so cancellation awaits its confirmed exit. A job whose owner lock is free is `interrupted`; the next submit for that scope takes it over as a new attempt, and a rebuild that takes over a refresh makes it a rebuild.
- Stages: `stage` (owner), `hash`, `parse`, `write`, `verify` (child), `publish` (owner). The child writes an unpublished generation in batches of 2,000 rows or an estimated 8 MiB of row bytes (`BATCH_BYTES`), whichever comes first; a single row estimated larger is its own batch. Its connection sets `max_page_count` to `DISK_CAP` = 2 GiB less a 64 MiB headroom; SQLite never sets it below the file's current size, so a file already past it can only reuse its freed pages. SQLite refuses any page past the ceiling it set: that batch rolls back, the build fails and the prior generation serves; a failed build's pages are freed and reused, so a retry fits again without a `VACUUM`. Publication, registry and job writes are not capped and use the headroom. After each batch the child checkpoints and waits up to the lock wait for the WAL to drop under `WAL_CAP` = 64 MiB, else fails `workspace_busy` (a long search read pins the WAL). `WAL_CAP` is a threshold, not a ceiling: the WAL can pass it by one batch's pages (its estimated rows, or one larger row, plus b-tree overhead).
- Publication is one IMMEDIATE transaction fenced by the workspace incarnation and binding, the owner and attempt, the source's current membership, the cancel flag and the expected previous generation. Any mismatch publishes nothing and the prior generation keeps serving. Remove plus add of the same path gets a new membership, so a build started before cannot publish (ABA). Remove and relocate cancel the source's builds; a submit while one of the old membership still stops is `workspace_busy` with a wait on it, never a join.
- `jobs cancel` sets the flag; the owner polls it every 100 ms, kills the child, waits for its exit, drops its unpublished generation and only then reports `cancelled`. When the owner died, cancel waits for its orphaned worker to exit. The owner creates an empty `claims/<jobId>.<attempt>.worker` before spawning it; the worker locks it, writes `started` and holds the lock until the process exits. Locked, or empty and unlocked (a worker that has not locked yet), counts as running and cancel replies `pending`; a worker whose stdin closes during startup marks the file before it exits, so it never stays empty; a takeover by the next submit starts a new attempt that fences the old worker out. `cancel` replies `accepted` with the terminal state, or `pending` after `CANCEL_WAIT` = 2 s.
- A CLI interrupted job is retried as its own kind: an interrupted rebuild stays a rebuild, never a refresh that reuses an unchanged generation. An interrupted, failed or cancelled rebuild's `nextSteps` is a rebuild too.
- `FIGLENS_TEST_HOLD=<stage>:<file>` holds a debug build at a stage: a worker stage (`start` is before it locks its worker file), or `submit`, `compare`, `changes` (after the read chose its sources) or `clear_cache` (after it chose what to delete) while the file exists and creates `<file>.reached`; `FIGLENS_TEST_ORPHAN` keeps it running after its owner died; `tests/index_jobs.rs` uses both.

## Reuse keys

- Generation (records, terms, edges, definitions, fingerprints, facets): `refresh` reuses a complete one with the same snapshot sha256, `store::versions()` and membership; `rebuild` never does.
- Overrides, main definitions and variable modes are in the file bytes, so the hash covers them; no generation depends on another file. The fingerprint fold and `diff::facet_version()` are not in `versions()`: bump `RELATIONS` with them.
- Any byte change parses and builds the whole file: no node-level incremental decoding, no chunk or vector cache.
- Preview renders: path hash + crate version + mtime + length (no content hash). `gs://` downloads: URI hash + object generation. MCP parsed tree: path + mtime + length.

## Actions

| Action | Effect |
|---|---|
| `refresh <select> [--source ID] [--scope NODE]` | Builds the chosen published scopes again when their bytes changed |
| `rebuild <select> [--source ID] [--scope NODE]` | Builds the chosen published scopes again even when unchanged (a corrupt or deferred one included); the previous generations serve until each replacement publishes |
| `clear_cache <select>` | Deletes retired generations, unpublished ones no live job or orphaned worker writes, and unreferenced snapshots; keeps the registry, published indexes, jobs and location. Each deletion reauthorizes its source: one moved out of a confined caller's view meanwhile keeps its data, and the reply has `partial: true` |
| `jobs status <jobId>` / `jobs cancel <jobId>` | A job's `workspaceId`, state, stage, progress, attempt, generation and next steps |

The CLI runs owned jobs in the foreground with stderr progress, then exits 1 if any did not succeed. MCP runs them on its build lane and returns at once; see [mcp](mcp.md).

## Workspace search

`search --workspace DIR|--file F|--id WS [--source ID]...` (MCP: `workspaceId`, `sourceId`) re-resolves location and binding per request and reads one transaction:

- Sources outside the caller's visibility (MCP `--root`) are dropped before any row, count or path is read. A reply over a corpus that lost sources this way has `coverage.complete: false`, `totalRecords: null` and reason `authorization_restricted`, naming no source and no count.
- Each source is `ready`, `pending`, `not_prepared`, `stale` (size or mtime differs from the verified snapshot, or relocated since), `incompatible` or `unreadable`. Only ready sources are searched; the rest are listed with their next step and make coverage partial (`sources_not_ready`). A `not_prepared` source whose scope has no published generation carries `reason: scope_not_indexed`; with nothing ready the reason is `index_pending` while a job runs, else `scope_not_indexed`. When every source is stale the reply is `source_stale` with refresh recovery.
- Loading reads only the query terms' postings, intersects them, and fetches the shown page's records and their ancestors. Corpus statistics, phrase tiers and ranking cover every survivor of every source before the global top-k; independently ranked pages are never merged.
- A search that cannot use the workspace (storage unavailable, busy, incompatible) and named a `.fig` the caller may read adds a `stateless_search` recovery with only the arguments the request gave (node and filters; not the cursor).
- A cursor binds the query, filters, ranking versions, and the generations and corpus statistics (`corpus_rank`: record count, field lengths, term df) of only the sources it read: `cursor_stale` when one was replaced, never for an unselected source; `cursor_expired` when one was deleted or an hour (`search::CURSOR_LEASE_MS`) after the page was served.

Results equal stateless search of the same scope; only `ref.sourceId` differs after a relocate, by design.

## Capacity

Hard refusals, checked before the work they bound:

| Limit | Value | When crossed |
|---|---|---|
| Ready sources one workspace search reads | 10 | `query_work_limit` before the generation that would cross it loads; recovery lists one search per visible source, keeping the request's node and filters (not its cursor) |
| Records of those generations in total | 100,000 (not 10 times a per-source cap) | the same, plus `index_narrower_scope` when one source alone exceeds it |
| Record text and edge targets and evidence one query copies over all its loads, checked before copying | 512 MiB | `query_work_limit`, never `index_incompatible` |
| One scope extraction (build) | `WORK` 10M units | the generation is partial with `work_limit` |
| Main database file | `DISK_CAP` 2 GiB: a hard `max_page_count` ceiling on builds at 2 GiB less 64 MiB headroom, or the current size when already larger (no growth, freed pages reused) | the build fails; the prior generation serves |
| A generation's serialized metadata (pages, roots, ancestors, as JSON) | 64 MiB, the size a search reads back | the build fails `parse_budget_exceeded` before writing any row; the prior generation serves |
| WAL | `WAL_CAP` 64 MiB threshold after each batch; one batch's pages over it at most | the build fails `workspace_busy`; the prior generation serves |
| Published scopes per source, active jobs per workspace | 8, 16 | oldest scope retired; submit refused |

The registry and every published index stay when a search exceeds the envelope. Measured so far: one source of 36,171 records and the two-source and eleven-source test corpora; a corpus near 100,000 records over 10 sources is not measured yet.

## Measurements

Measured at `fd8d4bf` (the contended and base rows at `ba08580`) in Docker (linux/aarch64 on Apple silicon), release build, a public 60 MB Community file kept in the OS page cache (warm file cache); each figlens call is a new process. n=20 unless stated.

| Case | Scope `0:0` (36,171 records, 2,349 terms, 6,351 edges) | Page scope (2,263 records) |
|---|---|---|
| First `prepare --scope`, new workspace | p50 865 ms, p95 962 ms | p50 461 ms, p95 562 ms |
| `rebuild` | p50 865 ms, p95 945 ms | p50 531 ms, p95 546 ms |
| Stages p50 (stage, hash, parse, write, verify) | 13, 21, 558, 161, 100 ms | 15, 21, 318, 24, 133 ms |
| Peak RSS, owner plus worker summed | 606 MiB | 533 MiB |
| Database after rebuilds (WAL after checkpoint) | 12.4 MiB (0) | 780 KiB (0) |
| SQL subset of the publication transaction (3 selects, update, insert) on a copy, n=100; not the fenced `publish()` | p50 1.0 ms, p95 1.5 ms | p50 0.9 ms, p95 1.2 ms |
| Warm workspace search, n=100, p95 | 4.9 ms (`button`), 2.1 ms (`sign in`), 8.8 ms (most common term, df 5,054) | 1.9, 1.9, 2.7 ms (df 532) |
| Stateless search / `find` / `inspect`, p50 | 600 / 319 / 326 ms | 351 / 320 / 382 ms |
| Workspace search, n=100, while a rebuild loop writes and publishes (7 publishes, 0 errors) | p50 4.6, p95 5.4, max 9.6 ms (idle: 3.9, 4.3, 5.6) | — |
| `find` / `inspect 0:1` / stateless search vs base `0b6d8a6`, n=20 interleaved, p50 (p95) | find 318 (412) vs 320 (352); inspect 317 (324) vs 313 (322); search 582 (605) vs 578 (638) ms | — |

- Hashing the 57 MiB file (sha256, page-cached) takes about 26 ms; verify also re-reads the written generation.
- Cancel, request to terminal state with no worker left, x20 per stage, max: 132 (stage), 121 (hash), 104 (parse), 186 (write), 137 (verify) ms.
- Parity: 36 queries and filters including page 2 return the same results as stateless search.
- Stage times come from a 2 ms poll of the job row; whole rebuild timings include the real fenced publication. `find` and `inspect` output is byte-identical to the base; stateless search adds only `coverage.layers.relations` (plus its per-run freshness timestamps).

Worker memory is the parse of the whole `.fig`, as in stateless search. Not measured yet: a cold OS cache, native Windows and macOS (PR3 hosts), and a corpus near the 100,000-record, 10-source envelope.

## Tests

`src/workspace/tests.rs` covers the v1 upgrade and registry. `tests/index_jobs.rs` drives the real binary with `HOME` and `%LOCALAPPDATA%` set to its own test home and the XDG roots cleared, and checks every reported root stays inside it: parity and rebuild, two sources ranked as one corpus, a stale cursor after rebuild, per-stage cancel that stops the worker first and keeps the prior index, a killed owner whose worker exits and whose job a refresh retries, concurrent prepares joining one job, remove plus add during a build not publishing, source-scoped relocate, MCP background builds with responsive status, search and cancel, MCP `--root` hiding sources and jobs, and no database from listing or stateless calls.
