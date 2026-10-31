# DTCG token export

Use this for conversion, selection, diagnostics and interoperability; variable decoding remains in [fig-format](fig-format.md).

`variables <file> [node]` keeps its REST-shaped output and decoder unchanged. Explicit `variables <file.fig> --format dtcg --collection ID --mode ID --out FILE [--alias-mode COLLECTION=MODE ...]` converts the live tree's variables snapshot, not the search index; no workspace or index is required. Inspect REST variables first and supply exact collection and mode IDs. A collection the file does not define, such as a library collection `variables <file> <node>` lists, is exported from the library's copies cached in the file and marked `$extensions.org.figlens.libraryCopy: true`: the values saved with this file, which may lag the published library (#161). Before, it failed with `unknown_collection` although figlens itself had listed the id. DTCG requires all three options and forbids a node; REST forbids collection/mode/output options. Create the output parent directory first. No default mode is guessed for the selected collection.

## Mapping and validation

The shared bounded converter in `src/tokens.rs` follows the official [DTCG 2025.10 format](https://www.designtokens.org/TR/2025.10/format/) and [color module](https://www.designtokens.org/TR/2025.10/color/), stable Community specifications, not W3C Recommendations:

- `COLOR` becomes `$type: "color"` with `$value: {colorSpace: "srgb", components: [r, g, b], alpha: a}`; every channel must be numeric, finite and in `0..1`. No rounding, clamping or guessed channels.
- `FLOAT` becomes `$type: "number"` with a finite numeric value. It stays unitless: dimensions and units are not inferred. Strings, booleans, nulls and unsupported types fail.
- Names split on `/` into group paths. Empty segments, segments beginning with `$`, any `.`, `{` or `}`, duplicate paths and group/token prefix collisions fail; names are never silently rewritten. Names remain case-sensitive; consumers can impose additional target-language naming restrictions.
- Aliases within the collection remain DTCG references such as `{color.brand}`. Targets must exist in the selected collection and mode with the same supported `resolvedType`. Missing targets and cycles fail. Values from other modes never repair a missing selected-mode value. Alias validation is iterative, linear in the selected variable count.
- An alias into another collection (a semantic collection over a primitives one, #176) exports the value it resolves to, following further aliases. Each collection on the way is read in the selected mode if it is the exported one, else the mode `--alias-mode COLLECTION=MODE` (MCP `aliasModes: {collectionId: modeId}`) names for it, else its only mode. A target collection with several modes and no mapping fails `cross_collection_alias`, naming the option; a mapping that names the exported collection, an unknown collection or a mode it lacks fails `unknown_alias_mode`. A reference into a document that is not written would not resolve, so the value is literal and the first target is recorded as the token's `aliasOf: {variableId, name, collectionId, modeId}`, with `libraryCopy: true` when that collection is a library's. Resolution is memoized per variable, so it stays linear.

Identity lives in reverse-domain `$extensions["org.figlens"]`, separate from standard token values. The root contains `converterVersion: "1"`, `collectionId`, `collectionName`, `modeId`, `modeName`, plus `source` (the canonical source-file path, consistent between CLI and MCP). Each token contains `variableId`, `collectionId`, `modeId` and `sourceName`, plus `aliasOf` for a resolved cross-collection alias. Names remain readable paths; IDs preserve source identity when names change. The path is provenance, not a content fingerprint or proof of cross-file identity. A non-UTF-8 canonical source path fails before publication rather than losing identity through replacement characters.

Diagnostics are a separate JSON report `{diagnostics: [{code, message, collectionId, modeId, variableId?, name?}]}`, never embedded in the standard token tree. Any conversion diagnostic fails the entire export (CLI nonzero with report in stderr; MCP `isError: true`, `operation_failed`, report in text content) and leaves the output unchanged. Success writes only standard tokens plus the documented identity extension, returning `{path, collectionId, modeId, tokenCount, diagnostics: []}`. Publication uses the shared source-safe writer, including symlink handling; review exports before changing project tokens.

The converter caps each top-level array and selected collection's mode list at 100,000 records, each identity/name at 4 KiB, relevant metadata at 16 MiB, each path at 64 segments, total segments at one million, and compact token output at 64 MiB. Diagnostic overflow is explicit: `resource_limit` includes `omittedDiagnostics`. Parsing and MCP's complete 8 MiB reply budget still apply independently. Oversized selector text is not copied into diagnostics. These are safety ceilings, not measured capacity guarantees.

## Interfaces and scope

CLI and MCP share one converter. Pi registers the MCP server directly; OpenCode uses its managed MCP catalog and native permissions; OMP uses its existing descriptor/stdio bridge and host approval tier. No adapter-specific converter or viewer UI is needed. MCP marks `variables` as potentially writable/destructive because DTCG publication can replace an existing output. OpenCode configurations deliberately disabling MCP retain their existing limited CLI-tool fallback; use the CLI explicitly for tokens there. Installed host sessions must reload an updated binary/catalog; source inspection and adapter tests do not prove installed-host acceptance.

The consuming-project task is an offline JavaScript token pipeline whose input is only a saved `.fig`. Direct export removes the project's custom REST-to-DTCG converter and its alias/name/mode validation. If an equivalent token export already exists, use it instead: figlens adds no benefit to that path. This exports one selected collection/mode, not themes, cross-file merges, CSS, sync or writeback.

Persistent caching is deferred. A future key needs at least the source content hash, collection ID, mode ID and converter version; a search generation is not a complete variable snapshot or a reusable parsed tree.

## Verification

`scripts/make-token-fixture.rs` generates an original synthetic `.fig` under `samples/`, never a Figma export: two modes, sRGB/alpha, unitless numbers, a two-hop alias chain and an alias into a second, single-mode collection. Core regression tests cover invalid paths/collisions, missing values/targets, cycles, cross-collection references with and without a mode mapping, cross-mode references, unsupported/null values and bounded work. See `node scripts/check-dtcg-consumer.mjs --help` for the pinned independent Terrazzo setup and commands; it resolves aliases, builds JavaScript modules, imports them and asserts concrete mode-specific values, not merely JSON/schema validity.

### Observed interoperability and timings

2026-10-31: reviewed implementation with buffered publication, Docker Linux aarch64/glibc 2.41, Rust 1.97.1, release profile. The original synthetic snapshot is 714 bytes (SHA-256 `7d9b65d11ef27aca0edc39bc40a69dbc4ac3eec1200e2e3d8a8b41bc53a0d232`); it is generated, not distributed as a binary fixture. Build and export commands:

```sh
docker compose run --rm dev cargo build --release --locked --bin figlens --example token-fixture --example token-probe
docker compose run --rm dev target/release/examples/token-fixture samples/issue-146-tokens.fig
docker compose run --rm dev target/release/figlens variables samples/issue-146-tokens.fig --format dtcg --collection VariableCollectionId:0:10 --mode 0:11 --out samples/issue-146-light.tokens.json
docker compose run --rm dev target/release/figlens variables samples/issue-146-tokens.fig --format dtcg --collection VariableCollectionId:0:10 --mode 0:12 --out samples/issue-146-dark.tokens.json
node scripts/check-dtcg-consumer.mjs samples/issue-146-light.tokens.json samples/issue-146-dark.tokens.json
samples/issue-146-consumer/node_modules/.bin/tz check samples/issue-146-light.tokens.json
samples/issue-146-consumer/node_modules/.bin/tz check samples/issue-146-dark.tokens.json
```

Independent consumers: `@terrazzo/cli` **2.10.0** and `@terrazzo/parser` **2.10.0**. Both exports passed `tz check`; the parser/build path produced and imported JavaScript modules whose brand, link and action resolved to `[0, 0.25, 1]` / alpha `0.5` / ratio `1.5` for Light, and `[1, 0.25, 0]` / alpha `1` / ratio `2` for Dark. This proves the supported slice interoperates with that consumer, not every DTCG implementation or type.

2026-10-31 (#176): the fixture gained a single-mode Primitives collection and `semantic/accent` aliasing into it; the snapshot size, hashes and timings in this section are from the earlier four-variable fixture. With Terrazzo 2.10.0, both exports passed `tz check` and the consumer check, with accent resolved to `[0, 0.5, 1]` / alpha `1` in both modes and its `aliasOf` intact. On a private design whose two-mode semantic collection aliases all of its colors into a one-mode primitives library collection, both modes exported where 0.10.1 failed with `cross_collection_alias`.

An actual warm stdio MCP session exported both modes byte-identically to CLI, retained REST output, rejected source overwrite/root escape, preserved an existing output on conversion failure and answered the next request. CLI failure exited 1 with JSON stderr and no stdout; a non-UTF-8 canonical source failed cleanly without changing the output. Help and served `workflow build` included the opt-in export. Raw smoke metadata and benchmark samples are local under `samples/issue-146-run-z6gl80fh/{smoke,timings}.json`; CLI binary SHA-256 `e137d400b5ca5e8ca63e514825cdf301d68724cc306091f220e37b88a39aa9fd`.

`scripts/probe-tokens.rs` measures each boundary with a separate timer. Ten fresh processes per mode, OS cache not flushed; medians in milliseconds:

| Boundary | Light | Dark |
|---|---:|---:|
| Parse: open/inflate/schema + preflight/decode + Tree | 0.1081 | 0.1036 |
| Existing variable extraction | 0.0363 | 0.0370 |
| DTCG conversion + source extension | 0.0097 | 0.0106 |
| Buffered pretty JSON + safe write/publication | 0.2372 | 0.2513 |

One probe invocation (repeat with distinct output names; mode `0:12` selects Dark):

```sh
docker compose run --rm dev /work/target/release/examples/token-probe /work/samples/issue-146-tokens.fig VariableCollectionId:0:10 0:11 /work/samples/issue-146-timing.tokens.json
```

Probe SHA-256 `f5ce6b1692bde075f94689c78814bd4736489bcb715e553870fcc7aa4dedf379`. These tiny-fixture measurements do not establish large-file latency, peak memory or cross-platform performance. They give no evidence that persistent token caching is necessary; publication dominates this sample and caching would not remove it. Decoder/REST emission is unchanged, so no new Figma field-accuracy claim is made.
