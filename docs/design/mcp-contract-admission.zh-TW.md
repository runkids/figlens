# MCP 契約與請求准入設計

狀態：已實作。Docker Rust、stdio 與 POSIX 撤銷讀取權限檢查通過；發版前仍須完成實際 Pi 註冊路徑／封裝 launcher 與 Windows 驗收。下方原始碼依據保留實作前的調查紀錄。

[英文原稿](mcp-contract-admission.md)

## 決策與範圍

依序深化兩個既有的 module：

1. **工具契約 module**：負責 MCP 參數的解讀。
2. **請求准入 module**：負責載入設計檔前，必須完成的判定。

使用者選擇兩個方向，並明確接受 MCP 的不相容變更，因為不需要保留 legacy 相容性。因此直接移除 legacy 轉譯，不新增相容用的 adapter、模式開關或遷移機制。CLI 行為維持不變。本文件記錄已認可的設計；設計認可與實作驗證證據分開判定。

目標是提高 locality（知識與修改集中在同一處）與 leverage（呼叫端透過較小的 interface 取得更多能力），不是宣稱效能提升，也不是為了縮短檔案。本文的領域用語沿用既有程式：tool、.fig、design、document、Tree、export 與 bundle。不引入新的領域概念，也不需要新增 GLOSSARY 詞條。

## 現況依據

檢視的原始碼版本：`bb45fed`。

| 位置 | 目前的摩擦 |
| --- | --- |
| `src/main.rs::TOOLS` 與 `src/contract.rs::property` | 參數宣告與工具探索所需的 metadata 分散在不同位置。 |
| `src/contract.rs::arguments` | 驗證與正規化參數，同時判定 legacy 輸入，但省略參數的預設值留到執行階段處理。 |
| `src/command.rs::Command::from_tool` | 再次定義工具探索已宣告的 MCP 預設值。 |
| `src/main.rs::call_tool` | 為 legacy 呼叫重建 CLI 參數，另行補上 export 預設值。 |
| `src/main.rs::refused`、`direct_file`、`is_direct`、`mcp_reply_with` | 在授權、預載與執行前，反覆解析或分類同一份請求。 |
| `src/main.rs::serve` | 在借用 Tree 的重用迴圈內外，都必須理解請求准入規則。 |

這些是架構上的摩擦，不是已證明會產生錯誤輸出的 bug。現有測試已涵蓋許多必須保留的行為。

## 1. 深化工具契約 module

### 責任與 interface

以 `src/contract.rs` 為 module 的核心。它的 interface 提供工具探索，以及具名工具呼叫的準備作業；統一負責參數定義、驗證、正規化與 MCP 預設值。工具說明可維持靜態資料，不建立通用 schema 解譯器。

準備作業回傳結構化的 `Failure`，或一份擁有自身資料、準備完成的工具呼叫。這份值包含工具身分與正規化後的參數。欄位與未經驗證的建構函式必須限制在契約 module 內，只提供不可變、用途明確的存取方式；不得暴露可修改的 JSON、setter、`DerefMut`，也不得透過 `Default` 或反序列化繞過驗證建立它。驗證後，呼叫端不能再修改工具身分、預設值或參數。它提供准入判定所需的來源與輸出資訊，呼叫端不必重新驗證原始 JSON。

內部優先沿用既有 JSON 表示方式，除非實作時有具體證據需要其他表示法。不為每個工具各建一個參數 struct，不新增 registry 抽象或可設定的驗證框架。這份準備完成的值之所以值得存在，是因為它保證資料已符合條件，而不是單純包裝任意 JSON。

`Command::from_tool` 必須要求這個封裝的準備完成呼叫型別，不能接受工具名稱搭配裸 `Value`，也不能立刻拆開 wrapper 後交回原本未驗證的 interface。MCP 的來源與輸出作業，也必須使用下文定義的已准入路徑。它保留必要的 Tree 相關檢查，例如目標是否存在、是否為支援的匯出根節點，但不再自行選擇 MCP 預設值。對於可省略的值，保留「未提供」的語意；在準備完成的表示方式中明確表達「不限深度」，不要讓多個呼叫端各自決定它的意思。驗收包含可見性與 interface 檢查：外部呼叫端不能自行建立或修改準備完成的呼叫，也不能用裸 JSON 繞過它。

