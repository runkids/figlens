# MCP workflow discovery

Use this topic for MCP instructions, tool discovery and built-in workflow guidance. Design decoding and result contracts are in `fig-format`; Pi-specific commands and tools are in `pi`; OpenCode/OMP MCP registration, override policy and host permissions are in [integrations](integrations.md).

## Intent and tool selection

`src/mcp.rs` serves stdio JSON-RPC; `src/main.rs` configures roots and starts it. Initialization returns a short intent router: resolve recognizable screens through `find`/`tree` (or `search` for effective instance copy), read compact values with `inspect`, full style/mask/component data with `nodes`, tokens with `variables`, and pixels/assets with `export`. `bundle` writes a scoped handoff, `verify` checks it. For repeated searches it points to `index` prepare with a scope, `jobs` status, then `search` with `workspaceId`. Ordinary questions and exports use those tools directly.

`inspect` and `bundle` accept optional `states: true` (CLI `--states`) for a bounded, source-scoped list of explicitly related component variants. The agent supplies this per-call argument when related states are requested; no MCP configuration change is needed. Inspect adds `states`; bundle adds `design/states.json`, a manifest pointer and a bounded context summary. Independent inventory truncation/coverage must be checked; supported prototype fields retain exact sources and unknown markers, without business inference. Both reuse their selected effective subtree and do not render alternate variants. See [related variants](states.md) for evidence, batch usage and limits.

