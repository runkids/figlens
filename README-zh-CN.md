<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">让 coding agent 直接读 Figma 文件。<br>不开 Figma。不登录。没有 API 配额。</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://www.npmjs.com/package/@runkids/figlens"><img src="https://img.shields.io/npm/dm/@runkids/figlens?label=npm%20downloads&amp;color=8b78c8" alt="npm downloads per month"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">开始使用</a> · <a href="#demo">演示</a> · <a href="#why">为什么选 figlens</a> · <a href="#bundle">agent 能拿到什么</a> · <a href="#pi">Pi</a> · <a href="#limitations">限制</a>
</p>

**问题。** Agent 需要布局数值、文字和素材才能实现画面。在远程主机或 CI，这些设计上下文需要与代码放在一起，让 agent 无需依赖运行中的桌面应用。

**解法。** 在 Figma 中选择 **File > Save local copy**，把 `design.fig` 放在代码旁边。figlens 在本地解析它，给你的 agent 提供布局、文字、样式、变量、图标、图片，以及任意 frame 的参考渲染图。一个原生可执行文件，不向 Figma 发出任何网络请求。

<a id="start"></a>

## 开始使用

把 `design.fig` 放进项目，然后选择你的 agent。

**Claude Code**（Node.js 18+，macOS/Linux）：

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

