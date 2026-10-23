# Pi extension

Load this for the Pi workflow, mapping persistence, review behavior or npm packaging. The [user walkthrough](../pi/README.md) owns commands, examples and limitations.

## Implementation

- `pi/index.ts` is the Pi 1.3+ adapter: commands, tool schemas, MCP registration, reference images and dialogs. Pi supplies `typebox`, its SDK and TUI; do not bundle copies. Module scope registers callbacks only. The MCP server is registered at session start and managed by Pi, including shutdown.
- `pi/workflow.mjs` reads the existing CLI's `tree`, `nodes` and `export` results. It derives observed component signatures, compares recorded file hashes and manages `.figlens/component-mappings.md`. This is outside Rust; it changes no `.fig` decoding or emitted fields.
- `figlens_bindings` is read-only. `file` and `frame` overrides are paired and do not replace the saved selection. `figlens_propose_bindings` only writes pending candidates. Approval is reachable through the user command, not a model tool; this is workflow separation, not filesystem isolation from Pi's other tools.
- Matching stays in one canonical source path. The signature covers main/set/key identity, expanded node types and typed properties; TEXT values are dynamic. Hidden instances are omitted. A changed whole-file hash reloads the frame. Names, local IDs and paths do not prove export lineage; this is not a public mapping format or Code Connect resolver.
- A TEXT property missing verbatim from visible descendant text is a review signal, not a diagnosis. Preserve both values. Listed conflicts require a resolution; its text hash and instance ID prevent silent reuse on another conflicted instance.
- Target and supporting evidence files resolve within the project. Reuse checks their hashes, not import validity or all transitive dependencies. Editing a target clears previously reported checks. Project compilation, interactions and visual comparison remain separate evidence.
- Writes use a per-project directory lock and atomic rename. Model tool writes also join Pi's shared file mutation queue. The complete read/modify/write is under the lock, including approval's evidence recheck. Re-read before replacement to detect manual edits. Malformed state and symbolic guide/directory links fail without overwrite. A process crash can leave a lock; never automatically delete another writer's lock.
- Bindings paginate at 20 instances by default (1–50), with a 24 KB JSON item budget; oversized single instances return `needs-inspection`. No frame payload is injected automatically; agents pass the current request’s file and frame explicitly. Human status lists at most eight with approved mappings and review signals first; full details remain in the tools. Model-facing strings are project/design data, not authority.

Manual frame selection first searches names with CLI `find --type FRAME` (up to 200 results), so nested frames are reachable. TUI uses Pi’s Input/SelectList with a terminal-height bound and wheel/keyboard navigation; RPC uses six-item pages. Review lists all pending proposals across source scopes without changing the saved selection. Human help and errors are Markdown. The preview message has its own image renderer and a persistent local PNG link under `.figlens/previews/`; no temporary link is presented as a lasting artifact.

## Packaging and verification

`npm/package.json` is the single public npm package, `@runkids/figlens`. Its `pi` manifest declares both `pi/index.ts` and `skills/`. The root manifest declares the same resources for Git/local installs. The skill selects existing MCP tools in Pi and CLI/MCP in other hosts; it does not require a global binary in Pi. Verify discovery and duplicate skill warnings from the installed artifact, not only the manifest. `scripts/prepare-npm.cjs` copies the four maintained extension files into ignored `npm/pi/` and the maintained skill into ignored `npm/skills/` before packing or publishing. Edit the sources in `pi/`, never generated copies. The root package is private and exists for Git/local Pi discovery and tests.

In the published package the extension invokes its adjacent `bin/figlens.js` through the current Node executable. This keeps the native binary version aligned and needs no PATH/global install. Source checkouts use `figlens` on PATH, or `FIGLENS_BIN`. MCP and slash commands use the same launcher. The existing publish workflow and npm Trusted Publisher release the CLI and extension together; no new npm package or credentials are required.