For two selected variants, `inspect` accepts `compareVariant: "other-node-id"` (CLI `--compare-variant ID`) and adds `variantComparison`. Both IDs must be stored component definitions in the same set. It compares bounded raw root fields and descendant value multisets, retains exact sources/unknowns and leaves descendants unpaired. It performs no alternate expansion/rendering and changes no bundle or preview controls. See [variant comparison](states.md#compare-two-variants).

`inspect` accepts `flat: true` (CLI `--flat`): the content text is one line per node and `structuredContent` is `{flat, truncated}` without `node`, which is why the inspect output schema requires only `truncated`. `export` accepts `matchDir` (CLI `--match-dir`) for SVG output; it is admitted and rechecked under `--root` like `out`, and the reply adds `matches` and `similar` (paths relative to that folder; see [render](render.md)).

The `workflow` tool lets a model retrieve detailed guidance without a separate skill installation:

| `topic` | Use when |
|---|---|
| `build` | Implement a design with the project's components, tokens and architecture, then compare and correct the running UI |
| `audit` | Compare existing UI with a design, report evidence and coverage, and correct confirmed differences only when requested |
| `images` | Prepare website assets using the project's image pipeline and verify dimensions, crops, bytes and delivery |
| `plan` | Plan an implementation or carefully reproduce a design; stop at the plan for plan-only or plan-first requests |
| `planning-details` | Detail page/multi-page scope or a specific unresolved design/project dependency |
| `yolo` | The user explicitly requests autonomous implementation and cleanup of this run's temporary plan after successful checks |

The tool requires only an enumerated `topic`, accepts no file/path argument and rejects extra properties. Reading guidance neither reads a project nor writes, implements, verifies or cleans it. The current agent uses its host tools for project inspection/edits, directory creation, bundle extraction and browser checks. Host permissions still apply. A guide is not an implementation result or a plan-cleanup guarantee.

## Shared sources and runtime behavior

Stdio request lines are capped at 1 MiB including line endings before JSON parsing or admission. Oversized lines are drained through their newline and return JSON-RPC `-32600` with a null ID; invalid UTF-8 returns `-32700`. The next request can still run, including against a warm document. This is a byte limit, not an elapsed-time limit on a peer that never finishes a line.

Complete serialized replies, including escaping, structured content, inline base64 and newline, are capped at 8 MiB. The server counts serialization without allocating a second full reply buffer. Over-budget tool replies become `isError: true` with `_meta.figlens.code: response_budget_exceeded`; completed bundle/export files remain and their path is retained as `_meta.figlens.outputPath`. Full design JSON is never silently truncated. MCP `nodes` also applies the budget while building: once the selection's compact node fields, counted without children, exceed it, building stops with the same error before the full JSON exists. That count is a lower bound of the reply, so no reply that fits is rejected. For a stored root, instance expansion stops the same way, counting each expanded node's emitted `id`, `name` and `type`. For an instance sublayer, the outer instance still expands and builds first; `inspect`, `variables`, `bundle` and the CLI do not use this guard. See [measurement](performance.md#early-mcp-nodes-budget-2026-10-29). Use scoped inspect/bundles, CLI nodes, or a smaller export `maxSize`. Non-tool protocol replies return `-32603` on overflow. This transport policy leaves headroom below a reported 10 MiB client limit; it is not a guarantee for every MCP client or a process-memory limit. CLI output remains uncapped. See [measurements](handoff-research.md#mcp-message-sizes).

`src/workflow.rs` embeds `skills/figlens/SKILL.md` and its workflow references with `include_str!`. Build includes the font/asset preflight: check required font faces and asset files, report missing/unverified dependencies, and verify actual browser loading before acceptance. `document.fonts.check` alone can pass on fallback. Bundle `context.md` also carries a pre-implementation warning: project font availability starts `UNVERIFIED`, even with no bundle warnings. Missing/unverified inputs do not block implementation: use documented project-font fallbacks or labelled image placeholders at the intended dimensions, retaining exact gaps and remedies in the handoff. Only final visual acceptance remains pending for unresolved inputs. This is agent guidance, not an automatic project-font scan or a proven visual-quality improvement; decoding, assets, reference pixels and output schemas are unchanged. Build, audit and images return relevant level-two sections, including where outputs go and known gaps; `out` descriptions repeat the location rule. Plan, planning-details and YOLO return their maintained reference. Pi and the portable skill use these sources too; CLI `figlens workflow <topic>` prints them with a CLI preamble instead of the MCP one. Update section selectors if a selected heading changes. Cargo's source inclusion covers the embedded Markdown too.

Audit adds `references/audit.md` to the shared evidence rules also returned by build: compare elements, visual relationships and supported variations, identify scope/design/code evidence and distinguish static review from actual screenshot comparison. Scope coverage records structure-only and unexamined items separately. Reuse current snapshot evidence; stop an unresolved item when another read cannot add evidence, with its missing input and next action. Browser restrictions leave visual acceptance pending rather than blocking independent static work. Review-only requests grant no application-edit permission. CLI exposes `figlens workflow audit`; Pi discovers it through the same MCP schema and packaged skill, without a separate command or mapping change. These instructions are not an automatic audit engine or evidence that a model's detection rate improved.

Local reference links in returned text become `workflow:<topic>#<optional-section>`; the agent follows them with another call to this tool. No runtime Markdown files, network fetch or skill discovery mechanism is required. The binary can serve guidance from an empty working directory, including under `--root`; design/output path restrictions remain in force for design tools.

The client decides whether initialization instructions enter model context. The workflow tool's description also exposes the choices so retrieval is discoverable through `tools/list`. This does not prove a particular model will select or follow the right guide. MCP prompts/resources are not required for this model-selected tool path.

## Verification

Run Docker Rust tests/lint and `python3 scripts/ai-context.py check`. Tests cover schema/defaults, admission, notification side effects, cached-source changes and rooted/unrooted paths. After building, run `docker compose run --rm dev python3 -B scripts/check-mcp-admission.py --binary /work/target/debug/figlens` for actual POSIX warm-cache permission denial/recovery. Pi/npm tests and source inspection alone do not prove candidate runtime compatibility or model/browser behavior.

## Parameter contract and recoverable failures

`src/contract.rs` owns metadata, discovery, normalization and defaults. `prepare` returns a private-field, immutable `PreparedCall` required by `Command::from_tool`, once before admission/loading. Mathematical JSON integers (`2.0`) normalize; fractions, negative integers, overflow and nonfinite scales fail.

| Parameter | Range / default |
|---|---|
| `tree.depth` | 0 through `2^53-1` (JSON-safe); omitted means every level; past 256 levels the call fails with `parse_budget_exceeded` ([layer depth](parsing.md#layer-depth)) |
| `inspect.depth` | 0–10 / 1 |
| `find.limit` | 1–200 / 20 |
| `search.limit` | 1–200 / 20 |
| `export.scale` | finite and greater than 0 / 1 |
| `export.quality` | 1–100 / 80 |
| `export.maxKb` | 0 through `2^53-1`; omitted means no byte budget |
| `export.maxSize` | 0 through `u32::MAX` / 1600; 0 disables the raster side cap |
| `export.bounds` | `render` or `layout` / `render` |

`tree`/`find`/`search` root defaults to `0:0`; booleans default false. `tree` `topFrames: true` ignores `depth` and returns the screens under the root (rule in the `discovery` topic): `id TYPE "name"` lines whose JSON-escaped names keep one id per line, or with `json` a flat array of entries without children plus `page` and innermost `section`. With `json`, children of a page or section (and every `topFrames` entry) add `firstText`. Search queries take at most 256 characters and 64 distinct terms (a CJK run of n characters is n-1 bigrams), types 16 names, scopes 1 KiB and cursors offset 100,000 (past it `hasMore` stays true without a `nextCursor`, so narrow the query); a page whose estimated copied names, paths and snippets exceed 8 MiB is refused with `response_budget_exceeded` before results are built (the serialized-reply cap above still applies), so lower `limit` or narrow `node`; `scripts/measure-search.py` measures cold CLI, MCP first and warm search cost and peak RSS on one approved input. File/output strings are nonempty; find queries contain non-whitespace. Export extensions follow the existing case-insensitive encoder selection, including `.jpeg`. Search type remains a free-form, case-insensitive plugin type, not an invented enum. Successful outputs and CLI parameter order remain unchanged.

Tools reject unknown fields (`additionalProperties: false`), so a misspelled `nodeId` cannot fall back to `0:0`; workflow enumerates topics. **Breaking change:** MCP no longer translates arguments into CLI flags. Replace `depth: "0"` with `depth: 0`, `scale: "2x"` with `scale: 2`, and uncapped `maxSize: null`/`false` with `maxSize: 0` (omission means **1600**, not uncapped). Other optional null/non-boolean false values fail; boolean false is valid. Flag-like nodes are not CLI options. No legacy mode marker remains. Under `--root`, former legacy calls return `invalid_arguments`, not `permission_denied`. CLI is unchanged.

Tool failures retain `isError: true` and text content; additive `_meta.figlens` carries stable `code` and corrective `nextStep`. Success payload schemas do not change:

| Code | Meaning / next action |
|---|---|
| `invalid_arguments` | Missing/wrong/invalid tool parameters or unknown tool; consult `tools/list` |
| `source_unreadable` | Local or remote source cannot be read/decoded; check valid local copy and authorized source access |
| `node_not_found` | Resolve again with find/tree or inspect the containing instance; retain full IDs |
| `unsupported_node` | Target exists but that operation cannot use it; use supported inspection/export or a drawable node as directed |
| `parse_budget_exceeded` | A named finite parsing budget was crossed; use a smaller copy or report the limit |
| `output_write_failed` | Bundle packing or export writing failed; create an authorized writable directory and check space/permissions |
| `permission_denied` | Configured roots forbid the source/output; retain the restriction |
| `workspace_busy` | Another process holds the workspace, or a lane is full; retry once clear, never kill it |
| `storage_unavailable` | The workspace's storage is missing or offline; follow `_meta.figlens.recovery`: restore it, or with the user's confirmation recreate elsewhere |
| `index_incompatible` | Another schema or index version; follow the recovery (`rebuild`, or that figlens version) |
| `query_work_limit` | Over 10 sources or 100k records per search; run a listed narrower one |
| `source_stale` | Every searched source changed since indexing; run the listed `refresh`, then search again |
| `cursor_stale` / `cursor_expired` | A paged index was replaced or deleted; search again without the cursor |
| `cancelled` | The job was cancelled; the previous index still serves |
| `operation_failed` | Other existing operation error; inspect its diagnostic without assuming a more specific category |

MCP source failures hide raw paths/cloud stderr. Local CLI errors retain their diagnostics; URL-source errors are redacted in both channels. Error categories follow typed provenance in the existing `FigError`, not human-text matching. They are a recoverability aid, not a promise that every possible process failure is caught.

JSON parsing (`-32700`), invalid JSON-RPC envelopes (`-32600`), unknown methods (`-32601`) and a missing/non-string `tools/call` name (`-32602`) remain protocol errors. Business/argument failures inside a named tool call are tool results. Envelope-valid messages without `id` are ignored before preparation/I/O, even with unknown methods or invalid arguments. `id: null` gets a reply. Omitted `jsonrpc` remains accepted; prefer `"jsonrpc":"2.0"`.

Workspace calls (`index`, `jobs`, `search` with `workspaceId`) are not served on the document thread. The stdin reader only frames and parses; `jobs` and index `status` go to a control lane, document tools and workspace search to the interactive lane (which keeps the warm document), and build-submitting `index` actions to an index lane whose jobs run one at a time on a build lane. The reader answers protocol calls (`ping`, `tools/list`) at once, possibly before earlier tool replies; pair replies by id. Lanes queue 64, 64 and 8 calls, the build lane 8 submissions; past that the call gets `workspace_busy` at once. `notifications/cancelled` drops a still-queued call unanswered; a running call finishes, and its build stops via `jobs cancel`. Path checks and blocking I/O run in the executing lane, so status and cancel answer while a build or a slow search runs; closing stdin cancels running builds before exit. Storage roots (`--storage-root`) and `--root` visibility are in [workspace](workspace.md#mcp-authorization); jobs are in [index](index-jobs.md).

Admission stores checked paths even without roots; execution/cache use them. Reply labels and encoders survive symlinks; missing parents stay missing. Roots reject dangling links/escapes. Cache reuse rechecks policy, readability, mtime and length.

See the [GCS cookbook](gcs.md) for download versus parsed-tree caching and [bounded baseline](performance.md) for measurement limits. `scripts/mcp_smoke.py` shares its real subprocess session with GCS mock checks and measurement; no model, sockets or paid API are needed. The candidate installation matrix runs the same contract assertions through native and packaged wrapper stdio paths.
