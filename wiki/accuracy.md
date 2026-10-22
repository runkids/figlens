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

The source take is approximately six minutes. `scripts/record-orca-window.py` captured 1,013 snapshots of the verified Orca window over 360.16 seconds, preserving their real timing in the source video. The README's 45-second MP4 and GIF speed the whole take up uniformly by **8×**, without simulated typing or reconstructed terminal output. The capture is about 2.8 snapshots per second, so encoding it as video does not provide native 30 fps motion. Design files, bundles, raw captures and the full take stay in the gitignored `samples/demo/` directory.
