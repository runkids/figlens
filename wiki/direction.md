# Direction

Load this to decide what to work on next or to revisit an architecture decision.

## Next steps

1. `nodes` emits partial constraints, layoutAlign/layoutGrow and componentPropertyDefinitions with provenance; [same-version Figma calibration remains pending](semantics.md). layoutGrids are still omitted.
2. Text reflow for overridden text (stale derived sizes), only if a same-day reference shows it is real.
3. Renderer gaps (`src/render.rs`; the ones in code carry `shortcut:` comments): image contrast/temperature/tint/highlights/shadows adjustments (the largest remaining reference difference), background blur of content outside the export, through ancestor masks/clips/opacity/blend modes or of an earlier blur, GLASS effects, angular/diamond gradients drawn as radial, TILE drawn as FILL, JPEG fills not pre-downscaled, INSIDE strokes doubling edge antialiasing (small ellipses).
4. Batch mode: a stack overflow aborts the whole process, which `catch_unwind` cannot stop; run frames on threads with a guarded stack or in child processes before trusting unknown files in one batch.
5. Regression CI on community files, so a Figma format change is caught and fixed quickly.

## Background decisions

- Goal: AI design-to-code unconstrained by Figma (no API rate limits, no login, no running Figma), not replacing Figma for designers (2026-10-21).
- Raster export encoders (2026-10-22): `jpeg-encoder` and `ravif` (pure Rust, `asm` off so no nasm), and libwebp via `webp` because the pure-Rust WebP encoder is lossless only and export needs a quality setting.
- Rust chosen for rendering; Go lacks 2D/CJK text shaping. The renderer draws the outlines the `.fig` already stores (shape geometry, text glyphs) to SVG and rasterizes with resvg, so no font files or text shaping are needed.
- Rejected: Figma remote MCP and REST as the read path (rate limits; REST file reads are Tier 1), a self-built Figma replacement, unreviewed `.fig` parsers.
