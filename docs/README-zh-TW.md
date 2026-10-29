# figlens — 使用指南

[← README](../README-zh-TW.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [獨立執行檔安裝](#install)
- [為什麼用 figlens，而不是官方 Figma MCP？](#why)
- [Agent 拿到什麼](#bundle)
- [Pi extension](#pi)
- [不用 Figma 也能瀏覽設計](#viewer)
- [MCP 伺服器與 agent skill](#mcp-and-skill)
- [CLI 參考](#commands)

<a id="install"></a>

## 獨立執行檔安裝

**沒有 Node.js？** 安裝獨立執行檔，並改用 `"command": "figlens", "args": ["mcp"]`：

| 平台 | 指令 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

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
| 精確度 | 獨立的 parser，有[已知缺漏](../README-zh-TW.md#limitations) | 由 Figma 維護 |

讀取 `.fig` 可離線進行。安裝與更新 figlens、`gs://` 輸入、你的模型供應商，以及專案的相依套件仍需要網路。

<a id="bundle"></a>

## Agent 拿到什麼

一個 frame，一個 zip。有可依循的結構、可使用的素材，以及可供比對的圖片。

```text
frame.zip
├── context.md                從這裡開始：給你的 agent 的說明
├── design/
│   ├── nodes.json            排版、文字、樣式與展開後的 instance
│   ├── fonts.json            可見文字用到的字型與節點 ID
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

<a id="accuracy"></a>

### 與 Figma 有多接近？

以 Figma 自己的 JSON、PNG 與 SVG 匯出為基準，在 **826 個正式環境 frame** 與五個 Community 檔案上測量（2026-10-22）：

| 檢查項目 | 結果 |
|---|---|
| 節點結構、名稱、可見性、邊界與文字 | 826 個 frame 中有 771 個完全一致 |
| 原始圖片填色 | 2,608 / 2,608 位元組完全一致 |
| SVG 素材 | 5,014 / 5,197 在像素差異門檻內 |
| Community frame bundle | 306 / 306 沒有缺少素材 |

這是在這些檔案上的結果，不保證每個設計都如此。方法與門檻：[精確度報告](../wiki/accuracy.md)。

<a id="pi"></a>

## Pi extension

```sh
pi install npm:@runkids/figlens
```

安裝說明: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

重新啟動 Pi 或執行 `/reload`，然後用一般的話請它做一個畫面。Pi 會找到 frame，透過自動註冊的 MCP 伺服器讀取，並用你的技術堆疊實作。不需要 node ID，不需要全域 CLI。

- `/figlens plan <request>` 只產生計劃書，不修改應用程式。
- `/figlens yolo <request>` 規劃、實作、檢查，並清理自己的暫存規劃。
- `/figlens preview [description]` 開啟本機 viewer。
- `/figlens review` 核准 agent 提出的可重用元件對應。

![請 Pi 做一個畫面，讓它找到設計、用你的技術堆疊實作，並檢查結果](../assets/pi-workflow.svg)

完整流程、指令與限制：[pi/README.md](../pi/README.md)。

<a id="viewer"></a>

## 不用 Figma 也能瀏覽設計

[![在本機檢視 MCP Apps 設計，包含圖層樹、排版與元件屬性](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # 本機 viewer，按 Ctrl+C 停止
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # 可寄給任何人的離線 HTML 快照
```

可瀏覽頁面與圖層、搜尋、量測，檢視排版、字體、variables 與 CSS，匯出任何圖層，複製實作 prompt，或下載所選項目的 bundle。拖入你實作成果的截圖，即可疊在設計上比對。詳情：[預覽指南](../wiki/preview.md)。

<a id="mcp-and-skill"></a>

## MCP 伺服器與 agent skill

**MCP。** 伺服器透過 stdio 在本機執行，提供 `workflow`、`tree`、`find`、`inspect`、`nodes`、`variables`、`bundle`、`verify` 與 `export`。請傳入絕對路徑。加上 `--root /path/to/designs`（可重複）可將所有檔案與輸出路徑限制在指定的資料夾。點陣圖匯出會縮小到 `maxSize`（預設 1600 px），讓內嵌圖片維持小巧；SVG 只回傳 metadata。

agent 可透過 `workflow` 讀取內建的 `build`、`audit`、`images`、`plan`、`planning-details`，或明確要求時的 `yolo` 指引。它們與 portable skill 共用來源；MCP 使用者可自行選擇是否另裝 skill。實際工作仍由 agent 使用專案與瀏覽器工具，並遵守 host 權限。讀取指引不會執行工作流程；使用方式取決於客戶端支援與模型行為。

在 Windows 原生環境使用 Claude Code：請依照其 [stdio 設定說明](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server)，並為 `npx` 加上 `cmd /c` 包裝。

**Skill。** [figlens skill](../skills/figlens/SKILL.md) 教 agent 找出 frame、分小段讀取 bundle、規劃範圍明確的工作，並對照設計驗證執行中的 UI。它只安裝說明；仍需有可用的 CLI 或 MCP 伺服器。可用 [Skills CLI](https://github.com/vercel-labs/skills) 安裝，或將 `skills/figlens/` 複製到 agent 的 skills 目錄（Claude Code：`~/.claude/skills/`）：

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

**安裝說明。** npm launcher 在首次使用時下載對應的原生執行檔，以發行版的 SHA-256 checksum 驗證後快取。用安裝腳本裝的以 `figlens update` 更新；Homebrew 裝的以 `brew upgrade figlens` 更新。執行檔涵蓋 macOS（Apple silicon、Intel）、Linux（arm64、x86_64、靜態 musl）與 Windows（x86_64）。

<a id="commands"></a>

## CLI 參考

| 指令 | 輸出 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 節點 id、REST 類型（與 `nodes` 相同）與名稱；縮排文字，或附上每個節點尺寸、隱藏狀態與 auto layout 模式的巢狀 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 不分大小寫搜尋圖層名稱與文字；路徑、尺寸與預覽。預設 20 筆結果（1-200） |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID]` | 精簡的展開後排版/文字摘要，最多 200 個節點。`--images` 會加上點陣圖尺寸與裁切範圍 |
| `nodes <file> <node>` | REST nodes 回應中單一節點的物件（`document`、`components`、`componentSets`、`styles`）：展開後的 instance 與 override、auto-layout、文字樣式、填色、效果、綁定的 variables、元件屬性、渲染邊界 |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | 一次解碼讀取多個節點：清單中每個 id 一行 JSON（帶 `nodeId` 的 `nodes` 輸出，或 `{nodeId, error}`） |
| `variables <file> [node]` | frame 綁定的 variables；未指定節點時，列出所有本機 variables |
| `bundle <file> <node> <out.zip> [--states]` | 單一 frame 的設計 bundle；輸出 `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | 一次解碼，平行產生多個 bundle |
| `verify <bundle>` | 依 manifest 檢查 bundle（`.zip` 或解開後的目錄）：已驗證、缺少與被改動的檔案、缺少的素材，需在專案中確認的字型，以及 `findings`：每項缺少、不支援或未驗證的 handoff 輸入，附節點、證據與下一步 |
| `export <file> <node> <out> [--bounds layout] [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG、PNG、JPG、WebP 或 AVIF；`--max-kb` 會降低有損品質直到符合大小；`--bounds layout` 以 layout box 而非 render bounds 匯出 |
| `export <file> --frames <list> --out-dir <dir> [--format F] [--scale N] ...`, `--frames -` (stdin) | 一次解碼匯出多個節點（預設 `png`）：寫入 `<dir>/<nodeId>.<format>`，每個 id 一行 JSON（帶 `nodeId` 的 `export` 輸出，或 `{nodeId, error}`） |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 本機唯讀 viewer；`--out` 會寫出獨立的離線快照 |
| `compare <file> <nodes.json> <node>` | 與同一 frame 的 REST 匯出做差異比對 |
| `node <file> <node>` | 單一原始 `.fig` 節點 |
| `mcp` | 透過 stdio 的本機 MCP 伺服器 |
| `update` | 將執行檔換成最新發行版 |

需要相關元件 variants 時，使用 `inspect --states` 或 `bundle --states`（MCP 傳入 `states: true`，預設 false；不需修改設定）。證據保留來源、未知與截斷標記。Bundle 會新增 `design/states.json` 與 `context.md` 摘要。範圍及限制請見[狀態證據](../wiki/states.md)。

比較同一 component set 的兩個 variants：`inspect <file> <variantA> --compare-variant <variantB>`（MCP `compareVariant`）。`variantComparison` 列出原始欄位及值集合差異，保留來源與未知標記；子節點不猜配對。詳見[比較限制](../wiki/states.md#compare-two-variants)。

node 引數可接受 `12:345`、`12-345`，或帶有 `?node-id=` 的 Figma URL。`<file>` 也可以是 `gs://bucket/object`，會用你的 `gcloud` 登入下載並快取。固定 object 交接、所需存取權限與更新限制，請參閱 [GCS 快照操作說明](../wiki/gcs.md)。每次 CLI 呼叫都會解碼檔案；MCP 只重用最後讀取且未變更的本機檔案，遠端輸入仍會重新解析。執行 `figlens --help` 或 `figlens inspect --help`（也可用 `-h`）查看完整 CLI 用法。

`inspect` 包含已記錄的文字尺寸調整、截斷、最大行數與裝飾。`handoffDiagnostics` 會列出已儲存但尚未輸出的語意、數量、範例及[涵蓋限制](../wiki/semantics.md#known-handoff-gaps)。輸出會保護來源，完整寫入後才替換目標。MCP 請求行上限為 1 MiB，完整回應上限為 8 MiB，超限會明確回報錯誤；CLI 仍可輸出完整 JSON。
