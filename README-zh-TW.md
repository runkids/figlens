<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">讓 coding agent 直接讀 Figma 檔案。<br>不用開 Figma、不用登入、沒有 API 額度限制。</h1>

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
  <a href="#start">開始使用</a> · <a href="#demo">示範</a> · <a href="#why">為什麼選 figlens</a> · <a href="#bundle">Agent 拿到什麼</a> · <a href="#pi">Pi</a> · <a href="#limitations">限制</a>
</p>

**問題。** Agent 需要排版數值、文字與素材才能實作畫面。在遠端主機或 CI，這些設計上下文需要和程式碼放在一起，讓 agent 不必依賴執行中的桌面 App。

**解法。** 在 Figma 選擇 **File > Save local copy**，把 `design.fig` 放在程式碼旁邊。figlens 在本機解析，給你的 agent 排版、文字、樣式、variables、圖示、圖片，以及任一 frame 的參考渲染圖。一個原生執行檔，完全不對 Figma 發出網路呼叫。

<a id="start"></a>

## 開始使用

**還沒有設計檔？** [用小型公開範例檢查環境](examples/first-success/README.md)：驗證所選執行檔、找到一個 frame，再核對 bundle 內容。不需要模型。

把 `design.fig` 放進你的專案，再選你的 agent。

**Claude Code**（Node.js 18+、macOS/Linux）：

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

開啟新的 session 並檢查 `/mcp`。用自己的話描述想做的畫面即可：MCP 內建按需讀取的規劃、實作及圖片處理指引，不必另外安裝 skill。

**Pi**（1.3.0+）：一個套件包含 CLI、MCP、viewer 與 skill。

```sh
pi install npm:@runkids/figlens
```

**任何 MCP client**（Cursor、Codex、Gemini CLI 等）：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**沒有 Node.js？** 安裝獨立執行檔，並改用 `"command": "figlens"`：

| 平台 | 指令 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

接著請它做一個畫面：

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

Agent 會找到 frame，建立包含設計資料、素材與參考圖的 bundle，並用你的技術堆疊實作畫面。請它列出變更的檔案與比對結果；只有 bundle 不能證明 UI 正確。

<a id="demo"></a>

## 示範：Pi 從 .fig 做出 React 畫面

