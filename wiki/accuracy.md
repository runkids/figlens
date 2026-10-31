# Accuracy: measuring figlens against Figma

Load this to measure how close figlens's output is to Figma's, to run `compare`, or to find test files.

## Test files

- Design files never enter the repo; put them in the gitignored `samples/`.
- Figma Community files work well as unfamiliar input: open one in Figma (Open in Figma copies it to your drafts), then File > Save local copy. Files used so far: MCP Apps for Claude, Aurora Design System, Landing Page Free, Nucleus UI Lite, Real Estate App UI Kit. Check each file's license before sharing outputs.
- A new behaviour is trusted only after it ran on a file it was not written against: each community file above found a bug the first file never hit.

## Getting a reference

- Structure: export the same frame from a Figma plugin with `node.exportAsync({ format: "JSON_REST_V1" })` (the "plugin" export), or read it from the REST API (`GET /v1/files/:key/nodes?ids=<id>`). Save the plugin's `document` wrapper as `nodes.json`.
- Variables: a plugin can write `design/variables.json` from `figma.variables`; `compare` diffs it when given as a third field.
- Pixels: export the frame and its assets from Figma (Export panel or `exportAsync` PNG/SVG) next to the JSON.
- Export the `.fig` and the reference on the same day, so design edits do not show up as differences.

## Running compare

```bash
docker compose run --rm dev cargo run --release -- compare samples/design.fig samples/ref/nodes.json 12:345
# many frames: one `node|nodes.json[|variables.json]` per line
docker compose run --rm dev cargo run --release -- compare samples/design.fig --frames samples/frames.txt
```

It reports per frame: ids missing on either side, then name, type, visibility, bounding box, characters, fills and the emitted fields in `compare::FIELDS`, and a summary of identical frames per field.

## Regression corpus

CI's `test` job runs a small owned corpus so a decode, projection or render change cannot silently alter shapes the unit fixtures miss. It is a **regression** lane, not an accuracy measure: the baselines in `scripts/corpus/` are figlens's own output for original synthetic inputs, never Figma exports, so a difference means behaviour changed, not which side is right. Claims about accuracy still need the independent references above.

| Case | Input, node | Catches |
|---|---|---|
| `first-success` | public `examples/first-success/design.fig`, `0:1` | glyph text, original image bytes, vector SVG |
| `auto-layout` | corpus `0:1` | layoutMode, spacing, padding, FILL/FIXED sizing, absolute child |
| `paints-strokes` | corpus `0:2` | linear gradient, inside stroke, mixed corner radii, paint opacity |
| `effects-clipping` | corpus `0:3` | drop shadow, rotation, layer opacity, clipped overflow |
| `components` | corpus `0:4` | instance expansion ids and `componentId` |

The corpus `.fig` is not committed: `scripts/make-corpus.rs` (example `corpus-fixture`) writes it at run time with a fixed ZIP timestamp, so its SHA-256 is reproducible. `scripts/corpus/expected.json` records both input SHA-256s and every bundle asset's SHA-256. For each case `scripts/corpus-check.py` checks, from one temporary directory:

- exact: CLI `nodes` equals `<case>.nodes.json` and the bundle's own (stripped) `design/nodes.json` equals `<case>.bundle.nodes.json`, value types included (the first 20 differing JSON paths are printed), bundle asset hashes match, `missing-assets.json` is empty, and MCP `nodes` and `bundle` give the same documents, asset hashes and reference bytes as the CLI;
- threshold: the bundle's `references/screen.png` keeps its size and at most 0.5% of its pixels differ from `<case>.png` by more than 16/255 in any channel.

