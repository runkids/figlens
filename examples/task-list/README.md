# Case study: a local design file to an editable task list

A saved local Figma design supplied the layout, text, assets and reference render for an editable HTML/CSS task list. All six figlens-based attempts retained all 19 visible text occurrences as real DOM text and all four icons. CLI and MCP extraction also passed with Docker networking disabled.

## The complete path

```text
Owned Figma task-list frame
  ├─ Save local copy → design.fig → figlens bundle → coding agent → editable HTML/CSS
  └─ Independent Figma 1x PNG ───────────────────────────────────→ browser comparison
                                                                  ├─ 50% overlay
                                                                  └─ pixel difference
```

The screen is an original fixed-layout task list: 960 × 640, 35 nodes, 19 visible text occurrences, four small task icons and Source Code Pro Regular. Its outside stroke extends the exported image to **962 × 642**. The browser canvas uses that export size, with the frame origin at (1, 1); resizing the reference to the frame dimensions would invalidate the comparison.

The [fixture plugin source](../../scripts/e2e-fixture-plugin/code.js) describes the artwork, copy, sizes and colors. It is a fixture creator, not an application implementation. It refuses nonempty files and removes its newly created frame if creation fails. It does not replace an existing design.

### 1. Prepare an owned source and independent reference

In a **new blank Figma Design file**, import [the development plugin manifest](../../scripts/e2e-fixture-plugin/manifest.json) through **Plugins > Development > Import plugin from manifest**, then run **figlens comparison fixture**. Source Code Pro Regular must be available in Figma; creation stops if it is not.

Save a local `.fig` copy and export the selected frame at **1x PNG** into an ignored local directory such as `samples/task-list/`. Keep that native Figma PNG as independent reference evidence. Figlens's own reference render is useful for implementation, but comparing it against itself cannot establish Figma fidelity.

