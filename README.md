<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Your coding agent reads Figma files.<br>No Figma app. No login. No API quota.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">Start</a> · <a href="#demo">Demo</a> · <a href="#why">Why</a> · <a href="#bundle">What the agent gets</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limitations</a>
</p>

**The problem.** Your agent needs layout values, text and assets to build a screen. On a remote host or in CI, that design context needs to be available alongside the code, without relying on a running desktop app.

**The fix.** In Figma, **File > Save local copy**. Drop `design.fig` next to your code. figlens parses it locally and gives your agent layout, text, styles, variables, icons, images and a reference render of any frame. One native binary, zero network calls to Figma.

<a id="start"></a>

## Get started

**No design file yet?** [Check your setup with the tiny public example](examples/first-success/README.md): verify your chosen binary, find one frame and validate its bundle. No model required.

Put `design.fig` in your project, then pick your agent.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Start a new session and check `/mcp`. Ask for a screen in your own words: MCP includes on-demand guidance for planning, implementation and image preparation; no separate skill install is required.

**Pi** (1.3.0+): one package ships the CLI, MCP, viewer and skill.

```sh
pi install npm:@runkids/figlens
```

**Any MCP client** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

[Standalone installation](docs/README.md#install)

Then ask for a screen:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

The agent finds the frame, bundles its design data, assets and reference image, then builds the screen with your stack. The shared workflow guides font and asset checks; missing inputs are reported while layout implementation continues with documented fallbacks. Ask for the changed files and a browser comparison.

<a id="demo"></a>

## Demo: Pi builds a React screen from a .fig

[![Pi plans from a local .fig, builds a React screen and checks it beside a live browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

Looping GIF at 3× the edited video’s playback speed. Click it for the full 78-second video.

**[78-second video](assets/taskflow-demo.mp4).** Pi runs `/figlens plan`, reads a dashboard frame from a local `.fig`, builds it with React and Tailwind CSS, then checks and corrects the running UI. Edited time-lapse, not a benchmark or a pixel-perfect promise. Design: [Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [recording notes](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

**[From .fig to editable HTML/CSS](examples/task-list/README.md):** in a controlled task-list study, all six figlens-based attempts preserved all 19 visible text occurrences as real DOM text and all four icons. CLI and MCP extraction also passed with Docker networking disabled. Follow the case study for setup, measurements and independent image comparisons.

<a id="why"></a>

## Why figlens instead of the official Figma MCP?

Use **figlens** when the design should travel as a file: remote agents, CI, offline work, handoff between agents, unlimited repeated reads. Use the **official Figma MCP** when the agent must see the live design without an export step.

Reading a `.fig` is offline. Installing and updating figlens, `gs://` inputs, your model provider and your project's dependencies still need a network.

[Full comparison](docs/README.md#why)

<a id="bundle"></a>

## What the agent gets

One frame, one zip. Structure to build from, assets to use, a picture to check against.

[Bundle contents and examples](docs/README.md#bundle) · [How close is it to Figma?](docs/README.md#accuracy)

## Documentation

- <a id="pi"></a>[Pi extension](docs/README.md#pi)
- <a id="viewer"></a>[Browse a design without Figma](docs/README.md#viewer)
- <a id="mcp-and-skill"></a>[MCP server and agent skill](docs/README.md#mcp-and-skill)
- <a id="commands"></a>[CLI reference](docs/README.md#commands)

<a id="limitations"></a>

## Limitations

- **Snapshots, not live sync.** Export a new `.fig` when the design changes. figlens cannot download a design from a Figma URL.
- **Undocumented format.** figlens decodes the file's embedded schema; new Figma exports may need a figlens update.
- **Fonts and text:** font files are not bundled; overridden text can retain Figma's saved layout. Use licensed project fonts and compare the result.
- **Support coverage:** follow [rendering improvements](https://github.com/runkids/figlens/issues/36) and [handoff semantics](https://github.com/runkids/figlens/issues/37) in the issue tracker. See [current semantic coverage](wiki/semantics.md).
- **Check the output.** Read bundle `warnings`, `missing-assets.json` and unresolved variables in `context.md`, then compare the implementation with the design.
- Known visible rendering limitations appear in node `_figlens.renderDiagnostics`, bounded `inspect` summaries and bundle `warnings`; `missing-assets.json` lists only asset failures. No warnings does not establish pixel fidelity.

## Help figlens grow

If figlens turned a design into code for you, [star the repo](https://github.com/runkids/figlens) so others find it. Hit a file it cannot read? [Open an issue](https://github.com/runkids/figlens/issues) with the command and what you expected.

Development runs Rust in Docker only: `docker compose run --rm dev cargo test`. See [AGENTS.md](AGENTS.md) and the [wiki](wiki/README.md).

## License

[MIT](LICENSE). figlens is an independent project, not affiliated with, endorsed by or sponsored by Figma. The `.fig` format is undocumented and may change; use only files and assets you have permission to access and share.
