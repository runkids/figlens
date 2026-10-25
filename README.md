<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Your coding agent reads Figma files.<br>No Figma app. No login. No API quota.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">Start</a> · <a href="#demo">Demo</a> · <a href="#why">Why</a> · <a href="#bundle">What the agent gets</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limitations</a>
</p>

**The problem.** Your agent needs layout values, text and assets to build a screen. On a remote host or in CI, that design context needs to be available alongside the code, without relying on a running desktop app.

**The fix.** In Figma, **File > Save local copy**. Drop `design.fig` next to your code. figlens parses it locally and gives your agent layout, text, styles, variables, icons, images and a reference render of any frame. One native binary, zero network calls to Figma.

<a id="start"></a>

## Get started

**No design file yet?** [Check your setup with the tiny public example](examples/first-success/README.md): verify your chosen binary, find one frame and validate its bundle. No model required.

Put `design.fig` in your project, then pick your agent.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Start a new session and check `/mcp`. Ask for a screen in your own words: MCP includes on-demand guidance for planning, implementation and image preparation; no separate skill install is required.

**Pi** (1.3.0+): one package ships the CLI, MCP, viewer and skill.

```sh
pi install npm:@runkids/figlens
```

**Any MCP client** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**No Node.js?** Install the standalone binary and use `"command": "figlens"` instead:

| Platform | Command |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Then ask for a screen:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

The agent finds the frame, builds a bundle with design data, assets and a reference image, and implements the screen with your stack. Ask it to show the changed files and the comparison; a bundle alone does not prove the UI is right.

<a id="demo"></a>

## Demo: Pi builds a React screen from a .fig