Obtain Source Code Pro from an authorized source such as [Adobe's repository](https://github.com/adobe-fonts/source-code-pro), retain its SIL Open Font License, and configure the actual project to load the required weight. The experiment used one shared variable font binary at weight 400 in every group. Font files, saved designs and Figma exports are not distributed with this case study.

### 2. Extract the offline handoff

With a native figlens binary already installed:

```sh
figlens find samples/task-list/design.fig "Owned fixture / Task list" --type FRAME
# Use the nodeId returned by find, not an ID copied from a different file.
figlens bundle samples/task-list/design.fig <nodeId> samples/task-list/frame.zip
```

Create a fresh handoff directory and extract the ZIP there. Read `context.md`, `missing-assets.json`, `assets.json`, the manifest and the selected structure. Check required font faces and asset files using the [font and asset preflight](../../skills/figlens/SKILL.md#font-and-asset-preflight). Report missing inputs and continue with documented project-font fallbacks or labelled image placeholders; leave final visual acceptance pending for unresolved inputs. A successful bundle is not a font-readiness check.

Once the `.fig`, binary and project dependencies are local, design reads require neither Figma sign-in nor a running Figma app. Model inference, installation and dependency downloads are separate network requirements.

### 3. Build real UI and capture it

Give the configured agent the local source, intended frame and existing project. For example:

```text
Build only "Owned fixture / Task list" from ./samples/task-list/design.fig.
Use this project's stack and components, real DOM text, and available local fonts/assets.
Warn about missing/unverified inputs and continue with documented font fallbacks
or labelled image placeholders. Leave final visual acceptance pending for unresolved inputs.
Compare the running UI with the independent Figma PNG at native 962x642, DPR 1,
with the 960x640 frame at (1,1). Fix confirmed differences within this task.
Show the changed files, browser capture, overlay/difference, and remaining limitations.
```

Before capturing, await font loading and image decoding. Inspect actual DOM text, image URLs and browser errors: a screenshot alone cannot establish that the implementation is editable or that its assets load. Keep the final HTML/CSS alongside its browser capture.

### 4. Compare and retain the misses

Compare native-size images without cropping or resampling, after compositing RGB over white. A 50% overlay exposes misalignment; a pixel difference exposes color and rasterization differences. Inspect both, including typography, export margins, strokes and each icon. Preserve every assigned attempt, not just the best one.

The retained local experiment includes an offline `comparison.html`: independent original, all twelve browser outputs, per-attempt overlays/differences, and editable HTML links. `report.md`, source/font/input proofs, DOM audits, prompts and timings sit beside it. These private artifacts stay under ignored `samples/`; they are **not included in a fresh clone or public image download**. The [runner](../../scripts/e2e-owned-pilot.py) records the experiment's checks and comparison procedure; it requires the prepared evidence directory and model/browser tools, rather than being a one-command public benchmark. The [public smoke check](../first-success/README.md) is the deterministic, no-model setup check.

## What the measured run found

One simple fixture, `figlens 0.7.3`, `openai-codex/gpt-6.1-sol` with medium thinking, three clean attempts per input group, shuffled order, 600-second limit per attempt, no rerolls or human quality fixes. Every group received identical licensed font bytes, leaf PNG icons, blank starter, viewport and DPR. Common supplied icons replace each tool's vector exports for this comparison, so these numbers are not an asset-export quality benchmark.

All twelve outputs retained all 19 visible text occurrences as DOM text, four icons and the locally loaded font. None used a whole-screen image/SVG/canvas implementation. Independent browser console/error logs were empty.

Medians with [minimum, maximum]; lower pixel error is better:

| Input | RGB MAE, 0–255 | Pixels with max channel error >48 | Prepared-input agent seconds |
|---|---:|---:|---:|
| Screenshot + common assets | 0.752171 [0.650253, 1.192202] | 0.5643% [0.4247%, 0.8855%] | 107.34 [103.15, 129.72] |
| Official MCP + common assets | 0.664479 [0.663853, 0.737541] | 0.3998% [0.3998%, 0.5003%] | 109.30 [64.60, 456.93] |
| Figlens bundle + common assets | 0.663853 [0.663840, 0.663853] | 0.3998% [0.3998%, 0.3998%] | 105.18 [57.65, 107.19] |
| Same figlens input + generic visual checklist | 0.663349 [0.514480, 0.663349] | 0.3998% [0.2706%, 0.3998%] | 77.18 [68.13, 92.94] |

**Comparator caveat:** official attempt 3 obtained the official screenshot and metadata but failed to obtain full design context after transport/tool errors. It is retained as a degraded workflow outcome, not a full-context baseline. The two full-context official attempts have MAE 0.663853 and 0.664479, each with 0.3998% differing pixels: effectively indistinguishable from figlens on this fixture. The failures' cause was not established and does not prove a Figma product defect.

**Offline check:** CLI inspect/nodes/bundle and stdio MCP inspect succeeded with the static Linux 0.7.3 binary inside Docker with `--network none`. Structure, bundled variables and reference image hash-matched the frozen native macOS outputs for this input. This proves offline extraction, not offline model inference or cold-start installation.

## Measurement scope

- Large uniform backgrounds dilute pixel metrics. These values are not accuracy percentages or production acceptance thresholds.
- This is a simple fixed-layout fixture, not evidence for complex production screens, responsive layouts, behavior, accessibility or backend correctness.
- Agent seconds exclude earlier human source/font setup. With three attempts and variable cache usage, they do not establish speed or monetary savings. Actual billed cost was not verified.
- The generic checklist produced one notably better attempt, not a repeatable improvement across all three. Its median differing-pixel fraction was unchanged. We did not ship it as a demonstrated quality improvement.
- Font rasterization/weight and small baseline/border differences remain. The original experiment's pre-model font-width assumption failed and was replaced, before model runs, with a glyph-shape negative control; that setup failure is retained in the private evidence.
- Dependency checks in the shared workflow guide the agent; they do not automatically inspect an arbitrary project's fonts or change the historical experiment inputs.

## Follow-up work

[Project font and asset readiness checks](https://github.com/runkids/figlens/issues/38) track non-blocking automation. [Broader end-to-end validation](https://github.com/runkids/figlens/issues/39) tracks representative complex screens and reproducible evidence beyond this fixture.

Use figlens when the design must travel with the code: the saved file supplies an offline, scoped handoff that an agent can turn into editable UI. Use official MCP when the agent needs the live design.
