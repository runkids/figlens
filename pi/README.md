# Your design. Your components.

Give Pi a saved Figma file and describe the screen to build. Pi finds the design, reads its layout and assets, and implements in your project’s stack. Saving component mappings is optional.

![Ask for a screen, find its design, build in your stack, and check the result](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-workflow.svg)

## Optional component reuse demo

[![Actual Pi mapping review with the local design viewer](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-demo-poster.png)](https://github.com/runkids/figlens/blob/main/assets/pi-demo.mp4)

**[Watch the v0.3.0 recording](https://github.com/runkids/figlens/blob/main/assets/pi-demo.mp4)** · See [recording notes](https://github.com/runkids/figlens/blob/main/wiki/pi.md#demo-and-verification) for exactly what is shown and how to reproduce it.

The MCP Apps for Claude example maps its **App header** to an existing plain HTML block, then reuses that reviewed mapping in a second List views frame. The candidate is prepared before recording; model generation is not shown or timed.

![The reviewed App header reused in a second frame after starting a new Pi session](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-reuse.png)

## Install once

Requires **Pi 1.3.0 or later** (`@earendil-works/pi-coding-agent`). The CLI, extension and agent skill ship in the **same npm package and release**:

```sh
pi install npm:@runkids/figlens
```

Restart Pi, or run `/reload` in an existing session. Run Pi from the project you want to implement in. The extension registers the local figlens MCP server automatically. An existing `figlens` server in your Pi MCP settings takes precedence.

The npm package includes the CLI launcher: no separate global installation is required. Its first launch, including automatic MCP startup, downloads and verifies the matching native release binary; later launches use the local cache. Reading a saved `.fig` makes no Figma API calls. Pi uses your configured model to interpret the request and build the UI. Check Pi’s loaded resources for the packaged `figlens` skill. If another installed skill has the same name, resolve the duplicate so the package’s current instructions are used.

From a source checkout, use an installed native `figlens` binary, or set `FIGLENS_BIN` to its executable path:

```sh
pi -e /absolute/path/to/figlens/pi/index.ts
# Or save the package in Pi's settings:
pi install /absolute/path/to/figlens
```

## Ask for the screen

In Figma, choose **File > Save local copy**, then give Pi the local path:

```text
Build the List views screen from samples/mcp-apps-for-claude.fig.
Use this project's existing components and assets. Show me the result
and check it against the design.
```

Pi can use `find`, `inspect`, `export` and `bundle` through the automatically registered MCP server. It searches nested screens and resolves node IDs itself. When several candidates remain ambiguous, it should ask with recognizable page/screen names, dimensions and differences or reference images. You do not need to run `/figlens open` or identify a frame by its ID.

The agent uses the current request’s file and frame. An older manual selection is not injected into each turn. React, Vue, Svelte and plain HTML use the same workflow: Pi follows the project’s conventions. A screen with zero component instances can still be built from its layout, text and assets.

Design reading also works in print/RPC modes; no picker is required. Use MCP image export or a saved offline viewer for visual inspection. If the client cannot display images or use a browser, verify the artifact elsewhere before claiming a visual match.

## Plan before implementing

For a careful reconstruction, ask Pi to inspect the design and relevant existing code, then plan the work. A small region needs only a small plan; several pages get a screen/state outline with the next slice detailed. Ordinary design questions and image exports keep their direct workflow.

```text
/figlens plan Plan only the homepage hero from ./design.fig in this project. Use this project's existing image workflow. Include existing component reuse, image delivery and acceptance checks.
```

No `/figlens open` is required. With no arguments, `/figlens plan` uses the task discussed in the conversation and asks for missing source/scope. It does not assume an older manual selection. The command loads the packaged planning workflow into the current agent; it requests a plan, not application edits or execution. Ask Pi to implement when you are ready. A normal request that already authorizes planning and implementation can proceed through both without this command.

The agent follows the project's actual architecture and image pipeline, records design/code evidence and separates assumptions from verified facts. When saving is useful, it uses the project's location conventions or `.figlens/plans/<scope>.md`; small plans can stay in chat. On continuation, it rechecks relevant design/code and updates the plan. There is no separate planner agent, automatic task runner or new progress database. Host permissions and any other plan-mode extension still apply; this shortcut does not disable tools or enforce filesystem isolation.

The same [planning workflow](../skills/figlens/references/planning.md) is available through the skill in other agents. Model quality, visual fidelity and efficient image delivery still require actual project and browser verification.

The shortcut dispatches a model request; dispatch is not a completed plan. RPC clients should wait for the model's final result. For a one-shot print client, send a normal planning prompt through the skill rather than relying on a slash-command callback to keep the process alive.

## Optional: remember component matches

When you want to keep reusable decisions, ask Pi:

```text
Save reusable matches between this design and our existing components.
Inspect the real imports, props and usage. Leave proposals for me to review.
```

Pi calls `figlens_bindings` and `figlens_propose_bindings` with the current `file` + `frame`. These calls do not change the manual selection. Build work can proceed without proposing or approving mappings.

```text
/figlens review
```

Review lists **all pending proposals in this project**, including proposals made without open or for another frame. Each shows its source file/frame, implementation file, import, usage, rationale, reported checks and conflicts. Choose **Accept**, **Edit**, **Skip** or **Stop**. Approval requires confirmation and a written resolution for listed conflicts; cancellation leaves proposals pending. Non-interactive modes leave them pending too.

For example, Aurora’s Quiet Button has a `Label` property of **Not today**, while its visible descendant text is **Hold to confirm**. A resolution belongs to that observed instance and text. Approval records a mapping decision; compilation, behavior and visual checks still belong to the project’s verification.

## Find a design by describing it

```text
/figlens open
/figlens open VIP panel
/figlens open The login section on the homepage in ./design.fig
```

Tell Pi what you know: a local file, a screen name, its purpose or visible text. It uses the current conversation, asks only for missing information and offers recognizable choices when candidates remain ambiguous. You do not need to know node IDs or exact layer names. This shortcut identifies the scope in the conversation; it does not edit the application, propose mappings or save a manual selection. Ask Pi to plan or build once the scope is clear.

`open` is the main discovery shortcut. The legacy `frame` command is an advanced control for a manually saved file, not a second required discovery step.

## Advanced: saved selection and reference image

When you already have a node ID, you can save it explicitly:

```text
/figlens open "./design files/mobile.fig" --frame 12:345
```

Pi saves this manual selection and attaches a reference image. A renderer displays it on image-capable terminals, with an **Open reference image** link on every terminal. PNGs remain in `.figlens/previews/`; treat these as local output, exclude them from Git and delete them when no longer needed.

The legacy `/figlens frame` searches names in that saved file. The TUI picker filters by name/path/ID, limits its height to the terminal and supports arrow keys and the mouse wheel. RPC clients receive six-item pages. If a description does not match a name, the agent can use visible text and surrounding context instead. A known ID can still be supplied to `/figlens frame` for compatibility.

`/figlens map` resolves the design from the current conversation and requests reusable component matches. It uses a saved selection only when explicitly requested. It does not authorize application edits or automatic approval. `/figlens status` reports the manual selection; `/figlens close` clears it. None of these controls is required to implement a design.

For a browser canvas, run `npx @runkids/figlens preview design.fig` in another terminal. The viewer and extension read the same design but do not synchronize selections. Agents installed through this Pi package use the registered MCP tools directly; they do not need a global binary or another `npx` download.

## What survives a session

The selection, candidates and reviewed mappings live in **`.figlens/component-mappings.md`** inside the project. Commit it when your team wants to share those decisions. Keep personal notes outside the managed markers; the extension preserves them. The JSON block is internal state, not a public mapping schema or Code Connect format.

The manual selection is restored for manual commands in the next session. It is not sent as automatic design context. Pi requests mappings with the current file and frame; `/figlens close` clears only the manual selection and keeps saved mappings.

| Situation | What happens |
|---|---|
| Same component and observed variant/structure in another frame | The reviewed mapping can be reused; current text is returned separately |
| Relevant recorded implementation/supporting file changes or disappears | Mapping is marked **stale** |
| Same component family, new variant or expanded structure | It remains **unmatched** and needs review |
| Unrelated edits elsewhere in the same `.fig` | Whole-file hash changes, but compatible rules remain usable after reinspection |
| Another `.fig` path with coincidentally identical node IDs | No automatic cross-file reuse |
| No interactive UI | Tools still work; review lists candidates and leaves them pending |

Identity is scoped to the canonical file path and observed component IDs, properties and expanded type structure. A path asserts source continuity; it does not prove Figma lineage across exports. TEXT values are dynamic; other typed component property values participate in matching. Style/layout changes outside that signature still need visual review. Hash checks cover the files listed with a candidate, not every transitive dependency. Imports, props, behavior and screenshots still need project checks.

## Commands and agent tools

| Command | Purpose |
|---|---|
| `/figlens plan [request]` | Plan only the requested scope using the current conversation; no manual selection required |
| `/figlens open [description]` | Find a design conversationally from a file, screen name, purpose or visible text |
| `/figlens map` | Propose reusable component matches for the design in this conversation |
| `/figlens review` | Accept, edit or skip pending candidates across the project |
| `/figlens status` | Show the current selection and mapping states |
| `/figlens close` | Clear the manual selection; preserve mappings |

Advanced compatibility controls: `/figlens open <file.fig> --frame ID` saves a known node; `/figlens frame [ID]` searches or changes the selection within that saved file. The main discovery entry remains `open`.

| Tool | Purpose |
|---|---|
| `figlens_bindings` | Read current instances, approved/pending/stale mappings and conflicts. Defaults to 20 instances; `nextOffset` gives the next page; `limit` accepts 1–50 |
| `figlens_propose_bindings` | Save 1–20 pending candidates. Supply instance ID, target file/export/import/example, rationale, optional supporting `dependencies`, reported `checks` and `issues` |

Both tools accept a `file` + `frame` pair to inspect or propose for another selection without changing the saved selection. Review its proposals directly with `/figlens review`. Large instance details are replaced with a request to inspect the full node through MCP. Full geometry, variables, assets and bundles remain in the existing MCP tools; the extension does not duplicate them.

TUI provides the picker and multiline target editor. RPC clients can use supported select/input/confirm dialogs; editing uses individual text fields. Print/JSON modes do not auto-approve or wait for dialogs. The extension is a workflow aid running with Pi's filesystem permissions, not an approval security boundary against an agent that edits files through other tools.

## Troubleshooting

- **Binary unavailable:** npm installs use the bundled launcher; source installs need `figlens` on PATH or `FIGLENS_BIN`. Check the first-download connection and the error message.
- **Stale proposal:** ask Pi for a new candidate after design or code changes. Approval rechecks evidence after the review dialog and refuses changed evidence.
- **Preview not visible:** use the Open reference image link. If rendering failed, the saved selection remains usable; ask Pi to export a smaller PNG or inspect it in the browser viewer.
- **Guide modified or malformed:** the extension refuses to overwrite invalid state. Restore the valid managed block from Git before retrying.
- **Another writer:** wait for the other Pi session to finish. Remove `.figlens/.mapping-lock` only after verifying its writer has stopped; a crashed process can leave a lock.

See the [maintainer guide](https://github.com/runkids/figlens/blob/main/wiki/pi.md) for verification and release packaging.
