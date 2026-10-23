---
name: figlens
description: Build UI from local Figma .fig files using figlens CLI or MCP. Use when given a .fig file or a screen name to implement, a Figma URL with a local copy, requests to extract design assets or tokens, or when Figma API access is unavailable. In Pi, use its automatically registered figlens MCP tools to find the screen and build it directly.
---

# figlens

`figlens` reads a `.fig` file (Figma: **File > Save local copy**) and prints what the Figma REST API would return for a node: layout, text, styles, variables, instances already expanded. It also renders any node and writes a **bundle**: one zip with everything needed to build a frame. The `.fig` is a **snapshot**: it shows the design as it was when saved, so a newer design needs a newer copy.

## Setup

**In Pi with the figlens extension:** use its already registered figlens MCP tools (`tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `export`). They use the package's own launcher and cached native binary. No global CLI, shell `npx`, `/figlens open`, or saved selection is needed. The CLI examples below describe the equivalent operations: pass each argument through the corresponding tool schema, using absolute `file` and output paths. The tool namespace is supplied by the host.

**Other agents:** use configured figlens MCP tools when available, or the CLI below.

Prefer `npx @runkids/figlens` for one-off use (Node.js 18+, no global install). The examples below use `figlens` as shorthand: replace that prefix with `npx @runkids/figlens` when no standalone binary is installed. The first run downloads and verifies the matching native binary; later reads use its local cache.

Other installation options (no Rust or Docker needed):

```sh
curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh   # macOS, Linux
irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex       # Windows PowerShell
npx @runkids/figlens --help                                                                 # or run through npm
```

## Building a frame

1. **Get the `.fig`.** Ask the designer for File > Save local copy when there is none. Done when the file opens: `figlens tree <file.fig> --depth 1` lists its pages.
2. **Find the node yourself.** A Figma URL (`...?node-id=12-345`), `12-345` and `12:345` all work as-is. Without one, use `figlens find <file.fig> "screen name" --type FRAME` or search visible copy with `--type TEXT`. Search is a case-insensitive substring over stored layer names and text; `--node <id>` scopes it and `--limit N` sets 1-200 results (default 20). Inspect candidate paths, dimensions, visible copy and reference renders to resolve the user's intent. If ambiguity remains, ask once with a short list of recognizable page/screen names and their differences or images. Resolve IDs internally. `truncated: true` means refine or scope the search. Instance children appear only after expansion: use `figlens inspect <file.fig> <instance-id>`. `tree --depth 2` is only an initial outline; descend into relevant containers or use `find` for nested screens. Done when you have identified the intended screen, without requiring a manual open or picker.
3. **Bundle it.** `figlens bundle <file.fig> <node> frame.zip`, then unzip into its own directory and read `context.md` first: it says what each file holds. Done when you know the reference image, the structure file and the asset map.
4. **Outline before reading.** A screen is hundreds of KB of JSON, too large to read whole. Print the tree with sizes and layout, then read the subtrees you are building with `jq`:

   ```sh
   jq -r 'def o(d): "\("  " * d)\(.id) \(.type) \"\(.name)\" \(.absoluteBoundingBox.width // 0 | floor)x\(.absoluteBoundingBox.height // 0 | floor)\(if .layoutMode then " \(.layoutMode) gap=\(.itemSpacing // 0)" else "" end)\(if .layoutPositioning then " ABSOLUTE" else "" end)", (.children[]? | o(d + 1)); .document | o(0)' design/nodes.json
   jq '.. | objects | select(.id == "12:345")' design/nodes.json     # one node in full
   ```
5. **Build from the values**, not from the picture: sizes, padding, gaps, radii, colors, shadows and fonts are all in the JSON, and the reference image only confirms them. Inspect the project's stack and existing components, then implement using its conventions and the bundle's assets (see Reading the output). Mapping proposals and approval are optional; a scope with zero component instances can still be implemented from its layout, text and assets.
6. **Check against the reference.** Render your build at the frame's size and compare it with `references/screen.png` side by side or as a pixel diff. Done when every box, gap and asset lines up and the remaining differences are text antialiasing, line breaks from font metrics, or a known gap below.

For a question about layout, typography or copy, start with `figlens inspect <file.fig> <node> [--depth N]`: expanded bounds, auto-layout and typography, without paints or vector geometry. Depth defaults to 1 (0-10); at most 200 nodes and 200 characters per text are returned. `childCount` includes omitted children; `truncated` marks omitted descendants and `textTruncated` marks shortened text. Use `figlens nodes <file.fig> <node>` for full text, paints and other details, and `figlens variables <file.fig> <node>` for bound variables. Each call still decodes the whole file; summaries bound output, not parsing cost. Save full output once when making repeated queries.

## Link back to Figma

When referring to a specific frame or node in an answer or design document, link its actual name to its Figma location. Use ids read from the `.fig`, and include the page/frame path when names are ambiguous.

- Take the file key from the user's source URL (`/design/<file-key>/` or `/file/<file-key>/`), or the corresponding bundle's `manifest.json` → `source.fileKey`. Keep each `.fig` paired with its own key when reading multiple files. Pass the known key to `bundle --file-key` so its manifest preserves the source link.
- Build `https://www.figma.com/design/<file-key>/?node-id=<node-id>&m=dev`. For ordinary ids, change `112:319` to `112-319`. For expanded `I<instance>;…` ids, link the containing instance and show the full expanded id separately. Constructing links needs no Figma API or network access.
- With no known source URL or file key, ask once for the original Figma URL; meanwhile report the `.fig` name, page/frame path and node id. Never guess a file key from the filename or node id.
- Variables and modes are not frames: identify them as `collection → mode` (for example, `Color → Warm`), and link a related frame only after verifying that it uses the mode.

## Reading the output

- **Shape** is Figma REST: `document` is the frame; `children`; `absoluteBoundingBox {x,y,width,height}` in page coordinates (subtract the frame's x/y for positions inside it); `fills`/`strokes`/`effects`; auto-layout in `layoutMode`, `itemSpacing`, `padding*`, `primaryAxisAlignItems`, `counterAxisAlignItems`, `layoutSizingHorizontal`/`Vertical` (`FILL`/`HUG`/`FIXED`).
- **Absolute children**: `layoutPositioning: "ABSOLUTE"` marks a child of an auto-layout frame that sits outside the flow (title bars, floating inputs, badges). Place it with its bounding box, out of the flex flow.
- **Colors** are `r,g,b,a` floats in 0–1: multiply by 255. A paint's own `opacity` multiplies `a`.
- **Text**: `characters` and `style` (family, weight, size, `lineHeightPx`, letter spacing). Mixed styles inside one text are in `characterStyleOverrides` (one style id per character) and `styleOverrideTable` (id → the fields that differ), e.g. one bold word in a sentence.
- **Instances**: ids starting with `I` (`I<instance>;<path>`) are layers inside a component instance, already expanded with its overrides; `componentProperties` holds the variant values.
- **Variables**: `boundVariables` maps a field to `VariableID:…` aliases; look the id up in `design/variables.json` for its name, collection and per-mode values, so the build can use a token (`BG/100`) instead of a hex value:

  ```sh
  jq -r '.variables[] | "\(.id)\t\(.name)\t\(.resolvedType)"' design/variables.json
  ```
- **Assets**: `assets.json` maps `imageRef` (original image fills, keep each node's crop and scale from its fill), `node:<id>` (an SVG of one vector) and `render:<id>` (a 2x PNG of a node that composites an image through a blend mode, mask or effect) to files under `assets/`. For a complete icon or group, call `export` with that instance/group ID and an `.svg` output path to get one standalone SVG instead of assembling its individual vector assets.
- `visible: false` nodes are not drawn; skip them.

## Images and slices

`figlens export <file.fig> <node> <out.svg|png|jpg|webp|avif> [--scale 2] [--quality 80] [--max-kb 150] [--max-size 1600]` renders one node like Figma's Export panel, shadows included; instance sublayer ids work. The output extension picks the format. SVG keeps vectors, embeds original image fills and uses glyph outlines for text; scale and quality do not apply. `--max-kb` lowers lossy quality until the file fits; PNG/SVG report an error if over budget; `--max-size` scales raster output down to fit its longest side; JPG has no transparency and composites on white. It prints `{path, width, height, bytes, quality}`; `quality` is the raster quality used (the requested value for SVG, where it has no effect).

Through MCP, call the `export` tool with absolute `file` and `out` paths: it writes the file and returns text metadata and `structuredContent`. Raster formats also return the same rendered bytes as inline image content; SVG returns metadata only. Prefer PNG for client compatibility. Raster exports are scaled down to fit `maxSize` (default 1600 px on the longest side; pass 0 for the full scaled size). `find` and `inspect` are also MCP tools with the same options and structured results.

## Visual preview and handoff

Use `figlens preview <file.fig> [node]` when a person needs to browse or select a scope. Open the printed local URL. The viewer provides page/frame browsing, name/text search, a synchronized layer tree, bounds-based overlap picking and property inspection. Download a selected-scope bundle or copy IDs/context without guessing them; expanded `I<instance>;…` IDs remain exact and work with `bundle` and `export`. The implementation prompt is grounded in design data, not an automatic mapping to project components. To drive an open preview from a browser-automation tool, call the page's `window.figlens` API (`state()`, `select(id)`, `context()`, `exportImage(...)`; see the `preview` topic) instead of clicking through the UI.

For a portable handoff, run `figlens preview <file.fig> <node> --out preview.html`. The HTML works offline and embeds design data, original images and the snapshot-scope bundle. Selecting a child changes context, not the embedded bundle's scope. Share it only with people allowed to see those assets. Import a local implementation screenshot for side-by-side or opacity comparison; this is visual checking, not measured accuracy.

Live preview polls the saved local file. Treat a stale/error banner as unavailable fresh evidence; a failed save must not be described as a refreshed design. Snapshots never refresh. For `gs://` inputs the live viewer watches its local cached copy, not the remote object. No viewer action replaces checking the bundle's `missing-assets.json` and the known renderer gaps below.

## Pi component mappings

When the figlens Pi extension is available, `figlens_bindings` can retrieve saved component matches. Supply the **current request's `file` and `frame` together**; this works without open and leaves any manual selection unchanged. The current request determines the scope even if a different selection was saved earlier. It returns actual instance properties/text, approved rules, pending/stale candidates and pagination. Follow `nextOffset` when more instances are needed. Reuse recorded mappings marked `approved`, using current design text rather than example literals. Empty, unmatched or stale results do not block implementation: inspect source and design directly.

When the user wants to save reusable component matches, read the real target implementation and usage first. Call `figlens_propose_bindings` with explicit file/frame, its instance ID, target path/export/import/example, rationale, supporting file dependencies, checks actually executed and unresolved issues. Plain HTML can name an existing block and explain that no import is needed. The user reviews with `/figlens review`, which lists pending proposals across this project without open; proposals stay pending in non-interactive modes. Approval is a mapping decision, so run the project's permitted compilation/runtime checks and compare the screen separately. The [walkthrough](https://github.com/runkids/figlens/tree/main/pi) documents installation and optional commands. Without the extension, continue with the CLI workflow above.

## Many frames

`figlens bundle <file.fig> --frames <list> --out-dir <dir>` decodes once and writes one bundle per id in `<list>` (one id per line), in parallel, printing a JSON line per frame (`{nodeId, path, warnings}` or `{nodeId, error}`). A single `bundle` prints the same object; non-empty `warnings` name assets the bundle could not include. Use it instead of one `bundle` call per frame.

## Known gaps

- Not emitted yet: constraints, layoutAlign/layoutGrow, layout grids, componentPropertyDefinitions.
- Hidden overridden text can carry a stale size.
- Renders skip background blur and image contrast/temperature/tint/highlights/shadows adjustments, and draw angular/diamond gradients as radial; check those areas with the designer rather than matching the reference there.
- Font files are not in the `.fig`: name the fonts the design uses, and report the ones the project lacks instead of substituting silently.
