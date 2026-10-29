# Defensive parsing budgets

Load this when changing untrusted container, embedded-schema or value decoding. The reported historical MCP stack overflow is a risk lead, not a reproduced or fully established root cause.

## Protected path

CLI, MCP and preview use `Document::open` → `from_canvas`; `search` stages at most 512 MiB of source bytes, hashes them and decodes the same bytes with `Document::from_bytes`, which shares `from_archive` and every preflight below. Both go → `message` → `Tree::new`. The shared document path bounds the canvas and decompressed chunks, preflights the embedded schema before `Schema::decode`, and preflights actual values before `Value::decode`. `src/parse.rs` follows the pinned `kiwi-schema` decoder's type and field rules; only primitive skipping delegates to its non-recursive branch. Review this correspondence when upgrading that dependency.

The schema's list of definitions is flat; its types can reference other types recursively. Preflight checks actual value nesting rather than rejecting every recursive type graph, so a finite recursive message can remain valid. It checks depth before descending, and declared schema/array counts against remaining work before entering their loops. Counters are local to one preflight; repeated message calls and subsequent files start fresh. No partial decoded result is returned on failure.

`Tree::new` separately drops parent cycles with an iterative walk. That layer-graph protection occurs after decoding and does not protect Kiwi recursion. The accepted value-item budget also bounds the node-change list used to construct a tree. Valid tree semantics are unchanged. The shared `guid_key` requires both `sessionID` and `localID` to be unsigned integers and rejects unset GUIDs; malformed fields never alias node zero in tree indexes, expansion or evidence lookups.

## Initial fixed ceilings

| Limit | Default | Meaning |
|---|---:|---|
| Canvas bytes | 512 MiB | Advertised ZIP entry size is checked before reading; a bounded reader also checks actual output. Direct `from_canvas`/`split_chunks` input is checked too. |
| Chunks | 1,024 | Checked before appending another chunk slice. |
| Schema bytes | 16 MiB | Decompressed embedded schema output. |
| Message bytes | 256 MiB | Decompressed message output; also the public `unpack` ceiling. |
| Original image bytes | 64 MiB each | ZIP entry size is checked before reading; a bounded reader checks actual decompressed bytes too. |
| Total original image bytes | 256 MiB per scope preparation | Unique referenced images retained together by export, bundle, inspect images and offline preview; a new preparation starts fresh. |
| Schema items | 262,144 | Definitions plus their fields, including enum fields. |
| Value depth | 64 | Root value is depth one; object/array containers and their children count. This is not layer-parent depth. |
| Decoded items | 16,777,216 | Every decoded scalar, enum, object and array container, including non-byte array elements and overwritten message fields. A compact byte array counts as one item. |

These are initial safety ceilings, **not tuned against representative large designs**. The 64-level ceiling limits recursive call exposure; the smaller schema allowance treats definitions as metadata, while the larger message/item allowances leave room for byte/blob arrays. The byte ceilings keep individual preliminary buffers in the MiB range rather than permitting unbounded output. These are policy choices, not measured valid maxima. Depth limits act independently of compressed input size; item limits bound expansion even for types consuming few bytes. They may reject legitimate large files. Byte arrays decode to `Value::Bytes(Vec<u8>)` in the vendored `kiwi-schema` 0.2.1: preflight checks their declared length against Message bytes before reading the slice, without spending an item or depth level per byte. Their total payload is also bounded by the decompressed message ceiling. Non-byte arrays retain per-element depth/item accounting. This representation was checked on three local community designs; these are not calibrated valid maxima. Reduce scope in a new local copy or report the limit for review rather than assuming it proves corruption.

The budgets are not a precise process-memory or elapsed-time cap. Heap amplification, codec buffers and concurrent users add overhead. Preflight reads schema/message structure before the decoder reads it again; the local decode measurements in [performance](performance.md#compact-kiwi-byte-arrays-2026-10-26) include this preflight separately from raw Kiwi decoding. There are no new CLI flags, environment settings or dependencies.

Original-image reads share `read_image` in `src/lib.rs`. `render::images` spends one total budget across unique references; the live preview's single-original route uses the same per-image check. Oversized images fail the operation before an export, bundle or snapshot is written, rather than disappearing as missing assets. Missing archive entries retain their existing behavior. These limits cover encoded image bytes after ZIP decompression, not decoded pixel buffers, generated images, concurrent preparations or the live viewer's separate memory cache. The initial image ceilings were checked against four local real designs (largest original 10,050,996 bytes; largest whole-file image total 175,987,148 bytes), not calibrated against all large designs.

## Errors and remaining boundaries

Budget failures use the existing `FigError` channel with the stable prefix `parse budget exceeded:`, a limit kind, its numeric ceiling and a next step. CLI calls fail; MCP returns `isError: true` with the text in `content`. Keep design contents out of error text. Panic catching and larger stacks do not replace these checks.

This batch does not bound ZIP central-directory indexing before `canvas.fig` is selected, codec internals beyond their existing checks (including ruzstd's default window limit), every third-party decoder call made directly by library consumers, or later expansion/rendering/bundle work. Caller-constructed `Value` trees bypass document parsing. Layer-parent depth, output JSON size and request transport size are separate concerns. Do not claim that this makes every MCP input crash-proof.

## Completed output publication

Bundle, export and offline snapshot outputs share `src/output.rs`. Existing output symlinks resolve to their target, including unrooted dangling links; the caller retains the requested encoding and reply label. A canonical output equal to the source is refused. An alternate hard link is replaced by a new file, preserving the original source inode. Ordinary MCP root admission/rechecks still apply.

Writers exclusively create a sibling `.figlens-<pid>-<counter>.part`, retry occupied names, close it and rename it over the destination only after success. Cleanup removes only the owned staging file, including on callback failure or unwind. Existing read-only outputs are refused and existing permissions are retained. A failed write or rename preserves the previous output. This is atomic file publication, not a disk-sync/durable-save guarantee or protection against arbitrary concurrent directory replacement. It does not change preview disk-cache or GCS-cache writes.

## Safe verification

Use small acyclic schemas/values and test-only low limits for below/exact/over depth and work boundaries, decompression byte boundaries and recovery after failure. Generated files use `TestDir`; never use private designs or regenerate the historical crash payload. The MCP integration case uses a tiny finite acyclic message chain one level over the default depth, followed by ping and a normal generated `.fig` request in the same server loop. This is synthetic integration evidence, not large-design compatibility or a guarantee that MCP can never terminate.

Image-budget tests use tiny readers for actual/cumulative byte boundaries and a copy of the public synthetic fixture with an oversized image-directory declaration for early rejection. Check CLI, MCP recovery, offline output absence and the live original-image route without allocating a limit-sized fixture. Pi receives the existing `parse_budget_exceeded` MCP error through its discovered tools; no Pi-specific argument or adapter is needed.

Run Docker Rust tests, format and Clippy, and the repository's relevant Node/Python checks. Run the unchanged doc router check; distinguish ignored local demo instructions from maintained-file failures. Record any skipped native-process or real-design end-to-end checks explicitly.
