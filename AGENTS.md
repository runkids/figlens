# AGENTS.md — figlens

Reads Figma `.fig` files (File > Save local copy) so AI design-to-code pipelines never call Figma: no API rate limits, no login, no running Figma, works offline and in CI. Designers keep working in Figma; figlens owns the read path. A Rust crate (`src/`) with the `figlens` CLI, shipped as native binaries. This file holds only the rules every task needs; task details load per topic, so the whole wiki never lands in one context.

## Language

- Code, identifiers, comments, commits, instruction files (this file, `wiki/`, `skills/`) and `README.md`: English.

## Always Applies

- **Rust runs only in Docker** for development: `docker compose run --rm dev ...`; do not install or use a host Rust toolchain. Users run the native `figlens` binary with no Docker or Rust installed.
- **Format changes need evidence.** Before changing how nodes are decoded, expanded or emitted, load `fig-format`; claim accuracy only after measuring with the `accuracy` topic, on a file the change was not written against.
- **Keep the docs true.** When a doc disagrees with the code, trust the code: verify, then fix the stale doc in the same task. When your change alters behaviour that a topic describes, update that topic page in the same change. Put a new lesson in the kernel as a one-line rule and in its topic as the procedure. Put a finished milestone in `wiki/history/` and add it to the index. Run `python3 scripts/ai-context.py check` after any doc change. The procedure is in the `ai-context` topic.
- **Search before you edit, and change as little as possible.** Match the existing style.
- **Helper files go in the repo.** Helper scripts go in `scripts/`; design files, Figma exports and run outputs go in the gitignored `samples/`.

## Hard Limits

- Never commit design files or Figma exports; `samples/` stays gitignored.
- Shipped binaries build only from `Dockerfile.dist`'s digest-pinned base image; never point the release at a mutable tag.
- The release tag, `Cargo.toml` and `npm/package.json` carry one version.

## Load Context Per Task

```sh
python3 scripts/ai-context.py list      # topics and when to pick them
python3 scripts/ai-context.py <topic>   # print only that topic's sections
python3 scripts/ai-context.py check     # validate paths, headings and byte budgets
```

| Topic | Use when |
|---|---|
| `fig-format` | Changing decoding, instance expansion, emitted REST fields, variables, rendering or the bundle |
| `preview` | Changing the local viewer, offline HTML, handoff or screenshot comparison |
| `accuracy` | Measuring output against Figma's own exports, running `compare`, finding test files |
| `release` | Packaging, GitHub releases, Homebrew, npm, install scripts and `figlens update` |
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

Generated test fixtures use `src/test_support.rs` for atomic directory ownership and scoped cleanup; see `fig-format`.

```sh
docker compose run --rm dev cargo test
docker compose run --rm dev sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings'
(cd npm && node --test)
python3 scripts/ai-context.py check    # doc/topic changes; also runs in CI
```
