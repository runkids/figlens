//! MCP parameter metadata/preflight and additive tool-error diagnostics.
use std::error::Error;

use figlens::{ErrorKind, FigError};
use serde_json::{Value, json};

/// An MCP parameter: key, JSON type, requiredness and description.
type Param = (&'static str, &'static str, bool, &'static str);
const FILE: Param = (
    "file",
    "string",
    true,
    "the saved .fig: a local path (absolute recommended), or gs://bucket/object using gcloud; remote sources are unavailable with --root",
);
const NODE: Param = (
    "node",
    "string",
    true,
    "a stored node id (12:345 or 12-345), full instance sublayer id (I12:345;67:89) from inspect/nodes, or a Figma URL with ?node-id=",
);
const STORAGE: Param = (
    "storage",
    "string",
    false,
    "absolute custom storage directory, only for a workspace kept outside the default storage; must be under the server's --storage-root",
);
const WORKSPACE_ID: Param = (
    "workspaceId",
    "string",
    false,
    "ws_... from a previous index reply",
);
/// CLI and tools share command execution and result formatting.
pub const TOOLS: &[(&str, &str, &[Param])] = &[
    (
        "workflow",
        "Read built-in figlens skill guidance for this agent: build for design-to-code and project reuse, audit for comparing existing UI with a design and correcting confirmed differences, images for website asset preparation, plan for scoped implementation planning, planning-details for page/multi-page work or unresolved design dependencies, yolo for explicitly requested autonomous implementation and temporary-plan cleanup. Returns the requested guide only; no design file, project read/write, implementation, verification or cleanup is performed. Ordinary design questions and exports can use the design tools directly.",
        &[(
            "topic",
            "string",
            true,
            "Guide to read: build, audit, images, plan, planning-details or yolo.",
        )],
    ),
    (
        "tree",
        "List node ids, REST types (as in nodes and find) and names under a chosen root. A shallow outline locates pages and frames when their names are unknown; find searches known names or visible text directly.",
        &[
            FILE,
            (
                "node",
                "string",
                false,
                "a stored root id; default the whole document; use inspect for instance sublayers",
            ),
            ("depth", "integer", false, "levels to list below the root"),
            (
                "json",
                "boolean",
                false,
                "nested {id,type,name,size,visible,layoutMode,children} JSON instead of indented text (about 1.7x larger); size is the local pre-rotation [width,height] (absent on the document and pages), visible appears only when false and layoutMode only on auto-layout frames; nodes on a page or in a section add firstText, their first visible stored text (breadth-first, instances not expanded, at most 200 characters), which tells a canvas label (a box around one line of text) from a screen",
            ),
            (
                "topFrames",
                "boolean",
                false,
                "list only the screens under the root, at any depth: FRAME, COMPONENT, COMPONENT_SET and INSTANCE children of a page or of a section on one (groups, shapes and text are not screens; internal pages are skipped unless they are the root); depth does not apply; text lines start with the id, as nodes/export/bundle frame lists read them; with json, a flat array of entries without children (with firstText), adding page and, inside a section, section ids",
            ),
        ],
    ),
    (
        "find",
        "Search stored layer names and text (case-insensitive). Returns ids, paths, local sizes and text previews. Does not expand instance children; use inspect on the instance.",
        &[
            FILE,
            (
                "query",
                "string",
                true,
                "literal substring in a layer name or text",
            ),
            (
                "node",
                "string",
                false,
                "search this stored subtree; default the document; use inspect for instance sublayers",
            ),
            (
                "type",
                "string",
                false,
                "Figma node type, e.g. FRAME or TEXT",
            ),
            (
                "limit",
                "integer",
                false,
                "1-200 results, default 20; truncated signals more matches",
            ),
        ],
    ),
    (
        "search",
        "Ranked search of layer names and full text in one stored scope, including instance text overrides, text/boolean properties and nested swaps as REST nodes resolve them. Each result has an exact ref {sourceId, snapshotId, nodeId} (instance sublayer IDs work with inspect/nodes/export), origin stored or instance-effective, textSource, page, scopeRoot (a navigation container, not a screen claim), path, and matches with reasons and UTF-16 spans. coverage says whether the scope was fully indexed (ready) or a work limit stopped it (partial, totalRecords null); hasMore/nextCursor page the same candidates. Hidden layers and internal pages are excluded unless requested. Give file to read and hash that .fig now (nothing is written), or workspaceId to rank the workspace's prepared indexes of exactly this scope across its sources as one corpus (sourceId narrows it, refs keep each source's sourceId); a workspace search never builds: sources without a ready index are listed in sources with state pending, not_prepared, stale, incompatible or unreadable and the nextSteps index call, and coverage then says partial.",
        &[
            (
                "file",
                "string",
                false,
                "the saved .fig to read now: a local path or gs://bucket/object; give file or workspaceId",
            ),
            (
                "workspaceId",
                "string",
                false,
                "ws_... from the index tool: search its prepared indexes instead of reading a file",
            ),
            (
                "sourceId",
                "string",
                false,
                "with workspaceId, search only this registered source",
            ),
            STORAGE,
            (
                "query",
                "string",
                true,
                "words or a phrase; case, width and accents fold, CJK matches by character pairs or a single character; no regex",
            ),
            (
                "node",
                "string",
                false,
                "stored scope root; default the document; narrow it when coverage is partial",
            ),
            (
                "type",
                "string",
                false,
                "Figma node types to keep, comma-separated, e.g. TEXT or TEXT,INSTANCE",
            ),
            (
                "limit",
                "integer",
                false,
                "1-200 results per page, default 20; independent of indexing work limits",
            ),
            (
                "cursor",
                "string",
                false,
                "nextCursor from the previous page of the same query, filters and snapshot",
            ),
            (
                "includeHidden",
                "boolean",
                false,
                "also return hidden layers and instance copies hidden by overrides or properties",
            ),
            (
                "includeInternal",
                "boolean",
                false,
                "also return layers on Figma's internal-only page",
            ),
            (
                "mode",
                "string",
                false,
                "text, the only mode in this release",
            ),
        ],
    ),
    (
        "usages",
        "Exact relations of one node or resource in one stored scope, from the same effective index as search (nested instances and swaps included): direction in (default) lists the locations that point at it, out lists what it points at. Relations: instance_of (instance to main component, recorded symbolData or resolved definition/swap), variant_of (component to its set; a COMPONENT_SET node also lists its known members and returns their instances with via), binds_variable (paint, effect and field variable bindings, local or library), uses_style, uses_image, prototype_target (recorded transitions; never a claim that a flow is reachable) and contains (asked for explicitly). Every result keeps an exact ref {sourceId, snapshotId, nodeId}, relation, state (resolved, missing, unresolved, unsupported, external, recorded), basis (recorded or resolved), evidence, page, scopeRoot and path; names never establish identity. The definition may lie outside the scope; scope limits only the returned locations. No total is reported; coverage says complete or partial with reasons (work limits, hidden content not expanded, truncated members, library_identity unavailable), and nextCursor pages in a stable order. Give file to read that .fig now (nothing is written), or workspaceId to read a source's prepared index of exactly this scope (never builds). Pass snapshotId from the ref you hold: another current snapshot is source_stale.",
        &[
            (
                "file",
                "string",
                false,
                "the saved .fig to read now: a local path or gs://bucket/object; give file or workspaceId",
            ),
            (
                "workspaceId",
                "string",
                false,
                "ws_... from the index tool: read its prepared index instead of the file",
            ),
            (
                "sourceId",
                "string",
                false,
                "with workspaceId, the source to read; needed when several have this scope",
            ),
            STORAGE,
            (
                "node",
                "string",
                false,
                "the exact layer id, stored or an instance sublayer I...;...; give node or resource",
            ),
            (
                "resource",
                "string",
                false,
                "a resource id instead of a layer: VariableID:..., StyleKey:... or image:<hash>",
            ),
            (
                "scope",
                "string",
                false,
                "stored scope root whose locations are returned; default the document; a workspace needs a prepared scope",
            ),
            (
                "relation",
                "string",
                false,
                "relations to keep, comma-separated; default every one but contains",
            ),
            (
                "direction",
                "string",
                false,
                "in (what points at it) or out (what it points at)",
            ),
            (
                "snapshotId",
                "string",
                false,
                "the snapshot your ref came from; a different current snapshot is source_stale",
            ),
            (
                "limit",
                "integer",
                false,
                "1-200 results per page, default 50",
            ),
            (
                "cursor",
                "string",
                false,
                "nextCursor from the previous page of the same query and snapshot",
            ),
            (
                "includeHidden",
                "boolean",
                false,
                "also return hidden locations",
            ),
        ],
    ),
    (
        "diff",
        "What changed between two saved revisions of a design (old, file): exact layer ids and types only, so a renamed or recreated frame is reported as removed plus added, with a labelled possibleMatch (same name, else same rounded size and position) that never claims identity; lineage between the two files is not verified. Each changed top-level frame (layers directly on a page group under the page) has one summary line with counts per facet: text, layout (size, transform, constraints, auto layout), appearance (fills, strokes, effects, opacity, radius, visibility), properties (component property definitions and assignments), structure (name, type, parent, order, hidden) and bindings (variable, style, image, main component and prototype edges, old and new compared so removed ones are listed). Nodes carry changed fields with old/new values (each within 512 bytes and 4 KiB per node, 256 KiB per page; larger values say truncated, never unchanged) and edge additions/removals, with swap naming both mains. A side that stopped short reports its missing layers as unknown, never removed. Pages of changes in stable frame order; nextCursor binds both snapshots, the node and the built corpora. Nothing is written.",
        &[
            (
                "old",
                "string",
                true,
                "the earlier saved .fig: a local path",
            ),
            ("file", "string", true, "the later saved .fig: a local path"),
            (
                "node",
                "string",
                false,
                "stored scope root on both sides; default the document",
            ),
            (
                "limit",
                "integer",
                false,
                "1-500 changed layers per page, default 100",
            ),
            (
                "cursor",
                "string",
                false,
                "nextCursor from the previous page of the same pair and node",
            ),
        ],
    ),
    (
        "inspect",
        "Compact expanded node summary: bounds, layout, typography, text previews and child counts. A parent without auto-layout has childSpacing \"derived\": gaps between its children are coordinate differences, not an auto-layout rule; constraints may still pin them. For image decisions, images=true adds original dimensions/bytes and source-pixel rectangles before masks, not blur diagnoses. Depth and a 200-node budget bound the summary; image paints have their own 200-item limit. nodes returns full design data.",
        &[
            FILE,
            NODE,
            (
                "depth",
                "integer",
                false,
                "0-10 levels below the node, default 1; text previews up to 200 characters",
            ),
            (
                "images",
                "boolean",
                false,
                "include source bitmap dimensions/bytes and per-paint source-pixel crop rectangles; no files written",
            ),
            (
                "states",
                "boolean",
                false,
                "include bounded variants and prototype field evidence with exact sources/unknowns; retains instance overrides, infers no business conditions and renders no other variants",
            ),
            (
                "compareVariant",
                "string",
                false,
                "compare node with this other stored variant in the same component set; adds bounded raw-field and descendant-multiset differences, exact sources and unknown pairing; no alternate rendering or business inference",
            ),
            (
                "flat",
                "boolean",
                false,
                "one line per node instead of node JSON, for scanning a screen against an implementation: id, type, name, @x,y WxH relative to the inspected node, then hidden, rot, component, text, font=family size/weight/lineHeight, fill, stroke=paint/weight, r, H|V gap or GRID (grid gaps are not decoded), pad=top,right,bottom,left (or spacing=derived without auto-layout), opacity when set; children=N and '… N more' mark omitted nodes; same depth and node limits; not combinable with images, states or compareVariant; default false",
            ),
        ],
    ),
    (
        "nodes",
        "A node or exact instance sublayer as the per-node object of a Figma REST nodes response ({document, components, componentSets, styles, schemaVersion}): instances expanded, fills, strokes, effects, full text, auto-layout, masks and component data. Use it when inspect's compact summary lacks paints, masks or full text. A whole screen can be hundreds of KB; replies over the MCP reply budget fail with response_budget_exceeded instead of being truncated, so scope to a smaller node, use inspect, or write a bundle.",
        &[FILE, NODE],
    ),
    (
        "variables",
        "The variables a frame binds (collections, modes, aliases); without a node, every local variable.",
        &[
            FILE,
            (
                "node",
                "string",
                false,
                "the node, including full instance sublayer IDs; omit for every local variable",
            ),
        ],
    ),
    (
        "bundle",
        "Write a design bundle zip for one node or exact instance sublayer: nodes, variables, assets, reference render, context.md. When the user requests related component states, pass states=true to include their explicit variants and evidence gaps; no MCP configuration change is needed.",
        &[
            FILE,
            NODE,
            (
                "out",
                "string",
                true,
                "output path for the bundle ZIP; unless the user names one, use .figlens/exports/ under the project root, created with a .gitignore containing * so it stays out of Git; create the parent directory first",
            ),
            (
                "fileKey",
                "string",
                false,
                "Figma file key for the manifest",
            ),
            ("name", "string", false, "frame name for the manifest"),
            (
                "states",
                "boolean",
                false,
                "include design/states.json and a bounded context.md summary; retains inspect's source, unknown and truncation markers; default false",
            ),
        ],
    ),
    (
        "verify",
        "Check a bundle, zipped or extracted, against its manifest before building from it: verified, missing and changed files, the bundle's missing assets, and the font faces the visible text uses (with node ids). Fonts are always unverified here: confirm each face in the running project with document.fonts.load, and check browser delivery of assets there. findings gathers these checks with the bundle's unresolved variables, truncated state inventory, render limitations and unemitted semantics into one handoff preflight list, each with node ids, evidence, impact and next action. Reports without blocking; no design file is read.",
        &[(
            "bundle",
            "string",
            true,
            "path to a bundle .zip or the directory it was extracted to",
        )],
    ),
    (
        "export",
        "Export a node to SVG, PNG, JPG, WebP or AVIF. Create the output directory first. Returns metadata and, for raster formats, an inline image. PNG suits inline references. For website assets, follow the target project's existing image pipeline; otherwise start with WebP for bitmaps, sized for display dimensions and DPR using scale/maxSize and quality/maxKb. SVG embeds original images and can be large; keep it for vector artwork. When inlining or optimizing SVG, keep its root transform and unique ids (SVGO: prefixIds, not cleanupIds). The export covers what the node draws; bounds: layout exports the layout box, and warnings report when the two differ. scale, quality and maxSize affect raster formats only; maxSize defaults to 1600. Check returned dimensions/bytes and isError; read content on failure.",
        &[
            FILE,
            NODE,
            (
                "out",
                "string",
                true,
                "output path ending in .png, .jpg, .jpeg, .webp, .avif or .svg; shipped assets follow the project's asset conventions, reference and scratch files go in .figlens/exports/ under the project root, created with a .gitignore containing * so it stays out of Git; create the parent directory first",
            ),
            (
                "bounds",
                "string",
                false,
                "render (default): what the node draws, shadows, outside strokes and overflowing children included, as Figma's Export panel; layout: the node's own box (absoluteBoundingBox), cutting off what is drawn past it",
            ),
            ("scale", "number", false, "default 1"),
            ("quality", "integer", false, "1-100, default 80"),
            (
                "maxKb",
                "integer",
                false,
                "KiB (1024 bytes) budget; lower lossy quality until it fits; PNG/SVG fail if over budget",
            ),
            (
                "maxSize",
                "integer",
                false,
                "longest side in pixels, default 1600; 0 keeps the scaled size",
            ),
            (
                "matchDir",
                "string",
                false,
                "SVG output only: a folder of SVG files, subfolders included, to compare with the export, to find an icon the project already has; matches lists the files (relative paths) that draw the same outlines once both are moved to the origin, ignoring paint, ids and path syntax; similar lists up to 10 others with a similar shape (drawn with strokes, or the same outlines at another size, which score 1) as {file, score} from 0.8, best first; confirm a similar file by looking at both",
            ),
        ],
    ),
    (
        "index",
        "Local workspace indexes for search: prepare registers a .fig (file) in its own workspace, or in a project workspace (workspace: a project directory), and with scope starts a background build of exactly that stored scope; status reports sources, indexed scopes and storage; add, remove and relocate change the registered sources (remove withdraws their indexes); refresh rebuilds changed scopes and rebuild every indexed scope even when unchanged (sourceId and scope narrow both), while the previous index keeps serving; track starts recording one scope's changes, read with the changes tool, with a baseline build, and untrack stops it, keeping its history; a reply that could not admit every job has admission partial and deferred items, each with the nextStep that builds it later; clear_cache deletes only what a build can recreate; reset deletes this workspace's database after the user confirmed it, keeping every .fig and export. Builds run as jobs: the reply has job {jobId, state}; poll the jobs tool, cancel there. Nothing is built implicitly: connecting, listing, find, inspect and file search create no database. Errors carry _meta.figlens.recovery with only the actions that apply; recreate_elsewhere and reset need the user's confirmation.",
        &[
            (
                "action",
                "string",
                true,
                "prepare, status, add, remove, relocate, refresh, rebuild, track, untrack, clear_cache or reset",
            ),
            (
                "file",
                "string",
                false,
                "a local .fig: the source to prepare, add or relocate to; also selects its source-scoped workspace",
            ),
            (
                "workspace",
                "string",
                false,
                "a project directory: selects or prepares that project's workspace",
            ),
            WORKSPACE_ID,
            (
                "sourceId",
                "string",
                false,
                "a registered source, for remove, relocate, refresh, rebuild, track and untrack",
            ),
            (
                "scope",
                "string",
                false,
                "stored node id to index (prepare), refresh, rebuild, track or untrack; 0:0 is the whole file",
            ),
            (
                "requestId",
                "string",
                false,
                "an idempotency key: repeating a prepare, refresh, rebuild or track of one source and scope with it returns the same job",
            ),
            STORAGE,
            (
                "replaceUnavailable",
                "boolean",
                false,
                "prepare only, after the user confirmed: recreate a workspace whose storage is unavailable, denied or unreadable in storage, knowing only file; the old files stay untouched",
            ),
        ],
    ),
    (
        "jobs",
        "Status or cancellation of an index job. status reports state (queued, running, succeeded, failed, cancelled, interrupted), stage, progress and the error or published generation; cancel stops the build and its worker process before reporting cancelled, and a job already publishing finishes (too_late). Responds while builds and searches run.",
        &[
            ("action", "string", true, "status or cancel"),
            ("jobId", "string", true, "the jobId from an index reply"),
            STORAGE,
        ],
    ),
    (
        "changes",
        "Recorded design changes of the scopes index track follows; never writes or builds. Each revision is one published build of a tracked scope: kind baseline (tracking started, or nothing comparable before it, with a reason) or changes, with coverage, exact summary counts per change kind and facet, and detail complete, truncated or expired. Pages run oldest first after cursor, else after the consumer's checkpoint (moved by watch ack), else from the oldest kept. Pass revision to page that revision's changed nodes instead. tracking lists each tracked scope's state, lastRevisionId and refreshRequired (the file changed since its last build: refresh the scope to record it; an empty page never proves the file unchanged); monitor is the watch monitor, separate from tracking. A cursor is bound to the filters, the visible sources and the workspace incarnation: any other use is cursor_stale. event_gap means retention deleted revisions before they were read: follow its recovery and treat the scope as possibly changed.",
        &[
            ("workspaceId", "string", true, "ws_... from the index tool"),
            ("sourceId", "string", false, "only this source's revisions"),
            ("scope", "string", false, "only this scope's revisions"),
            (
                "consumer",
                "string",
                false,
                "without cursor, start after this consumer's checkpoint",
            ),
            ("cursor", "string", false, "nextCursor of the previous page"),
            (
                "revision",
                "string",
                false,
                "a revisionId: page its changed nodes instead",
            ),
            (
                "limit",
                "integer",
                false,
                "revisions per page, 1-50 (default 20); with revision, nodes per page, 1-500 (default 100)",
            ),
            STORAGE,
        ],
    ),
    (
        "watch",
        "Change-tracking consumers. ack moves consumer's checkpoint for these filters (sourceId, scope) forward to cursor, the nextCursor of the last changes page it handled with the same filters; an older or equal cursor never moves it, so a retry is safe, and a cursor issued before retention deleted revisions behind it is event_gap, writing nothing: follow its recovery. Replies with advanced and the checkpoint held.",
        &[
            ("action", "string", true, "ack"),
            ("workspaceId", "string", true, "ws_... from the index tool"),
            (
                "consumer",
                "string",
                true,
                "the consumer's name, 1-128 bytes",
            ),
            (
                "cursor",
                "string",
                true,
                "nextCursor from changes with the same filters",
            ),
            (
                "sourceId",
                "string",
                false,
                "the sourceId filter of that changes call",
            ),
            (
                "scope",
                "string",
                false,
                "the scope filter of that changes call",
            ),
            STORAGE,
        ],
    ),
];

/// The MCP outputSchema of the tools whose CLI command prints one JSON object.
pub fn output_schema(tool: &str) -> Option<serde_json::Value> {
    let (properties, required) = match tool {
        "find" => (
            serde_json::json!({
                "root": {"type": "string"},
                "results": {"type": "array", "items": {"type": "object"}},
                "truncated": {"type": "boolean"},
            }),
            ["root", "results", "truncated"].as_slice(),
        ),
        "search" => (
            serde_json::json!({
                "query": {"type": "object", "description": "the query as given, its normalized phrase, mode and the whole tokens actually searched"},
                "source": {"type": "object", "description": "sourceId (opaque, from this path), snapshotId (sha256 of the exact .fig bytes decoded) and freshness: file-stamp reuse, not a per-query disk hash"},
                "scope": {"type": "object"},
                "ranking": {"type": "object", "description": "ranking and tokenizer versions, field weights and tie order"},
                "results": {"type": "array", "items": {"type": "object"}},
                "coverage": {"type": "object", "description": "complete, scannedRecords, totalRecords (null unless complete), layers.text.state ready|partial, layers.relations (per kind: ready or partial when built; stateless search builds contains, instance_of and variant_of and lists the others not_built, which says nothing about absence; library_identity relation_unavailable), reasons with counts, excluded hidden/internal matches, knownGaps and work limits"},
                "hasMore": {"type": "boolean"},
                "nextCursor": {"type": ["string", "null"]},
                "workspaceId": {"type": "string", "description": "workspace searches only"},
                "state": {"type": "string", "description": "workspace searches: ready, pending or not_prepared"},
                "sources": {"type": "array", "items": {"type": "object"}, "description": "workspace searches: each visible source's sourceId, state, snapshotId, freshness and active job"},
                "nextSteps": {"type": "array", "items": {"type": "object"}, "description": "workspace searches: the index or jobs calls that make more sources ready"},
            }),
            ["query", "results", "coverage", "hasMore", "nextCursor"].as_slice(),
        ),
        "usages" => (
            serde_json::json!({
                "definition": {"type": ["object", "null"], "description": "null while a workspace scope is pending or not_prepared; else ref or resource, state (indexed in scope, outside_scope, or the state of the edges that name it: missing, unresolved, external, recorded), name, type, setId and page when known"},
                "direction": {"type": "string"},
                "relations": {"type": "array", "items": {"type": "string"}},
                "scope": {"type": "object"},
                "members": {"type": "object", "description": "COMPONENT_SET queries: the known member ids and whether they are complete"},
                "results": {"type": "array", "items": {"type": "object"}, "description": "in: each location's ref, name, type, origin, hidden, page, scopeRoot, path, relation, state, basis, evidence and via (the member a set query matched); out: target ref or resource, its definition and the edge fields"},
                "coverage": {"type": "object", "description": "complete, layers.relations kinds and version, reasons; never a total count"},
                "hasMore": {"type": "boolean"},
                "nextCursor": {"type": ["string", "null"]},
                "source": {"type": "object", "description": "sourceId and snapshotId; stateless calls add freshness"},
                "workspaceId": {"type": "string", "description": "workspace queries only"},
                "state": {"type": "string", "description": "workspace queries without a ready index: pending or not_prepared"},
                "nextSteps": {"type": "array", "items": {"type": "object"}, "description": "the jobs or index call that makes the scope ready"},
            }),
            ["definition", "results", "coverage", "hasMore", "nextCursor"].as_slice(),
        ),
        "diff" => (
            serde_json::json!({
                "source": {"type": "object", "description": "old and new sourceId and snapshotId, and freshness.reusedPair"},
                "node": {"type": "string"},
                "matching": {"type": "object", "description": "basis node_id+kind; lineage unverified"},
                "coverage": {"type": "object", "description": "complete; old and new stop and scannedRecords; changes (layers listed across pages, added/removed subtrees counted at their top), possibleMatches and reasons (a side's stop, diff_work_limit, unread_fields, relation_unverified, hidden_content_not_expanded, selection_context_unknown, definition_not_compared for styles, variables and main components outside the compared records)"},
                "frames": {"type": "array", "items": {"type": "object"}, "description": "id, name, page, change when the frame itself changed, summary, possibleMatch {id, basis, identity possible_match}, and this page's nodes: id, kind, name, change, facets, fields {key: {facet, old, new}}, edges {added, removed, swap}, descendants, identity_uncertain with oldKind"},
                "nextCursor": {"type": ["string", "null"]},
            }),
            ["source", "coverage", "frames", "nextCursor"].as_slice(),
        ),
        "index" => (
            serde_json::json!({
                "workspaceId": {"type": "string"},
                "state": {"type": "string", "description": "initialized, ready, absent or reset"},
                "schemaVersion": {"type": "integer"},
                "incarnation": {"type": "string", "description": "changes when the database is recreated; cached results of another incarnation are void"},
                "sources": {"type": "array", "items": {"type": "object"}, "description": "visible sources: sourceId, path, present and indexed scopes with snapshotId, records, complete and current"},
                "job": {"type": "object", "description": "the build prepare started or joined: jobId, state, stage, progress"},
                "jobs": {"type": "array", "items": {"type": "object"}, "description": "refresh, rebuild and track: one job per admitted source and scope, each with joined"},
                "admission": {"type": "string", "description": "refresh and rebuild: complete, or partial when some jobs were not admitted"},
                "deferred": {"type": "array", "items": {"type": "object"}, "description": "partial admission: sourceId, scope and the nextStep index call that builds it later"},
                "deferredBecause": {"type": ["object", "null"], "description": "partial admission: the error that stopped admission, {code, message}"},
                "coverage": {"type": "object", "description": "prepare without scope: text and relations not_requested"},
                "nextSteps": {"type": "array", "items": {"type": "object"}},
                "storage": {"type": "object"},
                "paths": {"type": ["object", "null"], "description": "workspace, database and lock paths (the lock sits beside the workspace directory); null when the workspace does not exist"},
            }),
            ["workspaceId", "state"].as_slice(),
        ),
        "jobs" => (
            serde_json::json!({
                "jobId": {"type": "string"},
                "workspaceId": {"type": "string", "description": "the workspace the job builds in, for index status"},
                "kind": {"type": "string", "description": "prepare, refresh or rebuild"},
                "state": {"type": "string", "description": "queued, running, succeeded, failed, cancelled or interrupted (its owner exited)"},
                "stage": {"type": ["string", "null"], "description": "null until the job starts"},
                "progress": {"type": "object"},
                "cancel": {"type": "string", "description": "cancel only: accepted, pending, too_late or not_active"},
                "error": {"type": ["object", "null"]},
                "evictedScopes": {"type": "array", "description": "a publish that retired the source's least recently published scope (past 8) names it"},
                "nextSteps": {"type": "array", "items": {"type": "object"}},
            }),
            ["jobId", "state"].as_slice(),
        ),
        "changes" => (
            serde_json::json!({
                "workspaceId": {"type": "string"},
                "revisions": {"type": "array", "items": {"type": "object"}, "description": "oldest first: revisionId, sourceId, scope, kind (baseline or changes), from (the previous revisionId, null for a baseline or when deleted), snapshotId, coverage (a baseline's reason), summary (changes, kinds, facets, possibleMatches), detail (complete, truncated or expired), detailRows"},
                "nextCursor": {"type": ["string", "null"], "description": "pass as cursor for the next page, or to watch ack once handled"},
                "hasMore": {"type": "boolean"},
                "tracking": {"type": "array", "items": {"type": "object"}, "description": "each tracked scope of the visible sources: sourceId, scope, state (baseline_pending, active or disabled), lastRevisionId, lastCheckedAtMs (when its last build verified the file), refreshRequired (the file's size or mtime differs from that build's)"},
                "trackingTruncated": {"type": "boolean", "description": "true when more tracked scopes exist than tracking lists"},
                "monitor": {"type": "object", "description": "the watch monitor's state, separate from tracking"},
                "revisionId": {"type": "string", "description": "with revision"},
                "detail": {"type": "string", "description": "with revision: complete, truncated (rows past the cap were not kept) or expired"},
                "nodes": {"type": "array", "items": {"type": "object"}, "description": "with revision: frame, id, kind, change, facets, unread, descendants"},
            }),
            ["workspaceId", "nextCursor"].as_slice(),
        ),
        "watch" => (
            serde_json::json!({
                "workspaceId": {"type": "string"},
                "consumer": {"type": "string"},
                "advanced": {"type": "boolean", "description": "true only when the visible checkpoint moved forward"},
                "checkpoint": {"type": ["string", "null"], "description": "a cursor at the checkpoint now held; null when retention deleted that revision"},
            }),
            ["workspaceId", "consumer", "advanced", "checkpoint"].as_slice(),
        ),
        "inspect" => (
            serde_json::json!({
                "flat": {"type": "string", "description": "the one-line-per-node text when flat=true, instead of node"},
                "node": {"type": "object", "description": "Compact node with partial layout semantics and type-only property definitions; childSpacing \"derived\" marks a parent without auto-layout; _figlens.semantics records source/verification. Definitions, definition evidence and _figlens.renderDiagnostics each have at most 20 entries per node; truncated includes omitted nodes or entries"},
                "truncated": {"type": "boolean"},
                "images": {"type": "object", "description": "optional source image evidence when images=true"},
                "states": {"type": "object", "description": "optional variants and prototype field evidence when states=true; independent truncation, source file, node/override provenance and unknown markers; no inferred business logic"},
                "variantComparison": {"type":"object", "description":"optional same-set definition comparison when compareVariant is supplied; raw fields, descendant value multisets, unpaired nodes, unknowns and independent truncation; not a pixel comparison"},
                "handoffDiagnostics": {"type":"object", "description":"optional known stored semantics not emitted by the projection; counts cover the selected expanded subtree independently of summary depth, with at most five example IDs per feature and explicit coverage limits"},
            }),
            ["truncated"].as_slice(),
        ),
        "export" => (
            serde_json::json!({
                "path": { "type": "string" },
                "width": { "type": "integer" },
                "height": { "type": "integer" },
                "bytes": { "type": "integer" },
                "quality": { "type": "integer", "description": "the quality used; --max-kb may lower it" },
                "layoutBox": { "type": ["object", "null"], "description": "absoluteBoundingBox in page coordinates" },
                "renderBox": { "type": ["object", "null"], "description": "absoluteRenderBounds in page coordinates" },
                "warnings": { "type": "array", "items": { "type": "string" }, "description": "empty unless the drawn area differs from the layout box; then how far it reaches past each side, what draws there when known, and which box was exported" },
                "matches": { "type": "array", "items": { "type": "string" }, "description": "with matchDir: files there (relative paths) that draw the same outlines" },
                "similar": { "type": "array", "items": { "type": "object" }, "description": "with matchDir: up to 10 other files with a similar shape as {file, score}, score 0.8-1, best first; not proof of the same icon" },
            }),
            ["path", "width", "height", "bytes", "quality"].as_slice(),
        ),
        "bundle" => (
            serde_json::json!({
                "nodeId": { "type": "string" },
                "path": { "type": "string" },
                "warnings": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "missing assets, known rendering limits and truncated optional state inventory; missing-assets.json contains only asset failures; design/nodes.json has per-node _figlens.renderDiagnostics; design/states.json has inventory coverage when requested",
                },
            }),
            ["nodeId", "path", "warnings"].as_slice(),
        ),
        "verify" => (
            serde_json::json!({
                "path": { "type": "string" },
                "files": {
                    "type": "object",
                    "description": "verified count; missing paths; changed entries with expected and actual size/sha256 (actual null when larger than declared, unreadable, with error, or not a regular file inside the bundle); invalid manifest paths",
                },
                "missingAssets": { "type": "array", "description": "missing-assets.json entries" },
                "fonts": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "family, style, weight, postScriptName, nodeIds and status, always unverified",
                },
                "unverified": { "type": "array", "items": { "type": "string" } },
                "findings": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "handoff preflight, missing inputs first: category (integrity, asset, variable, scope, render, semantics, font), status (missing, unsupported or unverified), source file, nodeIds (at most 20, none over 1 KiB) and nodeCount, evidence, impact and action; at most 500 entries and 1 MiB",
                },
                "findingsSummary": {
                    "type": "object",
                    "description": "scope (bundle root nodeId and name), total, returned, truncated, counts per category/status, unexamined metadata files, semanticsCoverage (the handoff inventory's coverage) and the coverage the findings do not establish",
                },
                "truncated": {
                    "type": "object",
                    "description": "original length of each other list cut to stay within the reply budget (files.missing, files.changed, files.invalid, missingAssets, fonts); empty when nothing was cut",
                },
            }),
            [
                "path",
                "files",
                "missingAssets",
                "fonts",
                "unverified",
                "findings",
                "findingsSummary",
                "truncated",
            ]
            .as_slice(),
        ),
        _ => return None,
    };
    Some(serde_json::json!({ "type": "object", "properties": properties, "required": required }))
}

