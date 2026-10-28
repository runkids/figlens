# OpenCode V2 integration

The single `@runkids/figlens` npm package exposes an OpenCode server entry, `./tui` and `./rpc`, alongside its CLI, Pi and OMP entries. The root stays CommonJS for the CLI; `opencode/` has its own module boundary. Local acceptance targets OpenCode 2.2.0. Other OpenCode generations are not covered.

Install the package in OpenCode V2:

```sh
opencode plugin add @runkids/figlens
```

For a local candidate, use the unpacked package directory as a `file://` URL. The package export lets the host load server and TUI entries separately. No other host SDK is imported by these entries.

Use `/figlens help`, or ask OpenCode to build a screen from a saved `.fig`. The shortcuts are `open`, `preview`, `plan`, `yolo`, `map`, `frame`, `status`, `review` and `close`. Plan authorizes a plan only; yolo authorizes implementation within the requested scope. Dispatch is not proof of model completion. Native TUI review shows evidence, edits all four target fields, resolves conflicts and confirms approval; noninteractive clients leave proposals pending. There is no model approval tool.

The server registers the shared skill and supplies a local `figlens` MCP configuration only when no same-name configuration exists. It preserves disabled, remote and local user overrides. Overrides whose version/snapshot cannot be verified block MCP-dependent shortcuts, while explicit local preview, frame selection, mappings and review remain available. The managed default uses the same launcher and project directory as local operations; `FIGLENS_BIN` selects both paths. OpenCode is a Bun-built binary, so the bundled Node launcher runs with the `node` on PATH (Node 18 or later); `FIGLENS_BIN` bypasses it.

Server operations validate session/project ownership and request read/edit/bash permissions before side effects. Authenticated host clients are trusted: RPC does not prove that a human made the call. UI confirmation is a mapping decision, not a sandbox against other authenticated clients. Read/write errors, cancellation, stale evidence and writer contention remain errors.

Viewer URLs and design paths belong to the server machine. Reference attachments carry PNG bytes through the host file-content format; a textual path is not an image. Each session owns its viewer; deleting a session, reloading or unloading the plugin closes owned viewers. Client disconnect alone does not delete a durable session, so use `/figlens close` when finished. The adapter creates no tunnel.

Shared mapping files retain Pi's schema and human notes. See [Pi](../pi/README.md) for the underlying workflow and [integration acceptance](https://github.com/runkids/figlens/blob/main/wiki/integrations.md) for source-checkout checks.