Exit 0 passes, 1 is a product regression (`FAIL NODES|BUNDLE|ASSET|RENDER|MCP <case> ...`, or a failed command), and 2 is setup (`SETUP:` missing binary, input or any case's baselines, or an input whose SHA-256 is not the recorded one). A failing case also prints what it covers. Each CLI command has a 60-second timeout and each MCP reply 30 seconds; the whole lane runs in about a second. Memory is bounded by the inputs: both are fixed, a few KB, with frames up to 240 × 120, so there is no separate memory cap. Only the synthetic cases' inputs and figlens's output are printed. Only `first-success` has bundle assets to hash; the corpus frames' shapes export no assets.

From a clean checkout:

```sh
docker compose run --rm dev sh -c 'mkdir -p samples/corpus && cargo build --locked --bin figlens && cargo run --locked --example corpus-fixture -- samples/corpus/corpus.fig && python3 -B scripts/corpus-check.py --binary target/debug/figlens --corpus samples/corpus/corpus.fig'
```

The generator refuses to overwrite, so delete `samples/corpus/corpus.fig` before a rerun. Verified on 2026-10-29: removing the `itemSpacing` emission in `src/rest.rs` fails with `FAIL NODES auto-layout $.document.itemSpacing: missing` and exit 1; restoring it passes.

When a change is meant to alter output, or the generator or public fixture changes, rerun the check with `--update`, review the `scripts/corpus/` diff (and the PNGs) as part of the change, and say why it moved. A new case adds a frame to the generator and a row to `CASES` and to the table above; keep it original and synthetic.

## Results

- Sweep of a large production file (2026-10-22, 826 frames with plugin captures): structure (id/name/type/visible/bbox/characters) identical in 771 frames; blendMode, effects, rotation, layoutWrap, isMask, componentProperties 826; boundVariables 796; styles 798; renderBounds (1 px) 759; `variables.json` 783; text `style` 792, `styleOverrideTable` 820, `characterStyleOverrides`/`lineTypes`/`lineIndentations` 826. Pixels ("ok" = at most 1.5% of pixels off by more than 48/255): image fills 2608/2608 byte-identical, SVG assets 5014/5197 ok, reference renders 659/825, render assets 88/221. The remaining differences traced mostly to design edits between capture and `.fig` export and to the renderer gaps in the `direction` topic.
- Community files (2026-10-22): all 306 top-level frames of the five files bundle with no missing assets. A hand-written HTML/CSS page built only from one bundle's `nodes.json` and SVGs (MCP Apps for Claude, "List views") matches the bundle's reference to a mean of 5.2/255 with 4.25% of pixels off by more than 32, all of it text antialiasing and line breaks.

## Local design-to-HTML demo

Recorded on 2026-10-22 with Pi building the “List views” frame (`467:24651`) from [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). This is a separate implementation from the hand-written page measured above; no pixel-difference score was measured for Pi's result.

### Bundle extraction timing

The input was 60,260,522 bytes (57.5 MiB), SHA-256 `ca728ac74c75024ffbe3760edaf86dcf68aa2c02123443248a76062e05184d29`. The selected 1440 × 1024 frame contained 153 nodes. Its bundle contained 32 files, including 25 SVG assets, with an empty `missing-assets.json` list.

Environment: figlens 0.1.2, checkout `42b15c36cab89be736c656409456fb9915afb16c`, prebuilt release binary in the Docker dev container, Linux aarch64 on OrbStack, 18 visible logical CPUs. The filesystem cache was warm; build time and AI execution were excluded. These are measurements of one frame on one machine, not a general performance guarantee.

| Run | Wall-clock seconds |
|---|---|
| 1 | 0.567396 |
| 2 | 0.469512 |
| 3 | 0.471480 |
| 4 | 0.475669 |
| 5 | 0.463792 |
| **Median** | **0.471480** |

To repeat, save the same Community file as `samples/mcp-apps-for-claude.fig`, build the release binary in Docker, then time five subprocesses inside the container:

```sh
docker compose run --rm dev cargo build --release
docker compose run --rm -T dev python3 - <<'PY'
import statistics
import subprocess
import time
from pathlib import Path

out = Path('samples/demo/timing')
out.mkdir(parents=True, exist_ok=True)
command = [
    'target/release/figlens', 'bundle',
    'samples/mcp-apps-for-claude.fig', '467:24651',
]
subprocess.run(command + [str(out / 'warmup.zip')], check=True, capture_output=True)
times = []
for i in range(5):
    started = time.perf_counter()
    subprocess.run(command + [str(out / f'{i}.zip')], check=True, capture_output=True)
    times.append(time.perf_counter() - started)
print({'seconds': times, 'median_seconds': statistics.median(times)})
PY
```

### Recording and implementation

Pi read the local bundle and wrote HTML/CSS with 19 SVG icons. All 25 path geometries and their path attributes in those icons match the bundle's SVGs; outer SVG wrappers adapt the icon bounds. The page has a local Share dialog and composer. Desktop and narrow layouts, including 375 px, were checked in the browser. System sans and Georgia replace unavailable Anthropic fonts; replies reset on reload. No Figma API or running Figma app was used for the design read.

The source take is approximately six minutes. `scripts/record-orca-window.py` captured 1,013 snapshots of the verified Orca window over 360.16 seconds, preserving their real timing in the source video. The earlier [8× video](https://github.com/runkids/figlens/blob/v0.10.1/assets/demo-8x.mp4) and [GIF](https://raw.githubusercontent.com/runkids/figlens/v0.10.1/assets/demo-8x.gif) speed the whole take up uniformly by **8×**, without simulated typing or reconstructed terminal output. The capture is about 2.8 snapshots per second, so encoding it as video does not provide native 30 fps motion. Design files, bundles, raw captures and the full take stay in the gitignored `samples/demo/` directory.

## Taskflow React and Tailwind demo

The current README demo is [taskflow-demo.mp4](https://github.com/runkids/figlens/blob/v0.10.1/assets/taskflow-demo.mp4), with an [animated preview](https://raw.githubusercontent.com/runkids/figlens/v0.10.1/assets/taskflow-demo.gif) linked to the full clip and a [static comparison poster](https://raw.githubusercontent.com/runkids/figlens/v0.10.1/assets/taskflow-demo-poster.jpg) for the Pi catalog. The looping GIF plays the edited clip at another 3× speed, at 960 × 540 and 6 fps with a 256-color palette. It preserves the full frame without cropping; the linked MP4 remains 77.5 seconds. It shows an actual Pi session in Orca, starting with a clean React/TypeScript/Tailwind project, frontend conventions and one editable local `.fig`. The main workflow uses `/figlens plan`, then implementation, browser checks and corrections. Pi and the live browser are visible together; source-code cards show the delivered component structure. No template implementation was supplied to the agent.

The design is the Cloth project board, `Desktop - 1`, frame `2:54` at 1440 × 1024, from [Task Flow — Project Management Dashboard UI](https://www.figma.com/community/file/1224268866120711793). The supplied design credits Eymen Eid. The video adapts that design into React; no endorsement is implied. The source `.fig` and raw Figma exports are not distributed. The source listing and embedded third-party asset permissions have not been independently confirmed; figlens's MIT license does not license those designs or assets.

The full-display capture is 3840 × 2160 at 15 fps, lasting 28 minutes 51.933 seconds. The edited clip is 77.5 seconds, 1920 × 1080 at 24 fps. It starts with a design/result comparison, then shows planning, asset inspection, live implementation, checking/refinement, source structure and a real keyboard Todo interaction. Development excerpts use different playback speeds (12× and 60×), with pauses and scene changes edited. Display frames are scaled without cropping. This is not an agent-speed benchmark or a claim of pixel-perfect reproduction; the session ran locally, not over SSH.

The implementation passed lint, formatting, strict TypeScript/production build, DPR 1/2 browser checks and an independent React Doctor scan of all nine React source files. Fonts are self-hosted with their OFL notice, and the banner has DPR-specific image variants. These checks apply to this recorded screen, not every design. The screen uses fixed desktop dimensions; responsive behavior and a production backend are outside the demo. Todo strike-through follows the explicit node style, although figlens's reference renderer omits it. Image compression and text antialiasing can also differ.

Raw captures, the initial/final plan, source hashes, edit intervals, browser evidence and acceptance notes remain under ignored `samples/taskflow-demo-production/take-3/`. The recording demonstrates the plan/build/check workflow; it does not show `/figlens yolo` or its temporary-plan cleanup.
