<p align="center">
  <img src="https://raw.githubusercontent.com/runkids/figlens/main/assets/logo.png" alt="figlens" width="320">
</p>

# @runkids/figlens

Build UI anywhere. Just bring a `.fig`.

Give your AI coding agent a saved Figma file (**File > Save local copy**). figlens reads the design offline, on your laptop, a CI runner or a cloud AI server. No Figma installation, login or API key needed.

This package runs the native `figlens` binary: the first run downloads the release build for your platform, checks it against the release's `SHA256SUMS` and caches it. Node.js 18 or later; macOS, Linux and Windows.

## Use it

```sh
npx @runkids/figlens tree design.fig --depth 2                 # pages and frames, with their ids
npx @runkids/figlens nodes design.fig 12:345 > frame.json      # a frame as Figma REST JSON
npx @runkids/figlens bundle design.fig 12:345 frame.zip        # nodes, variables, assets and a reference render
npx @runkids/figlens export design.fig 12:345 frame.png --scale 2
```

Or install it once: `npm i -g @runkids/figlens`, then run `figlens`.

## Pi extension

The same package includes the Pi extension and agent skill (Pi 1.3.0+). No separate global CLI installation is required:

```sh
pi install npm:@runkids/figlens
```

Restart Pi or run `/reload`, then ask: **"Build the List views screen from ./design.fig using this project's components."** Pi uses the automatically registered MCP tools to find the screen, read its assets and implement it. No manual open or node ID is required. Optional reusable component matches can be saved and reviewed with `/figlens review`; decisions survive sessions in `.figlens/component-mappings.md`. [Walkthrough and demo](https://github.com/runkids/figlens/tree/main/pi).

Browse the saved design separately with `npx @runkids/figlens preview design.fig`, or create a standalone offline HTML snapshot with `preview design.fig 12:345 --out preview.html`.

## MCP server

Let your agent call `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` and `export` as tools:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

## More

Documentation, the design bundle layout, accuracy measurements and other install methods (Homebrew, install scripts): [github.com/runkids/figlens](https://github.com/runkids/figlens).

MIT license.
