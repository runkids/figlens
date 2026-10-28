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

OMP discovers the packaged `.mcp.json`; native user configuration has priority. MCP runs from the package directory, so design paths sent to MCP must be absolute paths on the agent's machine. `FIGLENS_BIN` overrides both the extension launcher and descriptor bridge; otherwise both use the adjacent npm launcher and its existing cache/download logic.

**MCP shortcut limit:** the public extension API cannot verify the effective MCP server version or a remote snapshot. `/figlens open` without an explicit frame, descriptive `preview`, `plan`, `yolo` and `map` therefore report the unverified state and do not dispatch. The shared skill and host-managed MCP remain available in ordinary conversation. This is partial integration, not full Pi workflow parity. No second MCP client or user configuration rewrite is used.

Each session owns its viewer. Session switch and shutdown abort owned operations and close those viewers. The viewer is a loopback service on the agent machine; remote terminals need access to that machine. Approved mappings remain in the existing `.figlens/component-mappings.md` format and can be read by Pi.