fn default(tool: &str, key: &str) -> Option<Value> {
    match (tool, key) {
        ("tree", "depth") => Some(json!(usize::MAX)),
        ("tree" | "find" | "search", "node") => Some(json!("0:0")),
        ("tree", "json" | "topFrames")
        | ("inspect", "images" | "states" | "flat")
        | ("bundle", "states")
        | ("search", "includeHidden" | "includeInternal") => Some(json!(false)),
        ("inspect", "depth") | ("export", "scale") => Some(json!(1)),
        ("find" | "search", "limit") => Some(json!(20)),
        ("search", "mode") => Some(json!("text")),
        ("usages", "scope") | ("diff", "node") => Some(json!("0:0")),
        ("diff", "limit") => Some(json!(100)),
        ("usages", "direction") => Some(json!("in")),
        ("usages", "limit") => Some(json!(50)),
        ("usages", "includeHidden") => Some(json!(false)),
        ("export", "bounds") => Some(json!("render")),
        ("export", "quality") => Some(json!(80)),
        ("export", "maxSize") => Some(json!(1600)),
        _ => None,
    }
}

const SOURCE_HELP: &str = "Cannot read the requested .fig source; check the path and valid local copy, or gcloud availability, connectivity and authorized object read access.";

// Larger bounds break int64 and JavaScript schema parsers; some model backends reject every tool.
const JSON_SAFE: u64 = (1 << 53) - 1;

