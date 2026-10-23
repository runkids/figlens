<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">让你的 coding agent，直接从 Figma 设计开始。</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  把任何画面的布局、文字、变量、素材与参考图交给你的 coding agent。<br>
  在笔记本、云端或 CI 上，想读几次就读几次。<br>
  <strong>运行工作的机器不需要 Figma App、登录、seat，也没有 API 速率限制。</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">观看示范</a> · <a href="#quick-start">试试你的设计</a> · <a href="#for-ai-agents">连接 agent</a> · <a href="#pi-extension">搭配 Pi 使用</a> · <a href="#how-close-is-it-to-figma">准确度</a>
</p>

<a id="why-figlens"></a>

## 为什么使用 figlens？

当你说「做出这个画面」，agent 应该同时拿到它的布局、文字与素材。只有截图时，agent 得估算间距与字体样式，图标还是要你手动导出。Figma MCP server 读的是真实设计，但配额绑在文件所属的团队上，agent 一个 prompt 就可能用光一天的配额，而且机器必须登录 Figma。

figlens 改从保存的副本读取设计。在 Figma 选择 **File > Save local copy**（只要所有者没有关闭复制，查看权限就够）。把这个 `.fig` 通过 CLI 或本地 MCP server 交给 agent，它就能寻找画面、查看布局与文字，并提取所需素材。也可以把一个 frame 的设计数据、素材与参考图打包成一个 zip，任何 agent 或队友都能使用。

| 如果你遇过这些情况 | figlens 可以帮你 |
|---|---|
| 付费 seat 也遇到「Rate limit exceeded, try again tomorrow」 | 随时读取保存的文件，不调用 Figma API，不需要 token 或 seat |
| agent 在云端服务器或 CI 执行 | 使用原生 CLI 读取设计，不需要 Figma App 或浏览器会话 |
| 不断解释尺寸、导出素材 | 提供结构化布局、字体样式、展开的 instance、绑定变量与原始素材 |
| 其他 agent 或队友需要相同画面 | 分享包含设计数据、素材、参考图与说明的 frame bundle |

设计师继续使用 Figma，agent 使用项目既有的技术栈。figlens 提供设计上下文，coding agent 负责撰写实现。

<details>
<summary>.fig 格式没有公开文档。Figma 改了格式怎么办？</summary>

