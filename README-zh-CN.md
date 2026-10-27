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
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">开始使用</a> · <a href="#demo">演示</a> · <a href="#why">为什么选 figlens</a> · <a href="#bundle">agent 能拿到什么</a> · <a href="#pi">Pi</a> · <a href="#limitations">限制</a>
</p>

**问题。** Agent 需要布局数值、文字和素材才能实现画面。在远程主机或 CI，这些设计上下文需要与代码放在一起，让 agent 无需依赖运行中的桌面应用。

**解法。** 在 Figma 中选择 **File > Save local copy**，把 `design.fig` 放在代码旁边。figlens 在本地解析它，给你的 agent 提供布局、文字、样式、变量、图标、图片，以及任意 frame 的参考渲染图。一个原生可执行文件，不向 Figma 发出任何网络请求。

<a id="start"></a>

## 开始使用

**还没有设计文件？** [用小型公开范例检查环境](examples/first-success/README.md)：验证所选的可执行文件、找到一个 frame，再核对 bundle 内容。不需要模型。

把 `design.fig` 放进项目，然后选择你的 agent。

**Claude Code**（Node.js 18+，macOS/Linux）：

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

开启新会话并检查 `/mcp`。用自己的话描述想做的画面即可：MCP 内建按需读取的规划、实现及图片处理指引，不必另外安装 skill。

**Pi**（1.3.0+）：一个包就包含 CLI、MCP、viewer 和 skill。

```sh
pi install npm:@runkids/figlens
```

**任意 MCP 客户端**（Cursor、Codex、Gemini CLI 等）：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

[独立可执行文件安装](docs/README-zh-CN.md#install)

然后让它做一个画面：

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

agent 会找到 frame，将设计数据、素材和参考图打包成 bundle，并用你的技术栈构建画面。共享工作流会指导字体与素材检查；缺失项会被报告，布局实现仍使用已记录的替代方案继续。请让它展示修改的文件与浏览器对比结果。

<a id="demo"></a>

## 演示：Pi 从 .fig 构建 React 画面

[![Pi 从本地 .fig 规划并构建 React 画面，并在实时浏览器旁检查](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

GIF 以剪辑版视频的 3 倍速度循环播放；点击可观看完整 78 秒视频。

**[78 秒视频](assets/taskflow-demo.mp4)。** Pi 运行 `/figlens plan`，从本地 `.fig` 读取一个 dashboard frame，用 React 和 Tailwind CSS 实现，然后检查并修正运行中的 UI。视频经过缩时剪辑，不是 benchmark，也不保证 pixel-perfect。设计：[Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [录制说明](wiki/accuracy.md#taskflow-react-and-tailwind-demo)。

**[从 .fig 到可编辑 HTML/CSS](examples/task-list/README.md)：** 在受控的任务列表实验中，6 次基于 figlens 的尝试均保留了全部 19 处可见文字为真实 DOM 文本，以及全部 4 个图标。CLI 与 MCP 提取也通过了禁用 Docker 网络的验证。案例页提供设置步骤、测量与独立图片对比。

<a id="why"></a>

## 为什么用 figlens，而不是官方 Figma MCP？

当设计需要以文件形式流转时，用 **figlens**：远程 agent、CI、离线工作、agent 之间交接、不限次数的重复读取。当 agent 必须无需导出步骤就能看到实时设计时，用**官方 Figma MCP**。

读取 `.fig` 可离线进行。安装和更新 figlens、`gs://` 输入、你的模型提供商以及项目依赖仍然需要网络。

[完整对比](docs/README-zh-CN.md#why)

<a id="bundle"></a>

## agent 能拿到什么

一个 frame，一个 zip。有可据以构建的结构、可直接使用的素材，以及用来比对的图片。

[Bundle 内容与使用示例](docs/README-zh-CN.md#bundle) · [与 Figma 有多接近？](docs/README-zh-CN.md#accuracy)

## 详细文档

- <a id="pi"></a>[Pi 扩展](docs/README-zh-CN.md#pi)
- <a id="viewer"></a>[无需 Figma 浏览设计](docs/README-zh-CN.md#viewer)
- <a id="mcp-and-skill"></a>[MCP server 与 agent skill](docs/README-zh-CN.md#mcp-and-skill)
- <a id="commands"></a>[CLI 参考](docs/README-zh-CN.md#commands)

<a id="limitations"></a>

## 限制

- **快照，不是实时同步。** 设计变更后请导出新的 `.fig`。figlens 无法通过 Figma URL 下载设计。
- **格式未公开。** figlens 解码文件内嵌的 schema；新的 Figma 导出可能需要更新 figlens。
- **字体与文字：** 不包含字体文件；被覆写的文字可能保留 Figma 保存的布局。请使用有授权的项目字体并对比结果。
- **支持范围：** [渲染改进](https://github.com/runkids/figlens/issues/36)与[交接语义](https://github.com/runkids/figlens/issues/37)已在 issue 中跟踪。另见[当前语义支持范围](wiki/semantics.md)。
- **检查输出。** 阅读 bundle 的 `warnings`、`missing-assets.json` 和 `context.md` 中未解析的变量，然后将实现与设计对比。
- 已知可见渲染限制会列在节点 `_figlens.renderDiagnostics`、有数量上限的 `inspect` 摘要及 bundle `warnings`；`missing-assets.json` 只列素材失败。没有警告不代表像素一致。

## 帮助 figlens 成长

如果 figlens 帮你把设计变成了代码，欢迎[给仓库点个 star](https://github.com/runkids/figlens)，让更多人发现它。遇到它读不了的文件？请[提交 issue](https://github.com/runkids/figlens/issues)，附上命令和你预期的结果。

开发时 Rust 只在 Docker 中运行：`docker compose run --rm dev cargo test`。参见 [AGENTS.md](AGENTS.md) 和 [wiki](wiki/README.md)。

## 许可

[MIT](LICENSE)。figlens 是独立项目，与 Figma 没有隶属、背书或赞助关系。`.fig` 格式未公开，可能变更；请仅使用你有权访问和分享的文件与素材。