seam 位於原始 MCP 參數與準備完成的呼叫之間。CLI adapter 繼續使用 `Command::from_cli`；兩個 adapter 繼續透過既有指令執行 module 取得 leverage。不要強迫 CLI 的解析規則或預設值經過 MCP 契約。

### 必須保留的預設值與行為

| 參數 | 未提供時的值 |
| --- | --- |
| `tree.node`、`find.node` | `0:0` |
| `tree.depth` | 不限深度 |
| `tree.json`、`inspect.images` | `false` |
| `inspect.depth` | `1` |
| `find.limit` | `20` |
| `export.scale` | `1` |
| `export.quality` | `80` |
| `export.maxKb` | 不設位元組數限制 |
| `export.maxSize` | `1600`；明確提供 `0` 時停用邊長上限 |
| `variables.node` 與其他可省略字串 | 維持未提供的狀態 |

工具探索與呼叫準備必須共用同一份預設值決策。保留目前的數值範圍、數學上為整數的 JSON 數字正規化（`2.0` 合法）、副檔名處理、不分大小寫的搜尋類型，以及成功輸出的結構。設計工具繼續忽略未知欄位；`workflow` 繼續拒絕額外欄位。嚴格限制參數型別，不代表必須在 Tree 建立前就把所有字串驗證為有效的節點識別碼。

### 刻意接受的不相容變更

| 原本的輸入／行為 | 新行為 |
| --- | --- |
| 數字字串，例如 `depth: "0"` | 回傳 `invalid_arguments`；應傳 JSON 數字。 |
| 縮放字串，例如 `scale: "2x"` | 回傳 `invalid_arguments`；應傳 JSON 數字。 |
| 可省略參數傳入 `null`，或非布林參數傳入 `false` | 回傳 `invalid_arguments`；只有省略後的預設值符合原意時，才直接省略。 |
| `export.maxSize: null` 或 `export.maxSize: false`（原本表示無上限） | 改用 `export.maxSize: 0`；省略表示邊長上限 1600，不是無上限。 |
| 把看起來像旗標的節點字串解讀為 CLI 選項 | 不再視為 CLI 選項；依一般節點字串流程處理並檢查目標。 |
| MCP 呼叫轉入 `Command::from_cli` 或 `run` | 移除。 |
| `_meta.figlens.argumentMode: "legacy"` | 移除，不以其他模式標記取代。 |

布林參數仍可使用 `false`。CLI 的字串與旗標語法維持支援。在 `--root` 下，原本可進入 legacy 路徑、如今不合法的呼叫，會在路徑准入判定前回傳 `invalid_arguments`，不再回傳原本 legacy 專用的 `permission_denied`。

### Breaking change 的發布說明

實作發布時，release notes 必須提供保留原意的替代寫法：`depth: "0"` 改為 `depth: 0`；`scale: "2x"` 改為 `scale: 2`；原本無上限的 `maxSize: null` 或 `maxSize: false` 改為 `maxSize: 0`。只有要使用 1600 的預設值時才省略 `maxSize`。其他以特殊值表示省略的參數，也必須依原有語意提供替代方式，不能一律叫使用者省略。另須說明 legacy 模式 metadata 的移除，以及受限 root 下的錯誤差異。不需要相容 adapter，但仍需要發布說明。

### Deletion test：刪掉什麼才真的簡化？

刪除 legacy 分支、legacy 分類，以及重複的 MCP 預設值決策。不要刪除共用的指令 module，否則其行為只會重新散落到 CLI 與 MCP adapter。若只是把函式搬到另一個檔案，沒有減少呼叫端必須知道的事，就沒有增加 depth。

## 2. 深化請求准入 module

### 責任與 interface

維持一個 MCP 請求准入 module，初期可留在 `src/main.rs`。只有在能讓責任更完整集中時，才抽到 `src/mcp.rs`，不要只是因為檔案很大就拆。不要新增 trait 或第二個傳輸 adapter：只有一個 stdio adapter，還不足以證明需要真正的傳輸 seam。

概念上的 interface 是 `admit(line, roots)`，有三種結果：

