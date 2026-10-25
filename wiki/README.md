# figlens Wiki Router

The wiki holds procedures, reference and decisions, and no task needs all of it at once. Rules for every task are in the root `AGENTS.md`. The topic-to-section map is in `wiki/ai-context.json`. Load a topic with `python3 scripts/ai-context.py <topic>`.

## Task routing

| Topic | Use when | Main sources |
|---|---|---|
| `mcp` | Changing MCP instructions, tool discovery or built-in workflow guidance | `mcp.md` |
| `fig-format` | Changing decoding, instance expansion, emitted REST fields, variables or the bundle | `fig-format.md` |
| `render` | Changing SVG or raster output: text and glyphs, image fills, effects, export areas, image adjustments | `render.md` |
| `preview` | Changing the local viewer, offline HTML, handoff or screenshot comparison | `preview.md` |
| `pi` | Changing the Pi extension, reviewed component mappings or its npm packaging | `pi.md` |
| `accuracy` | Measuring output against Figma's own exports, running `compare`, finding test files | `accuracy.md` |
| `release` | Packaging, GitHub releases, Homebrew, npm, install scripts and `figlens update` | `release.md` |
| `review` | Reviewing changes, creating a PR and following up on AI review | `review.md` |
| `direction` | Deciding what to build next, or revisiting a past decision | `direction.md` |
| `ai-context` | Maintaining this router: add, move or split docs, fix `check` | `ai-context.md` |

`wiki/ai-context.json` is the only source of truth. This table is the human entry point. After changing topics, run `python3 scripts/ai-context.py check`.

## History

- [Reviewed component mappings in Pi](history/pi-workflow.md): implementation and verification scope for the preview/Pi release.
