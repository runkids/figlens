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

`src/workflow.rs` embeds `skills/figlens/SKILL.md` and its planning references with `include_str!`. Build and images return relevant level-two sections, including known gaps. Plan, planning-details and YOLO return their maintained reference. These are the same sources used by Pi and the portable skill; edit them rather than a separate MCP copy. Update section selectors if a selected heading changes. Cargo's source inclusion covers the embedded Markdown too.

Local reference links in returned text become `workflow:<topic>#<optional-section>`; the agent follows them with another call to this tool. No runtime Markdown files, network fetch or skill discovery mechanism is required. The binary can serve guidance from an empty working directory, including under `--root`; design/output path restrictions remain in force for design tools.

The client decides whether initialization instructions enter model context. The workflow tool's description also exposes the choices so retrieval is discoverable through `tools/list`. This does not prove a particular model will select or follow the right guide. MCP prompts/resources are not required for this model-selected tool path.

## Verification

Run Docker Rust tests and lint. MCP tests cover discovery/schema, restricted-root reads without a design, invalid topics/arguments, local-link resolution and preservation of planning/image/cleanup boundaries. Existing design-tool tests verify their behavior remains intact. Also run Pi/npm packaging checks after editing shared skill files, and `python3 scripts/ai-context.py check` after doc changes. A wire-protocol smoke test from an empty working directory checks the compiled binary independently of local skill installation. Model behavior and browser reconstruction quality require separate acceptance evidence.