pub fn property(tool: &str, param: &Param) -> Value {
    let (key, kind, _, about) = *param;
    let mut p = json!({"type": kind, "description": about});
    let range = match (tool, key) {
        ("inspect", "depth") => Some((0, 10)),
        ("tree", "depth") => Some((0, JSON_SAFE.min(usize::MAX as u64))),
        ("find" | "search" | "usages", "limit") => Some((1, 200)),
        ("diff" | "changes", "limit") => Some((1, 500)),
        ("export", "quality") => Some((1, 100)),
        ("export", "maxKb") => Some((0, JSON_SAFE.min((usize::MAX / 1024) as u64))),
        ("export", "maxSize") => Some((0, u64::from(u32::MAX))),
        _ => None,
    };
    if let Some((min, max)) = range {
        p["minimum"] = min.into();
        p["maximum"] = max.into();
    }
    if let Some(value) = default(tool, key).filter(|_| (tool, key) != ("tree", "depth")) {
        p["default"] = value;
    }
    if matches!(
        key,
        "file" | "old" | "out" | "query" | "compareVariant" | "bundle" | "cursor" | "matchDir"
    ) {
        p["minLength"] = json!(1);
    }
    if key == "query" {
        p["pattern"] = json!(r"\S");
    }
    if tool == "export" && key == "scale" {
        p["exclusiveMinimum"] = json!(0);
    }
    if tool == "export" && key == "bounds" {
        p["enum"] = json!(["render", "layout"]);
    }
    if tool == "workflow" && key == "topic" {
        p["enum"] = json!(crate::workflow::TOPICS);
    }
    if tool == "index" && key == "action" {
        p["enum"] = json!([
            "prepare",
            "status",
            "add",
            "remove",
            "relocate",
            "refresh",
            "rebuild",
            "track",
            "untrack",
            "clear_cache",
            "reset"
        ]);
    }
    if tool == "jobs" && key == "action" {
        p["enum"] = json!(["status", "cancel"]);
    }
    if tool == "watch" && key == "action" {
        p["enum"] = json!(["ack"]);
    }
    if tool == "search" && key == "mode" {
        p["enum"] = json!(["text"]);
    }
    if tool == "usages" && key == "direction" {
        p["enum"] = json!(["in", "out"]);
    }
    p
}

