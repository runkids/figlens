# Usages

Exact, revision-bound relationships over one indexed scope (issue #91, core slice). `figlens usages` and the MCP `usages` tool answer "where is this component, set, variable, style or image used" (incoming, default) and "what does this layer point at" (`--out` / `direction: "out"`). Code: `src/search/usages.rs` (extraction, query, cursor), `src/search/store.rs` (`Stored` graph over SQLite), `src/workspace/usages.rs` (workspace entry).

## Edges

Edges are extracted during the search build (`search::build` → `usages::extract`), under the same `expand::Bounds`, so a budget stop leaves coverage partial instead of dropping edges silently. Each edge is `(rid, relation, seq, target, state, basis, evidence)`.

| Relation | From | Target | State |
|---|---|---|---|
| `contains` | record `parent` | parent node id | resolved |
| `instance_of` | stored `symbolData` (basis `recorded`) or the expansion's definition/swap (`resolved`, evidence `definition`/`swap`) | main component id | `resolved`, `missing` (absent), `unsupported` (not a COMPONENT) |
| `variant_of` | COMPONENT whose parent is a COMPONENT_SET | set id | resolved |
| `binds_variable` | `rest::bound_variables` over effective fields, plus vector-region fills (`vectorRegionFills`) and a local text style's `fontSize`/`fontWeight` bindings the text does not bind itself (`styleIdForText.<field>`) | `VariableID:…` | local `resolved`/`missing`, library `external` |
| `uses_style` | `styleIdFor*`, plus `styleOverrideTable` rows a character uses (last row per styleID, as `rest::text_overrides`) | node id, or `StyleKey:<key>` for an assetRef | `resolved`/`missing`, `external` |
| `uses_image` | IMAGE fill/stroke hash, and per-range text / per-region vector fills (`styleOverrideTable`) of active rows only | `image:<hash>` | `recorded` |
| `prototype_target` | `prototype::node` recorded actions | destination node | `resolved`, `missing`, `unsupported`, `unresolved` |

Rules: a target that is not in the file stays as an explicit `missing`/`unresolved`/`external` edge, never dropped. A name is never identity: two components with one name keep separate instances. Inherited prototype links from a component definition are skipped until their inheritance is verified. Cross-file library identity is #93; a local copy of a library style is `external`, not resolved.

`definitions` stores every resolved target plus in-scope set members and their sets (name, type, set, nearest page), so a definition outside the queried scope still resolves (`definition.state: outside_scope`).

## Query and paging

- One result is one edge. A layer with two edges (fills and strokes bound to one variable) returns two results.
- Order and cursor key: `(rid, relation, seq)`, rid in document order. Cursor `u1.<fingerprint>.<snapshot>.<gen>.<rid>.<relation>.<seq>`. Another snapshot or generation → `cursor_stale` (workspace: a deleted generation → `cursor_expired`); another query → `invalid_arguments`.
- A COMPONENT_SET query returns `members` (≤ 200, `complete` says whether more exist) and the instances of the set and its members, each with `via`.
- No totals. `coverage.complete` is false after a budget stop or truncated members. Hidden instance content that was never expanded is a reason, not incompleteness. Hidden layers are left out unless requested; when the last page left any out, `hidden_locations_excluded` says so, so an empty page never reads as "unused". `layers.relations` stays `partial` with `relation_unavailable: library_identity`.
- `snapshotId` pins the revision: a mismatch is `source_stale` with `use_current_snapshot`.
- The scope filters returned locations only; definitions are looked up anywhere in the index.
- The queried id, its definition (name, type, set, page) and the set's member ids start the reply byte budget and are checked even when no row follows; results add to it, checked before serialization, like search. Row paints copied for bindings are charged to the build bounds before they are copied.

## Stateless vs workspace

Stateless usages reuse search's per-process index cache and create no database. With `--file`/`--workspace`/`--id` or `workspaceId`, usages read only the published generation of exactly `--scope`, in one read transaction, never building. A scope not yet ready returns `state: pending` (with the job to poll) or `not_prepared` (with the `index` call), `definition: null` and no results. A changed file is `source_stale` with a refresh step; a generation built by another figlens or a schema below 3 is `index_incompatible` with a rebuild step. Sources outside the MCP roots are filtered before anything is read.

Schema V3 (`workspace/database.rs`) recreates `edges` with `seq/state` and a `(gen, target, rid, relation, seq)` index, adds `definitions`, `records_node` and `records_parent`. The relations version is part of `store::versions()`, so older generations read as incompatible.

Other coverage reasons: `members_truncated` (over 200 members), `members_outside_scope` (the set is not in the scope: members are those the scope uses), `relation_unverified` (prototype links inherited from a component definition or past the prototype reader's caps are left out and counted, a swapped nested instance's own links are kept but counted because their precedence is unverified, and binding fields too large to scan or read, or a `characterStyleIDs` list past the visit cap, are left out and counted; that kind is `partial` and `complete` is false when it was asked for). Workspace replies without a ready index carry reason `index_pending` or `scope_not_indexed`. A target longer than any stored id stays an edge with no target, state `unsupported`. `node` and `scope` take any id an index stores (`store::ID_BYTES`), so the deepest expanded id `search` returns is a valid target. Active rows: a text row applies where `characterStyleIDs` uses its styleID (last row per styleID); a vector region row where the effective `fillGeometry` (an instance's derived outline, else the source's; no styleID is style 0) carries its styleID, first row with fill paints per styleID, as `render::region_fills`. Inactive rows are blanked before image, style and `vectorRegionFills` readers run, keeping kept rows' evidence index; a missing or oversize `characterStyleIDs` claims no row (oversize counts unread). REST `nodes` output is unchanged and its raw `textRangeFills`/region data may still show inactive bindings. `node_not_found` for a target referenced only from unread fields means not indexed or unknown, not unused. The paint prefilter that skips plain paints charges every paint and stop it visits to the build bounds, even when nothing links; it keeps text whose range rows name a style. `RELATIONS` (`relations-3`) versions the edges, so older stored generations are `incompatible` and rebuild. The viewer reuses its revision's tree through `search::index_tree` and `usages_index`, passing its own exact source identity.

## Cost

Ordinary stateless search builds a text-only index: `contains`, `instance_of` and `variant_of`, no field relations or definitions, and its coverage lists the other kinds `not_built` (never empty or absent; text coverage is unaffected). In a full index, search's and workspace search's relation layer (`search::gapped`, summed over sources) marks `prototype_target` and the binding-field kinds `partial` with a `relation_unverified` reason when extraction counted unverified links or unread fields, as usages does; text coverage stays independent. The first stateless `usages` call on that cached index rebuilds it once with every relation and replaces the cache entry, which then also serves search with the same results and cursors. Workspace builds and the viewer's `index_tree` always build every relation.

Pages are keyset reads: the workspace reads one `edges_target` range per target from the cursor (named with `INDEXED BY`; without statistics SQLite prefers the primary key and reads every later edge), and children from `records_parent`; a stateless index builds a reverse lookup (target → edge positions, parent → children, id → rid, set → members) on its first usages query and binary-searches the cursor. A page passes over at most `usages::SCAN` (50,000) filtered-out edges, else `query_work_limit`. A component set reads at most 201 targets, each up to `limit` rows. On the 100 MB library, paging 25 pages of one style costs the same per page (last page 0.6 ms stateless, 1.5 ms workspace).

On one private 100 MB component library (11,557 records, scope `0:0`): cold CLI search 210 → 227 ms (+8%) and 193 → 199 MiB; index rebuild 335 → 474 ms (+41%) and database 4.6 → 11.1 MB; warm MCP usages 1.2 ms stateless, 2.4 ms from the workspace; `nodes`/`tree`/`inspect` unchanged. Edge extraction is the cost, mostly style ids and paint fields; plain paints without a bound variable or image are not converted.

## Known gaps

- A ref without `snapshotId` is answered from the current revision; pass it to have an older ref rejected.
- Alias chains between variables, variable modes, and detached copies (no detach field is decoded) are not edges.
- Viewer relation panel and Pi/OpenCode/OMP host tools are later #91 slices.
