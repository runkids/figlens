# Semantic and hybrid search (#94)

Optional, experimental local semantic ranking over the same stored scope as [search](search.md), also for workspace search. Code: `src/search/semantic.rs` (layer, fusion, coverage), `src/semantic/profile.rs` (profile, hashing, runtime start), `src/command.rs` (mode parsing and errors), `src/main.rs` (`search --mode`, `models`), `src/mcp.rs` (`models` tool, `mode`). Quality is unproven: do not claim it meets the gates.

## Modes and reply

- `--mode text|semantic|hybrid` (MCP `mode`). `text` is the default and is unchanged. `semantic` ranks layers by the best name or text chunk per layer (exact scan, no ANN). `hybrid` fuses the text and semantic rankings with Reciprocal Rank Fusion, `sum(1/(60+rank))`, ranking version `rrf-60-1`, 1000 candidates per channel.
- Semantic matches carry `{field: name|text, chunk, basis: "semantic"}` and no text spans. In hybrid, lexical matches get `basis: "lexical"`.
- The reply adds `query.requestedMode`/`usedModes`, `profileId` and `ranking.version`/`channels`/`candidatesPerChannel`.
- `coverage.layers.semantic` is `{state: ready|partial|pending|unavailable, profileId, inputs: [name, text], chunks, texts, embeddedTexts, excluded, reasons}`. Reason codes: `semantic_layer_building`, `chunk_refused`, `semantic_capacity`, `work_limit`/`byte_limit`/`depth_limit`/`deadline`, `no_profile`, `profile_unreadable`, `runtime_failed`. A partial layer, or a hybrid page downgraded to text, has `nextCursor: null`.
- No profile: `semantic` fails with `mode_unavailable` (recovery names the `models` tool); `hybrid` returns the text results with `usedModes: ["text"]` and the reason.

## Layer

Stateless search: the semantic layer is built per process for one source snapshot, scope and profile. Identical chunk texts share one vector. A build spends at most 30 s per request and resumes on the next call, reporting `semantic_layer_building` meanwhile.

## Workspace layer

Code: `src/workspace/query.rs` (`search_mode`, `model_layers`), `src/workspace/model.rs`, `src/workspace/jobs.rs` (embed job), schema 9 in `src/workspace/database.rs`.

- Workspace search (`--workspace`/`--file`/`--id`, MCP `workspaceId`) supports `semantic` and `hybrid`. It reads each ready source's stored model layer under the active profile and never embeds design text; only the query is embedded, by the warm local runtime. Filters apply before ranking, all sources share one vector space, and hybrid uses the same RRF. Semantic without a profile is `mode_unavailable`; hybrid answers as text with the reason.
- A source whose layer was never built is `pending` in `coverage.layers.semantic.sources[].state` with an `index embed` next step. A layer still building or partial, or missing vectors, makes the page partial with `nextCursor: null`; ready layers page normally.
- `index embed <select> [--source ID] [--scope NODE] [--request-id ID]` (MCP `index` action `embed`) submits a fenced job (kind `embed`, shown by `jobs status`) per published scope. Its worker is the profile's `figlens-embed` via `__model-worker`: one process, killed with its process tree on cancel. It reuses stored vectors of unchanged chunk texts (no model load if none is missing), commits batches with progress, and publishes the layer ready or partial only while its fence holds: same attempt, not cancelled, generation still published, membership and profile unchanged. A crash or cancel leaves the layer building. Without a profile it is `mode_unavailable`.
- An embed has its own active slot per source, scope, generation and profile, so it never blocks or joins a text build (nor `index track`); an interrupted one is retried as an embed.
- The MCP server and the watch runner queue an embed after each text build publishes and run it on their own model lane or thread, so a save or refresh never waits for it; a runner exit cancels an embed only when no other caller joined it. The one-shot CLI lists `index embed` in `nextSteps` instead of queueing a job.
- Schema 9 stores vectors once per profile and chunk-text SHA-256 (`model_vectors`), per-generation references (`model_refs`) and layer progress (`model_layers`). Dropping a generation (retention, rebuild, remove, clear) drops its references and any vector nothing references. Reset deletes the database as before.
- Diagnostic, not a gate: embedding 210 short texts took 5.8 s including model load on an 18-CPU Docker host; a cold CLI workspace semantic search took 1.6 s.

## Models

- `figlens models list|status`, `models configure bge-m3 --model DIR [--runtime FILE]`, `models remove bge-m3` (MCP `models`: `action`, `profile`, `modelDir`). The profile is stored in `models.json` under the config root (same XDG/`%LOCALAPPDATA%` rules as other state). `remove` forgets the profile and keeps the files.
- Text profile `bge-m3` = BAAI/bge-m3 revision `5617a9f61b028005a4858fdac845db406aefb181` (MIT); `config.json`, `pytorch_model.bin` and `tokenizer.json` are pinned by SHA-256. figlens never downloads: the user fetches that revision. `configure` hashes the files and starts the runtime once offline; the executable and model files are re-hashed before every runtime start.
- `models status` marks an unusable profile `runtime_missing`, `runtime_changed` (the runtime is re-hashed; an upgrade replaces it) or `model_missing`, with a `models configure` recovery for the same files and the shipped runtime; the new profile id keeps old vectors apart. Building from source: `docker compose run --rm dev sh -c 'cargo build --release --bin figlens && cargo build --release --features embed --bin figlens-embed'` puts both executables side by side in `target/release`.
- `models status` always reports `configRoot` (where `models.json` lives) and `defaultRuntime` `{path, available}` (the `figlens-embed` beside figlens; presence only), with or without a profile.
- Vision profile `siglip-base` = google/siglip-base-patch16-224 revision `7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed` (Apache-2.0); `config.json`, `preprocessor_config.json` and `model.safetensors` are pinned by SHA-256, and the child's hello adds `modality: image`, the `PREPROCESS` policy, revision and dim 768 to the profileId. Each record stores its modality (one written before has none and is text): semantic search takes the first text profile, visual search the first image one (`active_visual`). Both share the one warm child slot (bounded memory in v1): switching between a text and an image profile restarts the child, re-hashing the pinned files and loading the other model; for `siglip-base` that took about 1.0 s (with two images) against 0.38 s warm, on a warm OS cache in Docker arm64.
- Image requests (`kind: image`, `Runtime::embed_images`) carry 1 to 4 normalized query rasters as hex of exactly 256x256x3 RGB bytes (`QueryImage::rgb`/`from_rgb`: no resampling or metadata). The child checks count, encoding and length before decoding any, then embeds with `search::visual::model`; deadlines, ids, dimension, finite and L2 checks and cancellation are those of text requests. A text child refuses images and an image child texts, and both keep serving. No query image is written.
- Runtime: `figlens-embed`, looked up next to the figlens executable unless `--runtime` is given. CPU only, about 2.5 GiB RSS (diagnostic, not a cap). Ctrl-C (CLI) or `notifications/cancelled` (MCP) stops a running semantic search or `configure` and its child.
- MCP `configure` runs only the `figlens-embed` beside figlens, and `modelDir` and each pinned file must lie inside the server's roots.

## Known gaps

- Only the name and the effective text are model inputs: no component property values or scope summaries.
- No layer-ready or profile-change event is recorded: `changes` and subscriptions see design revisions only; read model readiness from `coverage.layers.semantic` and `jobs status`.
- On the frozen #94 suite the pinned profile missed the pre-set synonym/purpose wrong-top-1 gate (0.25 against 0.20). The feature stays optional and experimental.
- Pi, OpenCode and OMP use the MCP contract; their guidance lives in the served skill ([mcp](mcp.md)).
