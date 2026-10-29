# AGENTS.md — figlens

Reads Figma `.fig` files (File > Save local copy) so AI design-to-code pipelines never call Figma: no API rate limits, no login, no running Figma, works offline and in CI. Designers keep working in Figma; figlens owns the read path. A Rust crate (`src/`) with the `figlens` CLI, shipped as native binaries. This file holds only the rules every task needs; task details load per topic, so the whole wiki never lands in one context.

## Language

- Code, identifiers, comments, commits, instruction files (this file, `wiki/`, `skills/`) and `README.md`: English.

## Always Applies

- **Rust runs only in Docker** for development: `docker compose run --rm dev ...`; do not install or use a host Rust toolchain. Users run the native `figlens` binary with no Docker or Rust installed.
- **Format changes need evidence.** Before changing how nodes are decoded, expanded or emitted, load `fig-format`; claim accuracy only after measuring with the `accuracy` topic, on a file the change was not written against.
- **Keep the docs true.** When a doc disagrees with the code, trust the code: verify, then fix the stale doc in the same task. When your change alters behaviour that a topic describes, update that topic page in the same change. Put a new lesson in the kernel as a one-line rule and in its topic as the procedure. Run `python3 scripts/ai-context.py check` after any doc change. The procedure is in the `ai-context` topic.
- **Search before you edit, and change as little as possible.** Match the existing style.
- **Check CLI, MCP, Pi, OpenCode and OMP together.** For every feature or optimization, assess each interface, complete affected behavior/contracts/guidance/tests, and explain why any interface needs no change. A new feature or changed command also updates `--help`, every README locale, a served skill section and its wiki topic; see `review`.
- **Helper files go in the repo.** Helper scripts go in `scripts/`; private designs, Figma exports and run outputs go in the gitignored `samples/`. The original public fixture below and the reviewed regression baselines in `scripts/corpus/` are the exceptions.
- **Review before every PR.** Run the `figlens-code-review` passes on the pinned diff and fix verified findings before opening or updating a PR, docs- and wiki-only changes included; see `review`.
- **Open PRs as drafts.** Drafts skip CI; request `@codex review` on the new draft, fix review findings there, then mark the PR ready to run CI; see `review`.
- **Open issues by the rules.** English, `<scope>: <summary>` titles, one type label plus `area:` labels, no private design details; see `review`.
- **Own PR follow-up.** Check AI reviews and CI, fix verified findings, then request and check `@codex review` on the updated head; see `review`.

## Hard Limits

- Never commit private design files or Figma exports; `samples/` stays gitignored. Only the original synthetic `examples/first-success/design.fig` is distributable, with its [provenance and update procedure](examples/first-success/README.md).
- Shipped binaries build only from `Dockerfile.dist`'s digest-pinned base image; never point the release at a mutable tag.
- The release tag, `Cargo.toml` and `npm/package.json` carry one version.

## Load Context Per Task

```sh
python3 scripts/ai-context.py list      # topics and when to pick them
python3 scripts/ai-context.py <topic>   # print only that topic's sections
python3 scripts/ai-context.py check     # validate paths, headings, links, orphans and byte budgets
```

| Topic | Use when |
|---|---|
| `mcp` | Changing MCP instructions, tool discovery or built-in workflow guidance |
| `gcs` | Changing the `gs://` snapshot handoff, its download cache or refresh |
| `performance` | Measuring load time and memory, reading the perf baseline, changing cache or decode cost |
| `parsing` | Changing untrusted container/schema/value decoding or parsing budgets |
| `fig-format` | Changing decoding, instance expansion, emitted REST fields or variables |
| `discovery` | Changing find or inspect summaries (fields, limits, truncation, `--flat` lines), `tree --top-frames` screen lists or `export --match-dir` icon matching |
| `search` | Changing search indexing, normalization, ranking, cursors, coverage or its budgets |
| `bundle` | Changing the bundle zip, manifest, context.md, batch bundles, font inventory or `verify` |
| `render` | Changing SVG or raster output: text and glyphs, image fills, effects, export areas, image adjustments |
| `preview` | Changing the local viewer, offline HTML, handoff or screenshot comparison |
| `pi` | Changing the Pi extension, reviewed component mappings or its npm packaging |
| `integrations` | Changing OpenCode V2/OMP, shared package, permissions or installed-host acceptance |
| `accuracy` | Measuring output against Figma's own exports, running `compare`, finding test files, the CI regression corpus |
| `release` | Packaging, GitHub releases, Homebrew, npm, install scripts and `figlens update` |
| `review` | Delivering features/optimizations across CLI, MCP and Pi; reviews, PR follow-up and opening issues |
| `direction` | Deciding what to build next, or revisiting a past decision |
| `ai-context` | Adding, moving or splitting docs and topics; `check` failures |

Pick the single closest topic. Load a second one only for a task that really crosses two seams.

When no topic fits, read `wiki/README.md`. To add a topic:
1. Put the content in `wiki/`.
2. Map it in `wiki/ai-context.json`.
3. List it in both tables.
4. Run `check`.

If a topic goes over budget, split it rather than raising the cap. Skills are workflow adapters that load topics. They are not the source of truth.

## Verification

Pi viewer startup must wait for a successful catalog response, not just a printed URL; see `pi`.
Verify each host's independent entry from the same extracted npm candidate; dispatch is not model completion; see `integrations`.
Validate host reference-output directories before export, including symlinked parents; see `integrations`.
Cancel owned CLI process trees and await exit before reference cleanup; see `integrations`.
Check parsing budgets before Kiwi recursion or declared-length expansion; see `parsing`.
Run `expand::check_depth` before any walk that recurses once per layer; see `parsing`.
Bound declared and actual original-image bytes per image and per scope before retaining them; see `parsing`.
Publish bundle/export/snapshot files through `output::write` to protect sources and existing outputs; see `parsing`.
Apply MCP budgets before request admission and to complete serialized replies; see `mcp`.
Generated test fixtures use `src/test_support.rs` for atomic directory ownership and scoped cleanup; see `fig-format`.
Stroke checks read `render::stroke_weights`, not `strokeWeight`, which per-side strokes can leave 0; see `render`.
Charge search and bounded-expansion visits, copies and allocations to `expand::Bounds` before doing them; see `search`.
Keep OpenCode's managed MCP key apart from the plugin's `figlens_*` tool names, or OpenCode drops the server; see `integrations`.
Image-fill changes must test shifted sprites exported from a clipping frame; see `render`.
Background blur backdrops reference blur-free copies within the per-export budget, or nested blurs grow exponentially; see `render`.
Reproduce a browser SVG misdraw with the consumer's optimizer and inlining before changing export coordinates or ids; see `render`.

```sh
docker compose run --rm dev cargo test
docker compose run --rm dev sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings'
(cd npm && node --test)
npm test                             # Pi workflow and npm package contents
python3 scripts/ai-context.py check    # doc/topic changes; also runs in CI
docker compose run --rm dev python3 -B scripts/corpus-check.py ...  # output changes; setup and --update in `accuracy`
```