[![Pi 從本機 .fig 規劃、做出 React 畫面，並與即時瀏覽器並排檢查](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

GIF 以剪輯版影片的 3 倍速度循環播放；點擊可觀看完整 78 秒影片。

**[78 秒影片](assets/taskflow-demo.mp4)。** Pi 執行 `/figlens plan`，從本機 `.fig` 讀取儀表板 frame，用 React 與 Tailwind CSS 實作，再檢查並修正執行中的 UI。這是剪輯過的縮時影片，不是基準測試，也不保證像素級一致。設計稿：[Eymen Eid 的 Task Flow](https://www.figma.com/community/file/1224268866120711793) · [錄製筆記](wiki/accuracy.md#taskflow-react-and-tailwind-demo)。

<a id="why"></a>

## 為什麼用 figlens，而不是官方 Figma MCP？

當設計需要以檔案形式傳遞時，用 **figlens**：遠端 agent、CI、離線工作、agent 之間的交接、不限次數的重複讀取。當 agent 必須不經匯出步驟就看到即時設計時，用**官方 Figma MCP**。

| | figlens | 官方 Figma MCP |
|---|---|---|
| 來源 | 匯出的 `.fig` 快照 | 即時的 Figma 檔案 |
| 需求 | 該檔案 | Figma 登入；桌面版伺服器需要 App |
| 速率限制 | 無，本機解析 | Figma 服務限制 |
| 遠端 agent / CI | 複製檔案，執行執行檔 | 需登入的遠端伺服器 |
| 設計變更 | 匯出新的 `.fig` | 自動 |
| 交接 | 分享 `.fig` 或 frame bundle | 分享 Figma 存取權 |
| 精確度 | 獨立的 parser，有[已知缺漏](#limitations) | 由 Figma 維護 |

讀取 `.fig` 可離線進行。安裝與更新 figlens、`gs://` 輸入、你的模型供應商，以及專案的相依套件仍需要網路。

<a id="bundle"></a>

## Agent 拿到什麼

一個 frame，一個 zip。有可依循的結構、可使用的素材，以及可供比對的圖片。

```text
frame.zip
├── context.md                從這裡開始：給你的 agent 的說明
├── design/
│   ├── nodes.json            排版、文字、樣式與展開後的 instance
│   └── variables.json        綁定的 variables、collection、mode 與 alias
├── assets/                   圖片填色、SVG 與合成後的節點渲染圖
├── assets.json               將設計中的參照對應到素材檔案
├── references/screen.png     供比對的 frame 渲染圖
├── missing-assets.json       所有無法產生的項目
└── manifest.json             檔案角色、媒體類型、大小與 SHA-256 雜湊
```

只需要其中一項？CLI 可以直接做到：

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # 依名稱或文字找 frame id
npx -y @runkids/figlens inspect design.fig 12:345                 # 精簡的排版與文字摘要
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST 格式的 JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG、PNG、JPG、WebP 或 AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # 上面的 zip
```

SVG 匯出會內嵌原始圖片填色，並將文字轉為 glyph 外框，因此不需要字型。透過 MCP 時，點陣圖匯出也會以內嵌圖片回傳，agent 不必開啟檔案就能看到設計。

### 與 Figma 有多接近？

以 Figma 自己的 JSON、PNG 與 SVG 匯出為基準，在 **826 個正式環境 frame** 與五個 Community 檔案上測量（2026-10-22）：

| 檢查項目 | 結果 |
|---|---|
| 節點結構、名稱、可見性、邊界與文字 | 826 個 frame 中有 771 個完全一致 |
| 原始圖片填色 | 2,608 / 2,608 位元組完全一致 |
| SVG 素材 | 5,014 / 5,197 在像素差異門檻內 |
| Community frame bundle | 306 / 306 沒有缺少素材 |

這是在這些檔案上的結果，不保證每個設計都如此。方法與門檻：[精確度報告](wiki/accuracy.md)。

<a id="pi"></a>

## Pi extension

```sh
pi install npm:@runkids/figlens
```

重新啟動 Pi 或執行 `/reload`，然後用一般的話請它做一個畫面。Pi 會找到 frame，透過自動註冊的 MCP 伺服器讀取，並用你的技術堆疊實作。不需要 node ID，不需要全域 CLI。

- `/figlens plan <request>` 只產生計劃書，不修改應用程式。
- `/figlens yolo <request>` 規劃、實作、檢查，並清理自己的暫存規劃。
- `/figlens preview [description]` 開啟本機 viewer。
- `/figlens review` 核准 agent 提出的可重用元件對應。

![請 Pi 做一個畫面，讓它找到設計、用你的技術堆疊實作，並檢查結果](assets/pi-workflow.svg)

完整流程、指令與限制：[pi/README.md](pi/README.md)。

<a id="viewer"></a>

## 不用 Figma 也能瀏覽設計

[![在本機檢視 MCP Apps 設計，包含圖層樹、排版與元件屬性](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # 本機 viewer，按 Ctrl+C 停止
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # 可寄給任何人的離線 HTML 快照
```

可瀏覽頁面與圖層、搜尋、量測，檢視排版、字體、variables 與 CSS，匯出任何圖層，複製實作 prompt，或下載所選項目的 bundle。拖入你實作成果的截圖，即可疊在設計上比對。詳情：[預覽指南](wiki/preview.md)。

<a id="mcp-and-skill"></a>

## MCP 伺服器與 agent skill

**MCP。** 伺服器透過 stdio 在本機執行，提供 `workflow`、`tree`、`find`、`inspect`、`nodes`、`variables`、`bundle` 與 `export`。請傳入絕對路徑。加上 `--root /path/to/designs`（可重複）可將所有檔案與輸出路徑限制在指定的資料夾。點陣圖匯出會縮小到 `maxSize`（預設 1600 px），讓內嵌圖片維持小巧；SVG 只回傳 metadata。

agent 可透過 `workflow` 讀取內建的 `build`、`images`、`plan`、`planning-details`，或明確要求時的 `yolo` 指引。它們與 portable skill 共用來源；MCP 使用者可自行選擇是否另裝 skill。實際工作仍由 agent 使用專案與瀏覽器工具，並遵守 host 權限。讀取指引不會執行工作流程；使用方式取決於客戶端支援與模型行為。

在 Windows 原生環境使用 Claude Code：請依照其 [stdio 設定說明](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server)，並為 `npx` 加上 `cmd /c` 包裝。

**Skill。** [figlens skill](skills/figlens/SKILL.md) 教 agent 找出 frame、分小段讀取 bundle、規劃範圍明確的工作，並對照設計驗證執行中的 UI。它只安裝說明；仍需有可用的 CLI 或 MCP 伺服器。可用 [Skills CLI](https://github.com/vercel-labs/skills) 安裝，或將 `skills/figlens/` 複製到 agent 的 skills 目錄（Claude Code：`~/.claude/skills/`）：

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare。** 用 [skillshare](https://github.com/runkids/skillshare) 在同一處管理 Claude Code、Codex 與 Cursor 的 skill 與 MCP 連線：

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**安裝說明。** npm launcher 在首次使用時下載對應的原生執行檔，以發行版的 SHA-256 checksum 驗證後快取。獨立安裝的版本用 `figlens update` 更新。執行檔涵蓋 macOS（Apple silicon、Intel）、Linux（arm64、x86_64、靜態 musl）與 Windows（x86_64）。

<a id="commands"></a>

## CLI 參考

| 指令 | 輸出 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 節點 id、類型與名稱；縮排文字或巢狀 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 不分大小寫搜尋圖層名稱與文字；路徑、尺寸與預覽。預設 20 筆結果（1-200） |
| `inspect <file> <node> [--depth N] [--images]` | 精簡的展開後排版/文字摘要，最多 200 個節點。`--images` 會加上點陣圖尺寸與裁切範圍 |
| `nodes <file> <node>` | REST 格式的 JSON：展開後的 instance 與 override、auto-layout、文字樣式、填色、效果、綁定的 variables、元件屬性、渲染邊界 |
| `variables <file> [node]` | frame 綁定的 variables；未指定節點時，列出所有本機 variables |
| `bundle <file> <node> <out.zip>` | 單一 frame 的設計 bundle；輸出 `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | 一次解碼，平行產生多個 bundle |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG、PNG、JPG、WebP 或 AVIF；`--max-kb` 會降低有損品質直到符合大小 |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 本機唯讀 viewer；`--out` 會寫出獨立的離線快照 |
| `compare <file> <nodes.json> <node>` | 與同一 frame 的 REST 匯出做差異比對 |
| `node <file> <node>` | 單一原始 `.fig` 節點 |
| `mcp` | 透過 stdio 的本機 MCP 伺服器 |
| `update` | 將執行檔換成最新發行版 |

node 引數可接受 `12:345`、`12-345`，或帶有 `?node-id=` 的 Figma URL。`<file>` 也可以是 `gs://bucket/object`，會用你的 `gcloud` 登入下載並快取。固定 object 交接、所需存取權限與更新限制，請參閱 [GCS 快照操作說明](wiki/gcs.md)。每次 CLI 呼叫都會解碼檔案；MCP 只重用最後讀取且未變更的本機檔案，遠端輸入仍會重新解析。執行 `figlens --help` 查看用法。

<a id="limitations"></a>

## 限制

- **快照，不是即時同步。** 設計變更時請匯出新的 `.fig`。figlens 無法從 Figma URL 下載設計。
- **格式未公開。** figlens 解碼檔案內嵌的 schema；新版 Figma 匯出可能需要更新 figlens。
- **尚未輸出：** Dev Mode 註解、constraints、layoutAlign/layoutGrow、layout grid、元件屬性定義。
- **渲染缺漏：** 背景模糊與部分圖片調整會被略過；angular 與 diamond 漸層會以 radial 繪製。被覆寫的文字可能保留 Figma 最後一次的排版。不包含字型檔。
- **請檢查輸出。** 閱讀 bundle 的 `warnings`、`missing-assets.json`，以及 `context.md` 中未解析的 variables，再將實作與設計比對。

## 幫助 figlens 成長

如果 figlens 幫你把設計變成程式碼，請[為這個 repo 加星](https://github.com/runkids/figlens)，讓更多人找到它。遇到讀不了的檔案？請[開 issue](https://github.com/runkids/figlens/issues)，附上指令與你預期的結果。

開發只在 Docker 中執行 Rust：`docker compose run --rm dev cargo test`。請見 [AGENTS.md](AGENTS.md) 與 [wiki](wiki/README.md)。

## 授權

[MIT](LICENSE)。figlens 是獨立專案，與 Figma 無隸屬、背書或贊助關係。`.fig` 格式未公開，可能會變動；請只使用你有權存取與分享的檔案和素材。
