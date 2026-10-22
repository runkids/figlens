<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Build UI anywhere. Just bring a .fig.</h1>

<p align="center">
  Give your AI coding agent a saved Figma file. figlens reads the design offline,<br>
  on your laptop, a CI runner or a cloud AI server.<br>
  <strong>No Figma installation, login or API key needed to read it.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">Watch the demo</a> · <a href="#quick-start">Quick start</a> · <a href="#browse-and-hand-off-a-design">Preview</a> · <a href="#pi-extension">Pi extension</a> · <a href="#how-close-is-it-to-figma">Accuracy</a>
</p>

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

## One .fig file is the design handoff

Your designer saves a local copy in Figma. Your AI gets the `.fig` file. figlens extracts layout, text, variables, assets and a reference image into one zip, ready to build UI from. The machine doing the work never needs Figma installed, including a headless cloud AI server.

Screenshots show what a screen looks like. The bundle gives your agent the spacing, typography, component structure and original assets behind it.

- **Keep building offline.** Reading a local file makes no Figma API calls, so there are no API rate limits or tokens to set up.
- **Run wherever your AI runs.** A laptop, CI runner or cloud server can read the same file with the CLI or local MCP server. No Figma desktop app or browser session required.
- **Give your agent more than a picture.** Get REST-shaped JSON with expanded instances, auto-layout, text styles and bound variables, plus image and SVG assets.

Offline means the design read path: your AI provider and project dependencies may still need a network connection. A live Figma URL alone is not enough; you need the saved `.fig` file.

## Quick start

### 1. Try it without installing

```sh
npx @runkids/figlens --help
```

Node.js 18+. The first run downloads and verifies the native binary for your platform, then caches it. No Figma, Rust or Docker needed.

Prefer a standalone command? Other options:

| Platform | Command |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Release binaries cover macOS (Apple silicon, Intel), Linux (arm64, x86_64, static musl) and Windows (x86_64; ARM64 runs it under emulation). Standalone installs update with `figlens update`.

### 2. Save your design and find a frame

In Figma, choose **File > Save local copy** and save it as `design.fig`. Then list its pages and frames:

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
```

The file is a snapshot you can reuse locally or in CI. Save a new copy when the design changes.

Search layer names or text, then inspect a frame before requesting its full design data:

```sh
npx @runkids/figlens find design.fig "Sign in" --type TEXT --limit 5
npx @runkids/figlens inspect design.fig 12:345 --depth 1
```

`find` returns ids, ancestor paths, local sizes and text previews. It searches stored layers; use `inspect` to see expanded instance children. `inspect` summarizes bounds, auto-layout and typography, with child counts. Its output is capped at 200 nodes and 200 characters per text; `truncated` signals omitted descendants and `textTruncated` marks shortened text. These limits reduce output size; each call still decodes the file.

### 3. Bundle it for your agent

Replace `12:345` with a frame id from the tree:

```sh
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Give your coding agent `frame.zip` and this prompt:

> Extract frame.zip into its own directory and read context.md. Build this screen using this project's stack and conventions. Use the supplied layout values and assets, then check your result against references/screen.png.

A node can be an id (`12:345`), its URL form (`12-345`) or a Figma URL with `?node-id=`. The URL selects a node in your saved file; it does not download the design.

