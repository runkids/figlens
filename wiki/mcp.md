# MCP workflow discovery

Use this topic for MCP instructions, tool discovery and built-in workflow guidance. Design decoding and result contracts are in `fig-format`; Pi-specific commands and tools are in `pi`.

## Intent and tool selection

`src/main.rs` serves stdio JSON-RPC. Initialization returns a short intent router: resolve recognizable screens through `find`/`tree`, read compact values with `inspect`, full style/mask/component data with `nodes`, tokens with `variables`, and pixels/assets with `export`. `bundle` writes a scoped design handoff. Ordinary questions and exports use those tools directly.

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

`src/workflow.rs` embeds `skills/figlens/SKILL.md` and its planning references with `include_str!`. Build and images return relevant level-two sections, including known gaps. Plan, planning-details and YOLO return their maintained reference. Pi and the portable skill use these sources too. Update section selectors if a selected heading changes. Cargo's source inclusion covers the embedded Markdown too.

Local reference links in returned text become `workflow:<topic>#<optional-section>`; the agent follows them with another call to this tool. No runtime Markdown files, network fetch or skill discovery mechanism is required. The binary can serve guidance from an empty working directory, including under `--root`; design/output path restrictions remain in force for design tools.

The client decides whether initialization instructions enter model context. The workflow tool's description also exposes the choices so retrieval is discoverable through `tools/list`. This does not prove a particular model will select or follow the right guide. MCP prompts/resources are not required for this model-selected tool path.

## Verification

Run Docker Rust tests and lint. MCP tests cover discovery/schema, restricted-root reads without a design, invalid topics/arguments, local-link resolution and preservation of planning/image/cleanup boundaries. Existing design-tool tests verify their behavior remains intact. Also run Pi/npm packaging checks after editing shared skill files, and `python3 scripts/ai-context.py check` after doc changes. A wire-protocol smoke test from an empty working directory checks the compiled binary independently of local skill installation. Model behavior and browser reconstruction quality require separate acceptance evidence.

## Parameter contract and recoverable failures

`src/main.rs::TOOLS` declares shared parameter names/types/requiredness; `src/contract.rs::property` supplies ranges/defaults for discovery and runtime preflight. Validated arguments are checked before source loading, including the persistent server's preload path. JSON mathematical integers such as `2.0` normalize to integers; fractions, negative integers, overflow and nonfinite scales fail preflight.

| Parameter | Range / default |
|---|---|
| `tree.depth` | 0 through platform `usize::MAX`; omitted means unlimited tree depth |
| `inspect.depth` | 0–10 / 1 |
| `find.limit` | 1–200 / 20 |
| `export.scale` | finite and greater than 0 / 1 |
| `export.quality` | 1–100 / 80 |
| `export.maxKb` | 0 through `usize::MAX / 1024`; omitted means no byte budget |
| `export.maxSize` | 0 through `u32::MAX` / 1600; 0 disables the raster side cap |

`tree`/`find` root defaults to `0:0`; booleans default false. File/output strings are nonempty; find queries contain non-whitespace. Export extensions follow the existing case-insensitive encoder selection, including `.jpeg`. Search type remains a free-form, case-insensitive plugin type, not an invented enum. Successful outputs and CLI parameter order remain unchanged.

Design tools ignore unknown fields (`additionalProperties: true`); workflow rejects extras and enumerates topics. The unrestricted **legacy CLI translation path** preserves numeric strings, scales such as `2x`, optional null/false skipping and flag-as-node shortcuts, marked `_meta.figlens.argumentMode: "legacy"`. For export `maxSize`, null/false means **no side cap**; an absent field defaults to **1600**. Use schema types for new clients. Required null and other wrong types fail. `--root` disallows legacy translation and retains path restrictions. Invalid numeric legacy values fail before source loading.

Tool failures retain `isError: true` and text content; additive `_meta.figlens` carries stable `code` and corrective `nextStep`. Success payload schemas do not change:

| Code | Meaning / next action |
|---|---|
| `invalid_arguments` | Missing/wrong/invalid tool parameters or unknown tool; consult `tools/list` |
| `source_unreadable` | Local or remote source cannot be read/decoded; check valid local copy and authorized source access |
| `node_not_found` | Resolve again with find/tree or inspect the containing instance; retain full IDs |
| `unsupported_node` | Target exists but that operation cannot use it; use supported inspection/export or a drawable node as directed |
| `parse_budget_exceeded` | A named finite parsing budget was crossed; use a smaller copy or report the limit |
| `output_write_failed` | Bundle packing or export writing failed; create an authorized writable directory and check space/permissions |
| `permission_denied` | Configured roots forbid the source/output or legacy path; retain the restriction |
| `operation_failed` | Other existing operation error; inspect its diagnostic without assuming a more specific category |

MCP source failures hide raw paths/cloud stderr. Local CLI errors retain their diagnostics; URL-source errors are redacted in both channels. Error categories follow typed provenance in the existing `FigError`, not human-text matching. They are a recoverability aid, not a promise that every possible process failure is caught.

JSON parsing (`-32700`), invalid JSON-RPC envelopes (`-32600`), unknown methods (`-32601`) and a missing/non-string `tools/call` name (`-32602`) remain protocol errors. Business/argument failures inside a named tool call are tool results. Valid notifications get no reply. For existing clients, envelopes omitting `jsonrpc` remain accepted; new clients should send `"jsonrpc":"2.0"`.

See the [GCS cookbook](gcs.md) for download versus parsed-tree caching and [bounded baseline](performance.md) for measurement limits. `scripts/mcp-smoke.py` shares its real subprocess session with GCS mock checks and measurement; no model, sockets or paid API are needed. The candidate installation matrix runs the same contract assertions through native and packaged wrapper stdio paths.
