# Comparing a screenshot with the design

`figlens compare-image <file.fig> <node> <screenshot.png> [--scale N] [--limit N] [--out diff.png]` (MCP `compare_image`: `file`, `node`, `image`, `scale`, `limit`, `out`) measures a screenshot of the built screen against the node as figlens renders it,, so visual checks can be "Proven by a tool". `compare` diffs a REST `nodes.json`, not pixels. Code: `src/compare_image.rs`; CLI and MCP share `command::compare_image`.

## How it compares

- The node is rendered over its layout box (else the export area), cut to it: a screenshot shows the box, not shadows past it. Scale = screenshot width / node width, or `--scale`; one narrower than the node, or more than 1 px off `round(width × scale)`, is refused.
- A taller screenshot is compared over the node's height, the rest reported as `uncompared.below`. One shorter by over 1% (and 2 px) is refused with a full-page hint, so a viewport capture cannot score only the visible part.
- Both images are composited over white. A pixel differs when no pixel within one CSS pixel (`ceil(scale)` device pixels, at most 4) in the other image is within 48/255 on every channel, checked both ways: anti-aliasing and sub-pixel edges drop out.
- Differing pixels are counted per 8 × 8 node-pixel cell; 8-connected cells form regions, and regions under 12 node px² are dropped. `mismatch` counts every differing pixel; `nonTextMismatch`/`textMismatch` split the kept regions' pixels by cell.
- Each cell is blamed on the smallest of the deepest visible layers (boxes cut by clipping ancestors) overlapping its pixels; a region names up to three layers with the most. Otherwise a background rectangle topped most regions, and a 1 px ring around a frame covered every layer. A cell over a TEXT layer is a text cell; a region with at least 90% text-cell pixels is `text: true`: font fallback, rasterization and line breaks, not layout.
- `warnings` are the node's known skipped or approximated rendering, as in `export`. `--out` writes the washed-out screenshot, differing pixels red, regions outlined red (layout) or blue (text), via `output::write`; the design and the screenshot are refused as outputs.

The screenshot is untrusted: at most 64 MiB is read, and the PNG signature and IHDR size (at most 25,000,000 pixels) are checked before decoding, then the decoded size. The render at that scale has the same cap. Decoding uses resvg's `tiny_skia`, no new dependency.

## Measured (2026-11-01)

| Input | mismatch | nonText | regions (non-text) |
|---|---:|---:|---:|
| Fixture `0:1` render vs itself at 1x/2x; a private 1440 × 1024 frame vs its render | 0 | 0 | 0 |
| Browser build of a private chat screen, fallback fonts, light / dark | 1.61% / 1.54% | 0.33% / 0.35% | 30 (5) / 27 (6) |
| Browser build of a private dashboard, fonts loaded, corrected | 0.45% | 0.14% | 49 (21) |
| The same build's first pass (content offset vertically) | 7.06% | 3.03% | 102 (77) |

The chat build's largest non-text region is a 1 px border drawn inside the frame where the design's stroke is outside it, blamed on the frame's edge layers. Deliberate edits are found: a 76 × 37 button moved 14 px down in a render gives one non-text region on its layers; moved 20 px left in the browser screenshot, a 326 px non-text region on it; the send button's red and blue swapped, a 964 px one on its layers.

Thresholds 16/24/32 raised the chat build's `nonTextMismatch` to 1.49/1.30/0.80% and the dashboard's to 1.34/0.38/0.18%; 48 keeps faithful builds low and still separates the first pass. Cost: differences under 48/255, such as that button's light gray border on off-white, are not seen; check colors with `inspect --colors`.

## Limits

Not perceptual; one viewport and state per screenshot; crop browser chrome or a wider page to the frame. A long thin difference gets a large box: read `pixels`. Agent use: skill Verification evidence.