开启新会话并检查 `/mcp`。要使用有引导的规划流程，请[添加 skill](#mcp-and-skill)。

**Pi**（1.3.0+）：一个包就包含 CLI、MCP、viewer 和 skill。

```sh
pi install npm:@runkids/figlens
```

**任意 MCP 客户端**（Cursor、Codex、Gemini CLI 等）：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**没有 Node.js？** 安装独立可执行文件，并改用 `"command": "figlens"`：

| 平台 | 命令 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

然后让它做一个画面：

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

agent 会找到 frame，生成包含设计数据、素材和参考图的 bundle，并用你的技术栈实现画面。请让它展示修改的文件和对比结果；只有 bundle 并不能证明 UI 是对的。

<a id="demo"></a>

## 演示：Pi 从 .fig 构建 React 画面

[![Pi 从本地 .fig 规划并构建 React 画面，并在实时浏览器旁检查](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

GIF 以剪辑版视频的 3 倍速度循环播放；点击可观看完整 78 秒视频。

**[78 秒视频](assets/taskflow-demo.mp4)。** Pi 运行 `/figlens plan`，从本地 `.fig` 读取一个 dashboard frame，用 React 和 Tailwind CSS 实现，然后检查并修正运行中的 UI。视频经过缩时剪辑，不是 benchmark，也不保证 pixel-perfect。设计：[Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [录制说明](wiki/accuracy.md#taskflow-react-and-tailwind-demo)。

<a id="why"></a>

## 为什么用 figlens，而不是官方 Figma MCP？

当设计需要以文件形式流转时，用 **figlens**：远程 agent、CI、离线工作、agent 之间交接、不限次数的重复读取。当 agent 必须无需导出步骤就能看到实时设计时，用**官方 Figma MCP**。

| | figlens | 官方 Figma MCP |
|---|---|---|
| 来源 | 导出的 `.fig` 快照 | 实时 Figma 文件 |
| 需要 | 文件本身 | Figma 登录；桌面端 server 需要应用 |
| 速率限制 | 无，本地解析 | Figma 服务限制 |
| 远程 agent / CI | 复制文件，运行可执行文件 | 需登录的远程 server |
| 设计变更 | 导出新的 `.fig` | 自动同步 |
| 交接 | 分享 `.fig` 或 frame bundle | 分享 Figma 访问权限 |
| 还原度 | 独立解析器，有[已知差距](#limitations) | 由 Figma 维护 |

读取 `.fig` 可离线进行。安装和更新 figlens、`gs://` 输入、你的模型提供商以及项目依赖仍然需要网络。

<a id="bundle"></a>

## agent 能拿到什么

一个 frame，一个 zip。有可据以构建的结构、可直接使用的素材，以及用来比对的图片。

```text
frame.zip
├── context.md                Start here: instructions for your agent
├── design/
│   ├── nodes.json            Layout, text, styles and expanded instances
│   └── variables.json        Bound variables, collections, modes and aliases
├── assets/                   Image fills, SVGs and composited node renders
├── assets.json               Maps design references to asset files
├── references/screen.png     A render of the frame to compare against
├── missing-assets.json       Anything that could not be produced
└── manifest.json             File roles, media types, sizes and SHA-256 hashes
```

只需要其中一部分？CLI 可以直接完成：

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # 按名称或文字查找 frame id
npx -y @runkids/figlens inspect design.fig 12:345                 # 精简的布局与文字摘要
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST 格式 JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG、PNG、JPG、WebP 或 AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # 上面的 zip
```

SVG 导出会嵌入原始图片填充，并把文字转为字形轮廓，因此不需要字体。通过 MCP，光栅导出也会以内联图片返回，agent 无需打开文件就能看到设计。

### 与 Figma 有多接近？

以 Figma 自身的 JSON、PNG 和 SVG 导出为基准，在 **826 个正式设计 frame** 和五个 Community 文件上测量（2026-10-22）：

| 检查项 | 结果 |
|---|---|
| 节点结构、名称、可见性、边界和文字 | 771 / 826 个 frame 完全一致 |
| 原始图片填充 | 2,608 / 2,608 字节完全一致 |
| SVG 素材 | 5,014 / 5,197 在像素差异阈值内 |
| Community frame bundle | 306 / 306 无缺失素材 |

这是对上述文件的结果，不保证适用于每个设计。方法和阈值：[准确度报告](wiki/accuracy.md)。

<a id="pi"></a>

## Pi 扩展

```sh
pi install npm:@runkids/figlens
```

重启 Pi 或执行 `/reload`，然后用日常语言要一个画面。Pi 会找到 frame，通过自动注册的 MCP server 读取，并用你的技术栈构建。不需要 node ID，也不需要全局 CLI。

- `/figlens plan <request>` 只生成计划，不修改应用程序。
- `/figlens yolo <request>` 规划、构建、检查，并清理自己创建的临时计划。
- `/figlens preview [description]` 打开本地 viewer。
- `/figlens review` 批准 agent 提出的可复用组件映射。

![向 Pi 要一个画面，让它找到设计、用你的技术栈构建并检查结果](assets/pi-workflow.svg)

完整流程、命令和限制：[pi/README.md](pi/README.md)。

<a id="viewer"></a>

## 无需 Figma 浏览设计

[![在本地检查 MCP Apps 设计的图层树、布局和组件属性](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # 本地 viewer，Ctrl+C 停止
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # 可发给任何人的离线 HTML 快照
```

浏览页面和图层、搜索、测量，检查布局、字体样式、变量和 CSS，导出任意图层，复制实现提示词，或下载所选内容的 bundle。导入你实现的截图，即可叠加在设计上比对。详情：[预览指南](wiki/preview.md)。

<a id="mcp-and-skill"></a>

## MCP server 与 agent skill

**MCP。** server 在本地通过 stdio 运行，提供 `tree`、`find`、`inspect`、`nodes`、`variables`、`bundle` 和 `export`。请传入绝对路径。加上 `--root /path/to/designs`（可重复）可把所有文件和输出路径限制在指定文件夹内。光栅导出会缩小到 `maxSize`（默认 1600 px），让内联图片保持小巧；SVG 只返回 metadata。

在原生 Windows 上使用 Claude Code：请参照其 [stdio 设置说明](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server)，并为 `npx` 加上 `cmd /c` 包装。

**Skill。** [figlens skill](skills/figlens/SKILL.md) 教 agent 查找 frame、分段阅读 bundle、规划限定范围的工作，并对照设计验证运行中的 UI。它只安装说明；CLI 或 MCP server 必须另行可用。可用 [Skills CLI](https://github.com/vercel-labs/skills) 安装，或把 `skills/figlens/` 复制到 agent 的 skills 目录（Claude Code：`~/.claude/skills/`）：

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare。** 用 [skillshare](https://github.com/runkids/skillshare) 在一处统一管理 Claude Code、Codex 和 Cursor 的 skill 与 MCP 连接：

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**安装说明。** npm launcher 会在首次使用时下载匹配的原生可执行文件，对照 release 的 SHA-256 校验和进行验证并缓存。独立安装可用 `figlens update` 更新。可执行文件覆盖 macOS（Apple silicon、Intel）、Linux（arm64、x86_64、静态 musl）和 Windows（x86_64）。

<a id="commands"></a>

## CLI 参考

| 命令 | 输出 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 节点 id、类型和名称；缩进文本或嵌套 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 不区分大小写地搜索图层名称和文字；返回路径、尺寸和预览。默认 20 条结果（1-200） |
| `inspect <file> <node> [--depth N] [--images]` | 展开后的精简布局/文字摘要，最多 200 个节点。`--images` 会加上位图尺寸和裁切范围 |
| `nodes <file> <node>` | REST 格式 JSON：展开的 instance 与覆写、auto-layout、文字样式、填充、效果、绑定变量、组件属性、渲染边界 |
| `variables <file> [node]` | 某个 frame 的绑定变量；不指定 node 则输出所有本地变量 |
| `bundle <file> <node> <out.zip>` | 单个 frame 的设计 bundle；输出 `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | 一次解码，并行生成多个 bundle |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG、PNG、JPG、WebP 或 AVIF；`--max-kb` 会降低有损质量直到符合大小 |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 本地只读 viewer；`--out` 写出独立的离线快照 |
| `compare <file> <nodes.json> <node>` | 与同一 frame 的 REST 导出做差异比较 |
| `node <file> <node>` | 单个原始 `.fig` 节点 |
| `mcp` | 通过 stdio 运行的本地 MCP server |
| `update` | 用最新 release 替换可执行文件 |

node 参数接受 `12:345`、`12-345`，或带 `?node-id=` 的 Figma URL。`<file>` 也可以是 `gs://bucket/object`，通过你的 `gcloud` 登录下载并缓存。每次 CLI 调用都会解码文件；MCP server 会在多次调用之间保留已解析的文件，直到它在磁盘上发生变化。运行 `figlens --help` 查看用法。

<a id="limitations"></a>

## 限制

- **快照，不是实时同步。** 设计变更后请导出新的 `.fig`。figlens 无法通过 Figma URL 下载设计。
- **格式未公开。** figlens 解码文件内嵌的 schema；新的 Figma 导出可能需要更新 figlens。
- **尚未输出：** Dev Mode 注释、constraints、layoutAlign/layoutGrow、layout grids、组件属性定义。
- **渲染差距：** 背景模糊和部分图片调整会被跳过；角度渐变和菱形渐变会按放射渐变绘制。被覆写的文字可能保留 Figma 上一次的布局。不包含字体文件。
- **检查输出。** 阅读 bundle 的 `warnings`、`missing-assets.json` 和 `context.md` 中未解析的变量，然后将实现与设计对比。

## 帮助 figlens 成长

如果 figlens 帮你把设计变成了代码，欢迎[给仓库点个 star](https://github.com/runkids/figlens)，让更多人发现它。遇到它读不了的文件？请[提交 issue](https://github.com/runkids/figlens/issues)，附上命令和你预期的结果。

开发时 Rust 只在 Docker 中运行：`docker compose run --rm dev cargo test`。参见 [AGENTS.md](AGENTS.md) 和 [wiki](wiki/README.md)。

## 许可

[MIT](LICENSE)。figlens 是独立项目，与 Figma 没有隶属、背书或赞助关系。`.fig` 格式未公开，可能变更；请仅使用你有权访问和分享的文件与素材。
