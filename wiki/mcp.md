# MCP workflow discovery

Use this topic for MCP instructions, tool discovery and built-in workflow guidance. Design decoding and result contracts are in `fig-format`; Pi-specific commands and tools are in `pi`.

## Intent and tool selection

`src/mcp.rs` serves stdio JSON-RPC; `src/main.rs` configures roots and starts it. Initialization returns a short intent router: resolve recognizable screens through `find`/`tree`, read compact values with `inspect`, full style/mask/component data with `nodes`, tokens with `variables`, and pixels/assets with `export`. `bundle` writes a scoped handoff, `verify` checks it. Ordinary questions and exports use those tools directly.

`inspect` and `bundle` accept optional `states: true` (CLI `--states`) for a bounded, source-scoped list of explicitly related component variants. The agent supplies this per-call argument when related states are requested; no MCP configuration change is needed. Inspect adds `states`; bundle adds `design/states.json`, a manifest pointer and a bounded context summary. Independent inventory truncation/coverage must be checked; supported prototype fields retain exact sources and unknown markers, without business inference. Both reuse their selected effective subtree and do not render alternate variants. See [related variants](states.md) for evidence, batch usage and limits.

For two selected variants, `inspect` accepts `compareVariant: "other-node-id"` (CLI `--compare-variant ID`) and adds `variantComparison`. Both IDs must be stored component definitions in the same set. It compares bounded raw root fields and descendant value multisets, retains exact sources/unknowns and leaves descendants unpaired. It performs no alternate expansion/rendering and changes no bundle or preview controls. See [variant comparison](states.md#compare-two-variants).

The `workflow` tool lets a model retrieve detailed guidance without a separate skill installation:

| `topic` | Use when |
|---|---|
| `build` | Implement a design with the project's components, tokens and architecture, then compare and correct the running UI |
| `images` | Prepare website assets using the project's image pipeline and verify dimensions, crops, bytes and delivery |
| `plan` | Plan an implementation or carefully reproduce a design; stop at the plan only when the request is plan-only |
| `planning-details` | Detail page/multi-page scope or a specific unresolved design/project dependency |
| `yolo` | The user explicitly requests autonomous implementation and cleanup of this run's temporary plan after successful checks |

The tool requires only an enumerated `topic`, accepts no file/path argument and rejects extra properties. Reading guidance neither reads a project nor writes, implements, verifies or cleans it. The current agent uses its host tools for project inspection/edits, directory creation, bundle extraction and browser checks. Host permissions still apply. A guide is not an implementation result or a plan-cleanup guarantee.

## Shared sources and runtime behavior

Stdio request lines are capped at 1 MiB including line endings before JSON parsing or admission. Oversized lines are drained through their newline and return JSON-RPC `-32600` with a null ID; invalid UTF-8 returns `-32700`. The next request can still run, including against a warm document. This is a byte limit, not an elapsed-time limit on a peer that never finishes a line.

Complete serialized replies, including escaping, structured content, inline base64 and newline, are capped at 8 MiB. The server counts serialization without allocating a second full reply buffer. Over-budget tool replies become `isError: true` with `_meta.figlens.code: response_budget_exceeded`; completed bundle/export files remain and their path is retained as `_meta.figlens.outputPath`. Full design JSON is never silently truncated. Use scoped inspect/bundles, CLI nodes, or a smaller export `maxSize`. Non-tool protocol replies return `-32603` on overflow. This transport policy leaves headroom below a reported 10 MiB client limit; it is not a guarantee for every MCP client or a process-memory limit. CLI output remains uncapped. See [measurements](handoff-research.md#mcp-message-sizes).

`src/workflow.rs` embeds `skills/figlens/SKILL.md` and its planning references with `include_str!`. Build includes the font/asset preflight: check required font faces and asset files, report missing/unverified dependencies, and verify actual browser loading before acceptance. `document.fonts.check` alone can pass on fallback. Bundle `context.md` also carries a pre-implementation warning: project font availability starts `UNVERIFIED`, even with no bundle warnings. Missing/unverified inputs do not block implementation: use documented project-font fallbacks or labelled image placeholders at the intended dimensions, retaining exact gaps and remedies in the handoff. Only final visual acceptance remains pending for unresolved inputs. This is agent guidance, not an automatic project-font scan or a proven visual-quality improvement; decoding, assets, reference pixels and output schemas are unchanged. Build and images return relevant level-two sections, including where outputs go and known gaps; `out` descriptions repeat the location rule. Plan, planning-details and YOLO return their maintained reference. Pi and the portable skill use these sources too; CLI `figlens workflow <topic>` prints them with a CLI preamble instead of the MCP one. Update section selectors if a selected heading changes. Cargo's source inclusion covers the embedded Markdown too.

Local reference links in returned text become `workflow:<topic>#<optional-section>`; the agent follows them with another call to this tool. No runtime Markdown files, network fetch or skill discovery mechanism is required. The binary can serve guidance from an empty working directory, including under `--root`; design/output path restrictions remain in force for design tools.

The client decides whether initialization instructions enter model context. The workflow tool's description also exposes the choices so retrieval is discoverable through `tools/list`. This does not prove a particular model will select or follow the right guide. MCP prompts/resources are not required for this model-selected tool path.

## Verification

Run Docker Rust tests/lint and `python3 scripts/ai-context.py check`. Tests cover schema/defaults, admission, notification side effects, cached-source changes and rooted/unrooted paths. After building, run `docker compose run --rm dev python3 -B scripts/check-mcp-admission.py --binary /work/target/debug/figlens` for actual POSIX warm-cache permission denial/recovery. Pi/npm tests and source inspection alone do not prove candidate runtime compatibility or model/browser behavior.

## Parameter contract and recoverable failures

`src/contract.rs` owns metadata, discovery, normalization and defaults. `prepare` returns a private-field, immutable `PreparedCall` required by `Command::from_tool`, once before admission/loading. Mathematical JSON integers (`2.0`) normalize; fractions, negative integers, overflow and nonfinite scales fail.

| Parameter | Range / default |
|---|---|
| `tree.depth` | 0 through `2^53-1` (JSON-safe); omitted means unlimited tree depth |
| `inspect.depth` | 0–10 / 1 |
| `find.limit` | 1–200 / 20 |
| `export.scale` | finite and greater than 0 / 1 |
| `export.quality` | 1–100 / 80 |
| `export.maxKb` | 0 through `2^53-1`; omitted means no byte budget |
| `export.maxSize` | 0 through `u32::MAX` / 1600; 0 disables the raster side cap |

`tree`/`find` root defaults to `0:0`; booleans default false. File/output strings are nonempty; find queries contain non-whitespace. Export extensions follow the existing case-insensitive encoder selection, including `.jpeg`. Search type remains a free-form, case-insensitive plugin type, not an invented enum. Successful outputs and CLI parameter order remain unchanged.

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
| `operation_failed` | Other existing operation error; inspect its diagnostic without assuming a more specific category |

MCP source failures hide raw paths/cloud stderr. Local CLI errors retain their diagnostics; URL-source errors are redacted in both channels. Error categories follow typed provenance in the existing `FigError`, not human-text matching. They are a recoverability aid, not a promise that every possible process failure is caught.

JSON parsing (`-32700`), invalid JSON-RPC envelopes (`-32600`), unknown methods (`-32601`) and a missing/non-string `tools/call` name (`-32602`) remain protocol errors. Business/argument failures inside a named tool call are tool results. Envelope-valid messages without `id` are ignored before preparation/I/O, even with unknown methods or invalid arguments. `id: null` gets a reply. Omitted `jsonrpc` remains accepted; prefer `"jsonrpc":"2.0"`.

Admission stores checked paths even without roots; execution/cache use them. Reply labels and encoders survive symlinks; missing parents stay missing. Roots reject dangling links/escapes. Cache reuse rechecks policy, readability, mtime and length.

See the [GCS cookbook](gcs.md) for download versus parsed-tree caching and [bounded baseline](performance.md) for measurement limits. `scripts/mcp_smoke.py` shares its real subprocess session with GCS mock checks and measurement; no model, sockets or paid API are needed. The candidate installation matrix runs the same contract assertions through native and packaged wrapper stdio paths.
