<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Your Figma design, ready for your coding agent.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  Turn a saved <code>.fig</code> into layout, text, variables, assets and a reference image.<br>
  Let your agent build with the design details, wherever it runs.<br>
  <strong>No Figma app, login or API rate limits on the machine doing the work.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">Watch the demo</a> · <a href="#quick-start">Try your design</a> · <a href="#for-ai-agents">Connect your agent</a> · <a href="#pi-extension">Use with Pi</a> · <a href="#how-close-is-it-to-figma">Accuracy</a>
</p>

## Why figlens?

“Build this screen” should come with the screen's layout, text and assets. With screenshots alone, the agent has to estimate spacing and typography, and you still have icons and images to export. With a live Figma connection, design reads depend on access setup and API availability on the machine doing the work.

figlens reads the saved design directly. Choose **File > Save local copy** in Figma, then give the `.fig` to your agent through the CLI or local MCP server. It can find a screen, inspect its layout and text, and extract the assets it needs. For a portable handoff, bundle a frame's design data, assets and reference render into one zip.

| If you've run into this | Use figlens to |
|---|---|
| API limits interrupt design reads | Read the local file as often as needed, with no Figma API calls or tokens |
| Your agent runs on a cloud server or in CI | Read the design with a native CLI, without a Figma app or browser session |
| You keep explaining measurements and exporting assets | Supply structured layout, typography, expanded instances, bound variables and original assets |
| Another agent or teammate needs the same screen | Share a frame bundle with design data, assets, a reference render and instructions |

Designers keep working in Figma. Your agent works in your project's stack. figlens supplies the design context; the coding agent writes the implementation.

## From a local .fig to working HTML

