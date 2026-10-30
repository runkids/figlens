# First success: a public local-file smoke check

No design file or model account is needed. This checks a specific native figlens executable, finds a known frame, and validates a usable bundle before you troubleshoot your own design or agent setup.

## Run the smoke check

From this checkout, with Python 3.8+ and an already available native binary:

```sh
python3 scripts/first-success-smoke.py --binary /absolute/path/to/figlens
```

Use `figlens.exe` on Windows. The argument is a file path, never a PATH lookup, npm launcher or installer. No downloads occur. The script prints the resolved binary path and its reported version. It copies only this example into a unique system temporary directory, writes the bundle there, checks it without extracting it, and removes its own directory on success or failure. Existing projects and caches are untouched. A failure prints `FAIL A`, `FAIL B` or `FAIL C` and exits nonzero.

For the current source, developers can use the repository's Docker toolchain:

```sh
docker compose run --rm dev sh -c 'cargo build --locked --bin figlens && python3 scripts/first-success-smoke.py --binary /work/target/debug/figlens'
```

The three checkpoints are:

| Checkpoint | Command inside the check | Required evidence |
|---|---|---|
| A: executable | `figlens --version` | Identified executable path and `figlens <version>` output |
| B: design | `figlens find design.fig "First success" --type FRAME` | One matching frame: `First success`, ID `0:1`, size 320 × 180; search not truncated |
| C: handoff | `figlens bundle design.fig 0:1 frame.zip` | Readable ZIP; correct manifest and structure target; `READY` / `LOCAL` text; original bitmap and triangle SVG; no missing assets; decodable 320 × 180 reference with expected content |

These filenames are local to the script's temporary directory. It also checks manifest file sizes/hashes and asset-to-node associations. It does not compare timestamps, absolute output paths, ZIP entry ordering or compressed bundle bytes.

To inspect the same steps manually on macOS/Linux, from the checkout root:

```sh
FIGLENS_BIN=/absolute/path/to/figlens
scratch="$(mktemp -d "${TMPDIR:-/tmp}/figlens-first-success-manual.XXXXXX")"
"$FIGLENS_BIN" --version
"$FIGLENS_BIN" find examples/first-success/design.fig "First success" --type FRAME
"$FIGLENS_BIN" bundle examples/first-success/design.fig 0:1 "$scratch/frame.zip"
python3 - "$scratch/frame.zip" <<'PY'
from pathlib import Path
import runpy, sys
runpy.run_path('scripts/first-success-smoke.py')['validate_bundle'](Path(sys.argv[1]))
print('PASS C: bundle contents and reference verified')
PY
```

The manual scratch directory remains for inspection; remove only that directory when finished. A ZIP merely existing or a command exiting zero is not the C checkpoint.

## What is in the file?

`design.fig` is an **original synthetic fixture**, built with the existing `kiwi-schema` encoder used by figlens's command tests. It is not a downloaded template, Figma export, or proof of full Figma format compatibility. **Opening/editing it in Figma has not been verified.**

| Item | Known content |
|---|---|
| Screen | `First success`, `0:1`, 320 × 180 |
| Card | `0:2`, 288 × 148 at (16, 16) |
| Bitmap | `0:3`, displayed at 32 × 32; original 4 × 4 teal/white tile, embedded as `images/first-success-tile` |
| Text | `0:4`: `READY`; `0:5`: `LOCAL` |
| Vector | `0:6`: an original 24 × 24 blue triangle |
| File size | 1,615 bytes |
| SHA-256 | `bd571867ce39471b45fc257538e785c25751a924e5485234cdb59c62ca2515e0` |

The bitmap, triangle and eight block-letter shapes were created specifically for this fixture, without third-party photos, trademarks or font files. Text uses embedded original `Figlens Blocks` outlines; there is no installable font. The reference therefore needs no system font or network. The smoke check normalizes supported lossless PNG color forms to RGBA and samples a few interior colors (±1 channel level for rounding), not antialiased glyph edges or whole-image equality. It catches an empty/wrong reference; it does not establish real-design pixel-perfect fidelity or font availability for a web implementation.

