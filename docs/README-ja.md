# figlens — ユーザーガイド

[← README](../README-ja.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [スタンドアロン版のインストール](#install)
- [公式 Figma MCP ではなく figlens を使う理由](#why)
- [エージェントが受け取るもの](#bundle)
- [Pi 拡張](#pi)
- [Figma なしでデザインを閲覧する](#viewer)
- [MCP サーバーとエージェント skill](#mcp-and-skill)
- [CLI リファレンス](#commands)

<a id="install"></a>

## スタンドアロン版のインストール

**Node.js がない場合は？** 単体バイナリをインストールし、代わりに `"command": "figlens", "args": ["mcp"]` を使います。

| プラットフォーム | コマンド |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

<a id="why"></a>

## 公式 Figma MCP ではなく figlens を使う理由

デザインをファイルとして持ち運びたいなら **figlens** を使います。リモートエージェント、CI、オフライン作業、エージェント間の引き継ぎ、無制限の繰り返し読み取りです。エクスポート手順なしでエージェントがライブのデザインを見る必要があるなら、**公式 Figma MCP** を使います。

| | figlens | 公式 Figma MCP |
|---|---|---|
| ソース | エクスポートした `.fig` スナップショット | ライブの Figma ファイル |
| 必要なもの | ファイル | Figma へのサインイン。デスクトップサーバーはアプリが必要 |
| レート制限 | なし（ローカル解析） | Figma サービスの制限 |
| リモートエージェント / CI | ファイルをコピーしてバイナリを実行 | サインイン付きのリモートサーバー |
| デザインの変更 | 新しい `.fig` をエクスポート | 自動 |
| 引き継ぎ | `.fig` または frame の bundle を共有 | Figma のアクセス権を共有 |
| 忠実度 | 独自のパーサー、[既知のギャップ](../README-ja.md#limitations)あり | Figma が保守 |

`.fig` の読み取りはオフラインで動作します。figlens のインストールと更新、`gs://` 入力、モデルプロバイダー、プロジェクトの依存関係にはネットワークが必要です。

<a id="bundle"></a>

## エージェントが受け取るもの

1 つの frame を 1 つの zip に。実装の土台となる構造、使う素材、照合用の画像です。

```text
frame.zip
├── context.md                ここから開始：エージェント向けの指示
├── design/
│   ├── nodes.json            レイアウト、テキスト、スタイル、展開済みインスタンス
│   ├── fonts.json            表示テキストが使うフォントフェイスとノード ID
│   └── variables.json        バインドされた variables、コレクション、モード、エイリアス
├── assets/                   画像フィル、SVG、合成済みノードのレンダリング
├── assets.json               デザイン上の参照を素材ファイルに対応付ける
├── references/screen.png     比較用の frame のレンダリング
├── missing-assets.json       生成できなかったもの
└── manifest.json             ファイルの役割、メディアタイプ、サイズ、SHA-256 ハッシュ
```

1 つだけ必要なら、CLI で直接取得できます。

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # 名前またはテキストで frame ID を検索
npx -y @runkids/figlens inspect design.fig 12:345                 # レイアウトとテキストの要約
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST 形式の JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG、PNG、JPG、WebP、AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # 上記の zip
```

SVG エクスポートは元の画像フィルを埋め込み、テキストをグリフにアウトライン化するため、フォントは不要です。MCP 経由では、ラスターエクスポートもインライン画像として返るので、エージェントはファイルを開かずにデザインを確認できます。

<a id="accuracy"></a>

### Figma にどこまで近いか

Figma 自身の JSON、PNG、SVG エクスポートと比較して測定しました。**本番 frame 826 個**と Community ファイル 5 つが対象です（2026-10-22）。

| チェック | 結果 |
|---|---|
| ノード構造、名前、可視性、bounds、テキスト | 826 frame 中 771 で一致 |
| 元の画像フィル | 2,608 / 2,608 がバイト単位で一致 |
| SVG 素材 | 5,197 中 5,014 がピクセル差の閾値内 |
| Community frame の bundle | 306 / 306 で欠落素材なし |

これらのファイルでの結果であり、すべてのデザインを保証するものではありません。手法と閾値：[accuracy レポート](../wiki/accuracy.md)。

<a id="pi"></a>

## Pi 拡張

```sh
pi install npm:@runkids/figlens
```

インストールガイド: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

Pi を再起動するか `/reload` を実行し、普通の言葉で画面を依頼します。Pi が frame を見つけ、自動登録された MCP サーバー経由で読み取り、あなたのスタックで実装します。ノード ID もグローバル CLI も不要です。

- `/figlens plan <request>`は計画のみを作成し、アプリケーションを変更しません。
- `/figlens yolo <request>` は計画、実装、確認を行い、自身の一時的な計画を片付けます。
- `/figlens preview [description]` はローカル viewer を開きます。
- `/figlens review` はエージェントが提案した再利用可能なコンポーネントのマッピングを承認します。

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](../assets/pi-workflow.svg)

詳しい手順、コマンド、制限：[pi/README.md](../pi/README.md)。

<a id="viewer"></a>

## Figma なしでデザインを閲覧する

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # ローカル viewer、Ctrl+C で停止
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # 誰にでも送れるオフライン HTML スナップショット
```

ページとレイヤーの閲覧、検索、計測、レイアウト・タイポグラフィ・variables・CSS の確認、任意のレイヤーのエクスポート、実装プロンプトのコピー、選択範囲の bundle のダウンロードができます。作成した画面のスクリーンショットをドロップすると、デザインに重ねて比較できます。詳細：[preview ガイド](../wiki/preview.md)。

<a id="mcp-and-skill"></a>

## MCP サーバーとエージェント skill

**MCP。** サーバーは stdio 経由でローカル実行され、`workflow`、`tree`、`find`、`search`、`inspect`、`nodes`、`variables`、`bundle`、`verify`、`index`、`jobs`、`export` を公開します。パスは絶対パスで渡します。`--root /path/to/designs`（複数指定可）を追加すると、すべてのファイルパスと出力パスを指定フォルダーに制限できます。figlens の既定ディレクトリ以外にインデックスを保存するには `--storage-root DIR`（複数指定可）が必要です。ラスターエクスポートは `maxSize`（デフォルト 1600 px）に縮小され、インライン画像は小さく保たれます。SVG はメタデータのみを返します。

エージェントは `workflow` で `build`、`audit`、`images`、`plan`、`planning-details`、または明示的に依頼された場合の `yolo` ガイドを取得できます。portable skill と同じソースを使うため、MCP 利用時の skill の別途インストールは任意です。実作業にはエージェント自身のプロジェクト・ブラウザツールとホストの権限を使います。ガイドを読むだけでは処理は実行されません。利用方法はクライアントの対応とモデルの動作に依存します。

Claude Code でネイティブ Windows を使う場合：その [stdio セットアップ手順](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server)と、`npx` 用の `cmd /c` ラッパーに従ってください。

**Skill。** [figlens skill](../skills/figlens/SKILL.md) は、エージェントに frame の検索、bundle の小分けでの読み取り、スコープを絞った作業の計画、動作中の UI とデザインの照合を教えます。インストールされるのは指示のみで、CLI または MCP サーバーが利用可能である必要があります。[Skills CLI](https://github.com/vercel-labs/skills) でインストールするか、`skills/figlens/` をエージェントの skills ディレクトリにコピーします（Claude Code：`~/.claude/skills/`）。

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare。** [skillshare](https://github.com/runkids/skillshare) を使うと、Claude Code、Codex、Cursor 全体の skill と MCP 接続を 1 か所で管理できます。

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**インストールの注意。** npm ランチャーは初回使用時に対応するネイティブバイナリをダウンロードし、リリースの SHA-256 チェックサムで検証してキャッシュします。スクリプトでインストールした場合は `figlens update`、Homebrew の場合は `brew upgrade figlens` で更新します。バイナリは macOS（Apple silicon、Intel）、Linux（arm64、x86_64、静的 musl）、Windows（x86_64）に対応しています。

<a id="local-index"></a>

## 通常の検索と任意のローカルインデックス

|  | 通常（デフォルト） | 上級：ローカルインデックス（任意） |
|---|---|---|
| できること | 呼び出しごとに `.fig` を読み、`search`、`find`、`inspect`、`export` に答えます。 | 選んだ範囲の検索インデックスを一度作り、以後の `search` はそこから、ソース横断でランク付けして答えます。 |
| 書き込まれるもの | workspace もインデックス用データベースも作りません（`export` は指定したファイルだけを書き込みます）。 | ユーザーごとの state ディレクトリ、または選んだ `--storage` の下にあるローカル workspace（SQLite データベース）。 |
| 使う場面 | 単発の質問、スクリプト、CI。 | 同じファイルを繰り返し検索する場合、プロジェクトの複数ファイルを一つのコーパスとして検索する場合（`--workspace <dir>`）、coverage で既知の親/インスタンス関係を使いたい場合。 |

通常のデザインに関する質問でインデックスが作られることはありません。明示的に一度だけ有効にします（「このデザインのインデックスを作成して」）。保存先は figlens が選び、SQLite などの設定は不要です。

```sh
# 通常：ステートレスで何も作らない
figlens search design.fig "checkout button"

# 上級、一度だけ：範囲（ノード ID、またはファイル全体なら 0:0）を指定。ビルドはフォアグラウンドで実行
figlens index prepare design.fig --scope 0:0

# インデックス検索。インデックス範囲がファイル全体でない場合は --node を付ける
figlens search --file design.fig "checkout button" --node 0:0

# .fig を編集・再エクスポートしたら、バイトが変わった範囲だけ再構築
figlens index refresh --file design.fig

# .fig を移動したら：workspace を維持して refresh（prepare が出力した workspaceId を使う）
figlens index relocate <sourceId> new/design.fig --id <workspaceId>
figlens index refresh --id <workspaceId>
```

MCP：`index` の prepare は job を返します。`succeeded` になるまで `jobs` を確認し、返信の `workspaceId` で検索します。

```json
{"tool": "index", "arguments": {"action": "prepare", "file": "/abs/path/design.fig", "scope": "0:0"}}
{"tool": "jobs", "arguments": {"action": "status", "jobId": "<jobId from the reply>"}}
{"tool": "search", "arguments": {"workspaceId": "<workspaceId from the reply>", "node": "0:0", "query": "checkout button"}}
```

ライフサイクル：`refresh` は `.fig` のバイトが変わった範囲だけを再構築し、`rebuild` は変更がなくても再構築します。`clear_cache` はビルドで再作成できるデータだけを削除し、レジストリと公開済みインデックスは残します。`reset` はこの workspace のデータベースを削除し（`.fig` やエクスポートは対象外）、明示的に求めたときだけ実行されます。新しいソース、範囲の拡大、reset、再作成は、それぞれ個別の明示的な依頼が必要です。

`.fig` を移動した場合：`index relocate` は `workspaceId`、`sourceId`、保存先を維持します。その後、影響を受けた範囲を `refresh` します。自動検出やハッシュによる自動統合はなく、reset は不要です。

データ：SQLite データベースには、登録されたローカルパス、選択した範囲のレイヤー名と実効テキスト、検索 postings、既知の親/インスタンス関係、ハッシュ、世代、job が入ります。元の `.fig` はその場所で変更されず、その生バイトはデータベースに保存されません。

Pi、OpenCode、OMP は、それぞれのオプトイン用エントリから同じインデックスを使い、workspace は再起動やホストをまたいで再利用されます。未実装：ファイル監視、usages/関係ツール、ベクトル、イベント、サブスクリプション。

<a id="commands"></a>

## CLI リファレンス

| コマンド | 出力 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | ノード ID、REST タイプ（`nodes` と同じ）、名前。インデント付きテキスト、または各ノードのサイズ、非表示状態、オートレイアウトのモードを含むネストした JSON。ページやセクション直下のノードには `firstText`（最初の表示テキスト）も付く |
| `tree <file> [node] --top-frames [--json]` | 画面だけを 1 行に 1 つ、`--frames -` にそのまま渡せる形で出力。ページまたはそのセクション直下のフレーム、コンポーネント、コンポーネントセット、インスタンス（グループと内部ページは除外）。`--json` はページとセクションの ID と、キャンバスのラベルと画面を見分けるための各画面の最初の表示テキスト（`firstText`）を追加 |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | レイヤー名とテキストの大文字小文字を区別しない検索。パス、サイズ、プレビュー。デフォルト 20 件（1-200） |
| `search <file> <query> [--node ID] [--type T[,T]] [--limit N] [--cursor C] [--include-hidden] [--include-internal] [--mode text]` | インスタンスのオーバーライド、テキスト/ブーリアンプロパティ、スワップを適用したレイヤー名と全文のランク付き検索。正確なノード参照（`inspect` で使える `I…;…` サブレイヤー ID を含む）、一致理由、UTF-16 範囲、カバレッジ、カーソルによるページ送り。デフォルト 20 件（1-200）。非表示レイヤーと内部ページはそれぞれのフラグが必要。`<file>` の代わりに `--file`/`--workspace`/`--id` を使うと、workspace の準備済みインデックスを読みます（`index` を参照） |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID] [--flat]` | 展開済みレイアウト/テキストのコンパクトな要約（最大 200 ノード）。`--images` でビットマップの寸法とクロップ範囲を追加。`--flat` は代わりに 1 ノード 1 行で出力（ノード基準の位置、サイズ、テキスト、フォント、塗り、線、角丸、auto-layout） |
| `nodes <file> <node>` | REST nodes レスポンスのノードごとのオブジェクト（`document`、`components`、`componentSets`、`styles`）：展開済みインスタンスとオーバーライド、auto-layout、テキストスタイル、フィル、エフェクト、バインドされた variables、コンポーネントプロパティ、レンダリング bounds |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | 1 回のデコードで複数ノード：リストの id ごとに 1 行の JSON（`nodeId` 付きの `nodes` 出力、または `{nodeId, error}`） |
| `variables <file> [node]` | frame にバインドされた variables。node を省略すると、すべてのローカル variables |
| `bundle <file> <node> <out.zip> [--states]` | 1 つの frame の design bundle。`{nodeId, path, warnings}` を出力 |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]`, `--frames -` (stdin) | 1 回のデコードから複数の bundle を並列生成 |
| `verify <bundle>` | bundle（`.zip` または展開したディレクトリ）を manifest と照合：検証済み・欠落・変更されたファイル、欠落アセット、プロジェクトで確認すべきフォント、および `findings`：欠落・未対応・未検証の handoff 入力ごとのノード、根拠、次のアクション |
| `export <file> <node> <out> [--bounds layout] [--scale N] [--quality 1-100] [--max-kb N] [--max-size N] [--match-dir DIR]` | SVG、PNG、JPG、WebP、AVIF。`--max-kb` は収まるまで非可逆の品質を下げる。`--bounds layout` は render bounds ではなく layout box で書き出す。`--match-dir DIR`（SVG のみ、サブフォルダ含む）は同じアウトラインを描く SVG（`matches`）と、スコア付きの類似形状を最大 10 件（`similar`）列挙 |
| `export <file> --frames <list> --out-dir <dir> [--format F] [--scale N] ...`, `--frames -` (stdin) | 1 回のデコードで複数を書き出し（既定は `png`）：`<dir>/<nodeId>.<format>` に書き、id ごとに 1 行の JSON（`nodeId` 付きの `export` 出力、または `{nodeId, error}`） |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | ローカルの読み取り専用 viewer。`--out` でスタンドアロンのオフラインスナップショットを書き出し |
| `compare <file> <nodes.json> <node>` | 同じ frame の REST エクスポートとの差分 |
| `node <file> <node>` | `.fig` の生ノード 1 つ |
| `index prepare\|refresh\|rebuild\|clear\|status\|add\|remove\|relocate\|reset` | ユーザーごとの figlens state ディレクトリ、または workspace が以後記憶する `--storage DIR` の下にあるローカル workspace。`prepare <file> --scope NODE` でその範囲の検索インデックスを一度だけ作成し、以後 `search --file <file>`（または `--workspace DIR`）は再解析せず、workspace の全ソースをまとめてランク付けして答えます。`refresh` は変更されたファイル、`rebuild` はすべて（または `--source`/`--scope` の一つ）を再作成し、`clear` は使われていないインデックスを削除します。新しいインデックスができるまで以前のものが使われます。`reset` は workspace データベースだけを削除し、デザイン、エクスポート、記憶された場所には触れません |
| `jobs status\|cancel <jobId>` | インデックス作成の状態表示または取り消し。`cancel` は作成が止まってから戻ります |
| `mcp` | stdio 経由のローカル MCP サーバー |
| `update` | バイナリを最新リリースに置き換え |

関連するコンポーネント variants には `inspect --states` または `bundle --states` を使用します（MCP は `states: true`、既定は false、設定変更は不要）。証拠にはソース、不明点、省略のマーカーが残ります。Bundle には `design/states.json` と `context.md` の要約が追加されます。範囲と制限は[状態の証拠](../wiki/states.md)を参照してください。

同じ component set の 2 つの variants は `inspect <file> <variantA> --compare-variant <variantB>`（MCP `compareVariant`）で比較できます。`variantComparison` は保存フィールドと値集合の差、ソース ID、不明点を返します。子ノードの対応関係は推測しません。[比較の制限](../wiki/states.md#compare-two-variants)を参照してください。

node 引数には `12:345`、`12-345`、または `?node-id=` 付きの Figma URL を指定できます。`<file>` には `gs://bucket/object` も指定でき、`gcloud` ログインでダウンロードしてキャッシュします。固定オブジェクトでの受け渡し、必要なアクセス権、更新の制限は [GCS スナップショットガイド](../wiki/gcs.md)を参照してください。CLI は呼び出しごとにファイルをデコードします。MCP が再利用するのは最後に読み取った変更のないローカルファイルだけです。リモート入力は毎回解析し直します。CLI の全使用法は `figlens --help` または `figlens inspect --help`（`-h` も可）で確認できます。

`inspect` は記録済みのテキストのリサイズ、切り詰め、最大行数、装飾を含みます。auto-layout のない親には `childSpacing: "derived"` が付きます。子要素の間隔は座標の差で、auto-layout のルールではありません。`constraints` で固定されている場合もあります。`handoffDiagnostics` は保存済みでも未出力の意味情報を件数、例、[対象範囲の制限](../wiki/semantics.md#known-handoff-gaps)とともに示します。出力は元ファイルを保護し、書き込み完了後に保存先を置き換えます。MCP のリクエスト行は 1 MiB、完全な応答は 8 MiB が上限で、超過時は明示的なエラーを返します。CLI では引き続き完全な JSON を出力できます。