fn integer(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    let n = value.as_f64()?;
    // u64::MAX rounds up to 2^64 as f64; exclude that endpoint before casting.
    ((0.0..18_446_744_073_709_551_616.0).contains(&n) && n.fract() == 0.0).then_some(n as u64)
}

/// An immutable call whose identity, defaults and arguments have passed the contract.
pub struct PreparedCall {
    name: &'static str,
    arguments: Value,
}

impl PreparedCall {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn argument(&self, key: &str) -> &Value {
        &self.arguments[key]
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.argument(key).as_str()
    }
}

/// Validate before source I/O, normalize mathematical integers and materialize MCP defaults.
pub fn prepare(tool: &str, args: &Value) -> Result<PreparedCall, Failure> {
    let (name, _, params) = TOOLS
        .iter()
        .find(|t| t.0 == tool)
        .ok_or_else(|| Failure::argument("unknown tool"))?;
    let object = args
        .as_object()
        .ok_or_else(|| Failure::argument("arguments must be an object"))?;
    // A misspelled optional parameter would otherwise fall back to its default and still succeed.
    if let Some(key) = object
        .keys()
        .find(|key| !params.iter().any(|p| p.0 == *key))
    {
        let accepted = params.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
        return Err(Failure::argument(format!(
            "unknown parameter {key}; accepted: {accepted}"
        )));
    }
    let mut checked = json!({});
    for param in *params {
        let (key, kind, required, _) = *param;
        let Some(value) = object.get(key) else {
            if required {
                return Err(Failure::argument(format!(
                    "missing required parameter {key}"
                )));
            }
            if let Some(value) = default(tool, key) {
                checked[key] = value;
            }
            continue;
        };
        let schema = property(tool, param);
        checked[key] = value.clone();
        let bad = || Failure::argument(format!("invalid parameter {key}; expected {schema}"));
        match kind {
            "integer" => {
                let n = integer(value).ok_or_else(bad)?;
                if n < schema["minimum"].as_u64().unwrap()
                    || n > schema["maximum"].as_u64().unwrap()
                {
                    return Err(bad());
                }
                checked[key] = n.into();
            }
            "number" => {
                let n = value.as_f64().ok_or_else(bad)?;
                if !n.is_finite() || n <= 0.0 {
                    return Err(bad());
                }
            }
            "boolean" if !value.is_boolean() => return Err(bad()),
            "string" => {
                let text = value.as_str().ok_or_else(bad)?;
                if (schema["minLength"] == 1 && text.is_empty())
                    || (key == "query" && text.trim().is_empty())
                {
                    return Err(bad());
                }
                if let Some(choices) = schema["enum"].as_array()
                    && !choices.contains(value)
                {
                    return Err(bad());
                }
            }
            _ => {}
        }
    }
    if tool == "export"
        && figlens::export::Format::of(std::path::Path::new(checked["out"].as_str().unwrap()))
            .is_none()
    {
        return Err(Failure::argument(
            "out must end in .png, .jpg, .jpeg, .webp, .avif or .svg",
        ));
    }
    let call = PreparedCall {
        name,
        arguments: checked,
    };
    if tool == "search" {
        let file = call.string("file").is_some();
        if file == call.string("workspaceId").is_some()
            || (file && (call.string("sourceId").is_some() || call.string("storage").is_some()))
        {
            return Err(Failure::argument(
                "search needs file or workspaceId, not both; sourceId and storage apply to workspaceId",
            ));
        }
        crate::command::search_tool(&call).map_err(Failure::from_error)?;
    }
    if tool == "usages" {
        let file = call.string("file").is_some();
        if file == call.string("workspaceId").is_some()
            || (file && (call.string("sourceId").is_some() || call.string("storage").is_some()))
        {
            return Err(Failure::argument(
                "usages needs file or workspaceId, not both; sourceId and storage apply to workspaceId",
            ));
        }
        crate::command::usages_tool(&call).map_err(Failure::from_error)?;
    }
    Ok(call)
}

