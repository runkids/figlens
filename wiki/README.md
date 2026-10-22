# figlens Wiki Router

The wiki holds procedures, reference and decisions, and no task needs all of it at once. Rules for every task are in the root `AGENTS.md`. The topic-to-section map is in `wiki/ai-context.json`. Load a topic with `python3 scripts/ai-context.py <topic>`.

## Task routing

| Topic | Use when | Main sources |
|---|---|---|
| `fig-format` | Changing decoding, instance expansion, emitted REST fields, variables, rendering or the bundle | `fig-format.md` |
| `preview` | Changing the local viewer, offline HTML, handoff or screenshot comparison | `preview.md` |
| `accuracy` | Measuring output against Figma's own exports, running `compare`, finding test files | `accuracy.md` |
| `release` | Packaging, GitHub releases, Homebrew, npm, install scripts and `figlens update` | `release.md` |
| `direction` | Deciding what to build next, or revisiting a past decision | `direction.md` |
| `ai-context` | Maintaining this router: add, move or split docs, fix `check` | `ai-context.md` |

`wiki/ai-context.json` is the only source of truth. This table is the human entry point. After changing topics, run `python3 scripts/ai-context.py check`.

## History (milestone logs; read on demand)

| File | Contents |
|---|---|
| [Shared command execution and rendering preparation](history/command-render-preparation.md) | CLI/MCP command results, shared raster preparation and behavior-preservation evidence |
| [Agent discovery and inline images](history/agent-discovery.md) | Search, bounded layout summaries and MCP image content; verification on two local designs |
| [Read-only preview and agent handoff](history/preview-viewer.md) | Local browsing, selectable offline HTML, real bundles, visual comparison and save-refresh verification |
