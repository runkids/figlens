# figlens — User guide

[← README](../README.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [Standalone installation](#install)
- [Why figlens instead of the official Figma MCP?](#why)
- [What the agent gets](#bundle)
- [Pi extension](#pi)
- [Browse a design without Figma](#viewer)
- [MCP server and agent skill](#mcp-and-skill)
- [CLI reference](#commands)

<a id="install"></a>

## Standalone installation

**No Node.js?** Install the standalone binary and use `"command": "figlens", "args": ["mcp"]` instead:

| Platform | Command |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

<a id="why"></a>

## Why figlens instead of the official Figma MCP?

Use **figlens** when the design should travel as a file: remote agents, CI, offline work, handoff between agents, unlimited repeated reads. Use the **official Figma MCP** when the agent must see the live design without an export step.

| | figlens | Official Figma MCP |
|---|---|---|
| Source | Exported `.fig` snapshot | Live Figma files |
| Needs | The file | Figma sign-in; desktop server needs the app |
| Rate limits | None, local parsing | Figma service limits |
| Remote agent / CI | Copy the file, run the binary | Remote server with sign-in |
| Design changes | Export a new `.fig` | Automatic |
| Handoff | Share the `.fig` or a frame bundle | Share Figma access |
| Fidelity | Independent parser with [known gaps](../README.md#limitations) | Maintained by Figma |

Reading a `.fig` is offline. Installing and updating figlens, `gs://` inputs, your model provider and your project's dependencies still need a network.

<a id="bundle"></a>

## What the agent gets

One frame, one zip. Structure to build from, assets to use, a picture to check against.

```text
frame.zip
├── context.md                Start here: instructions for your agent
├── design/
│   ├── nodes.json            Layout, text, styles and expanded instances
│   ├── fonts.json            Font faces the visible text uses, with node ids
│   └── variables.json        Bound variables, collections, modes and aliases
├── assets/                   Image fills, SVGs and composited node renders
├── assets.json               Maps design references to asset files
├── references/screen.png     A render of the frame to compare against
├── missing-assets.json       Anything that could not be produced
└── manifest.json             File roles, media types, sizes and SHA-256 hashes
```

Need one piece? The CLI does it directly:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # frame ids by name or text
npx -y @runkids/figlens inspect design.fig 12:345                 # compact layout and text summary
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST-shaped JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP or AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # the zip above
```

SVG exports embed original image fills and outline text as glyphs, so no fonts are needed. Through MCP, raster exports also come back as inline images, so the agent sees the design without opening a file.

<a id="accuracy"></a>

### How close is it to Figma?

Measured against Figma's own JSON, PNG and SVG exports on **826 production frames** and five Community files (2026-10-22):

| Check | Result |
|---|---|
| Node structure, names, visibility, bounds and text | Identical in 771 / 826 frames |
| Original image fills | 2,608 / 2,608 byte-identical |
| SVG assets | 5,014 / 5,197 within the pixel-difference threshold |
| Community frame bundles | 306 / 306 with no missing assets |

Results on those files, not a guarantee for every design. Method and thresholds: [accuracy report](../wiki/accuracy.md).

<a id="pi"></a>

## Pi extension

```sh
pi install npm:@runkids/figlens
```

Installation guides: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

Restart Pi or `/reload`, then ask for a screen in plain words. Pi finds the frame, reads it over the auto-registered MCP server and builds with your stack. No node IDs, no global CLI.

Asking to find copy or a screen in a design searches the `.fig` directly and stores nothing. For repeated searches, ask Pi to enable an index for a design and scope: it prepares a local index once (no setup command), reports the job until it is ready and reuses it across restarts and hosts. Resetting that index needs your explicit request. Details: [local index](#local-index).

- `/figlens plan <request>` writes a plan only; it does not edit the application.
- `/figlens yolo <request>` plans, builds, checks and cleans up its own temporary plan.
- `/figlens preview [description]` opens the local viewer.
- `/figlens review` approves reusable component mappings the agent proposed.

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](../assets/pi-workflow.svg)

Full walkthrough, commands and limits: [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Browse a design without Figma

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # local viewer, Ctrl+C to stop
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # offline HTML snapshot to send to anyone
```

Browse pages and layers, search, measure, inspect layout, typography, variables and CSS, export any layer, copy an implementation prompt or download a bundle for the selection. Drop in a screenshot of your build to overlay it on the design. Details: [preview guide](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## MCP server and agent skill

**MCP.** The server runs locally over stdio and exposes `workflow`, `tree`, `find`, `search`, `usages`, `diff`, `inspect`, `nodes`, `variables`, `bundle`, `verify`, `index`, `jobs` and `export`. The agent can read built-in `build`, `audit`, `images`, `plan`, `planning-details` or opt-in `yolo` guidance through `workflow`, then use its own project and browser tools to carry out the authorized task. These guides share the portable skill's source; installing that skill separately is optional for MCP users. Client support and model behavior determine how the guidance is used. Pass absolute paths. Add `--root /path/to/designs` (repeatable) to confine every file and output path to chosen folders. Index storage outside the default figlens directories needs `--storage-root DIR` (repeatable). Raster exports are scaled down to `maxSize` (default 1600 px) so inline images stay small; SVG returns metadata only.

Native Windows with Claude Code: follow its [stdio setup notes](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) and the `cmd /c` wrapper for `npx`.

**Skill.** The [figlens skill](../skills/figlens/SKILL.md) teaches an agent to find frames, read the bundle in small pieces, plan scoped work and verify the running UI against the design. It installs instructions only; the CLI or MCP server must be available. Without it, `figlens workflow <topic>` prints the same guidance from the binary, matching its version. Install with the [Skills CLI](https://github.com/vercel-labs/skills) or copy `skills/figlens/` into your agent's skills directory (Claude Code: `~/.claude/skills/`):

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** Manage the skill and the MCP connection across Claude Code, Codex and Cursor from one place with [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**Install notes.** The npm launcher downloads the matching native binary on first use, verifies it against the release's SHA-256 checksums and caches it. Script installs update with `figlens update`; Homebrew installs with `brew upgrade figlens`. Binaries cover macOS (Apple silicon, Intel), Linux (arm64, x86_64, static musl) and Windows (x86_64).

<a id="local-index"></a>

## Ordinary search and the optional local index

|  | Ordinary (default) | Advanced: local index (optional) |
|---|---|---|
| What it does | Reads the `.fig` on each call and answers `search`, `find`, `inspect` and `export`. | Builds a search index of a chosen scope once, then answers `search` from it, ranked across sources. |
| What is written | No workspace or index database (`export` writes only the files you ask for). | A local workspace (SQLite database) in the per-user state directory, or the `--storage` you chose. |
| When to use | One-off questions, scripts, CI. | Repeated searches of one file; several files of a project as one corpus (`--workspace <dir>`); exact usages of components, variables, styles and images. |

A normal design question never creates an index. You opt in once, explicitly ("create an index for this design"); figlens picks the storage and needs no SQLite or other setup.

```sh
# Ordinary: stateless, creates nothing
figlens search design.fig "checkout button"

# Advanced, once: pick a scope (a node id, or 0:0 for the whole file); the build runs in the foreground
figlens index prepare design.fig --scope 0:0

# Indexed search: pass the scope you prepared as --node
figlens search --file design.fig "checkout button" --node 0:0

# After you edit or re-export the .fig: rebuild only the scopes whose bytes changed
figlens index refresh --file design.fig

# After you move the .fig: keep the workspace, then refresh (use the workspaceId printed by prepare)
figlens index relocate <sourceId> new/design.fig --id <workspaceId>
figlens index refresh --id <workspaceId>
```

MCP: `index` prepare returns a job; poll `jobs` until `succeeded`, then search with the `workspaceId` from the reply and `node` set to the prepared scope (without `node`, search reads `0:0`).

```json
{"tool": "index", "arguments": {"action": "prepare", "file": "/abs/path/design.fig", "scope": "0:0"}}
{"tool": "jobs", "arguments": {"action": "status", "jobId": "<jobId from the reply>"}}
{"tool": "search", "arguments": {"workspaceId": "<workspaceId from the reply>", "node": "0:0", "query": "checkout button"}}
```

Lifecycle: `refresh` rebuilds only scopes whose `.fig` bytes changed; `rebuild` rebuilds even when unchanged; `clear_cache` deletes only data a build can recreate and keeps the registry and published indexes; `reset` deletes this workspace's database (never the `.fig` or exports) and runs only on your explicit request. New sources, a wider scope, reset and recreate each need their own explicit request.

Moved the `.fig`? `index relocate` keeps `workspaceId`, `sourceId` and storage; then `refresh` the affected scopes. There is no auto-discovery and no hash-based merge, and no reset is needed.

Data: the SQLite database holds registered local paths, the selected scopes' layer names and effective text, search postings, exact relations (containment, instances, variants, variable/style/image bindings, prototype targets) and the definitions they point at, hashes, generations and jobs. The original `.fig` stays unchanged where it is; its raw bytes are not stored in the database.

Pi, OpenCode and OMP use the same index through their own opt-in entry, and the workspace is reused across restarts and hosts. Not implemented: file watching, Pi, OpenCode and OMP usages tools, vectors, events and subscriptions.

<a id="commands"></a>

## CLI reference

| Command | Output |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Node ids, REST types (as in `nodes`) and names; indented text, or nested JSON with each node's size, hidden flag and auto-layout mode, plus `firstText` (first visible text) on nodes of a page or section |
| `tree <file> [node] --top-frames [--json]` | Only the screens, one per line, ready for `--frames -`: frames, components, component sets and instances on a page or in its sections (groups and internal pages skipped); `--json` adds page and section ids and each screen's first visible text (`firstText`), which tells canvas labels from screens |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Case-insensitive search over layer names and text; paths, sizes and previews. Default 20 results (1-200) |
| `search <file> <query> [--node ID] [--type T[,T]] [--limit N] [--cursor C] [--include-hidden] [--include-internal] [--mode text]` | Ranked search of layer names and full text with instance overrides, text/boolean properties and swaps applied; exact node refs (including `I…;…` sublayer ids for `inspect`), match reasons, UTF-16 spans, coverage and cursor pages. Default 20 (1-200); hidden layers and internal pages need their flag. With `--file`/`--workspace`/`--id` instead of `<file>` it reads the workspace's prepared index (see `index`) |
| `usages <file> <node\|resource> [--scope ID] [--relation R[,R]] [--out] [--snapshot S] [--include-hidden] [--limit N] [--cursor C]` | Exact relations of one layer or resource (`VariableID:…`, `StyleKey:…`, `image:…`) in one stored scope, nested instances and swaps included: where a component, set, variable, style or image is used (default), or what a layer points at (`--out`). Each result keeps its exact ref, relation, state (`resolved`, `missing`, `unresolved`, …), basis and evidence; names never count as identity. No totals: coverage says complete or partial. Default 50 (1-200). With `--file`/`--workspace`/`--id` instead of `<file>` it reads the workspace's prepared index of `--scope` |
| `diff <old.fig> <new.fig> [--node ID] [--limit N] [--cursor C] [--json]` | What changed between two saved revisions: one line per changed top-level frame with counts per facet (text, layout, appearance, properties, structure, bindings); `--json` adds old/new field values and edge changes. Layers match by exact id and type only; a same-name or same-place frame is only a labelled possible match, and a side that stopped short reports unknown, never removed. Pages of 100 changes (1-500) |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID] [--flat]` | Compact expanded layout/text summary, at most 200 nodes. `--images` adds bitmap dimensions and crop footprints. `--flat` prints one line per node instead: position relative to the node, size, text, font, fill, stroke, radius, auto-layout |
| `nodes <file> <node>` | The per-node object of a REST nodes response (`document`, `components`, `componentSets`, `styles`): expanded instances and overrides, auto-layout, text styles, fills, effects, bound variables, component properties, render bounds |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | One decode, many nodes: one JSON line per listed id, the `nodes` output with its `nodeId`, or `{nodeId, error}` |
| `variables <file> [node]` | A frame's bound variables; without a node, every local variable |
| `bundle <file> <node> <out.zip> [--states]` | A design bundle for one frame; prints `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]`, `--frames -` (stdin) | Many bundles from one decode, in parallel |
| `verify <bundle>` | Check a bundle `.zip` or its extracted directory against its manifest: verified, missing and changed files, missing assets, the font faces to confirm in the project, and `findings`: each missing, unsupported or unverified handoff input with its nodes, evidence and next action |
| `export <file> <node> <out> [--bounds layout] [--scale N] [--quality 1-100] [--max-kb N] [--max-size N] [--match-dir DIR]` | SVG, PNG, JPG, WebP or AVIF; `--max-kb` lowers lossy quality until it fits; `--bounds layout` exports the layout box instead of the render bounds; `--match-dir DIR` (SVG only, subfolders included) lists the SVGs there that draw the same outlines (`matches`) and up to 10 similar shapes with a score (`similar`) |
| `export <file> --frames <list> --out-dir <dir> [--format F] [--scale N] ...`, `--frames -` (stdin) | One decode, many exports (default `png`): writes `<dir>/<nodeId>.<format>`, one JSON line per id, the `export` output with its `nodeId`, or `{nodeId, error}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Local read-only viewer; `--out` writes a standalone offline snapshot |
| `compare <file> <nodes.json> <node>` | Diff against a REST export of the same frame |
| `node <file> <node>` | One raw `.fig` node |
| `index prepare\|refresh\|rebuild\|clear\|status\|add\|remove\|relocate\|reset` | Local workspace in the per-user figlens state directory, or under a `--storage DIR` that the workspace then remembers. `prepare <file> --scope NODE` builds that scope's search index once; then `search --file <file>` (or `--workspace DIR`) answers without parsing, ranked across the workspace's sources. `refresh` rebuilds changed files, `rebuild` all or one `--source`/`--scope`, `clear` drops unused indexes; the previous index serves until a new one is ready. `reset` removes only the workspace database, never designs, exports or the remembered location |
| `jobs status\|cancel <jobId>` | Show or cancel an index build; cancel returns after the build stopped |
| `mcp` | Local MCP server over stdio |
| `update` | Replace the binary with the latest release |

For related component variants, use `inspect --states` or `bundle --states` (MCP `states: true`, default false; no configuration change). Evidence retains source, unknown and truncation markers. Bundles add `design/states.json` and a `context.md` summary. See [state evidence](../wiki/states.md) for scope and limits.

To compare two variants in the same component set, use `inspect <file> <variantA> --compare-variant <variantB>` (MCP `compareVariant`). `variantComparison` reports stored field/value-multiset differences with source IDs and unknowns; descendants remain unpaired. See [comparison limits](../wiki/states.md#compare-two-variants).

A node argument accepts `12:345`, `12-345` or a Figma URL with `?node-id=`. `<file>` can also be `gs://bucket/object`, downloaded with your `gcloud` login and cached. See the [GCS snapshot cookbook](../wiki/gcs.md) for a fixed-object handoff, access requirements and refresh limits. Each CLI call decodes the file; MCP reuses the last unchanged local file, while remote calls parse again. Run `figlens --help` or `figlens inspect --help` (also `-h`) for full CLI usage.

`inspect` includes recorded text resizing, truncation, max lines and decoration. A parent without auto-layout carries `childSpacing: "derived"`: gaps between its children are coordinate differences, not an auto-layout rule; `constraints` may still pin them. `handoffDiagnostics` reports known stored semantics not emitted, with counts, examples and [coverage limits](../wiki/semantics.md#known-handoff-gaps). Outputs protect the source and replace the destination only after a completed write. MCP limits request lines to 1 MiB and complete replies to 8 MiB; over-budget replies return an explicit error. Full CLI JSON remains available.