pub fn tools() -> Value {
    json!({ "tools": TOOLS.iter().map(|(name, about, params)| {
        let mut tool = json!({
            "name": name,
            "description": about,
            "inputSchema": {
                "type": "object",
                "properties": params.iter().map(|param|
                    (param.0.to_owned(), property(name, param))
                ).collect::<serde_json::Map<_, _>>(),
                "required": params.iter().filter(|p| p.2).map(|p| p.0).collect::<Vec<_>>(),
                "additionalProperties": false,
            }
        });
        if let Some(schema) = output_schema(name) {
            tool["outputSchema"] = schema;
        }
        match *name {
            "workflow" => tool["annotations"] = json!({ "readOnlyHint": true, "openWorldHint": false }),
            // Mixed actions, reset among them.
            "index" => tool["annotations"] = json!({"readOnlyHint": false, "destructiveHint": true,
                "idempotentHint": false, "openWorldHint": false}),
            // cancel changes a job.
            "jobs" => tool["annotations"] = json!({"readOnlyHint": false, "destructiveHint": false,
                "openWorldHint": false}),
            "changes" => tool["annotations"] = json!({"readOnlyHint": true, "openWorldHint": false}),
            // ack only moves a checkpoint forward.
            "watch" => tool["annotations"] = json!({"readOnlyHint": false, "destructiveHint": false,
                "idempotentHint": true, "openWorldHint": false}),
            _ => {}
        }
        tool
    }).collect::<Vec<_>>() })
}

