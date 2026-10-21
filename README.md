# figlens

**Read Figma designs offline.** figlens turns a `.fig` file (Figma: **File > Save local copy**) into the same JSON the Figma REST API returns, plus the variables, image and SVG assets, and renders of any node. No Figma API key, no rate limits, no running Figma: one native binary that works offline and in CI.

It is built for AI design-to-code: `figlens bundle` writes one zip with everything an agent needs to build a screen, and a pinned file never changes under it.

> figlens is an independent project. It is not affiliated with, endorsed by or sponsored by Figma. The `.fig` format is undocumented and may change; figlens reads files you saved from your own Figma account.

## Install

| Platform | Command |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |
| Homebrew | `brew install runkids/tap/figlens` |
| npm (no install) | `npx figlens --help` |

Release binaries cover macOS (Apple silicon, Intel), Linux (arm64, x86_64, static musl) and Windows (x86_64; ARM64 runs it under emulation). `figlens update` replaces the binary with the latest release.

## Quick start

```sh
figlens tree design.fig --depth 2                     # pages and frames, with their ids
figlens nodes design.fig 12:345 > frame.json          # one frame as Figma REST JSON
figlens bundle design.fig 12:345 frame.zip            # everything to build the frame from
figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

A node can be an id (`12:345`), its URL form (`12-345`) or a Figma URL with `?node-id=`.

## Commands

| Command | Output |
|---|---|
| `tree <file> [node] [--depth N]` | ids, types and names |
| `nodes <file> <node>` | the frame as REST `nodes.json`: instances expanded with their overrides, auto-layout, text styles and per-character overrides, fills, effects, bound variables, component properties, render bounds |
| `variables <file> [node]` | the variables a frame binds, with collections, modes and aliases; without a node, every local variable |
| `bundle <file> <node> <out.zip>` | a design bundle (below) |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | many bundles from one decode, in parallel; one JSON line per frame |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | PNG, JPG, WebP or AVIF of any node, like Figma's Export panel; `--max-kb` lowers the quality until it fits |
| `compare <file> <nodes.json> <node>` | diff against a REST export of the same frame |
| `node <file> <node>` | one raw `.fig` node |
| `mcp` | an MCP server on stdio serving `tree`, `nodes`, `variables`, `bundle` and `export` as tools |

`<file>` can also be `gs://bucket/object` (read with your `gcloud` login and cached).

## The design bundle

```
manifest.json             files with role, media type, size and sha256; source file key and URL
design/nodes.json         the frame as Figma REST JSON
design/variables.json     the variables it binds
assets/ + assets.json     original image fills, SVGs of vectors, 2x PNGs of composited nodes
missing-assets.json       anything that could not be produced
references/screen.png     a render of the whole frame to check the result against
context.md                instructions for an agent building from the bundle
```

## How close is it to Figma?

figlens is measured against Figma's own exports (the plugin API's `JSON_REST_V1`, PNG and SVG). On an 826-frame production file: structure identical in 771 frames, most emitted fields in 790–826, image fills byte-identical, 96% of SVG assets pixel-equivalent. On five Figma Community files, all 306 top-level frames bundle with no missing assets. Details and method: [`wiki/accuracy.md`](wiki/accuracy.md).

Known gaps: constraints, layout grids and component property definitions are not emitted yet; renders skip background blur and some image adjustments (contrast, temperature, tint, highlights, shadows) and draw angular and diamond gradients as radial. Text is drawn from the glyph outlines stored in the file, so no fonts are needed, but a reflowed overridden text keeps Figma's last layout.

## For AI agents

[`skills/figlens/SKILL.md`](skills/figlens/SKILL.md) teaches an agent to use figlens: how to find a frame, extract it and read the output without loading it whole.

Install it into every AI tool you use (Claude Code, Codex, Cursor, Gemini CLI and more) with [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` keeps it current. Without skillshare, copy `skills/figlens/` into your agent's skills directory (for Claude Code, `~/.claude/skills/`); release archives include it too.

Or connect figlens as an MCP server, so the agent calls `tree`, `nodes`, `variables`, `bundle` and `export` as tools. In Claude Code: `claude mcp add figlens -- figlens mcp`. Other clients take the same command in their MCP config:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Give tools absolute paths: relative ones resolve against the directory the client started figlens in.

## Development

Rust runs in Docker only: `docker compose run --rm dev cargo test`. For a shell where `figlens` runs this checkout's source, run `make devc`; `make help` lists the other container targets, and VS Code's Reopen in Container uses the same service. See [`AGENTS.md`](AGENTS.md) for the rules and the `wiki/` topics, and [`wiki/release.md`](wiki/release.md) for releasing.

## License

[MIT](LICENSE)
