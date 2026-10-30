# Semantic and hybrid search (#94)

Optional, experimental local semantic ranking over the same stored scope as [search](search.md). Code: `src/search/semantic.rs` (layer, fusion, coverage), `src/semantic/profile.rs` (profile, hashing, runtime start), `src/command.rs` (mode parsing and errors), `src/main.rs` (`search --mode`, `models`), `src/mcp.rs` (`models` tool, `mode`). Quality is unproven: do not claim it meets the gates.

## Modes and reply

- `--mode text|semantic|hybrid` (MCP `mode`). `text` is the default and is unchanged. `semantic` ranks layers by the best name or text chunk per layer (exact scan, no ANN). `hybrid` fuses the text and semantic rankings with Reciprocal Rank Fusion, `sum(1/(60+rank))`, ranking version `rrf-60-1`, 1000 candidates per channel.
- Semantic matches carry `{field: name|text, chunk, basis: "semantic"}` and no text spans. In hybrid, lexical matches get `basis: "lexical"`.
- The reply adds `query.requestedMode`/`usedModes`, `profileId` and `ranking.version`/`channels`/`candidatesPerChannel`.
- `coverage.layers.semantic` is `{state: ready|partial|unavailable, profileId, inputs: [name, text], chunks, texts, embeddedTexts, excluded, reasons}`. Reason codes: `semantic_layer_building`, `chunk_refused`, `semantic_capacity`, `work_limit`/`byte_limit`/`deadline`, `no_profile`, `profile_unreadable`, `runtime_failed`. A partial layer, or a hybrid page downgraded to text, has `nextCursor: null`.
- No profile: `semantic` fails with `mode_unavailable` (recovery names the `models` tool); `hybrid` returns the text results with `usedModes: ["text"]` and the reason.

## Layer

The semantic layer is built per process for one source snapshot, scope and profile. Identical chunk texts share one vector. A build spends at most 30 s per request and resumes on the next call, reporting `semantic_layer_building` meanwhile.

## Models

- `figlens models list|status`, `models configure bge-m3 --model DIR [--runtime FILE]`, `models remove bge-m3` (MCP `models`: `action`, `profile`, `modelDir`). The profile is stored in `models.json` under the config root (same XDG/`%LOCALAPPDATA%` rules as other state). `remove` forgets the profile and keeps the files.
- One documented profile: `bge-m3` = BAAI/bge-m3 revision `5617a9f61b028005a4858fdac845db406aefb181` (MIT); `config.json`, `pytorch_model.bin` and `tokenizer.json` are pinned by SHA-256. figlens never downloads: the user fetches that revision. `configure` hashes the files and starts the runtime once offline; the executable and model files are re-hashed before every runtime start.
- Runtime: `figlens-embed`, looked up next to the figlens executable unless `--runtime` is given. CPU only, about 2.5 GiB RSS (diagnostic, not a cap). Ctrl-C (CLI) or `notifications/cancelled` (MCP) stops a running semantic search or `configure` and its child.
- MCP `configure` runs only the `figlens-embed` beside figlens, and `modelDir` and each pinned file must lie inside the server's roots.

## Known gaps

- Only the name and the effective text are model inputs: no component property values or scope summaries.
- Workspace search (`--workspace`/`--id`, MCP `workspaceId`) with `semantic` or `hybrid` returns `mode_unavailable`: the stored model layer is not implemented.
- On the frozen #94 suite the pinned profile missed the pre-set synonym/purpose wrong-top-1 gate (0.25 against 0.20). The feature stays optional and experimental.
- Pi, OpenCode and OMP use the MCP contract; their guidance lives in the served skill ([mcp](mcp.md)).