- **Ignore**：請求封套有效且缺少 `id` 的訊息，即使方法未知或工具參數不合法；不回覆，也不執行設計檔或輸出相關作業。
- **Reply**：完整的協定／控制回覆，或工具拒絕結果；不載入設計檔。
- **Call**：擁有自身資料、準備完成的工具呼叫，且其來源與輸出已通過設定的 root 規則。

這些名稱用來說明設計，不代表已決定確切的 Rust 宣告。不需要設計檔的呼叫，尤其是 `workflow`，仍是合法呼叫，但不提供預載路徑。interface 對 `serve` 隱藏 JSON 解析、請求封套檢查、notification 處理、工具呼叫準備與路徑准入細節。

准入階段不讀取設計檔內容，也不啟動遠端下載，但可為了執行既有的 canonical path 規則而檢查檔案系統 metadata。已准入的呼叫擁有不可變的準備完成呼叫，以及作為唯一依據的來源／輸出結果；其欄位為私有，准入之外不能以未檢查的方式建立。來源載入、本機預載、快取選擇、export 與 bundle 寫入，都必須使用這些保存的結果，不得再從原始或正規化後的 JSON 各自推導路徑。

本機來源或已存在的輸出，保存 root 判定所使用的解析後路徑；尚未存在的輸出，保存已核准的解析後父目錄與檔名。路徑檢查必須回傳可直接使用的結果，不能只回傳布林值，再由執行端另選路徑寫法。未設定 roots 時也採用同一套解讀。遠端來源保存其身分，交給既有的逐次呼叫 resolver；每次呼叫只解析一次，再把本機結果傳下去，不能由預載與執行各自解析。遠端呼叫仍不重用已解析的本機 Tree。原始輸入可以保留作為診斷標籤，但不是執行路徑。必要的檔案系統複查針對保存的結果進行；這不代表已消除競態。

### 處理順序

```text
收到一行請求
  -> 解析 JSON 並驗證請求封套
  -> 在方法／工具驗證前，忽略封套有效且缺少 id 的訊息
  -> 回覆控制方法／拒絕不合法的方法結構
  -> 透過契約 module 準備具名工具的參數
  -> 檢查來源與輸出是否符合設定的 roots
  -> 執行已准入的呼叫
       本機設計檔：重用未變更的 Tree，或重新載入
       遠端設計檔：每次呼叫各自解析來源並載入
       workflow：讀取內嵌指引，不需要設計檔
  -> 依既有回覆結構序列化
```

對有 `id` 的訊息，`tools/call` 缺少工具名稱，或名稱不是字串，仍屬協定錯誤。未知工具與不合法參數仍屬工具失敗。先準備參數，再檢查 roots，最後才能載入設計檔與建立輸出。不要載入設計檔後，才發現參數或輸出路徑不合法。

保留既有協定錯誤碼（`-32700`、`-32600`、`-32601`、`-32602`）與結構化的工具失敗分類。繼續接受省略 `jsonrpc` 的請求封套；移除參數相容性，不代表可以順便修改其他封套行為。缺少 `id` 表示 notification；明確提供 `id: null` 時，維持目前的請求處理方式。

### Notification 回歸測試表

已核對 `src/main.rs::valid_request` 與 `mcp_reply_with` 的現行順序：先驗證封套，再執行 `req.get("id")?`，之後才分派方法與準備工具呼叫。因此這裡的「合法 notification」指封套有效，不要求方法已知或工具呼叫有效。下列案例必須透過 `serve` 測試，並涵蓋有／無設定 roots：

| 輸入 | 預期的可觀察結果 |
| --- | --- |
| 無法解析的 JSON | 回覆 `-32700`、`id: null`；不處理設計檔或輸出。 |
| 不合法封套，包括缺少／非字串 method、不合法 `jsonrpc` 或布林 `id` | 即使缺少 `id`，仍回覆 `-32600`、`id: null`；不處理設計檔或輸出。 |
| 封套有效、缺少 `id`、方法未知 | 忽略；不回覆、不準備工具呼叫、不查路徑、不讀來源、不建立輸出。 |
| 封套有效、缺少 `id`、`tools/call` 缺少名稱、名稱非字串或工具未知 | 忽略，且同樣不進行上述作業。 |
| 封套有效、缺少 `id`、`tools/call` 參數不合法或合法 | 忽略，且同樣不進行上述作業；即使工具有效也不執行。 |
| `id: null`、方法未知 | 回覆 `-32601`，`id` 維持 `null`。 |
| `id: null`、`tools/call` 缺少名稱或名稱非字串 | 回覆 `-32602`，`id` 維持 `null`。 |
| `id: null`、未知工具或不合法工具參數 | 回覆包含 `invalid_arguments` 的工具結果，`id` 維持 `null`；不處理設計檔或輸出。 |
| `id: null`、合法 `ping` 或工具呼叫 | 依一般請求執行並回覆，`id` 維持 `null`；工具仍適用一般來源／root 規則。 |

