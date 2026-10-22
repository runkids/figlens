# Agent discovery and inline images (2026-10-23)

## Changes and decisions

- CLI and MCP expose `find` for case-insensitive substring search over stored names and text, with type/subtree filters, ancestor paths, local sizes and bounded results. Search does not expand component instance children; inspect the containing instance to read those.
- `inspect` projects the existing expanded REST result into bounds, layout, typography and text previews. Default depth is 1; the 200-node and 200-character limits reduce output, not file decoding or expansion cost. Full `nodes` and bundle output stay unchanged.
- MCP `export` returns the freshly rendered bytes as [image content](https://modelcontextprotocol.io/specification/2025-06-18/server/tools#image-content), alongside its existing metadata. CLI stdout remains metadata only. No dependencies or renderer changes were added.

## Verification

- Docker: `cargo test` passed 57 library and 15 binary tests; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` passed. Tests cover scoped text/name search, Unicode previews, deterministic truncation, type normalization, instance expansion, shared summary budgets, argument errors and CLI/MCP parity. All four image formats retain their MIME type and exact file bytes in MCP responses. Legacy PNG export coercions also return the image.
- npm: `node --test` passed 2 tests. `python3 scripts/ai-context.py check` passed.
- Release binary, Aurora (`10:200`) and MCP Apps for Claude (`467:24651`): summary fields matched the full REST output, stored text was searchable, CLI and MCP discovery results matched, inline PNGs matched the saved files byte for byte, and the MCP server continued after a missing-node error.

| Sample | Full JSON bytes / nodes | Depth 1 summary bytes / nodes | Depth 3 summary bytes / nodes |
|---|---|---|---|
| Aurora | 169,803 / 67 | 1,424 / 4 | 8,317 / 24 |
| MCP Apps for Claude | 388,361 / 153 | 1,036 / 3 | 6,566 / 19 |

Both depths reported omitted descendants. These compare CLI output sizes at different detail levels, not parsing speed or improved fidelity against Figma. Evidence remains in ignored `samples/discovery-check-qp0tiv6t/`. MCP wire responses were verified; rendering inside individual MCP client interfaces was not tested.
