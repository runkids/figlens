# Host integrations

Load this for OpenCode V2, OMP, cross-host ownership, the shared npm package or installed-adapter acceptance. Pi-specific UI and mapping rules remain in [pi.md](pi.md); native MCP contracts remain in [mcp.md](mcp.md).

## Package and shared behavior

`npm/package.json` is the only public package, `@runkids/figlens`. Its CLI root retains CommonJS semantics. Pi and OMP manifests point to independent `pi/index.ts` and `omp/index.ts`; OpenCode exports point to `opencode/index.mjs`, `./tui` and `./rpc`. Each host imports only its own SDK. Optional peers describe host dependencies without raising the CLI's Node 18 floor. Use actual installed loaders to verify resolution; a manifest or rewritten Pi import is insufficient.

`pi/workflow.mjs` owns mapping state, validation, evidence freshness, locking and atomic publication. `pi/viewer.mjs` owns viewer startup/readiness and process cleanup. `pi/requests.mjs` reads the one shared skill/reference tree and builds open/preview/plan/YOLO/map requests. Adapters retain native schemas, UI, permissions and dispatch; no separate host framework or mapping format exists.

`prepare-npm.cjs` stages maintained `pi/`, `omp/`, `opencode/`, the shared skill and `omp/mcp.json` as root `.mcp.json`. Generated copies are ignored. Exact package inventory is checked in `check-installation.py`; tests, probes, private files and generated `.fig` fixtures are not shipped. Native archives and npm resources take guidance from the same sources. Keep version, launcher/cache/checksum handling and release pipeline shared.

## OpenCode V2

Current local runtime evidence uses OpenCode 2.2.0. The server registers MCP/skill transforms, three model tools and the argument-taking `figlens` command. Native slash submission preserves files, agent/skill mentions and delivery; the server signals TUI review/frame dialogs through a typed RPC event. Human operations use `figlens-operation` and `figlens-review` RPC definitions. Noninteractive review leaves proposals pending. No model tool approves mappings.

Every operation checks that the host session's canonical directory matches the plugin location. Shared Workflow authorization checks dynamic source/code reads and writes; launcher execution, export paths, preview cache and owned temporary workspaces receive read/edit/bash authorization before side effects. Permission failure and cancellation remain errors. Review RPC keeps a 64 KiB request budget, complete pending evidence within a 4 MiB reply budget and the writer's field bounds. Authenticated clients are trusted; RPC context is not proof of human identity.

A session owns its Viewer and abort controller. Session deletion closes only that owner; project mismatch rejects operations and closes the old owner. Plugin unload/reload aborts operations, closes all owned viewers, stops the event subscription and disposes registrations. Durable sessions outlive a TUI disconnect; disconnect is not permission to delete a session. Use `close` or delete the session to stop its viewer. Preview replaces the old viewer only after catalog/node readiness. URLs belong to the server machine; no tunnel is created.

The MCP transform installs a default only when no `figlens` configuration exists. Existing local/remote/disabled settings remain unchanged. A managed default is ready only after connected status and a matching same-package launcher version. It shares the local project directory and snapshot filesystem. Existing overrides whose handshake version/snapshot are unverified block dependent open/description-preview/map/plan/YOLO shortcuts; explicit local preview, manual selection, status, review and local tools remain available. Do not launch another MCP client or overwrite settings to hide an unverified state.

Manual selection exports a real PNG and TUI sends its bytes through the host file-attachment format with `resume: false`. Viewer tools return actual loopback URLs. File-content transport is distinct from image rendering or model consumption; no model run is implied by dispatch or attachment creation.

Before a reference export, reuse `Workflow.read()` to reject symbolic `.figlens`/guide paths, then reject a symbolic `.figlens/previews` directory before creating or exporting anything. Lexical project-path authorization does not authorize a symlink's external target; the native writer canonicalizes output parents. Keep the external-directory-denial regression for both parent and preview links.

## OMP

Current production-entry evidence uses OMP 18.13.2 with Bun 1.4.3; earlier 18.12.3 results concern disposable probes. The entry uses OMP's injected zod schemas, own SDK/TUI, native read/write/exec tool approval tiers, same shared writer and native review dialogs. Commands are trusted extension code, as in Pi. OMP does not supply OpenCode's dynamic per-path permission assertion; tool approval tiers are not a filesystem sandbox.

Each session has a Viewer/lifetime. Switch and shutdown abort/close those resources. Each tool call creates a Workflow authorization closure containing its own cancellation signal; caching a closure with only the session signal loses cancellation on cached reads and writes. Keep the native-wrapper cancellation regression in installed acceptance.

The host discovers the packaged root `.mcp.json`, substitutes `OMP_PLUGIN_ROOT`, and uses `omp/mcp.cjs` to forward stdio/argv/exit/signal to `FIGLENS_BIN` or the adjacent CLI wrapper. Descriptor cwd is the package, so MCP arguments must use absolute session/project paths. Native user MCP configuration takes priority over package discovery; disabled/remote overrides are never connected by the test harness.

The public API exposes tool metadata, not the effective server configuration, handshake version or live client. Its `invokeTool` delegates only to the same native built-in. Under the current strict verification policy, MCP-dependent shortcuts report unverified state instead of dispatching. This also affects the default descriptor because the entry cannot prove whether an override replaced it. Local preview, frame selection, status, bindings, proposals, native review and the shared skill still work. Ordinary conversations may use the host-managed MCP. This is a documented partial integration, not full Pi parity.

## Acceptance

`npm test` checks shared state, cancellation, native review logic, ownership, override preservation and exact maintained staging. `scripts/check-installation.py` installs an extracted candidate tarball in a fresh consumer, verifies cold/warm/parallel launcher behavior and native/wrapper MCP, then optionally runs host profiles. Existing CLI Node 18 and Pi 1.3.0/1.4.0 lanes remain.

- OpenCode: `--opencode-bin <binary> --opencode-sdk <@opencode/client-root>` runs `check-opencode-integration.mjs`, including real server/TUI, managed MCP, PNG bytes, permissions, mapping round-trip, two viewer owners, session deletion and reload.
- OMP: `--omp-sdk <@oh-my-pi/pi-coding-agent-root> --bun <binary>` runs `check-omp-integration.mjs`: real loader/approval wrapper, descriptor/bridge MCP, override priority, cancellation, native TUI review and viewer switch cleanup.
- Both profiles use `FIGLENS_BIN` pointing to the verified candidate binary. The same candidate's default wrapper cold path is checked separately by CLI/Pi tests; do not label a native override run as a cold adapter download.

The mapping case is original synthetic input generated at test time, not a Figma export or accuracy oracle:

```sh
mkdir -p samples/plugin-research/fixtures
docker compose run --rm dev cargo run --locked --example mapping-fixture -- samples/plugin-research/fixtures/mapping.fig
```

The generator refuses an existing output. Public `examples/first-success/design.fig` remains the only distributable design fixture. Writer mocks, native synthetic parsing, real installed host/UI, image transport, actual model results and platform results are separate evidence. No-model acceptance never calls providers or reads user credentials/global plugins. Model plan/YOLO acceptance requires separate usage authorization; keep it untested until authorized. Local macOS acceptance does not prove Linux/Windows native TUI support or future CI success.
