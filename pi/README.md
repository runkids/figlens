# Build Figma screens in Pi from a local `.fig`

Save the design from Figma, put it next to your code, describe the screen. Pi finds the frame, reads layout, text and assets, and builds it with your project's components. No Figma app, login or API calls. Works on a remote machine.

## Install

Requires **Pi 1.3.0+**. One package ships the CLI, MCP server, viewer and agent skill:

```sh
pi install npm:@runkids/figlens
```

Restart Pi or run `/reload`. Run Pi from the project you want to build in. The extension registers the figlens MCP server automatically; an existing `figlens` server in your Pi MCP settings takes precedence. The first launch downloads and verifies the native binary. [Package page](https://pi.dev/packages/@runkids/figlens).

## Ask for the screen

In Figma, **File > Save local copy** → `design.fig` in your project. Then:

```text
Build the task board from ./design.fig.
Use this project's existing components and assets. Show me the result
and check it against the design.
```

Pi searches nested screens and resolves node IDs itself. If several frames match, it asks with recognizable names and sizes. React, Vue, Svelte and plain HTML use the same flow. The packaged [figlens skill](https://github.com/runkids/figlens/blob/main/skills/figlens/SKILL.md) has Pi compare the running UI with the design, verify fonts and images, run project checks and fix confirmed differences, then report what it could not check.

[![Pi plans a local design and builds React beside the live browser](https://raw.githubusercontent.com/runkids/figlens/main/assets/taskflow-demo.gif)](https://github.com/runkids/figlens/blob/main/assets/taskflow-demo.mp4)

Looping GIF at 3× the edited video’s playback speed. Click it for the full 78-second video.

**[78-second demo](https://github.com/runkids/figlens/blob/main/assets/taskflow-demo.mp4):** `/figlens plan` on one dashboard frame, built with React and Tailwind CSS, checked against the live browser. Edited time-lapse, not a benchmark. [Design credit and notes](https://github.com/runkids/figlens/blob/main/wiki/accuracy.md#taskflow-react-and-tailwind-demo).

## Commands

| Command | Does |
|---|---|
| `/figlens plan [request]` | Inspect the design and relevant code, write a plan. No edits |
| `/figlens yolo [request]` | Plan, implement, verify, then remove its own temporary plan |
| `/figlens open [description]` | Find a design from a file, screen name, purpose or visible text |
| `/figlens preview [description]` | Start the local viewer and return its link |
| `/figlens map` | Propose reusable component matches for the current design |
| `/figlens review` | Accept, edit or skip pending component mappings |
| `/figlens status` | Show the manual selection and mapping states |
| `/figlens close` | Stop the viewer and clear the manual selection |

None of these is required to build a screen. A plain request does the whole job.

### Plan

```text
/figlens plan Plan only the homepage hero from ./design.fig. Include existing component reuse, image delivery and acceptance checks.
```

A small region gets a small plan; several pages get an outline with the next slice detailed, or every slice detailed when you ask for the whole plan first. Without arguments it uses the task in the conversation and asks for missing source or scope. Saved plans go to `.figlens/plans/<scope>.md` (or `.figlens/plans/<design>/<scope>.md` when several designs are involved) unless the project has its own convention. The same [planning workflow](../skills/figlens/references/planning.md) works through the skill in other agents.

<a id="plan-and-build-autonomously"></a>

### YOLO

```text
/figlens yolo Implement only the task-board header from ./design.fig. Follow existing components and styles. Compare it with the design and run the relevant project checks.
```

Loads the [YOLO workflow](../skills/figlens/references/yolo.md): temporary plan, implementation, checks, corrections, then cleanup of that run's plan only. Failed, interrupted or blocked work keeps its plan. Existing plans and notes stay intact. Host permissions still apply; read the final report for the actual verification outcome.

### Preview

```text
/figlens preview ./design.fig
/figlens preview The homepage quick-entry card in ./design.fig
```

An existing path starts the viewer immediately. A description makes Pi resolve the file and screen first. The link is returned, not auto-opened; keep Pi running while you use it. `/figlens close` or session shutdown stops the viewer.

### Component mappings (optional)

```text
Save reusable matches between this design and our existing components.
Inspect the real imports, props and usage. Leave proposals for me to review.
```

Then `/figlens review` lists every pending proposal in the project with its source frame, implementation file, import, usage, rationale and conflicts. Accept, edit or skip. Decisions live in `.figlens/component-mappings.md`; commit it to share them. A mapping is reused when the same component and variant structure appears again, marked **stale** when its recorded files change, and left **unmatched** for a new variant. Non-interactive modes leave proposals pending.

[20-second mapping demo](https://github.com/runkids/figlens/blob/main/assets/pi-demo.mp4) (v0.3.0, edited, generation not timed) · [recording notes](https://github.com/runkids/figlens/blob/main/wiki/pi.md#demo-and-verification).

<a id="source-checkout"></a>

## Advanced

**Source checkout.** Install a native `figlens` binary or set `FIGLENS_BIN`, then:

```sh
pi -e /absolute/path/to/figlens/pi/index.ts
# or save it in Pi's settings:
pi install /absolute/path/to/figlens
```

**Manual selection.** `/figlens open "./design files/mobile.fig" --frame 12:345` saves a known node and attaches a reference image (PNGs in `.figlens/previews/`, keep them out of Git). The legacy `/figlens frame [ID]` searches within that saved file. The selection is restored for manual commands next session; it is not injected as design context. `/figlens close` clears it and keeps mappings.

**Agent tools.** The MCP `workflow` tool exposes on-demand build, image, planning and opt-in YOLO guides from the same skill sources used by the commands above. It returns guidance only; the current agent performs the authorized work with its project/browser tools. Besides the design tools (`tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `export`), the extension adds:

| Tool | Purpose |
|---|---|
| `figlens_preview` | Start a session-owned loopback viewer for `file` and optional `node`; return its URL |
| `figlens_bindings` | Read instances and approved/pending/stale mappings; 20 per page, `limit` 1–50 |
| `figlens_propose_bindings` | Save 1–20 pending candidates with target, import, usage, rationale, checks and issues |

Both binding tools take `file` + `frame` and do not change the manual selection. The extension runs with Pi's filesystem permissions; it is a workflow aid, not a security boundary.

**Print/RPC modes.** Design reading works without a picker. `/figlens plan` dispatches a model request; RPC clients wait for the final result, one-shot print clients send a normal planning prompt instead. A one-shot process cannot keep the viewer alive.

**Troubleshooting.**

- *Binary unavailable:* npm installs use the bundled launcher; source installs need `figlens` on PATH or `FIGLENS_BIN`.
- *Stale proposal:* ask for a new candidate after design or code changes; approval refuses changed evidence.
- *Guide modified or malformed:* the extension refuses to overwrite invalid state. Restore `.figlens/component-mappings.md` from Git.
- *Another writer:* remove `.figlens/.mapping-lock` only after the other Pi session has stopped.
- *Duplicate skill name:* if another installed skill is also called `figlens`, remove it so the package's instructions load.

Maintainers: [wiki/pi.md](https://github.com/runkids/figlens/blob/main/wiki/pi.md).