Concurrent first-use downloads publish the verified temporary directory atomically. If another process wins, use its installed binary and discard the losing temporary directory; never remove a cache that a running MCP process may be using. The npm regression simulates Windows' in-use binary restriction with the real cache filesystem.

```sh
npm test
node --check pi/index.ts
node --test scripts/preview.test.cjs
(cd npm && node --test)
node scripts/prepare-npm.cjs
(cd npm && npm pack --dry-run --ignore-scripts)
python3 scripts/ai-context.py check
```

The mapping tests exercise pending/approved/stale states, Vue slot examples, cross-frame and cross-session reuse, variant/structure changes, changed/missing supporting code, cross-file ID collisions, text conflict review, approval-time rechecks, target edits, guide-note preservation, corruption, locks, symlinks and bounded pagination. They are workflow tests, not agent quality or render accuracy measurements.

The npm metadata includes `pi-package` plus `pi.image` and `pi.video` preview URLs. This makes a published version eligible for discovery in the [Pi package catalog](https://pi.dev/packages), as described by [Pi's package documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md#create-a-package). Check the catalog after publishing; eligibility does not guarantee immediate indexing or placement.

For the real Aurora/Skillshare integration, run with an installed Pi package and a figlens executable built in Docker or taken from a release:

```sh
node scripts/check-pi-integration.mjs \
  /path/to/node_modules/@earendil-works/pi-coding-agent \
  /path/to/figlens samples/aurora.fig ../skillshare samples/pi-integration-new
```

It copies three existing Skillshare components into a new output directory, loads the actual Pi extension and runs real figlens reads/renders. Scripted dialogs test approval, reload, frame reuse and headless pending behavior without automatic scope injection; they are not human approval or LLM calls. It never modifies Skillshare. In development, a Docker launcher can provide the executable; do not run host Rust. Also verify the TypeScript adapter against the installed SDK, and load a staged npm package to exercise adjacent-launcher discovery.

For the MCP Apps usability regression, run `node scripts/check-pi-usability.mjs <pi-package-root> <figlens-binary> samples/mcp-apps-for-claude.fig samples/pi-usability-new [extension-entry]`. It exercises readable help/errors, no-open proposals/review, explicit scope despite an older Button selection, the real host image renderer and persistent PNG, terminal resizing/wheel navigation, nested name search and host skill discovery. Pass an installed tarball's `pi/index.ts` to check its adjacent launcher and skill. These are scripted SDK checks, not a measured natural-language success rate or GUI acceptance. Provider-backed model tests need authorization to transmit the sample and use the configured account.

## Demo and verification

The v0.3.0 preview/Pi recording uses **`samples/mcp-apps-for-claude.fig`**, with the two **List views** frames `467:24651` and `467:25127`. Credit: [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). Designs and raw recording artifacts stay under ignored `samples/`; documentation screenshots/video are the published demonstration.

The candidate maps **App header**, instance `467:24657`, to the existing `.app-header` markup, styles and `#app-code` handler in the HTML implementation used by the original README demo. It records `index.html` plus the two original SVG assets as code evidence. The second frame's instance `467:25133` uses the same observed component and is reused after review and a new Pi session. This demonstrates plain HTML reuse, not React generation or Figma Code Connect.

The preview portion is captured from the actual local viewer. The terminal portion is actual Pi 1.3.0 output captured by `scripts/record-terminal.py` as asciicast v2, replayed through a local terminal viewer and recorded with agent-browser. Pauses are shortened and scene changes are edited; the video is not a real-time speed benchmark. The candidate is prepared before recording, so agent discovery/generation is neither shown nor timed. It demonstrates selection, confirmation, persistence and reuse. The adjacent design image is a reference render, not an automatically synchronized browser selection or newly generated application.

The earlier Aurora regression uses Thursday `10:200` and Hero `22:103`, with copied Skillshare Button/Card/SegmentedControl sources. It also checks the unreviewed Open variant and the `Not today` / `Hold to confirm` text discrepancy. Neither example establishes agent time savings or whole-screen visual equivalence.