訊息序列最後接一個有識別碼的 ping，證明被忽略的訊息沒有多餘回覆，且後續處理繼續正常。除了沒有回覆，也要觀察沒有檔案系統／雲端／輸出作業。另納入省略 `jsonrpc` 的案例，維持既有封套接受規則。

### Ownership 與快取限制

保留目前 `Document`、解碼後 message 與借用的 `Tree` 在同一詞法作用域內的 ownership 關係。已准入的呼叫擁有自身資料，需要另一份本機設計檔時，可跨快取迴圈攜帶。攜帶這份值，而不是保留原始行、再解析與准入一次。

維持依路徑、修改時間與檔案長度重用本機 Tree 的行為。控制呼叫、遭拒的呼叫、workflow 與遠端呼叫，不應無故淘汰有效的本機 Tree。本機來源變更後必須重新載入；來源無法讀取時，應回傳可恢復的單次呼叫失敗，而不是終止後續請求。

「只准入一次」限制的是對請求的解讀次數，不是對可變檔案系統的觀察次數。使用快取前，仍須複查修改 metadata 與來源目前是否可讀；I/O 前也必須針對保存的已准入路徑，保留必要的路徑／安全檢查。准入不是永久授權或可讀性保證。測試必須涵蓋暖快取後來源變更、刪除、不可讀，metadata／讀取失敗，以及相關 symlink／輸出父目錄變更；斷言應拒絕或安全失敗，不得回傳過時的成功結果，且之後的合法請求仍能恢復。可讀性測試必須有測試程序實際權限下的可觀察證據，不能只假設 permission bits 一定生效。

不引入自我參照 ownership、全域快取、遠端解析後 Tree 快取、generation pinning 或非同步執行。保留 panic 復原與有限的 parsing budgets。減少重複解析是 locality 改善的結果，不是未經測量的加速保證。

root 檢查保留 canonical path 與 dangling symlink 的處理。在 `--root` 下，遠端 `gs://` 來源仍須在存取雲端前遭拒。本設計不會消除檔案系統競態，也不保證授權檢查不存在競態。

### Deletion test：不要在舊路徑上再包一層

取代 `refused`、`direct_file` 與 `is_direct` 重疊的責任，不要新增 wrapper 後，仍讓它們構成第二條准入路徑。執行階段不得再次準備參數。必要的授權檢查必須保留，不能為了簡化流程而刪掉。

## 實作順序

1. **工具契約**：集中參數與預設值知識，讓準備完成的呼叫保證已通過驗證，移除 legacy 轉譯，更新使用處與測試。保留共用的 Tree 相關檢查與指令執行邏輯。此階段完成時，既有 server 仍可能對同一份輸入重複準備。
2. **請求准入**：讓協定處理與 root 准入只使用一次準備完成的呼叫，將它帶入 `serve`，刪除多餘的原始請求分類。維持借用 Tree 的 ownership 關係。

每一步都必須通過相關測試，才能進入下一步。不要混入解碼、繪圖或快取重新設計。

## 呼叫端核對

在 `dd33bd2` 的靜態檢查中，`pi/index.ts` 透過 `registerMcpServer` 直接向 host 註冊 MCP，不自行組裝設計工具的 MCP 參數；其 mapping workflow 透過 `pi.exec` 呼叫 CLI。`pi/workflow.mjs` 傳入 `--limit 200` 等 CLI 字串，`pi/viewer.mjs` 則啟動 CLI 的 `preview` 指令；這些不是 legacy MCP 轉型，不能改成 JSON 數字。檢查時，對應的 `npm/pi/` 檔案內容完全相同。`npm/bin/figlens.js` 轉送 argv 與繼承的 stdio，不改寫 MCP 參數。以上已檢查路徑中，未發現對已移除 MCP 格式的依賴。