figlens 不靠猜测格式。每个版本都用同一批文件对照 Figma 自己导出的 JSON、PNG 与 SVG 检查（[结果](#how-close-is-it-to-figma)），所以格式变动会以可测量的回归显现，而不是悄悄出错。你已经保存的副本，用读取它的那个 figlens 版本仍可继续使用；只有从改版后的 Figma 新保存的文件才需要更新 figlens，`figlens update` 就能取得。

</details>

<a id="from-a-local-fig-to-working-html"></a>

## 从本地 .fig 到可操作的 HTML

[![Pi 从本地 Figma 文件建立可操作的界面，终端旁同步显示预览](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[观看视频](assets/demo-8x.mp4)** · 45 秒缩时视频，**8 倍速**。这是在 Orca 中进行的真实 Pi 会话：读取本地设计、建立 HTML/CSS，然后在即时预览中操作分享对话框与输入区。界面使用 bundle 内的 19 个 SVG 图标，是 HTML 实现。

| 此示例 | 实测结果 |
|---|---|
| 本地 `.fig` 文件 | 57.5 MiB |
| 选取的 frame | 153 个节点、25 个 SVG 素材，无缺失素材 |
| 提取设计 bundle | 五次暖缓存执行的中位数为 **0.47 秒** |

计时测量的是 `figlens bundle`，**不包含 AI 实现时间**。使用 figlens 0.1.2，在 ARM64 Docker 容器中测量；详见[指令、各次结果与录制说明](wiki/accuracy.md#local-design-to-html-demo)。Pi 撰写代码，figlens 提供设计数据；AI 服务仍需网络连接。

<details>
<summary>设计出处与示范限制</summary>

「List views」frame 来自 [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude)。原始 Anthropic 字体未包含在 bundle 中，因此示范使用系统 sans 与 Georgia。分享按钮打开本地对话框；回复保留在页面中，重新加载即清除。录制由真实 Orca 窗口的连续截图组成，动态效果较一般录制不流畅。

</details>

<a id="quick-start"></a>

## 快速开始

<a id="1-open-your-design-locally"></a>

### 1. 在本地打开设计

在 Figma 选择 **File > Save local copy**，保存为 `design.fig`，接着执行：

```sh
npx @runkids/figlens preview design.fig
```

打开终端显示的本地网址，浏览页面并选取要实现的画面。你可以直接查看间距、字体样式、变量与素材。

需要 Node.js 18+。首次执行会下载、验证并缓存原生可执行文件。[独立安装](#installation)不需要 Node.js、Rust 或 Docker。按 Ctrl+C 停止 viewer。

<a id="2-give-the-screen-to-your-agent"></a>

### 2. 将画面交给 agent

从 viewer 下载选取 frame 的 bundle，将 zip 和这段提示交给 coding agent：

> 将此 bundle 解压缩到独立目录并阅读 context.md。依照此项目的技术栈与惯例实现这个画面。使用提供的布局数值与素材，并以 references/screen.png 检查成果。报告缺失素材或无法取得的字体。

bundle 包含结构、素材与参考图，agent 可通过一般文件工具阅读。若要让它直接寻找与查看画面，请[连接 CLI 或 MCP server](#for-ai-agents)。[Pi 用户可以直接指定画面名称](#pi-extension)。

<a id="3-check-the-implementation"></a>

### 3. 检查实现成果

将实现画面的截图导入 viewer，与设计并排比较，或通过透明度叠图找出差异。截图只留在浏览器中。这是视觉检查；figlens 不会为实现计算准确度分数。

**`.fig` 是快照。** 设计变更后，请重新保存本地副本。Figma 网址只能选取该副本中的节点，不能用来下载设计。安装完成后可离线读取文件；AI 服务与项目依赖包仍可能需要网络连接。

<details>
<summary>偏好终端操作？直接寻找 frame 并打包。</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

将 `12:345` 换成树状清单或搜索结果中的 ID。节点可使用 ID（`12:345`）、网址格式（`12-345`）或带有 `?node-id=` 的 Figma 网址。用 `find` 搜索图层名称或文字；用 `inspect` 查看展开的 instance 子节点与精简的布局／文字摘要。输出细节请见[指令](#commands)。

</details>

<a id="browse-and-hand-off-a-design"></a>

## 浏览与交接设计

[![在本地查看 MCP Apps 设计的图层树、布局与组件属性](assets/preview-inspector.png)](assets/pi-demo.mp4)

本地 viewer 让你无需 Figma 会话就能探索保存的设计：

- 浏览页面与图层、搜索名称与文字、测量距离，并查看布局、字体样式、变量、组件属性与 CSS。
- 将选取图层导出为 SVG、PNG、JPG、WebP 或 AVIF，或下载其原始填色图片。
- 复制图层 ID、结构化上下文或实现提示；下载涵盖精确选取范围的 bundle，包含 instance 子图层。

想让其他人也能浏览设计？导出独立 HTML 快照：

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

它不需要服务器或网络即可打开，内含设计数据、图片与快照范围的 bundle。仅分享给有权查看素材的人。选取子节点会改变查看内容；内嵌 bundle 仍涵盖原本的快照范围。

即时 viewer 监看保存的本地文件，保存成功后更新；部分写入或保存失败会标记画面数据已过期。HTML 快照维持不变。操作方式、缓存管理、浏览器 agent 自动化与限制请见[预览指南](wiki/preview.md)。

<a id="pi-extension"></a>

## Pi 扩展包

**给 Pi 一个 `.fig`，告诉它要哪个画面。** 包包含 CLI、MCP、preview、扩展与 agent skill：

```sh
pi install npm:@runkids/figlens
```

重新启动 Pi 或执行 `/reload`，接着用一般语句提出要求：

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi 会寻找画面、读取布局与素材、查看项目，并使用现有技术栈实现。不需知道 frame ID，也不必执行 `/figlens open`。名称有歧义时，Pi 可列出少量候选项目。MCP 会自动启动，不需全域安装 CLI。

![指定 Pi 实现画面，让它寻找设计、使用你的技术栈并检查结果](assets/pi-workflow.svg)

**选用的组件重用：** 请 Pi 保存可重用的对应，再用 `/figlens review` 接受或编辑项目中的待审提案。决策存放于 `.figlens/component-mappings.md`；variant 或记录的代码变更后需重新审查。实现画面不需要先建立对应。

[![查看本地设计，在 Pi 中审查既有 HTML 组件，然后在下一个 frame 重用](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[观看选用的对应流程](assets/pi-demo.mp4)** · v0.3.0 的 20 秒剪辑录制：查看 MCP Apps 设计、核准预先准备的既有 HTML 对应，然后在新会话重用。未展示或计时模型生成。[完整流程、指令与限制](pi/README.md) · [录制与验证说明](wiki/pi.md#demo-and-verification)。

需要 Pi 1.3.0+。npm 扩展包使用随附的 CLI launcher；首次启动（包含自动启动 MCP）会下载并验证对应的原生可执行文件。Homebrew 与 release 压缩档提供原生 CLI 和 viewer。开发配置请见[源代码安装说明](pi/README.md#install-once)。

<a id="the-design-bundle"></a>

## 设计 bundle

一个 frame，一份 zip。内含实现所需的结构、可使用的素材，以及用来比对的图片。

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

只需要其中一部分？直接导出 JSON 或渲染节点：

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

SVG 导出支援 frame、群组、图标与精确的 instance 子图层：选取的子树会产生单一独立 SVG，嵌入原始填色图片，文字以字形轮廓表示。不需解压 bundle。SVG 维持原始尺寸；`--scale` 与 `--quality` 仅适用光栅格式。

<a id="how-close-is-it-to-figma"></a>

## 与 Figma 有多接近？

figlens 以 Figma 自身导出的 JSON、PNG 和 SVG 验证。以下是 **826 个正式设计 frame** 与 **五个 Community 文件**的记录结果：

| 检查项目 | 结果 |
|---|---|
| 节点结构、名称、可见性、边界与文字 | 771 / 826 个正式设计 frame 完全一致 |
| 原始填色图片 | 2,608 / 2,608 字节完全一致 |
| SVG 素材 | 5,014 / 5,197 在像素差异门槛内 |
| Community frame bundle | 306 / 306 无缺失素材 |

这些是对上述文件的测量结果，不保证每个设计皆相同。日期、门槛、渲染结果与方法请见[准确度报告](wiki/accuracy.md)。

**已知限制：** 尚未输出 constraints、layoutAlign/layoutGrow、layout grids 与 component property definitions。渲染会略过背景模糊与部分图片调整（对比、色温、色调、高光、阴影），并以放射渐层绘制角度与菱形渐层。文字使用已保存的字形轮廓，因此渲染不需字体档，但覆写文字可能保留 Figma 上次的布局。实现所需的字体档不包含在内。

<a id="for-ai-agents"></a>

## 给 AI agents

通过 agent skill 使用 CLI，或将同一个可执行文件接成 MCP server。

### Agent skill

[figlens skill](skills/figlens/SKILL.md) 教 agent 寻找 frame、分段阅读 bundle，并依照其中的数值实现。

通过 [Skills CLI](https://github.com/vercel-labs/skills) 安装：

```sh
npx skills add runkids/figlens --skill figlens
```

若要跨 Claude Code、Codex、Cursor、Gemini CLI 等工具管理与更新 skills，可使用 [skillshare](https://github.com/runkids/skillshare)：

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

用 `skillshare update --all` 保持最新。也可将 `skills/figlens/` 复制到 agent 的 skills 目录（Claude Code 为 `~/.claude/skills/`）。Release 压缩档也有附上。

### MCP server

使用 [skillshare](https://github.com/runkids/skillshare) 一起管理 MCP server 与 agent skills。定义 figlens 一次，再将连接配置同步到 Claude Code、Codex 与 Cursor：

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

其他 client 与导入既有配置的方式，请见 [Skillshare MCP 指南](https://skillshare.runkids.cc/docs/reference/commands/mcp/)。

直接配置 Claude Code：

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

其他 MCP client：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

若已独立安装 `figlens`，请使用：

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Server 在本地通过 stdio 执行，提供 `tree`、`find`、`inspect`、`nodes`、`variables`、`bundle` 与 `export`。工具请使用绝对路径：相对路径以 client 启动 figlens 的目录为准。

先用 `find` 与 `inspect` 选择 frame。`export` 保存文件并返回 metadata。光栅格式也会返回 MCP 图片内容，因此 agent 不需打开本地文件就能看见设计；SVG 仅返回 metadata。PNG 的 client 支援最广，大型 frame 可降低 `scale`。`find`、`inspect`、`bundle` 与 `export` 也会返回结构化 JSON。

<a id="installation"></a>

## 安装

搭配 Node.js 18+ 使用 `npx @runkids/figlens`，或安装独立 CLI：

| 平台 | 指令 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Release 可执行文件支援 macOS（Apple silicon、Intel）、Linux（arm64、x86_64、静态 musl）与 Windows（x86_64；ARM64 通过模拟执行）。独立安装可用 `figlens update` 更新。

<a id="commands"></a>

## 指令

| 指令 | 输出 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 节点 ID、类型与名称；缩进文字（减少 agent 上下文用量），加 `--json` 则输出巢状 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 不分大小写搜索图层名称与文字子字串；包含路径、尺寸与预览。默认 20 笔（1–200）；`truncated` 表示还有结果 |
| `inspect <file> <node> [--depth N]` | 展开后的精简布局／文字摘要。默认深度 1（0–10），最多 200 个节点；完整数据用 `nodes` |
| `nodes <file> <node>` | REST 格式 JSON：展开的 instance 与覆写、自动布局、文字样式、填色、效果、绑定变量、组件属性与渲染边界 |
| `variables <file> [node]` | frame 绑定的变量；未指定节点时输出所有本地变量 |
| `bundle <file> <node> <out.zip>` | 单一 frame 的设计 bundle；返回 `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | 解码一次，平行产生多个 bundle；每个 frame 输出一行 JSON，格式同上或 `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | SVG、PNG、JPG、WebP 或 AVIF；`--max-kb` 降低有损品质至符合大小；PNG/SVG 超限会失败；返回 `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 本地只读 viewer；`--out` 需指定节点，产生独立离线快照 |
| `compare <file> <nodes.json> <node>` | 与同一 frame 的 REST 导出比较差异 |
| `node <file> <node>` | 单一原始 `.fig` 节点 |
| `mcp` | 通过 stdio 执行的本地 MCP server |
| `update` | 以最新 release 取代可执行文件 |

`find` 搜索保存的图层，返回 ID、祖先路径、本地尺寸与文字预览。`inspect` 展开 instance，输出上限为 200 个节点与每段文字 200 个字符；`truncated` 和 `textTruncated` 标示省略内容。每次调用仍会解码整份文件。

`<file>` 也可使用 `gs://bucket/object`，通过你的 `gcloud` 登录下载并缓存。用法请执行 `figlens --help`。

<a id="help-figlens-grow"></a>

## 帮助 figlens 成长

如果 figlens 帮助你把下一个设计做成代码，欢迎[给它一颗星](https://github.com/runkids/figlens)，让更多人找到它。遇到无法处理的设计？请[开 issue](https://github.com/runkids/figlens/issues)，附上执行的指令与预期结果。

开发时 Rust 仅在 Docker 中执行：`docker compose run --rm dev cargo test`。`make devc` 打开 shell，当中的 `figlens` 执行此 checkout 的源代码；`make help` 列出容器目标。VS Code 的 Reopen in Container 使用相同服务。项目规范请见 [AGENTS.md](AGENTS.md)，技术主题请见 [wiki](wiki/README.md)。

<a id="license"></a>

## 许可

[MIT](LICENSE)。figlens 是独立项目，与 Figma 无隶属、背书或赞助关系。`.fig` 格式未公开文件，可能随时变更；figlens 读取你从自己的 Figma 帐号保存的文件。
