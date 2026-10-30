# Visual retrieval core

`src/search/visual.rs` supplies raster comparison primitives. It does not expose a CLI, MCP or viewer visual query yet; the public workflow remains tracked in #95. The `siglip-base` model profile and the model child's image requests are implemented ([semantic](semantic.md)). Exact SVG outline lookup remains `export::SvgIndex` and is a different comparison.

## Units and admission

`units` walks a full prepared `search::Index` under `expand::Bounds`, selecting screen kinds below pages/sections, component roots and explicit subscopes, in document order. It returns borrowed ids, record ids and visibility metadata. Hidden/internal flags are preserved for retrieval filters rather than dropping potential units during extraction. A render failure must be reported on that unit without removing its text record. A partial index or stopped walk never proves absence.

`QueryImage` admits PNG only: encoded bytes at most 8 MiB and decoded pixels at most 4,000,000, checked before decoding. The entry point must also bound its read to the byte cap plus one. Crops are positive integer pixel rectangles within the source. Crop before filtering, composite on white, aspect-fit to 256 by 256 with bilinear sampling; `visual-hash-1` pins this policy. Re-encoding the normalized raster strips source metadata. Query rasters remain request-owned and are not persisted by this module.

`render_unit` reuses the export renderer and original-image byte budgets, reading image paints from the same staged bytes used to decode the tree. It renders export bounds at at most 1024 pixels on the longest side, then applies the query normalization. It returns the export bounds and at most 200 known render/missing-image diagnostics, with a truncation flag. These diagnostics are not an exhaustive fidelity audit. File exports keep the same renderer path and behavior.

## Fingerprints and ranking

A fingerprint keeps the normalized pixel SHA-256 with reduced source aspect, 128 luma-gradient bits and mean RGB. Its fixed-size cache blob has an explicit version and rejects invalid dimensions. `visual-hash-distance-1` ranks equal normalized pixels first, then gradient, color, aspect distance and record order, retaining only bounded top-k candidates. Callers supply only authorized, filtered units from the requested generation. Work stops are partial coverage, cancellation is an error, and distances are not probabilities or component identity.

## Optional vision tower

The `embed` feature contains the CPU SigLIP vision tower at `google/siglip-base-patch16-224` revision `7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed` ([model card](https://huggingface.co/google/siglip-base-patch16-224)). The loader verifies the pinned weight SHA-256, uses safe buffered loading and retains only vision tensors. Its policy adds bilinear 224 RGB sampling, scaling to [-1,1], pooled output and L2 normalization; this differs from the upstream processor's bicubic resize and has its own version. A batch holds at most four images.

`scripts/visual-preflight.rs` and `scripts/visual-reference.py` compare the tower's forward output against the offline transformers reference on identical normalized NCHW tensors, including batch/single parity. This is not upstream image-processor parity or retrieval-quality acceptance. `models configure siglip-base` records the vision profile, and its child's vectors through the image protocol equal this tower run in process on the same rasters (a local check on the pinned weights). Image query and index routing, held-out quality, native performance and installed-host acceptance remain pending.

## Stored source rasters

Workspace schema 10 adds `visual_rasters` and `visual_units`. Normalized source thumbnails are content-addressed under the render policy; each generation retains its own record id, bounds, known diagnostics and render error. A failed render does not change its text record. Generation cleanup preserves shared rasters until their last reference is gone. Stored PNGs are capped at 300 KiB and evidence at 256 KiB per unit; a visual corpus holds at most 2,000 units. Query images never enter this cache. These tables are internal preparation support; the public image-query workflow is not exposed yet.