這不等於已驗證 host／模型的相容性。發布前，仍須透過 Pi 註冊的 MCP 路徑及打包後的 launcher，實際執行嚴格呼叫／拒絕／恢復序列，並核對共用 skill／範例指引是否仍教導舊參數寫法。`scripts/mcp-smoke.py` 已改為檢查嚴格拒絕與恢復，不再依賴 legacy 成功。不能只靠原始碼檢查就宣稱 Pi 相容性已驗證。

## 使用處與驗證

interface 就是測試面。優先測試可觀察的工具回覆與有順序的 `serve` 訊息序列，不以私有分類判斷函式的測試取代它們。更新過時的相容性斷言，保留其他回歸測試。產生測試檔案時，沿用既有的原子性目錄 ownership 與作用域清理 helper。

| 範圍 | 必要檢查 |
| --- | --- |
| `src/command/tests/contract.rs`、`src/command/tests.rs` | 工具探索與執行預設值一致；省略與明確提供值的差異；拒絕舊有轉型輸入；布林 `false` 與 `2.0` 合法；指令結果與 CLI 行為不變。 |
| `src/mcp.rs` 與 `src/mcp/admission_tests.rs` | notification 不回覆、不處理設計檔；workflow 不需要設計檔；協定錯誤與工具錯誤分離；作業前拒絕不合法來源／輸出；symlink；受限的遠端來源；不合法或不可讀呼叫後仍可恢復。 |
| `serve` 訊息序列 | 重複本機呼叫、檔案變更、切換本機檔案、交錯控制／workflow／遠端呼叫，以及失敗後不影響後續請求。透過既有觀察點或小型內部測試 hook 驗證重用語意，不新增公開快取 interface。 |
| `scripts/mcp-smoke.py` | 將 legacy 成功與 `legacyCompatibility` 回報改為嚴格拒絕與恢復斷言；保留成功呼叫及 stdout 只包含 JSON-RPC 的檢查。透過既有候選版本安裝矩陣執行。 |
| `scripts/check-gcs-snapshot.py` | 保留遠端 metadata／下載行為與受限 root 下的拒絕；不新增遠端快取能力宣稱。 |
| `npm/`、Pi extension 與既有安裝檢查 | 檢查產生或傳遞的參數是否依賴已移除的轉型；只修改確實依賴它們的呼叫端。相關檔案變更時執行套件檢查。 |
| `wiki/mcp.md`、`wiki/gcs.md`、使用者說明與範例、release notes | 實作時同步更新被移除的 legacy 行為、metadata、錯誤差異與更名後的執行路徑。提供保留原意的替代寫法，特別是無上限的 `maxSize: 0`。在實作前不要把提案寫成已發布行為。 |

實作時的驗證指令：

```sh
docker compose run --rm dev cargo test
docker compose run --rm dev sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings'
python3 scripts/ai-context.py check
```

子程序檢查沿用文件記載的 Docker／原生候選版本 smoke 流程，不引入主機 Rust toolchain。本設計不需要新增依賴、公開 fixture 或修改輸出的設計資料。繪圖準確度測量不能取代請求契約測試；除非實作修改了繪圖行為，否則不需要進行。

## 驗收條件

- MCP 參數語意與預設值只有一個負責位置，沒有 CLI fallback。
- 呼叫端不能偽造或修改準備完成／已准入的呼叫；`Command::from_tool` 不接受裸 JSON。
- 每份需要處理的請求，在載入設計檔前只解析、準備與准入一次；跨迴圈攜帶的呼叫不重新准入。被忽略的 notification 不準備工具呼叫。
- 執行使用檢查後保存的來源／輸出結果，不再重新解讀原始參數。
- `serve` 使用准入結果，不重複實作權限或參數規則，但保留必要的檔案系統 freshness、可讀性與安全複查。
- 發布前涵蓋 notification 測試表、暖快取失敗／恢復案例、Pi 呼叫端驗證與 release notes 的替代寫法。
- 除了上文明列的不相容變更，CLI 行為、合法 MCP 結果、workflow 探索、可恢復的失敗與本機 Tree 重用均維持不變。
- 實作時同步修改測試與現行行為文件；本提案本身不改變執行行為。