#[derive(Debug)]
pub struct Failure {
    pub kind: ErrorKind,
    pub text: String,
    /// Implemented recovery actions that apply, from the core's error.
    pub recovery: Option<Value>,
}

impl Failure {
    pub fn new(kind: ErrorKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            recovery: None,
        }
    }

    pub fn argument(text: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArguments, text)
    }

    pub fn from_error(error: Box<dyn Error>) -> Self {
        let kind = error
            .downcast_ref::<FigError>()
            .map_or(ErrorKind::OperationFailed, FigError::kind);
        let text = match kind {
            ErrorKind::SourceUnreadable | ErrorKind::InvalidData => SOURCE_HELP.to_owned(),
            _ => error.to_string(),
        };
        Self {
            recovery: error
                .downcast_ref::<FigError>()
                .and_then(FigError::recovery)
                .cloned(),
            ..Self::new(kind, text)
        }
    }

    pub fn metadata(&self) -> Value {
        let (code, next) = match self.kind {
            ErrorKind::InvalidArguments => (
                "invalid_arguments",
                "Use tools/list types and ranges; omit optional values for defaults, or use maxSize: 0 for no side cap.",
            ),
            ErrorKind::SourceUnreadable | ErrorKind::InvalidData => (
                "source_unreadable",
                "Check the source path, export a valid .fig, and verify authorized access; do not bypass permissions.",
            ),
            ErrorKind::NodeNotFound => (
                "node_not_found",
                "Use find/tree on a stored root or inspect the containing instance; copy the complete returned ID.",
            ),
            ErrorKind::UnsupportedNode => (
                "unsupported_node",
                "Use inspect/nodes for instance sublayers, or choose a sized drawable node for export/bundle.",
            ),
            ErrorKind::ParseBudget => (
                "parse_budget_exceeded",
                "Use a smaller local copy or report the named limit for review.",
            ),
            ErrorKind::ResponseBudget => (
                "response_budget_exceeded",
                "Use inspect, bundle one frame, or CLI nodes for full JSON; for inline exports lower maxSize. A completed file is identified by outputPath when present.",
            ),
            ErrorKind::OutputWrite => (
                "output_write_failed",
                "Create an authorized writable output directory; check free space and permissions.",
            ),
            ErrorKind::PermissionDenied => (
                "permission_denied",
                "Use an authorized local source and output inside the configured roots; retain the restrictions.",
            ),
            ErrorKind::OperationFailed => (
                "operation_failed",
                "Inspect the error and supported operation; report a reproducible case without private design data.",
            ),
            ErrorKind::WorkspaceBusy => (
                "workspace_busy",
                "Another figlens process is using this workspace; retry after it finishes, without killing it.",
            ),
            ErrorKind::IndexIncompatible => (
                "index_incompatible",
                "The workspace database was written by a different figlens schema; use that figlens version or reset the workspace.",
            ),
            ErrorKind::StorageUnavailable => (
                "storage_unavailable",
                "The workspace storage is missing or offline; reconnect or restore it and retry, or follow recovery to recreate the workspace elsewhere.",
            ),
            ErrorKind::SourceStale => (
                "source_stale",
                "The source changed since it was indexed; refresh it with the index tool, then search again.",
            ),
            ErrorKind::CursorStale => (
                "cursor_stale",
                "This cursor was issued for another index, filter set or view; call the same tool again without cursor.",
            ),
            ErrorKind::CursorExpired => (
                "cursor_expired",
                "The index this cursor paged was deleted; search again without cursor.",
            ),
            ErrorKind::EventGap => (
                "event_gap",
                "Revisions after this position were deleted before they were read; follow recovery to read changes from the oldest kept, and treat the scope as possibly changed.",
            ),
            ErrorKind::Cancelled => (
                "cancelled",
                "The operation was cancelled before it finished; nothing was published.",
            ),
            ErrorKind::QueryWorkLimit => (
                "query_work_limit",
                "The search would read more indexed data than one query supports; follow recovery to search fewer sources or index a narrower scope. Nothing is wrong with the index.",
            ),
        };
        // A storage failure is fixed by restoring that storage, not by the design roots.
        let storage = self
            .recovery
            .as_ref()
            .and_then(|r| r[0]["action"].as_str())
            .is_some_and(|a| a.starts_with("restore_"));
        let next = if storage {
            "The figlens storage named in the error is missing or denied; ask the user to restore it (never change permissions yourself), then retry; recreate_elsewhere only after the user confirms."
        } else {
            next
        };
        let mut meta = json!({"code": code, "nextStep": next});
        if let Some(recovery) = &self.recovery {
            meta["recovery"] = recovery.clone();
        }
        meta
    }
}

/// Keep local CLI diagnostics and budget provenance; URL-source diagnostics can contain secrets.
pub fn source(file: &str, error: Box<dyn Error>) -> Box<dyn Error> {
    if error
        .downcast_ref::<FigError>()
        .is_some_and(|e| e.kind() == ErrorKind::ParseBudget)
    {
        error
    } else {
        Box::new(FigError::new(
            ErrorKind::SourceUnreadable,
            if file.contains("://") {
                SOURCE_HELP.to_owned()
            } else {
                format!("{file}: {error}")
            },
        ))
    }
}