[![Pi plans from a local .fig, builds a React screen and checks it beside a live browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

Looping GIF at 3× the edited video’s playback speed. Click it for the full 78-second video.

**[78-second video](assets/taskflow-demo.mp4).** Pi runs `/figlens plan`, reads a dashboard frame from a local `.fig`, builds it with React and Tailwind CSS, then checks and corrects the running UI. Edited time-lapse, not a benchmark or a pixel-perfect promise. Design: [Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [recording notes](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

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
| Fidelity | Independent parser with [known gaps](#limitations) | Maintained by Figma |

Reading a `.fig` is offline. Installing and updating figlens, `gs://` inputs, your model provider and your project's dependencies still need a network.

<a id="bundle"></a>

## What the agent gets

One frame, one zip. Structure to build from, assets to use, a picture to check against.

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

Need one piece? The CLI does it directly:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # frame ids by name or text
npx -y @runkids/figlens inspect design.fig 12:345                 # compact layout and text summary
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST-shaped JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP or AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # the zip above
```

SVG exports embed original image fills and outline text as glyphs, so no fonts are needed. Through MCP, raster exports also come back as inline images, so the agent sees the design without opening a file.

### How close is it to Figma?

Measured against Figma's own JSON, PNG and SVG exports on **826 production frames** and five Community files (2026-10-22):

| Check | Result |
|---|---|
| Node structure, names, visibility, bounds and text | Identical in 771 / 826 frames |
| Original image fills | 2,608 / 2,608 byte-identical |
| SVG assets | 5,014 / 5,197 within the pixel-difference threshold |
| Community frame bundles | 306 / 306 with no missing assets |

Results on those files, not a guarantee for every design. Method and thresholds: [accuracy report](wiki/accuracy.md).

<a id="pi"></a>

## Pi extension

```sh
pi install npm:@runkids/figlens
```

Restart Pi or `/reload`, then ask for a screen in plain words. Pi finds the frame, reads it over the auto-registered MCP server and builds with your stack. No node IDs, no global CLI.

- `/figlens plan <request>` writes a plan only; it does not edit the application.
- `/figlens yolo <request>` plans, builds, checks and cleans up its own temporary plan.
- `/figlens preview [description]` opens the local viewer.
- `/figlens review` approves reusable component mappings the agent proposed.

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](assets/pi-workflow.svg)

Full walkthrough, commands and limits: [pi/README.md](pi/README.md).

<a id="viewer"></a>

## Browse a design without Figma

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # local viewer, Ctrl+C to stop
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # offline HTML snapshot to send to anyone
```

Browse pages and layers, search, measure, inspect layout, typography, variables and CSS, export any layer, copy an implementation prompt or download a bundle for the selection. Drop in a screenshot of your build to overlay it on the design. Details: [preview guide](wiki/preview.md).

<a id="mcp-and-skill"></a>

## MCP server and agent skill

**MCP.** The server runs locally over stdio and exposes `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` and `export`. The agent can read built-in `build`, `images`, `plan`, `planning-details` or opt-in `yolo` guidance through `workflow`, then use its own project and browser tools to carry out the authorized task. These guides share the portable skill's source; installing that skill separately is optional for MCP users. Client support and model behavior determine how the guidance is used. Pass absolute paths. Add `--root /path/to/designs` (repeatable) to confine every file and output path to chosen folders. Raster exports are scaled down to `maxSize` (default 1600 px) so inline images stay small; SVG returns metadata only.

Native Windows with Claude Code: follow its [stdio setup notes](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) and the `cmd /c` wrapper for `npx`.

**Skill.** The [figlens skill](skills/figlens/SKILL.md) teaches an agent to find frames, read the bundle in small pieces, plan scoped work and verify the running UI against the design. It installs instructions only; the CLI or MCP server must be available. Install with the [Skills CLI](https://github.com/vercel-labs/skills) or copy `skills/figlens/` into your agent's skills directory (Claude Code: `~/.claude/skills/`):

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

**Install notes.** The npm launcher downloads the matching native binary on first use, verifies it against the release's SHA-256 checksums and caches it. Standalone installs update with `figlens update`. Binaries cover macOS (Apple silicon, Intel), Linux (arm64, x86_64, static musl) and Windows (x86_64).

<a id="commands"></a>

## CLI reference

| Command | Output |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Node ids, types and names; indented text or nested JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Case-insensitive search over layer names and text; paths, sizes and previews. Default 20 results (1-200) |
| `inspect <file> <node> [--depth N] [--images]` | Compact expanded layout/text summary, at most 200 nodes. `--images` adds bitmap dimensions and crop footprints |
| `nodes <file> <node>` | REST-shaped JSON: expanded instances and overrides, auto-layout, text styles, fills, effects, bound variables, component properties, render bounds |
| `variables <file> [node]` | A frame's bound variables; without a node, every local variable |
| `bundle <file> <node> <out.zip>` | A design bundle for one frame; prints `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Many bundles from one decode, in parallel |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP or AVIF; `--max-kb` lowers lossy quality until it fits |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Local read-only viewer; `--out` writes a standalone offline snapshot |
| `compare <file> <nodes.json> <node>` | Diff against a REST export of the same frame |
| `node <file> <node>` | One raw `.fig` node |
| `mcp` | Local MCP server over stdio |
| `update` | Replace the binary with the latest release |

A node argument accepts `12:345`, `12-345` or a Figma URL with `?node-id=`. `<file>` can also be `gs://bucket/object`, downloaded with your `gcloud` login and cached. See the [GCS snapshot cookbook](wiki/gcs.md) for a fixed-object handoff, access requirements and refresh limits. Each CLI call decodes the file; MCP reuses the last unchanged local file, while remote calls parse again. Run `figlens --help` for usage.

<a id="limitations"></a>

## Limitations

- **Snapshots, not live sync.** Export a new `.fig` when the design changes. figlens cannot download a design from a Figma URL.
- **Undocumented format.** figlens decodes the file's embedded schema; new Figma exports may need a figlens update.
- **Not emitted yet:** Dev Mode annotations, constraints, layoutAlign/layoutGrow, layout grids, component property definitions.
- **Render gaps:** background blur and some image adjustments are skipped; angular and diamond gradients draw as radial. Overridden text can keep Figma's last layout. Font files are not included.
- **Check the output.** Read bundle `warnings`, `missing-assets.json` and unresolved variables in `context.md`, then compare the implementation with the design.

## Help figlens grow

If figlens turned a design into code for you, [star the repo](https://github.com/runkids/figlens) so others find it. Hit a file it cannot read? [Open an issue](https://github.com/runkids/figlens/issues) with the command and what you expected.

Development runs Rust in Docker only: `docker compose run --rm dev cargo test`. See [AGENTS.md](AGENTS.md) and the [wiki](wiki/README.md).

## License

[MIT](LICENSE). figlens is an independent project, not affiliated with, endorsed by or sponsored by Figma. The `.fig` format is undocumented and may change; use only files and assets you have permission to access and share.