**License and attribution:** this fixture, its embedded artwork, lettering and generator are provided under the repository's [MIT license](../../LICENSE), Copyright (c) 2026 runkids. Redistribution is allowed with that copyright and license notice. This statement applies to this original fixture only; it does not clear other files in ignored `samples/` or third-party demo assets.

The file and these steps belong to the source checkout; `samples/` is not a public download. No `.fig` is added to npm or native release archives.

## Recreate or update the example

The fixed generator is [scripts/make-first-success.rs](../../scripts/make-first-success.rs). It only writes this example and refuses to overwrite an existing path; it is not a general Figma writer. From the repository root:

```sh
mkdir -p samples
docker compose run --rm dev cargo run --locked --example first-success-fixture -- samples/first-success-new.fig
docker compose run --rm dev python3 scripts/first-success-smoke.py --binary /work/target/debug/figlens --example samples/first-success-new.fig
```

The ignored `samples/` directory must exist and the new filename must be unused. After an intentional generator change, verify the new file, review its content, replace only this public fixture, and update the byte count, SHA-256 and known expectations here and in the smoke check. The CI regression corpus records this file's SHA-256 and output too: rerun `scripts/corpus-check.py` with `--update` and review the `scripts/corpus/` diff (see the `accuracy` topic). Do not copy a private design over it. A different encoder/compressor can change bytes without changing meaning; semantic smoke validation is the acceptance gate, not byte-for-byte reproduction.

Local candidate acceptance passed checkpoints A/B/C on macOS 27.0.1 arm64 with Node 24.13.0 and npm 11.6.2. It installed the real npm tarball and exercised its Docker-built `aarch64-apple-darwin` candidate from `3811c40` plus batch-5 uncommitted changes (`figlens 0.6.0`; native SHA-256 `103bb22e4af49a32d58a294e2b4c045a0d57a12e1291f19dddf2dcb92aa84172`). This is local candidate evidence, not a published-release or hosted-CI result. That local run does not validate published release binaries, macOS Intel or Windows. These are existing CLI commands, not a new product interface.

## After the deterministic check: ask your agent

With figlens already configured for your agent and a frontend project open, give it the absolute example path and ask:

```text
Build only the "First success" screen from /absolute/path/to/figlens/examples/first-success/design.fig in this project.
Find the frame by name. Follow this project's stack, components and image workflow.
Read the text and assets, compare the running page with the bundled reference,
and fix confirmed differences. Report the fixture-only lettering, missing assets
and checks you could not run.
```

For an owned Figma design, editable implementation and independent screenshot comparison, follow the [task-list case study](../task-list/README.md). Its model experiment is separate from this deterministic setup check; private Figma exports are not included.

This second step depends on the model, project and available browser tools. The smoke script does not invoke an agent, build a website, guarantee visual equivalence or promise a completion time. The file is a fixed snapshot; the reference is figlens's own render. Existing [snapshot tradeoffs and renderer limitations](../../README.md#limitations) still apply.

## When a checkpoint fails

| Failure | Next check |
|---|---|
| A: binary missing, wrong version output, cannot execute | Confirm the explicit path, platform/architecture and execute permission. An npm launcher is not the native binary path. |
| B: example missing | Use this checkout's `examples/first-success/design.fig` or pass `--example` explicitly. An ignored local sample is not supplied by a fresh clone. |
| B: frame missing, different ID/size or parse error | Confirm you selected this fixture and check its published hash; keep the reported binary version with the error. |
| C: cannot write/read ZIP | Check free space and a writable system temporary directory (`TMPDIR` on POSIX); the checker creates its own output directory. |
| C: missing asset, wrong target, reference decode/content error | Keep the checkpoint error and binary version. This fixture embeds its bitmap and glyph outlines; do not fix it by downloading fonts or substituting assets. The smoke PNG decoder supports non-interlaced 1/2/4/8-bit indexed or grayscale and 8-bit RGB, grayscale-alpha or RGBA, including palette and color-key transparency; 16-bit samples and interlacing are unsupported. |

For later platform/release acceptance, reuse the same `--binary` entry with each explicitly selected build. Capture its output and exit status; no CI matrix or download mechanism is introduced here.
