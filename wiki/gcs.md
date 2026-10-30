# GCS snapshot handoff

Use this cookbook when a designer and an agent exchange a saved `.fig` through one fixed GCS object. This is a snapshot workflow: changes in Figma require another **File > Save local copy** and upload. It is not live synchronization.

## Prerequisites and ownership

The examples use a fictional object, `gs://example-design-handoff/designs/home.fig`. Replace it with an object you are authorized to use.

- **Designer/uploader:** exports the `.fig` and has permission to upload/replace that object. Replacing it is an explicit user action.
- **Agent host:** has Google Cloud CLI (`gcloud` on PATH; `gcloud.cmd` on Windows), connectivity, and existing credentials authorized to read the object's metadata and data. Figlens delegates access to that CLI; it supplies no credentials or permissions.
- **Figlens:** recognizes the exact lowercase `gs://` prefix. Other inputs are local paths. It queries the object generation, maintains a download cache and then reads the saved file.

## Fixed-object recipe

**User:** export the current design locally, then replace the agreed object:

```sh
gcloud storage cp ./home.fig gs://example-design-handoff/designs/home.fig
```

**Agent:** resolve a recognizable screen, inspect its returned ID, and bundle that target. For example, if `find` returns `12:345` for Home:

```sh
figlens find gs://example-design-handoff/designs/home.fig "Home" --type FRAME
figlens inspect gs://example-design-handoff/designs/home.fig 12:345
mkdir -p .figlens/exports && printf '*\n' > .figlens/exports/.gitignore
figlens bundle gs://example-design-handoff/designs/home.fig 12:345 .figlens/exports/home.zip
```

`12:345` is illustrative, not an ID promised by this recipe. The agent should discover it rather than asking users to learn IDs. After another export/upload to the same object, the **next tool/CLI call** queries generation again and downloads when its cache has no file for that generation. Read `context.md` and `missing-assets.json` from the new bundle; saved bundles remain snapshots.

With MCP `--root`, remote sources are unsupported. Keep the restriction: have an authorized user/host copy the object into an approved local directory, then give the agent that local path. Metadata checks and downloads are not attempted for a restricted remote call.

## Cache and refresh semantics

Source of truth: `src/remote.rs`, `src/mcp.rs::serve/AdmittedCall` and `src/main.rs::run` and the local preview watcher.

| Layer | Current behavior |
|---|---|
| Metadata | Every remote call runs `gcloud storage objects describe URI --format=value(generation)` before consulting the downloaded file. A missing CLI, denied access, network error or invalid/empty generation fails the call, even with a cached download. Remote reads are not offline reads. |
| Download | Stored under the platform cache's `figlens/<URI hash>/<generation>.pinned.fig`. Same generation reuses bytes, with no copy. A cache miss removes older files for that object (an index source's copy included), copies `URI#<generation>` to `.part`, then renames it. Older unpinned `<generation>.fig` copies are never reused. A failed refresh can leave the older cache unavailable. There is no generation-qualified copy or concurrent refresh coordination. |
| Parse | CLI parses on each call. MCP reuses its last unchanged **local** file by path, modification time and length. Remote calls do not enter that parsed-tree cache, even at the same generation. |
| Preview | Resolves GCS once at startup, then watches that local cached path. It does not poll GCS or switch to a new generation. Start a new preview to resolve the object again; another call's cache eviction can remove the old preview's source. Treat stale/error state as unavailable fresh evidence. |

Generation lookup and the copy are separate operations, but the copy is pinned: `cp URI#G` serves generation G's bytes or fails (an overwrite meanwhile, with no versioning keeping G), so the cached file is G's. Bundles and other outputs still carry no generation provenance; do not claim a bundle proves an exact remote revision.

For diagnostics, the user can query the **current remote** generation:

```sh
gcloud storage objects describe gs://example-design-handoff/designs/home.fig --format='value(generation)'
```

This does not prove which generation a previous figlens call actually used. Keep source identifiers and raw cloud diagnostics out of shared logs. MCP source errors expose safe corrective guidance instead of relaying gcloud stderr, which can contain sensitive URLs; URL-source diagnostics are redacted in the CLI error channel too.

## Index registration

`index add gs://bucket/object <select>` (MCP `index` `add` with that `file`; only `add` takes one) registers the object in a project workspace (#93). Code: `command::index_add` downloads through `remote::fetch`, then `registry::add_remote` registers the copy. A source-scoped workspace refuses it.

- `sources.path` is the pinned download and `sources.remote`/`generation` (schema 8) the URI and the generation it was pinned to. Status and the add reply list `remote {uri, generation}`. That generation names the bytes indexed, not the object now. The download sits in figlens' cache, which a local add refuses as storage; this add skips that check.
- `sourceId` hashes the URI. A repeat at the same generation downloads nothing and adds nothing. A newer generation moves the source to its download with a new membership and stops its builds, as relocate does; refresh then rebuilds it. `relocate` refuses a remote source: add again, or remove it. Moving between a local path and `gs://` is not supported.
- Snapshot identity is the worker's hash of the downloaded bytes, like any source.
- Network: only that add runs `gcloud`. Refresh, rebuild, status, search, changes and watch read the local copy. A watch lease watches that file, which a download never rewrites; it is not remote monitoring, and there is no remote polling. Another read that downloads a newer generation removes the copy, so the source reads missing until it is added again.
- MCP with `--root` refuses it before `gcloud` runs. The OpenCode CLI path refuses `gs://`; its MCP path serves it.

Tests: three `gs` and `gcloud` tests in `tests/index_jobs.rs` use a `gcloud` stand-in on `PATH`. They cover the pinned copy, the snapshot equal to a local copy's, an idempotent repeat, a moved generation, no `gcloud` from refresh, track, status, search, changes or a running watch, and MCP root refusal.

## Evidence and limitations

Behavior above is confirmed by source inspection. `python3 scripts/check-gcs-snapshot.py --binary /absolute/path/to/figlens` uses the public synthetic fixture, isolated HOME/cache and an owned `gcloud` stand-in. It verifies metadata checks, pinned download reuse/refresh, failure despite cache, recovery and root refusal. The same check runs on the candidate binary in the existing installation smoke.

This is mock evidence, not a real GCS/IAM/network test. It does not test interactive credentials, cloud races or a running remote preview. Google documents the [describe command](https://docs.cloud.google.com/sdk/gcloud/reference/storage/objects/describe), [copy command](https://docs.cloud.google.com/sdk/gcloud/reference/storage/cp) and [generation metadata](https://docs.cloud.google.com/storage/docs/metadata).