[![Pi builds a working interface from a local Figma file, with a live preview beside the terminal](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[Watch the video](assets/demo-8x.mp4)** · 45-second time-lapse, **8× speed**. A real Pi session in Orca: read the local design, build HTML/CSS, then try the Share dialog and composer in the live preview. The interface uses 19 SVG icons from the bundle; it is HTML, not a flattened screenshot.

| This example | Measured result |
|---|---|
| Local `.fig` file | 57.5 MiB |
| Selected frame | 153 nodes, 25 SVG assets, no missing assets |
| Extract the design bundle | **0.47 s median** over five warm runs |

The timing measures `figlens bundle`, **not the AI's implementation time**. Measured with figlens 0.1.2 in an ARM64 Docker container; see the [commands, individual runs and recording notes](wiki/accuracy.md#local-design-to-html-demo). Pi writes the code; figlens supplies the design data. The AI provider still uses a network connection.

<details>
<summary>Design credit and demo limitations</summary>

The “List views” frame comes from [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). The demo uses system sans and Georgia because the original Anthropic fonts are not bundled. Share opens a local dialog; replies stay in the page and reset on reload. The recording uses snapshots of the real Orca window, so motion is less smooth than a native video capture.

</details>

## Quick start

### 1. Open your design locally

In Figma, choose **File > Save local copy** and save it as `design.fig`. Then run:

```sh
npx @runkids/figlens preview design.fig
```

Open the local URL printed in the terminal. Browse pages and select the screen you want to build. You can inspect its spacing, typography, variables and assets right there.

Requires Node.js 18+. The first run downloads and verifies a native binary, then caches it. [Standalone installs](#installation) need no Node.js, Rust or Docker. Stop the viewer with Ctrl+C.

### 2. Give the screen to your agent

Download the selected frame's bundle from the viewer. Give your coding agent the zip and this prompt:

> Extract this bundle into its own directory and read context.md. Build this screen using this project's stack and conventions. Use the supplied layout values and assets, then check your result against references/screen.png. Report missing assets or unavailable fonts.

The bundle includes the structure, assets and reference image together, so the agent can read it with ordinary file tools. To let it find and inspect screens directly, [connect the CLI or MCP server](#for-ai-agents). [Pi users can ask for a screen by name](#pi-extension).

### 3. Check the implementation

Import a screenshot of your implementation into the viewer. Compare it side by side with the design or use the opacity overlay to spot differences. The screenshot stays in your browser. This is a visual check; figlens does not assign an accuracy score to your implementation.

**The `.fig` is a snapshot.** Save a new local copy when the design changes. A Figma URL can select a node within that copy; it cannot download the design. Reading the saved file works offline after installation; your AI provider and project dependencies may still need a network connection.

<details>
<summary>Prefer the terminal? Find a frame and bundle it directly.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Replace `12:345` with an id from the tree or search results. A node can be an id (`12:345`), its URL form (`12-345`) or a Figma URL with `?node-id=`. Search layer names or text with `find`; use `inspect` for expanded instance children and a compact layout/text summary. See [Commands](#commands) for output details.

</details>

## Browse and hand off a design

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](assets/preview-inspector.png)](assets/pi-demo.mp4)

The local viewer lets you explore a saved design without a Figma session:

- Browse pages and layers, search names and text, measure distances, and inspect layout, typography, variables, component properties and CSS.
- Export a selected layer as SVG, PNG, JPG, WebP or AVIF, or download its original image fills.
- Copy a layer's ID, structured context or an implementation prompt; download a bundle for the exact selection, including instance sublayers.

Need to send someone a design they can browse? Export a standalone HTML snapshot:

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

It opens without a server or network and includes the design data, images and a bundle for the snapshot scope. Share it only with people allowed to see those assets. Selecting a child changes the inspected context; the embedded bundle still covers the snapshot scope.

The live viewer watches the saved local file and refreshes after successful saves; partial or failed saves mark the view stale. HTML snapshots stay fixed. For controls, cache management, browser-agent automation and limitations, see the [preview guide](wiki/preview.md).

## Pi extension

**Give Pi a `.fig` and ask for the screen.** The package includes the CLI, MCP, preview, extension and agent skill:

```sh
pi install npm:@runkids/figlens
```

Restart Pi or run `/reload`, then ask normally:

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi finds the screen, reads its layout and assets, inspects your project and implements in its existing stack. You do not need to know frame IDs or run `/figlens open`. When names are ambiguous, Pi can show a short set of candidates. MCP starts automatically; no global CLI installation is needed.

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](assets/pi-workflow.svg)

**Optional component reuse:** ask Pi to save reusable matches, then use `/figlens review` to accept or edit pending proposals across the project. Decisions stay in `.figlens/component-mappings.md`; changed variants or recorded code need review. Building a screen does not require mappings.

[![Inspect the local design, review an existing HTML component in Pi, and reuse it in the next frame](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Watch the optional mapping workflow](assets/pi-demo.mp4)** · 20-second edited recording from v0.3.0: inspect the MCP Apps design, approve a prepared mapping to existing HTML, and reuse it after a new session. Model generation is not shown or timed. [Full walkthrough, commands and limitations](pi/README.md) · [Recording and verification notes](wiki/pi.md#demo-and-verification).

Requires Pi 1.3.0+. The npm extension uses its bundled CLI launcher; the first launch, including automatic MCP startup, downloads and verifies the matching native binary. Homebrew and release archives provide the native CLI and viewer. See [source-checkout setup](pi/README.md#install-once) for development.

## The design bundle

One frame, one zip. Structure to build from, assets to use, and a picture to check against.

```text
frame.zip
├── context.md                Start here: instructions for your agent
├── design/
│   ├── nodes.json            Layout, text, styles and expanded instances
│   └── variables.json        Bound variables, collections, modes and aliases
├── assets/                   Image fills, SVGs and composited node renders
├── assets.json               Maps design references to asset files
├── references/screen.png     A render of the frame to compare against
├── missing-assets.json       Anything that could not be produced
└── manifest.json             File roles, media types, sizes and SHA-256 hashes
```

Need just one piece? Export JSON or render a node directly:

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

SVG exports work for frames, groups, icons and exact instance sublayers: one selected subtree becomes one standalone SVG, with original image fills embedded and text as glyph outlines. No bundle extraction needed. SVG keeps its original dimensions; `--scale` and `--quality` apply to raster formats only.

## How close is it to Figma?

figlens is checked against Figma's own JSON, PNG and SVG exports. The recorded results on **826 production frames** and **five Community files** include:

| Check | Result |
|---|---|
| Node structure, names, visibility, bounds and text | Identical in 771 / 826 production frames |
| Original image fills | 2,608 / 2,608 byte-identical |
| SVG assets | 5,014 / 5,197 within the pixel-difference threshold |
| Community frame bundles | 306 / 306 with no missing assets |

These are measured results on those files, not a guarantee for every design. See the [accuracy report](wiki/accuracy.md) for dates, thresholds, render results and method.

**Known gaps:** constraints, layoutAlign/layoutGrow, layout grids and component property definitions are not emitted yet. Renders skip background blur and some image adjustments (contrast, temperature, tint, highlights, shadows), and draw angular and diamond gradients as radial. Text uses stored glyph outlines, so rendering needs no fonts, but overridden text can retain Figma's last layout. Font files are not included for your implementation.

## For AI agents

Use the CLI through an agent skill, or connect the same binary as an MCP server.

### Agent skill

The [figlens skill](skills/figlens/SKILL.md) teaches your agent to find frames, read the bundle in small pieces and build from its values.

Install with the [Skills CLI](https://github.com/vercel-labs/skills):

```sh
npx skills add runkids/figlens --skill figlens
```

To manage and update skills across Claude Code, Codex, Cursor, Gemini CLI and more, use [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` keeps it current. You can also copy `skills/figlens/` into your agent's skills directory (for Claude Code, `~/.claude/skills/`). Release archives include it too.

### MCP server

Manage MCP servers alongside your agent skills with [skillshare](https://github.com/runkids/skillshare). Define figlens once and sync its connection settings to Claude Code, Codex and Cursor:

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

See the [Skillshare MCP guide](https://skillshare.runkids.cc/docs/reference/commands/mcp/) for other clients and importing existing settings.

For a direct Claude Code setup:

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

Other MCP clients:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

With a standalone `figlens` installation, use:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

The server runs locally over stdio and exposes `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` and `export`. Give tools absolute paths: relative ones resolve against the directory the client started figlens in.

Start with `find` and `inspect` to select a frame. `export` saves the file and returns metadata. Raster formats also return MCP image content, so your agent can see the design without opening a local file; SVG returns metadata only. PNG has the widest client support; reduce `scale` for large frames. `find`, `inspect`, `bundle` and `export` also return structured JSON results.

## Installation

Use `npx @runkids/figlens` with Node.js 18+, or install the standalone CLI:

| Platform | Command |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Release binaries cover macOS (Apple silicon, Intel), Linux (arm64, x86_64, static musl) and Windows (x86_64; ARM64 runs it under emulation). Standalone installs update with `figlens update`.

## Commands

| Command | Output |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Node ids, types and names; indented text (smaller for an agent's context) or, with `--json`, nested JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Case-insensitive substring search over stored layer names and text; paths, local sizes and previews. Default 20 results (1-200); `truncated` signals more matches |
| `inspect <file> <node> [--depth N]` | Compact expanded layout/text summary. Default depth 1 (0-10), at most 200 nodes; use `nodes` for full data |
| `nodes <file> <node>` | REST-shaped JSON: expanded instances and overrides, auto-layout, text styles, fills, effects, bound variables, component properties and render bounds |
| `variables <file> [node]` | A frame's bound variables; without a node, every local variable |
| `bundle <file> <node> <out.zip>` | A design bundle for one frame; prints `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Many bundles from one decode, in parallel; one JSON line per frame, as above or `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | SVG, PNG, JPG, WebP or AVIF; `--max-kb` lowers lossy quality until it fits; PNG/SVG fail if over budget; prints `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Local read-only viewer; `--out` requires a node and writes a standalone offline snapshot |
| `compare <file> <nodes.json> <node>` | Diff against a REST export of the same frame |
| `node <file> <node>` | One raw `.fig` node |
| `mcp` | Local MCP server over stdio |
| `update` | Replace the binary with the latest release |

`find` searches stored layers and returns ids, ancestor paths, local sizes and text previews. `inspect` expands instances and caps output at 200 nodes and 200 characters per text; `truncated` and `textTruncated` identify omissions. Each call still decodes the file.

`<file>` can also be `gs://bucket/object`, downloaded with your `gcloud` login and cached. Run `figlens --help` for usage.

## Help figlens grow

If figlens helps your next design become code, [give it a star](https://github.com/runkids/figlens) so others can find it too. Found a design it struggles with? [Open an issue](https://github.com/runkids/figlens/issues) with the command you ran and what you expected.

For development, Rust runs in Docker only: `docker compose run --rm dev cargo test`. `make devc` opens a shell where `figlens` runs this checkout's source; `make help` lists the container targets. VS Code's Reopen in Container uses the same service. See [AGENTS.md](AGENTS.md) for project rules and [the wiki](wiki/README.md) for technical topics.

## License

[MIT](LICENSE). figlens is an independent project, not affiliated with, endorsed by or sponsored by Figma. The `.fig` format is undocumented and may change; figlens reads files you saved from your own Figma account.
