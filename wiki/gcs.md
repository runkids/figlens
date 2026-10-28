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
| Download | Stored under the platform cache's `figlens/<URI hash>/<generation>.fig`. Same generation reuses bytes, with no copy. A cache miss removes older files for that object, copies to `.part`, then renames to `.fig`. A failed refresh can leave the older cache unavailable. There is no generation-qualified copy or concurrent refresh coordination. |
| Parse | CLI parses on each call. MCP reuses its last unchanged **local** file by path, modification time and length. Remote calls do not enter that parsed-tree cache, even at the same generation. |
| Preview | Resolves GCS once at startup, then watches that local cached path. It does not poll GCS or switch to a new generation. Start a new preview to resolve the object again; another call's cache eviction can remove the old preview's source. Treat stale/error state as unavailable fresh evidence. |

Generation lookup and `cp URI` are separate operations. An overwrite between them can download different bytes from the generation just observed. Successful outputs currently contain no GCS generation provenance. Do not claim a bundle proves an exact remote revision; generation-pinned download and operation-level provenance are follow-up suggestions, not implemented guarantees.

For diagnostics, the user can query the **current remote** generation:

```sh
gcloud storage objects describe gs://example-design-handoff/designs/home.fig --format='value(generation)'
```

This does not prove which generation a previous figlens call actually used. Keep source identifiers and raw cloud diagnostics out of shared logs. MCP source errors expose safe corrective guidance instead of relaying gcloud stderr, which can contain sensitive URLs; URL-source diagnostics are redacted in the CLI error channel too.

## Evidence and limitations

Behavior above is confirmed by source inspection. `python3 scripts/check-gcs-snapshot.py --binary /absolute/path/to/figlens` uses the public synthetic fixture, isolated HOME/cache and an owned `gcloud` stand-in. It verifies metadata checks, download reuse/refresh, failure despite cache, recovery and root refusal. The same check runs on the candidate binary in the existing installation smoke.

This is mock evidence, not a real GCS/IAM/network test. It does not test interactive credentials, cloud races or a running remote preview. Google documents the [describe command](https://docs.cloud.google.com/sdk/gcloud/reference/storage/objects/describe), [copy command](https://docs.cloud.google.com/sdk/gcloud/reference/storage/cp) and [generation metadata](https://docs.cloud.google.com/storage/docs/metadata).
