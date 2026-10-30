# Viewer search routes and controls

Read when changing indexed search, image inputs, preparation, cancellation or revision-bound refs. General rendering, canvas and cache behavior remain in [preview](preview.md).

## Text and usages

The Relations panel has two tabs. Ranked search calls `search/<revision>?q=…&scope=…&type=…&hidden=0|1&internal=0|1&limit=…&cursor=…&mode=…` ([semantic](semantic.md#viewer)); Usages calls `usages/<revision>?target=…&resource=0|1&relation=…&outgoing=0|1&hidden=0|1&snapshot=…` for the selection's exact ID. Queries are `URLSearchParams` form encoding (`+` is a space) with at most 32 known keys; route and query IDs take up to search's `ID_BYTES` (deep effective IDs); the request line fits two percent-encoded; headers stay ≤8 KiB; unknown keys, a bad scope or an old revision fail; refused values answer `{"error": {kind, message}, source}`. Replies are core's own JSON, coverage and reasons shown verbatim; load more keeps earlier pages' partial coverage and reasons, a new query starts over. Each revision stages the file's bytes once (search's 512 MiB source limit): `snapshotId` hashes exactly the bytes decoded; refs and cursors carry it. Indexes build lazily from the held tree with search's bounds, one per scope, shared by concurrent requests; at most two, never evicting one in use or building. A request for a scope still building, or a new scope with no idle slot, answers `WorkspaceBusy` (Retry), never holding a worker; requests are validated before any build, failed builds drop, deadline-stopped ones rebuild once idle; scoped search needs a stored layer. Usages keep Definition, Members and Instances apart; relation boxes list every kind core reports, checked as the last reply ran; a direction change resets to that direction's defaults; unknown, missing or external targets show their state; identity is the node ID, never a name. Each panel drops replies of an older request or snapshot; a save clears both. Open and copy re-read `revision` and refuse a stale ref; Escape in Layers returns to the row. Offline snapshots search only packaged layers (labelled); usages are unavailable. Agent calls are in [preview-api](preview-api.md).

## Image search

A person selects a PNG or chooses **Use selected stored layer**, optionally enters a positive whole-pixel crop, and chooses Visual or Hybrid. Visual takes an image alone; Hybrid can add text. A local PNG is limited to 8 MiB encoded and 4 MP decoded, with dimensions checked before decoding. The file stays browser-owned until a query sends it to the running local viewer. The service normalizes it in memory and stores no query image or metadata.

A selected layer must already have a visual raster in a current published scope. Its preview is that cached 256 × 256 raster: crop coordinates address those pixels. PNG crop coordinates address the original PNG pixels. The server constructs a reference to the loaded file and snapshot, retaining its registered workspace source identity after relocation; a candidate filter does not choose the reference's source or scope. A missing raster gives an actionable preparation error.

The PNG-query POST names only `image/<revision>`; existing `GET image/<revision>/<node>` renders keep their route and method.

The person presses **Enable visual index of the whole file** or the named layer (text-only setup says **Enable index of** that scope). It prepares text first, then the visual job; source thumbnails and optional image vectors are derived local data. A missing image model keeps hash matching available and is disclosed. Search never enables indexing, downloads a model or sends data remotely. Save and exit stop the build the viewer owns; a joined external build remains externally owned.

| Route | Body / result |
|---|---|
| `POST image/<revision>?q=…&scope=…&type=…&hidden=0\|1&internal=0\|1&mode=visual\|hybrid&crop=x,y,w,h&limit=…&cursor=…&requestId=…` | Raw `image/png`, declared and actual read ≤ 8 MiB; core search reply |
| `POST image-ref/<revision>?…` | `application/json`, exactly `{"nodeId": ID}`, ≤ 4 KiB; the same filters and reply |
| `GET visual-thumb/<revision>/<node>?scope=…` | The exact cached normalized PNG, or a JSON refusal; no render or index build |
| `POST cancel/<revision>` | `application/json`, exactly `{"requestId": ID}`, ≤ 4 KiB; `accepted` reports whether an active call was found |
| `POST enable/<revision>` | `application/json`, `{"scope": ID}` or `{"scope": ID, "visual": true}`, ≤ 4 KiB |

Body routes require the secret path, the expected Host, the viewer's own Origin and same-origin Sec-Fetch-Site when present. MIME and declared size are validated before allocation; the bounded read must complete. Browser inputs cannot name filesystem paths, remote-fetch URLs or output destinations. Unknown or duplicate scalar query keys, stale revisions and bad crop/selection values are refused.

Image result rows show normalized-pixel equality, perceptual distance, visual cosine or semantic field/chunk evidence separately. None proves component identity or business equivalence. Render diagnostics and per-unit visual state remain visible; an unavailable render can still appear through its text channel. A partial channel discloses the reason and provides no continuation cursor.

## Request ownership and cancellation

The UI and browser API assign a 128-bit random requestId to model queries. Escape, a replaced request or an AbortSignal sends the fixed cancel POST and drops its late reply. If the replaced call is still unwinding, the UI retries its replacement once after 500 ms; continued contention shows Retry. The server gives each active call a cancellation flag, shared with SQLite/model work; the entry is removed when the call exits. One optional indexed query runs at a time; another gets WorkspaceBusy with Retry, leaving HTTP workers available for cancellation and status instead of waiting on the single warm runtime. IDs are limited to 64 ASCII letters, digits, hyphens or underscores. Up to 64 early cancellation IDs stay within the revision so a cancel arriving before its queued call prevents that call from starting; older early IDs are evicted. No query text or image is kept in this registry.

A save or shutdown marks the revision ended, cancels its active calls and stops its owned build before joining workers. An old request cannot admit a new call or build afterward. Tests hold a fake model child through cancel, save and SIGTERM, verify the child exits and confirm cancellation leaves the current revision usable. These are correctness checks, separate from native p95 performance acceptance.
