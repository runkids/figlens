# OMP integration

The `@runkids/figlens` package includes a native OMP extension, shared skill, MCP descriptor and CLI launcher. It imports OMP's SDK, not Pi's compatibility layer. Current local acceptance uses OMP 18.13.2 and Bun 1.4.3; no model run is included.

```sh
omp plugin install @runkids/figlens
```

For a local candidate, install its unpacked package directory or start OMP with `-e /absolute/path/to/package`. Reload the extension, then use `/figlens help`.

Local controls:

- `/figlens preview ./design.fig` starts a viewer without a model.
- `/figlens open ./design.fig --frame 0:1` saves a selection and shows a reference image.
- `/figlens frame` searches the saved file; `/figlens frame 0:1` selects directly.
- `/figlens status`, `/figlens review` and `/figlens close` read state, review proposals or close this session's viewer.

The `figlens_bindings`, `figlens_propose_bindings` and `figlens_preview` tools use OMP's native read/write/exec approval tiers. Direct commands are trusted extension code, as in Pi; these tiers are not a per-path filesystem sandbox. Review shows evidence, permits four-field edits and conflict resolution, and requires final confirmation. No model tool approves mappings. Noninteractive review leaves proposals pending.

**Searching a design.** Ask OMP to find copy or a screen in a `.fig`: it searches the file directly and stores nothing. For repeated searches, ask it to *enable an index* for that design and a scope (for example "enable an index for ./design.fig, whole file"): the agent uses the host MCP `index` `prepare` with the file and scope (no setup command), polls `jobs` until the build succeeds, and from then on searches the published index by `workspaceId` and `node` set to the chosen scope and opens results with `inspect`, also after a restart or from another host. OMP approves every MCP tool at its `write` tier and shows the call's arguments, including the `index` action, but has no per-action rule: in `always-ask` modes a reset prompts like a prepare, a `tools.approval` entry for the figlens `index` MCP tool can deny or always prompt it, and OMP's default `yolo` approval mode runs it unprompted. The shared skill tells the agent to reset only on the user's explicit request. Indexes go to the figlens state directory, or to a location an earlier `--storage` prepare remembered; the packaged descriptor starts `figlens mcp` without `--storage-root`, so a new custom `storage` is refused. To allow one, define your own `figlens` server in OMP's MCP configuration with the arguments `mcp --storage-root /absolute/dir` and reload; native configuration has priority over the packaged descriptor.

**Finding usages.** Ask OMP where a component, variable, style or image is used: the agent calls the MCP `usages` tool with the exact layer or resource id, on the `.fig` directly (stateless) or, once an index is enabled, by `workspaceId`, the result's `sourceId` and the prepared scope; it never prepares, widens or resets an index for it. Results carry exact refs, a `snapshotId` and coverage; a hidden-layer exclusion or a source that changed is reported, not read as "unused". OMP approves it at the same `write` tier as every MCP tool.

**Following changes.** Ask OMP to follow a design's changes, which opts in to that scope's index and history: the agent prepares the scope when no index covers it, then calls `index` `track` with the workspace and the scope, which builds a baseline as an index rebuild does, then reads `changes` by `workspaceId` with a consumer name and acknowledges each page it handled with `watch` `ack`; `untrack` stops following and keeps the history. A `tracking` entry with `refreshRequired` means the file changed since its last build and `index refresh` records the change; an empty page never proves the file unchanged. OMP approves these at the same `write` tier too.


**Comparing revisions.** Ask OMP what changed between two saved copies of a design: the agent calls the MCP `diff` tool with `old` and `file` (local paths) and pages with `nextCursor`. Layers match by exact id and type only; `unknown` means a side stopped or left that layer unexpanded, not that it was removed. It reads both files and writes nothing. OMP approves it at the same `write` tier as every MCP tool.

OMP discovers the packaged `.mcp.json`; native user configuration has priority. MCP runs from the package directory, so design paths sent to MCP must be absolute paths on the agent's machine. `FIGLENS_BIN` overrides both the extension launcher and descriptor bridge; otherwise both use the adjacent npm launcher and its existing cache/download logic.

**MCP shortcut limit:** the public extension API cannot verify the effective MCP server version or a remote snapshot. `/figlens open` without an explicit frame, descriptive `preview`, `plan`, `yolo` and `map` therefore report the unverified state and do not dispatch. The shared skill and host-managed MCP remain available in ordinary conversation. This is partial integration, not full Pi workflow parity. No second MCP client or user configuration rewrite is used.

Each session owns its viewer. Session switch and shutdown abort owned operations and close those viewers. The viewer is a loopback service on the agent machine; remote terminals need access to that machine. Approved mappings remain in the existing `.figlens/component-mappings.md` format and can be read by Pi.
