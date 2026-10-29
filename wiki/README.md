# figlens Wiki Router

The wiki holds procedures, reference and decisions, and no task needs all of it at once. Rules for every task are in the root `AGENTS.md`. The topic-to-section map is in `wiki/ai-context.json`. Load a topic with `python3 scripts/ai-context.py <topic>`.

## Task routing

| Topic | Use when | Main sources |
|---|---|---|
| `mcp` | Changing MCP instructions, tool discovery or built-in workflow guidance | `mcp.md` |
| `gcs` | Changing the `gs://` snapshot handoff, its download cache or refresh | `gcs.md` |
| `performance` | Measuring load time and memory, reading the perf baseline, changing cache or decode cost | `performance.md` |
| `parsing` | Changing untrusted container/schema/value decoding or parsing budgets | `parsing.md` |
| `fig-format` | Changing decoding, instance expansion, emitted REST fields or variables | `fig-format.md` |
| `discovery` | Changing find or inspect summaries (fields, limits, truncation, `--flat` lines), `tree --top-frames` screen lists or `export --match-dir` icon matching | `discovery.md` |
| `search` | Changing search indexing, normalization, ranking, cursors, coverage or its budgets, or the diff engine | `search.md`, `usages.md`, `diff.md` |
| `workspace` | Changing local workspace roots, identity, ownership, the SQLite registry, locks or index actions | `workspace.md` |
| `index` | Changing durable index generations, index jobs (refresh, rebuild, clear_cache, cancel), publication fences or workspace search | `index-jobs.md` |
| `bundle` | Changing the bundle zip, manifest, context.md, batch bundles, font inventory or `verify` | `bundle.md` |
| `render` | Changing SVG or raster output: text and glyphs, image fills, effects, export areas, image adjustments | `render.md` |
| `preview` | Changing the local viewer, offline HTML, handoff or screenshot comparison | `preview.md` |
| `pi` | Changing the Pi extension, reviewed component mappings or its npm packaging | `pi.md` |
| `integrations` | Changing OpenCode V2/OMP, shared package, permissions or installed-host acceptance | `integrations.md` |
| `accuracy` | Measuring output against Figma's own exports, running `compare`, finding test files, the CI regression corpus | `accuracy.md` |
| `release` | Packaging, installation acceptance, GitHub releases, Homebrew and npm | `release.md`, `installation.md` |
| `review` | Delivering features/optimizations across CLI, MCP and Pi; reviews, PR follow-up and opening issues | `review.md` |
| `direction` | Deciding what to build next, or revisiting a past decision | `direction.md`, `handoff-research.md` |
| `ai-context` | Maintaining this router: add, move or split docs, fix `check` | `ai-context.md` |

`wiki/ai-context.json` is the only source of truth. This table is the human entry point. After changing topics, run `python3 scripts/ai-context.py check`.

## Field reference

- [Partial semantic fields](semantics.md): emitted mappings, unknowns, provenance and validation limits; read when working on these fields.
- [Related component variants](states.md): opt-in inspect inventory, source IDs, evidence gaps and bounds.
- [Prototype field evidence](prototype.md): exact event/action/destination sources, instance limits and unknowns.

## History

- [Reviewed component mappings in Pi](history/pi-workflow.md): implementation and verification scope for the preview/Pi release.
