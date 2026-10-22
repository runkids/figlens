# Your design. Your components.

Bring a saved Figma file to Pi. Let the agent find the components your project already uses, review its suggestions, and carry those decisions into the next screen.

![Open a local .fig, find project components, review the mappings, and reuse them](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-workflow.svg)

## See the workflow

[![Actual Pi mapping review with the local design viewer](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-demo-poster.png)](https://github.com/runkids/figlens/blob/main/assets/pi-demo.mp4)

**[Watch the recording](https://github.com/runkids/figlens/blob/main/assets/pi-demo.mp4)** · See [recording notes](https://github.com/runkids/figlens/blob/main/wiki/pi.md#demo-and-verification) for exactly what is shown and how to reproduce it.

The MCP Apps for Claude example maps its **App header** to an existing plain HTML block, then reuses that reviewed mapping in a second List views frame. The candidate is prepared before recording; model generation is not shown or timed.

![The reviewed App header reused in a second frame after starting a new Pi session](https://raw.githubusercontent.com/runkids/figlens/main/assets/pi-reuse.png)

## Install once

Requires **Pi 1.3.0 or later** (`@earendil-works/pi-coding-agent`). The CLI and extension ship in the **same npm package and release**:

```sh
pi install npm:@runkids/figlens
```

Restart Pi, or run `/reload` in an existing session. Run Pi from the project whose components you want to reuse. The extension registers the local figlens MCP server automatically. An existing `figlens` server in your Pi MCP settings takes precedence.

The npm package includes the CLI launcher: no separate global installation is required. Its first launch, including automatic MCP startup, downloads and verifies the matching native release binary; later launches use the local cache. Reading a saved `.fig` makes no Figma API calls. `/figlens map` uses your configured Pi model.

From a source checkout, use an installed native `figlens` binary, or set `FIGLENS_BIN` to its executable path:

```sh
pi -e /absolute/path/to/figlens/pi/index.ts
# Or save the package in Pi's settings:
pi install /absolute/path/to/figlens
```

## One frame at a time

**1. Open a design.** In Figma, choose **File > Save local copy**. In Pi:

```text
/figlens open ./design.fig
```

Choose a frame from the picker. Pi saves the selection and attaches a reference render to the conversation. To select a specific node, including a nested frame:

```text
/figlens open "./design files/mobile.fig" --frame 12:345
```

For a larger canvas alongside Pi, run `figlens preview design.fig` in another terminal and open its local URL. Browse layers, copy an exact ID, then use `/figlens frame <id>` in Pi. The viewer and extension share the design file; their selections are not synchronized automatically.

**2. Ask Pi to find existing components.**

```text
/figlens map
```

Pi reads this frame and your source code, then proposes actual imports, props and slots. This command asks for mapping proposals; it does not authorize application edits. A candidate might look like:

```tsx
// Source: ui/src/components/Card.tsx, default export
import Card from "./components/Card";

<Card padding="none">{contentFromCurrentFrame}</Card>
```

The workflow is framework-independent: the agent reads your project's conventions and writes React, Vue, Svelte or other native examples. The extension does not translate one framework into another. You can also implement a screen immediately without creating mappings first.

**3. Review the suggestions.**

```text
/figlens review
```

Each candidate shows the design instance, implementation file, import, usage example, rationale, reported checks and unresolved issues. Choose **Accept**, **Edit**, **Skip** or **Stop**. Acceptance asks for final confirmation. A listed conflict also requires a written resolution. Cancelling leaves the candidate pending.

For example, Aurora's Quiet Button has a `Label` property of **Not today**, while its expanded visible text is **Hold to confirm**. The extension flags the discrepancy; it does not choose the wording for you. A text-conflict resolution applies to that observed instance and text, not every sibling.

**4. Build with the approved decisions.**

```text
Implement this frame using the approved component mappings.
Use current design text and assets, then run this project's checks.
```

Switch frames with `/figlens frame`, or `/figlens frame 22:103`. Compatible reviewed mappings can be reused; a new variant or changed structure remains unreviewed. Approval records a mapping decision, not proof that the resulting screen compiles or matches the design. Ask Pi to run the project's permitted checks and compare the implementation with the design reference.

## What survives a session

The selection, candidates and reviewed mappings live in **`.figlens/component-mappings.md`** inside the project. Commit it when your team wants to share those decisions. Keep personal notes outside the managed markers; the extension preserves them. The JSON block is internal state, not a public mapping schema or Code Connect format.

On the next session, the extension restores the selection. Before each agent turn it refreshes a bounded context preview; the agent can request more instances through `figlens_bindings`. `/figlens close` stops automatic design context while keeping saved mappings.

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
| `/figlens open <file.fig> [--frame ID]` | Select a local design and frame; without UI, list frames or provide `--frame` |
| `/figlens frame [ID]` | Select another scope in the open file |
| `/figlens map` | Ask Pi to propose mappings for the current frame |
| `/figlens review` | Accept, edit or skip pending candidates for the selected frame |
| `/figlens status` | Show the current selection and mapping states |
| `/figlens close` | Clear active design context; preserve mappings |

| Tool | Purpose |
|---|---|
| `figlens_bindings` | Read current instances, approved/pending/stale mappings and conflicts. Defaults to 20 instances; `nextOffset` gives the next page; `limit` accepts 1–50 |
| `figlens_propose_bindings` | Save 1–20 pending candidates. Supply instance ID, target file/export/import/example, rationale, optional supporting `dependencies`, reported `checks` and `issues` |

Both tools accept a `file` + `frame` pair to inspect or propose for another selection without changing the saved selection. Open that selection to review its proposals. Large instance details are replaced with a request to inspect the full node through MCP. Full geometry, variables, assets and bundles remain in the existing MCP tools; the extension does not duplicate them.

TUI provides the picker and multiline target editor. RPC clients can use supported select/input/confirm dialogs; editing uses individual text fields. Print/JSON modes do not auto-approve or wait for dialogs. The extension is a workflow aid running with Pi's filesystem permissions, not an approval security boundary against an agent that edits files through other tools.

## Troubleshooting

- **Binary unavailable:** npm installs use the bundled launcher; source installs need `figlens` on PATH or `FIGLENS_BIN`. Check the first-download connection and the error message.
- **Stale proposal:** ask Pi for a new candidate after design or code changes. Approval rechecks evidence after the review dialog and refuses changed evidence.
- **Cancelled/failed preview:** the selection is still saved. Use MCP export or the browser viewer; the warning explains the render failure.
- **Guide modified or malformed:** the extension refuses to overwrite invalid state. Restore the valid managed block from Git before retrying.
- **Another writer:** wait for the other Pi session to finish. Remove `.figlens/.mapping-lock` only after verifying its writer has stopped; a crashed process can leave a lock.

See the [maintainer guide](https://github.com/runkids/figlens/blob/main/wiki/pi.md) for verification and release packaging.
