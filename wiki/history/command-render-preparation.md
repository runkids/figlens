# Shared command execution and rendering preparation (2026-10-22)

## Changes and decisions

- `src/command.rs` owns the binary-private commands and their text or JSON results; CLI and MCP share execution. Schema-shaped MCP arguments avoid the argv and stdout JSON round trip. Legacy coercions remain for out-of-schema arguments so this refactor preserves existing behavior. Batch, comparison and update stay in `src/main.rs`.
- `render::Prepared` owns expanded render input and the selected subtree's image bytes. Raster exports and bundle references share export bounds, downscaling and backdrop preparation. Bundle vectors retain full images, composites retain their 2x render bounds without a backdrop, and asset ordering stays unchanged.
- No dependencies, cache or renderer fidelity changes were added. The wiki's stale `capturedAt` name was corrected to the actual manifest field, `createdAt`.

## Verification

- `docker compose run --rm dev cargo test`: 56 library tests and 8 binary tests passed. New tests cover CLI/MCP success and error parity, legacy coercions, expanded sublayer selection, image loading and page backdrops.
- `docker compose run --rm dev sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings'`: passed.
- Before/after executables on `samples/mcp-apps.fig`: three bundles (234 entries total) matched in entry order and contents, excluding only `manifest.json.createdAt`. Frame and instance-sublayer PNGs were byte-identical. A derived input with one image removed produced the same 191-entry bundle, missing-assets report and warnings.
- Evidence stays in the ignored `samples/render-prep-evidence/` directory. Only one independent `.fig` was available; the missing-image derivative is not a second design. This proves behavior preservation on the tested inputs, not improved accuracy against Figma.

## Test hardening (2026-10-22)

- Reproduced a fixture ownership failure: a test reused and deleted a pre-existing directory with the same PID-derived name. Command and rendering fixtures now share a test-only directory guard that atomically claims a free name and cleans up its own directory. Concurrent Docker tests preserved a pre-existing owner marker.
- Bundle assertions inspect CLI and MCP ZIPs separately for the renamed root, inferred file key, required entries and stripped render input. Variables command tests use a bound and an unused local variable to distinguish global from frame-scoped results through both parsers.
- Isolated source copies that deliberately ignored the MCP bundle name or variables node argument each failed the corresponding regression test. The final checkout passed 57 library and 10 binary tests, format and clippy checks; no runtime behavior or dependencies changed.
