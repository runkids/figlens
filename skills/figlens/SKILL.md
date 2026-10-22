---
name: figlens
description: Read Figma designs offline from a `.fig` file with the `figlens` CLI, without the Figma API, MCP or a running Figma. Use when given a `.fig` file; when implementing a Figma frame, URL or node-id and a `.fig` copy exists or can be requested; when exporting images, icons or slices from a design; when reading a design's variables or tokens; or when Figma MCP/REST is rate-limited or unavailable.
---

# figlens

`figlens` reads a `.fig` file (Figma: **File > Save local copy**) and prints what the Figma REST API would return for a node: layout, text, styles, variables, instances already expanded. It also renders any node and writes a **bundle**: one zip with everything needed to build a frame. The `.fig` is a **snapshot**: it shows the design as it was when saved, so a newer design needs a newer copy.

## Setup

When `figlens` is not on PATH, install it (no Rust or Docker needed):

```sh
curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh   # macOS, Linux
irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex       # Windows PowerShell
npx @runkids/figlens --help                                                                 # or run through npm
```

## Building a frame

1. **Get the `.fig`.** Ask the designer for File > Save local copy when there is none. Done when the file opens: `figlens tree <file.fig> --depth 1` lists its pages.
2. **Find the node.** A Figma URL (`...?node-id=12-345`), `12-345` and `12:345` all work as-is. Without one, `figlens tree <file.fig> --depth 2` lists each page's top frames; `figlens tree <file.fig> <id> --depth 1` narrows down. Done when you hold the id of the frame to build.
3. **Bundle it.** `figlens bundle <file.fig> <node> frame.zip`, then unzip into its own directory and read `context.md` first: it says what each file holds. Done when you know the reference image, the structure file and the asset map.
4. **Outline before reading.** A screen is hundreds of KB of JSON, too large to read whole. Print the tree with sizes and layout, then read the subtrees you are building with `jq`:

   ```sh
   jq -r 'def o(d): "\("  " * d)\(.id) \(.type) \"\(.name)\" \(.absoluteBoundingBox.width // 0 | floor)x\(.absoluteBoundingBox.height // 0 | floor)\(if .layoutMode then " \(.layoutMode) gap=\(.itemSpacing // 0)" else "" end)\(if .layoutPositioning then " ABSOLUTE" else "" end)", (.children[]? | o(d + 1)); .document | o(0)' design/nodes.json
   jq '.. | objects | select(.id == "12:345")' design/nodes.json     # one node in full
   ```
5. **Build from the values**, not from the picture: sizes, padding, gaps, radii, colors, shadows and fonts are all in the JSON, and the reference image only confirms them. Use the bundle's assets for every image and icon (see Reading the output).
6. **Check against the reference.** Render your build at the frame's size and compare it with `references/screen.png` side by side or as a pixel diff. Done when every box, gap and asset lines up and the remaining differences are text antialiasing, line breaks from font metrics, or a known gap below.

For a question about a design rather than a build (a color, a spacing, which component a frame uses), skip the bundle: `figlens nodes <file.fig> <node>` prints the same JSON to stdout, and `figlens variables <file.fig> <node>` prints the variables it binds. Each call decodes the whole file, a few seconds for a large one, so save the output once and query the file.

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
- **Assets**: `assets.json` maps `imageRef` (original image fills, keep each node's crop and scale from its fill), `node:<id>` (an SVG of one vector) and `render:<id>` (a 2x PNG of a node that composites an image through a blend mode, mask or effect) to files under `assets/`. An icon instance is usually several vectors, each its own SVG: stack them inside a box of the icon's size, each at its own bounding-box offset from the icon's.
- `visible: false` nodes are not drawn; skip them.

## Images and slices

`figlens export <file.fig> <node> <out.png|jpg|webp|avif> [--scale 2] [--quality 80] [--max-kb 150]` renders one node like Figma's Export panel, shadows included; instance sublayer ids work. The extension picks the format. `--max-kb` lowers the quality until the file fits, so it has no effect on lossless PNG; JPG has no transparency and composites on white. It prints `{path, width, height, bytes, quality}`; `quality` is the one actually used.

## Many frames

`figlens bundle <file.fig> --frames <list> --out-dir <dir>` decodes once and writes one bundle per id in `<list>` (one id per line), in parallel, printing a JSON line per frame (`{nodeId, path, warnings}` or `{nodeId, error}`). A single `bundle` prints the same object; non-empty `warnings` name assets the bundle could not include. Use it instead of one `bundle` call per frame.

## Known gaps

- Not emitted yet: constraints, layoutAlign/layoutGrow, layout grids, componentPropertyDefinitions.
- Hidden overridden text can carry a stale size.
- Renders skip background blur and image contrast/temperature/tint/highlights/shadows adjustments, and draw angular/diamond gradients as radial; check those areas with the designer rather than matching the reference there.
- Font files are not in the `.fig`: name the fonts the design uses, and report the ones the project lacks instead of substituting silently.
