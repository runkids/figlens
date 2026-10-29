# Diff

`figlens diff <old.fig> <new.fig>` and MCP `diff` (`old`, `file`) compare one stored scope (`--node`/`node`, default `0:0`) of two saved revisions (#81). The engine is `src/search/diff.rs`; durable changes (#92) are to reuse it, not build another.

## Build

Each side is a full relations build ([usages](usages.md)) through `build_with` with a facet collector, so ordinary search (text-only) and usages never pay for it. Per record it keeps, in `diff::FIELDS` order, each present effective field: the instance override (`Expanded.props`, or the expansion's size and matrix) else the stored field, the same rule as usages. A field is converted to JSON (so a JSON override and the stored kiwi value digest alike), then kept as SHA-256 over key, length and bytes, plus its value when it fits 512 bytes and the record's 4 KiB. A field past `VISITS` (65,536 visits) is `unread`: never compared as equal, counted in `unread_fields`. Every visit, byte and kept value is charged to the side's `Bounds` before it is kept. A side decodes, builds and drops its tree before the next side loads; only the two indexes with their facets stay.

Facets: `text` (textData and font fields), `layout` (size, transform, constraints, stack fields, clipping), `appearance` (paints, stroke weights, effects, opacity, blend, mask, visibility, radii), `properties` (component property assignments, refs, variant specs), `structure` (name, type, parent, sibling order, hidden; from the record) and `bindings` (the record's edges compared as relation, target, state and evidence, so a removed edge is listed; an `instance_of` change adds `swap` with both mains).

## Matching and coverage

Records match by exact id within the selected pair, and the reply says `matching: {basis: node_id+kind, lineage: unverified}`: two caller-chosen files are never proven to be one Figma file. Same id with another type is `kind_changed` with `identity_uncertain`. A record missing on a side that stopped (`stop` set) is `unknown`, never removed or added. An added or removed subtree is reported at its top record with `descendants`. Removed and added top-level frames of one page that share a name, else the same rounded size and position, get a labelled `possibleMatch` (at most 200, every probe charged); they stay removed and added.

## Paging and cache

Changes are kept once per pair as a compact list (frame, record indexes, facet bits) charged to a pair budget (`PAIR_WORK`, `PAIR_BYTES`); past it the reply is partial with `diff_work_limit`. Pages slice that list in frame order (100, 1-500), and each frame's summary counts the whole list, so a frame split across pages still reports its full count. Field values are read only for the page, within 256 KiB; over it a value says `truncated`. The cursor binds the version, both sourceIds, snapshots and built signatures (generation and stop) and the node; another pair is `cursor_stale`.

The process keeps one pair (`PAIR`), replaced when a call names other paths, stamps or node, rechecked against both stamps on every page and rehashed after `REHASH`; a deadline-stopped side is never cached. MCP pages reuse it; each CLI call is a new process, so a CLI cursor rebuilds both sides. MCP admits `old` like `file` (checked against `--root` on admission and every page); `gs://` sources are refused.
