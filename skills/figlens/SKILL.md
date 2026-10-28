---
name: figlens
description: Plan, build and audit UI from local Figma .fig files using figlens CLI or MCP. Use when given a .fig file or a screen name to implement or plan, comparing an existing UI with a design or fixing layout differences, a Figma URL with a local copy, requests to extract design assets or tokens, or when Figma API access is unavailable. In Pi, use its automatically registered figlens MCP tools.
---

# figlens

`figlens` reads a `.fig` file (Figma: **File > Save local copy**) and prints REST-shaped snapshot data for a node: layout, text, styles, variables, instances already expanded. It also renders any node and writes a **bundle**: one zip with everything needed to build a frame. The `.fig` is a **snapshot**: it shows the design as it was when saved, so a newer design needs a newer copy.

## Setup

**In Pi with the figlens extension:** use its already registered figlens MCP tools (`tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `export`). They use the package's own launcher and cached native binary. No global CLI, shell `npx`, `/figlens open`, or saved selection is needed. The CLI examples below describe the equivalent operations: pass each argument through the corresponding tool schema, using absolute `file` and output paths. The tool namespace is supplied by the host.

**Other agents:** use configured figlens MCP tools when available, or the CLI below.

**MCP without a local skill installation:** the server's `workflow` tool returns built-in guidance from this same skill. Choose `build` for design reading and project reuse, `audit` for comparing existing UI with a design and correcting confirmed differences, `images` for website asset preparation, `plan` for scoped planning, `planning-details` for page/multi-page work or unresolved dependencies, and `yolo` only for explicitly requested autonomous implementation and temporary-plan cleanup. These are read-only guides, not commands that implement or verify a project. Follow `workflow:` links by requesting their topic from the same tool. Project edits, browser checks and filesystem permissions belong to the host agent.

Prefer `npx @runkids/figlens` for one-off use (Node.js 18+, no global install). The examples below use `figlens` as shorthand: replace that prefix with `npx @runkids/figlens` when no standalone binary is installed. The first run downloads and verifies the matching native binary; later reads use its local cache.

Other installation options (no Rust or Docker needed):

```sh
curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh   # macOS, Linux
irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex       # Windows PowerShell
npx @runkids/figlens --help                                                                 # or run through npm
```

## Choose the workflow

For comparing existing UI with a design, visual acceptance or correcting layout differences, read [the audit workflow](references/audit.md). A review-only request stays read-only; fix only when requested. CLI and MCP expose the same guide as `workflow audit` and `workflow(topic="audit")`; Pi uses the shared skill and MCP guide without a separate audit command.

For an explicit request to plan, implement, self-check and clean up the temporary plan autonomously, read [the YOLO workflow](references/yolo.md). Pi offers `/figlens yolo <request>`; other agents follow the same skill workflow from the user's request. Keep this opt-in intent distinct from plan-only requests and ordinary design questions.

For an explicit implementation plan or a request to carefully reproduce a design, read [the planning workflow](references/planning.md). Scale it to the requested region, component, state, page or pages. Plan-only requests end with the plan; an already authorized build continues into implementation. Pi also offers `/figlens plan <request>` as a plan-only shortcut, without open or a saved selection. Keep ordinary design questions and asset exports on the direct path below.

## Where outputs go

`export`, `bundle` and `preview --out` write only where you point them. Choose by purpose:

- A path the user names wins.
- Assets the site ships go where the project keeps its assets, following its conventions (see Images and slices).
- Files for your own checking (bundles and their extracted files, reference images, comparison screenshots, scratch SVGs) go in `.figlens/exports/` at the project root, beside the plans and previews figlens keeps in `.figlens/`. When creating it, add `.figlens/exports/.gitignore` containing `*`: the directory stays out of Git without editing the project's ignore rules, while `.figlens/component-mappings.md` is still committed.
- Do not write beside the `.fig` or elsewhere in the project unless the user asks.

Over MCP, `out` is absolute and must be inside the configured roots; `.figlens/exports/` under the project root is.

## Building a frame

1. **Get the `.fig`.** Ask the designer for File > Save local copy when there is none. For a `gs://` handoff, follow the [GCS snapshot cookbook](https://github.com/runkids/figlens/blob/main/wiki/gcs.md) for authorized access, refresh and preview limits. Done when the file opens: `figlens tree <file.fig> --depth 1` lists its pages.
2. **Find the node yourself.** A Figma URL (`...?node-id=12-345`), `12-345` and `12:345` all work as-is. Without one, use `figlens find <file.fig> "screen name" --type FRAME` or search visible copy with `--type TEXT`. Search is a case-insensitive substring over stored layer names and text; `--node <id>` scopes it and `--limit N` sets 1-200 results (default 20). Inspect candidate paths, dimensions, visible copy and reference renders to resolve the user's intent. If ambiguity remains, ask once with a short list of recognizable page/screen names and their differences or images. Resolve IDs internally. `truncated: true` means refine or scope the search. Instance children appear only after expansion: use `figlens inspect <file.fig> <instance-id>`. `tree --depth 2` is only an initial outline; descend into relevant containers or use `find` for nested screens. Done when you have identified the intended screen, without requiring a manual open or picker.
3. **Bundle it.** `figlens bundle <file.fig> <node> .figlens/exports/frame.zip`, then unzip into its own directory under `.figlens/exports/` and read `context.md` first: it says what each file holds. Done when you know the reference image, the structure file and the asset map. Check `handoffDiagnostics` in inspect and `_figlens.handoffDiagnostics` in nodes for known stored semantics not emitted; bundle warnings summarize them. Their counts concern the selected expanded subtree, with explicit hidden/SLOT coverage limits, not proof of complete semantics. Inspect includes recorded text resize, truncation, max lines and decoration. Keep outputs distinct from the source. On `response_budget_exceeded`, use inspect, a scoped bundle, CLI nodes or a smaller export `maxSize`; `outputPath` in error metadata identifies a file already written.
4. **Outline before reading.** A screen is hundreds of KB of JSON, too large to read whole. Print the tree with sizes and layout, then read the subtrees you are building with `jq`:

   ```sh
   jq -r 'def o(d): "\("  " * d)\(.id) \(.type) \"\(.name)\" \(.absoluteBoundingBox.width // 0 | floor)x\(.absoluteBoundingBox.height // 0 | floor)\(if .layoutSizingHorizontal then " \(.layoutSizingHorizontal)/\(.layoutSizingVertical)" else "" end)\(if .layoutMode then " \(.layoutMode) gap=\(.itemSpacing // 0)" else "" end)\(if .layoutPositioning then " ABSOLUTE" else "" end)", (.children[]? | o(d + 1)); .document | o(0)' design/nodes.json
   jq '.. | objects | select(.id == "12:345")' design/nodes.json     # one node in full
   ```
5. **Build from the values, with the project's own parts.** Use the font and asset preflight below to flag missing inputs while continuing implementation. Read sizes, padding, gaps, radii, colors, shadows and fonts from the JSON; use the reference to check visual relationships and an implementation screenshot to check the browser's actual result. These sources do not replace one another. Before writing markup, look at what the project already has: a screen built from raw values next to an existing component library is a second source of truth, and it drifts on the next design change.
   - **Components:** list the design's instances (`components` in `nodes.json`, the `I<instance>;…` ids, `componentProperties` for variants) and search the codebase for their counterparts, by name first (`Button`, `Card`), then by role (a button with `variant`/`size` props). Use the project's component with its props, set only what the design sets, and keep its defaults. Write raw markup only for parts with no counterpart, in the project's style. When the design and the component disagree (a radius the component does not offer), use the component and report the difference instead of forking it.
   - **Tokens:** `variables.json` holds the design's variables and modes. Match them to the project's tokens (CSS custom properties, theme object, Tailwind config) by name, then by value, and reference the token rather than the resolved number. Add a token only when nothing matches, and say so.
   - **Conventions:** framework, styling approach, file layout and naming. The new screen should read as if the team wrote it.
   - **Sizing intent:** before writing layout CSS, note each auto-layout child's `layoutSizingHorizontal`/`layoutSizingVertical`. HUG sizes to its content, FILL takes the parent's remaining space (grow on the parent's main axis, stretch on the cross axis), and only FIXED gets an explicit size. The reference shows one copy at one width, so copying its pixel widths onto HUG or FILL boxes matches the screenshot and breaks when copy, locale or container width changes. A growing text slot also needs to shrink: give it `min-width: 0` and decide whether long copy wraps or is clipped, so it cannot push past its parent.

6. **Prepare website images.** Inspect the target project's image components, asset directories, framework/loader/CDN configuration and neighboring pages; follow its established approach and applicable instructions. Use its existing optimization pipeline when present. Otherwise follow Images and slices below. Done when the deployed images have suitable formats and pixel dimensions for their displayed sizes, with loading behavior and byte sizes checked; bundle originals remain source material.
7. **Check against the reference.** Apply Verification evidence below, rendering the build at the frame's size and comparing it with `references/screen.png` side by side or as a pixel diff. Check image requests at relevant desktop/mobile widths: transferred bytes, oversized images, layout shifts and the main image's loading behavior. Correct confirmed differences and rerun the exposing state; use [the audit workflow](references/audit.md) for an existing UI or an unresolved mismatch. Finish with a supported result or explicit evidence gap for each scoped item; unresolved fonts, assets, rendering gaps or unavailable browser checks leave affected visual acceptance pending.

For a question about layout, typography or copy, start with `figlens inspect <file.fig> <node> [--depth N]`: expanded bounds, auto-layout and typography, without paints or vector geometry. Depth defaults to 1 (0-10); at most 200 nodes and 200 characters per text are returned. `childCount` includes omitted children; `truncated` marks omitted descendants and `textTruncated` marks shortened text. For image decisions add `--images` (MCP `images: true`): `images.items` reports source dimensions/bytes and each paint's source-pixel crop rectangle before masks/effects/clipping, with a separate 200-paint limit and `images.truncated`. Text-range fills carry `textStyleId` matching the node's `styleOverrideTable`; used vector-region fills carry `vectorStyleId`. Missing/unrecognized sources and unsupported rectangles are explicit; these counts do not diagnose blur. Use `nodes` for full text, paints and masks, and `variables` for bound variables. Each CLI call decodes the whole file, while MCP reuses the last unchanged local file (remote calls parse again); reuse saved output for repeated CLI queries. Across several `.fig` files, finish all queries on one file before switching, because each switch reloads. For several frames of one file, use `nodes --frames` (REST nodes only), `bundle --frames` or consecutive MCP calls in the same process.

When the user asks for related component states, add `--states` to `inspect` or `bundle` (including batch bundles); over MCP pass `states: true` on those tools. It defaults to false and needs no MCP configuration change. Keep `source.file` with every returned ID. Check each relationship, property support state, prototype inheritance/precedence, hidden coverage and truncation before describing behavior. `actionsTruncated: true` means omitted actions, even if the projected list is empty. Bundles include `design/states.json` and a bounded `context.md` summary; other variants' structures/assets require a separate inspect on the same source file. Names and prototype links do not establish business conditions. See [state evidence](https://github.com/runkids/figlens/blob/main/wiki/states.md) for scope and limits.

When the user asks how two variants differ, inspect the first definition with `--compare-variant <other-id>` (MCP `compareVariant`). Both must belong to the same component set. Read `variantComparison.rootChanges`, descendant `collectionChanges`, `unpairedNodes`, `unknownFields` and its independent truncation before describing differences. These are stored fields/value multisets, not paired descendant changes, alternate instance renderings or pixel fidelity; use returned source IDs for further inspection.

## Verification evidence

Use design data for structure and values, a design reference for visual relationships, and an implementation screenshot for the browser result. Record the actual viewport and state for both images; distinguish figlens renders from supplied Figma exports and check Known gaps before relying on them. A screenshot at one width or state covers only that case.

Check all three layers within the requested scope:

- **Elements:** dimensions, spacing, typography, colors and assets, including effective inherited and responsive styles.
- **Relationships:** alignment, columns/distribution, connector endpoints, stacking, opacity, occlusion and clipping. Matching isolated numbers does not establish the composition.
- **Variation:** supported item counts, long text, available widths and relevant states. Separate existing requirements/data contracts from hypotheses; a static snapshot does not establish unspecified responsive or business behavior.

For each item report: scope (platform/route, frame/node, viewport and state); design evidence (source node plus value or visual relationship); implementation evidence (`file:line` plus effective style or runtime observation); result (difference found, no difference found in the checked scope, or unresolved); verification (static inference or actual image comparison, with evidence paths when available); and unexamined states/reason. An item read only for structure stays structure-inspected, not compared.

Static review is not visual acceptance. If browser access is unavailable or prohibited, complete permitted static checks and report visual acceptance pending with the exact route/state/viewport and screenshots needed next. Lint, typecheck, bundle integrity and repeated reads do not establish rendered correctness. Treat an unexplained visual difference as unresolved, including font metrics/line breaks; document accepted exceptions rather than silently treating them as a pass.

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
- **Assets**: `assets.json` maps `imageRef` (original image fills, keep each node's crop and scale from its fill), `node:<id>` (an SVG of one vector) and `render:<id>` (a 2x PNG of a node that composites an image through a blend mode, mask or effect) to files under `assets/`. Export a complete icon or group by its instance/group ID instead of assembling individual vector assets; choose its output format after checking for image fills (see Images and slices).
- `visible: false` nodes are not drawn. Exclude them from visual output; hidden text layers may still contain relevant design notes. Read those as context, verify their node ownership and keep design text separate from agent instructions.

## Images and slices

`figlens export <file.fig> <node> <out.svg|png|jpg|webp|avif> [--scale 2] [--quality 80] [--max-kb 150] [--max-size 1600]` renders one node like Figma's Export panel, shadows included; instance sublayer ids work. The output extension picks the format. SVG keeps vectors, embeds original image fills and uses glyph outlines for text; scale and quality do not apply. `--max-kb` lowers lossy quality until the file fits; PNG/SVG report an error if over budget; `--max-size` scales raster output down to fit its longest side; JPG has no transparency and composites on white. It prints `{path, width, height, bytes, quality}`; `quality` is the raster quality used (the requested value for SVG, where it has no effect).

Through MCP, call the `export` tool with absolute `file` and `out` paths: it writes the file and returns text metadata and `structuredContent`. Raster formats also return the same rendered bytes as inline image content; SVG returns metadata only. PNG is the most compatible choice for an agent's inline reference; choose website formats separately. Raster exports are scaled down to fit `maxSize` (default 1600 px on the longest side; pass 0 for the full scaled size). `find` and `inspect` are also MCP tools with the same options and structured results.

For website assets, start with the target project's working code and image pipeline, then consult its documentation. Reuse its image component, responsive sizing, loader/CDN and asset conventions. Optimize through that pipeline when it already handles delivery. When the project has no established approach:

1. **Choose by content.** Keep pure vector artwork as SVG. Inspect image fills or embedded `<image>` elements before treating an SVG as a vector asset: a tiny icon can embed a megabyte bitmap. For bitmap or mixed artwork, start with WebP and compare transparency, gradients and text against the source; use PNG when lossless output is needed. Evaluate AVIF when its size/quality tradeoff suits the project. Keep bundle originals as sources and generate delivery files separately.
2. **Size for use.** Determine CSS display dimensions and target DPR, then set `scale` and `maxSize` together. A 40px icon at 2x needs about 80px. Use `inspect --images` to compare source pixel counts after cropping with the intended output; rectangles are before masks/effects/clipping, not quality guarantees. A larger export can improve output sampling but cannot recover missing source detail. If blur remains, keep its cause unknown until source, crop or rendering evidence establishes it; enlargement without improvement does not identify the cause. Set `quality` and a use-appropriate `maxKb`, then inspect returned `width`, `height` and `bytes`. `maxKb` lowers lossy quality, not dimensions; revise dimensions or budget when needed.
3. **Export and handle errors.** Create output directories before exporting. For a 40px bitmap icon at 2x, an example starting point is:

   ```sh
   mkdir -p public/assets
   figlens export design.fig 12:345 public/assets/icon.webp --scale 2 --max-size 80 --quality 80 --max-kb 20
   ```

   The equivalent MCP arguments are `file`, `node`, absolute `out` ending in `.webp`, `scale: 2`, `maxSize: 80`, `quality: 80`, `maxKb: 20`; substitute the real file/node/project paths. These dimensions and budget are examples, not universal limits. Check `isError` and read `content` on failure before retrying; successful transport alone does not mean an export succeeded. Use figlens's existing raster export before adding conversion tools unless a concrete limitation requires them. Export the selected node when its crop, mask or effects must be baked in.
4. **Deliver and measure.** Provide responsive variants with `srcset`/`sizes` (or responsive CSS backgrounds) for large images. Reserve space with dimensions or aspect ratio; this reduces image-induced shifts, while CLS still requires measurement. Choose loading explicitly: preserve default loading for a small inline icon, lazy-load offscreen content, and prioritize an actual above-the-fold/LCP image when evidence supports it. In the browser, check the requested asset, decoded dimensions and transferred bytes as well as sharpness, transparency and crops at relevant widths. Export-file bytes and browser transfer bytes are separate measurements. Measure LCP/CLS when browser tooling allows, and report unavailable measurements. A successful build or an empty broken-image list does not establish efficient image delivery.

## Visual preview and handoff

Use `figlens preview <file.fig> [node]` when a person needs to browse or select a scope. In Pi, `/figlens preview [description]` resolves the design conversationally and starts its session-owned viewer through `figlens_preview`; no manual open is required. Open the printed local URL. The viewer provides page/frame browsing, name/text search, a synchronized layer tree, bounds-based overlap picking and property inspection. Download a selected-scope bundle or copy IDs/context without guessing them; expanded `I<instance>;…` IDs remain exact and work with `bundle` and `export`. The implementation prompt is grounded in design data, not an automatic mapping to project components. To drive an open preview from a browser-automation tool, call the page's `window.figlens` API (`state()`, `select(id)`, `context()`, `exportImage(...)`; see [the Agent API](https://github.com/runkids/figlens/blob/main/wiki/preview.md#agent-api)) instead of clicking through the UI.

For a portable handoff, run `figlens preview <file.fig> <node> --out preview.html`. The HTML works offline and embeds design data, original images and the snapshot-scope bundle. Selecting a child changes context, not the embedded bundle's scope. Share it only with people allowed to see those assets. Import a local implementation screenshot for side-by-side or opacity comparison; this is visual checking, not measured accuracy.

Live preview polls the saved local file. Treat a stale/error banner as unavailable fresh evidence; a failed save must not be described as a refreshed design. Snapshots never refresh. For `gs://` inputs the live viewer watches its local cached copy, not the remote object. No viewer action replaces checking the bundle's `missing-assets.json` and the known renderer gaps below.

## Pi component mappings

When the figlens Pi extension is available, `figlens_bindings` can retrieve saved component matches. Supply the **current request's `file` and `frame` together**; this works without open and leaves any manual selection unchanged. The current request determines the scope even if a different selection was saved earlier. It returns actual instance properties/text, approved rules, pending/stale candidates and pagination. Follow `nextOffset` when more instances are needed. Reuse recorded mappings marked `approved`, using current design text rather than example literals. Empty, unmatched or stale results do not block implementation: inspect source and design directly.

When the user wants to save reusable component matches, read the real target implementation and usage first. Call `figlens_propose_bindings` with explicit file/frame, its instance ID, target path/export/import/example, rationale, supporting file dependencies, checks actually executed and unresolved issues. Plain HTML can name an existing block and explain that no import is needed. The user reviews with `/figlens review`, which lists pending proposals across this project without open; proposals stay pending in non-interactive modes. Approval is a mapping decision, so run the project's permitted compilation/runtime checks and compare the screen separately. The [walkthrough](https://github.com/runkids/figlens/tree/main/pi) documents installation and optional commands. Without the extension, continue with the CLI workflow above.

## Many frames

`figlens bundle <file.fig> --frames <list> --out-dir <dir>` decodes once and writes one bundle per id in `<list>` (one id per line), in parallel, printing a JSON line per frame (`{nodeId, path, warnings}` or `{nodeId, error}`). A single `bundle` prints the same object; non-empty `warnings` report missing assets, known skipped/approximate rendering and known unrepresented handoff semantics. `missing-assets.json` contains only asset failures; `_figlens.renderDiagnostics` in nodes locates render limitations. Use it instead of one `bundle` call per frame. When only REST nodes are needed, `figlens nodes <file.fig> --frames <list>` (`-` reads stdin) or `figlens nodes <file.fig> --ids <node>...` decodes once and prints one JSON line per id: the single `nodes` output with its `nodeId`, or `{nodeId, error}` (at most 10,000 ids). Over MCP, call `nodes` per frame for a local file, whose parse the server reuses; a `gs://` file parses again per call, so batch it with the CLI.

## Font and asset preflight

- **Fonts:** start from the bundle's `design/fonts.json` (or `verify`), which lists the families, styles and weights visible text uses, including applied overrides, with node ids; locate licensed project font files or verified system faces. The bundle contains outlines, not installable fonts. For web fonts, use `document.fonts.load` with the actual design text, await `document.fonts.ready`, and verify the returned faces and requested weight/style. An empty array or `document.fonts.check` alone does not prove availability: fallback can pass the latter. Verify system fonts separately rather than treating an empty web-font result as proof of absence.
- **Assets:** read `missing-assets.json`, bundle warnings and `assets.json`; run `verify` (CLI `figlens verify <bundle.zip|dir>`) to check required files against the manifest's sizes/hashes; it reports missing and changed files and leaves fonts unverified. Before visual acceptance, check the deployed image URLs, decode success and rendered dimensions. An empty missing-assets list proves neither font availability nor correct browser delivery.
- **Continue:** record each dependency as verified, missing or unverified, with its evidence. Continue implementation without waiting for missing inputs: use available project fonts as documented fallbacks and clearly labelled image placeholders at the intended dimensions. Report the exact family/weight or asset/node, temporary choice and remedy in the handoff. Leave final visual acceptance pending for unresolved inputs; browser checks that could not run stay unverified. A successful bundle command does not prove visual fidelity.

## Known gaps

- Native Dev Mode annotations are not emitted. Hidden TEXT notes and component descriptions are different sources; their presence does not establish annotation coverage. If a missing annotation affects implementation, request its wording/export and associated node from the user, record that provenance and leave the affected decision unresolved rather than guessing.

- Layout grids are not emitted. Recorded constraints, layoutAlign/layoutGrow and componentPropertyDefinitions are partial: read `_figlens.semantics` for source and recorded/absent/unsupported status. No absent defaults, complete variant options or SLOT semantics are inferred; these mappings have not been compared with same-version Figma exports. See [coverage](https://github.com/runkids/figlens/blob/main/wiki/semantics.md).
- Hidden overridden text can carry a stale size.
- Read `_figlens.renderDiagnostics` in nodes/bundles before matching reference pixels; `inspect` includes at most 20 per shown node and marks truncation. Absence of diagnostics does not establish fidelity. Renders skip image contrast/temperature/tint/highlights/shadows adjustments and glass effects, and draw angular/diamond gradients as radial; background blur blurs only what the same export draws behind the layer, so export the frame that contains the backdrop. Check those areas with the designer rather than matching the reference there.
- Font files are not in the `.fig`: name the fonts the design uses, and report the ones the project lacks instead of substituting silently.