## Browse and hand off a design

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx @runkids/figlens preview design.fig                      # browse pages, frames and stored names/text
npx @runkids/figlens preview design.fig 12:345               # open a specific scope
npx @runkids/figlens preview design.fig 12:345 --out preview.html
npx @runkids/figlens cache                                   # where cached renders live, and their size
npx @runkids/figlens cache clear                             # remove cached renders
```

Open the loopback URL printed in the terminal; stop with Ctrl+C. No frontend build or Figma session is needed. Each page opens like a Figma page: its frames and sections on one canvas, rendered on demand at the resolution the zoom needs, with a Layers panel, light and dark themes and Figma's selection model (click, double-click to go deeper, ⌘/Ctrl-click for the deepest layer, right-click for every layer under the pointer, Alt/Option to measure distances). Inspect layout, typography, variables, component properties, Dev Mode style CSS and original images, and export the selection as PNG, JPG, WebP or AVIF. Picking follows the actual outlines of vectors, rotated layers and ellipses, not just their bounding boxes. Renders are cached on disk so reopening a file is fast; `figlens cache clear` or the viewer's ⋯ menu removes them. Browser-automation agents can drive the open viewer through `window.figlens`.

Copy an exact ID, structured context or an implementation prompt, then download a real bundle for the selected layer. Expanded instance IDs stay intact. Import a local implementation screenshot for side-by-side or opacity-overlay comparison; it stays in your browser. This is visual checking, not an automated accuracy score or a mapping to project components.

The live viewer watches the saved local file through polling. Failed or partial saves visibly mark the view stale; successful saves refresh it and retain selection only if its ID still exists. For `gs://` input, this watches the downloaded cache, not the remote object.

`--out` (or `-o`) requires a node and writes a **snapshot**, usable as a local file with no server or network. It embeds the reference, layer data, original images and a bundle for the snapshot scope. Child inspection, context export and screenshot comparison work offline; the bundle always covers the snapshot scope, as its button states. Share carefully: the HTML contains the design data and assets. See [preview controls, safety and limitations](wiki/preview.md).

## Pi extension

**Build with the components your project already has.** Pi can read a local design, propose actual project imports and usage, and keep the mappings you review for the next screen. CLI, MCP, preview and the extension ship together:

```sh
pi install npm:@runkids/figlens
```

![Open a .fig, find project components, review mappings, and reuse them](assets/pi-workflow.svg)

```text
/figlens open ./design.fig
/figlens map
/figlens review
```

Select a frame, review the proposed code, then ask Pi to implement it using the approved mappings. Decisions stay in `.figlens/component-mappings.md` across sessions. New variants and changed recorded code need review. The workflow supports your project's stack, including plain HTML; it does not require React.

[![Inspect the local design, review an existing HTML component in Pi, and reuse it in the next frame](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Watch preview + Pi in action](assets/pi-demo.mp4)** · 20-second edited recording of the MCP Apps sample: inspect the design, approve a prepared mapping to existing HTML, and reuse it in another frame after a new session. Model generation is not shown or timed. [Full walkthrough, commands and limitations](pi/README.md) · [Recording and verification notes](wiki/pi.md#demo-and-verification).

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
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

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

Start with `find` and `inspect` to select a frame. `export` saves the image and also returns it directly as MCP image content, so your agent can see the design without opening a local file. PNG has the widest client support; reduce `scale` for large frames. `find`, `inspect`, `bundle` and `export` also return structured JSON results.

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
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | PNG, JPG, WebP or AVIF; `--max-kb` lowers quality until it fits (no effect on lossless PNG); prints `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Local read-only viewer; `--out` requires a node and writes a standalone offline snapshot |
| `compare <file> <nodes.json> <node>` | Diff against a REST export of the same frame |
| `node <file> <node>` | One raw `.fig` node |
| `mcp` | Local MCP server over stdio |
| `update` | Replace the binary with the latest release |

`<file>` can also be `gs://bucket/object`, downloaded with your `gcloud` login and cached. Run `figlens --help` for usage.

## Help figlens grow

If figlens helps your next design become code, [give it a star](https://github.com/runkids/figlens) so others can find it too. Found a design it struggles with? [Open an issue](https://github.com/runkids/figlens/issues) with the command you ran and what you expected.

For development, Rust runs in Docker only: `docker compose run --rm dev cargo test`. `make devc` opens a shell where `figlens` runs this checkout's source; `make help` lists the container targets. VS Code's Reopen in Container uses the same service. See [AGENTS.md](AGENTS.md) for project rules and [the wiki](wiki/README.md) for technical topics.

## License

[MIT](LICENSE). figlens is an independent project, not affiliated with, endorsed by or sponsored by Figma. The `.fig` format is undocumented and may change; figlens reads files you saved from your own Figma account.
