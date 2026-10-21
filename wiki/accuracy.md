# Accuracy: measuring figlens against Figma

Load this to measure how close figlens's output is to Figma's, to run `compare`, or to find test files.

## Test files

- Design files never enter the repo; put them in the gitignored `samples/`.
- Figma Community files work well as unfamiliar input: open one in Figma (Open in Figma copies it to your drafts), then File > Save local copy. Files used so far: MCP Apps for Claude, Aurora Design System, Landing Page Free, Nucleus UI Lite, Real Estate App UI Kit. Check each file's license before sharing outputs.
- A new behaviour is trusted only after it ran on a file it was not written against: each community file above found a bug the first file never hit.

## Getting a reference

- Structure: export the same frame from a Figma plugin with `node.exportAsync({ format: "JSON_REST_V1" })` (the "plugin" export), or read it from the REST API (`GET /v1/files/:key/nodes?ids=<id>`). Save the plugin's `document` wrapper as `nodes.json`.
- Variables: a plugin can write `design/variables.json` from `figma.variables`; `compare` diffs it when given as a third field.
- Pixels: export the frame and its assets from Figma (Export panel or `exportAsync` PNG/SVG) next to the JSON.
- Export the `.fig` and the reference on the same day, so design edits do not show up as differences.

## Running compare

```bash
docker compose run --rm dev cargo run --release -- compare samples/design.fig samples/ref/nodes.json 12:345
# many frames: one `node|nodes.json[|variables.json]` per line
docker compose run --rm dev cargo run --release -- compare samples/design.fig --frames samples/frames.txt
```

It reports per frame: ids missing on either side, then name, type, visibility, bounding box, characters, fills and the emitted fields in `compare::FIELDS`, and a summary of identical frames per field.

## Results

- Sweep of a large production file (2026-10-22, 826 frames with plugin captures): structure (id/name/type/visible/bbox/characters) identical in 771 frames; blendMode, effects, rotation, layoutWrap, isMask, componentProperties 826; boundVariables 796; styles 798; renderBounds (1 px) 759; `variables.json` 783; text `style` 792, `styleOverrideTable` 820, `characterStyleOverrides`/`lineTypes`/`lineIndentations` 826. Pixels ("ok" = at most 1.5% of pixels off by more than 48/255): image fills 2608/2608 byte-identical, SVG assets 5014/5197 ok, reference renders 659/825, render assets 88/221. The remaining differences traced mostly to design edits between capture and `.fig` export and to the renderer gaps in the `direction` topic.
- Community files (2026-10-22): all 306 top-level frames of the five files bundle with no missing assets. A hand-written HTML/CSS page built only from one bundle's `nodes.json` and SVGs (MCP Apps for Claude, "List views") matches the bundle's reference to a mean of 5.2/255 with 4.25% of pixels off by more than 32, all of it text antialiasing and line breaks.
