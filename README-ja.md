<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Figma のデザインを、コーディングエージェントへ。</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  どの画面でも、レイアウト、テキスト、変数、アセット、参照画像をコーディングエージェントに渡せます。<br>
  ノート PC でも、クラウドでも、CI でも、何度でも読めます。<br>
  <strong>作業するマシンに Figma アプリ、ログイン、シートは不要。API のレート制限もありません。</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">デモを見る</a> · <a href="#quick-start">デザインを試す</a> · <a href="#for-ai-agents">エージェントを接続</a> · <a href="#pi-extension">Pi で使う</a> · <a href="#how-close-is-it-to-figma">精度</a>
</p>

<a id="why-figlens"></a>

## figlens を使う理由

「この画面を作って」と頼むなら、画面のレイアウト、テキスト、アセットも渡したいものです。スクリーンショットでは、エージェントは余白や文字組みを推測するしかなく、アイコンは相変わらず手で書き出すことになります。Figma の MCP サーバーは本物のデザインを読めますが、クォータはファイルを所有するチームに紐づき、エージェントが一つのプロンプトで一日分を使い切ることもあり、マシンには Figma のログインが必要です。

figlens は代わりに保存したコピーからデザインを読み取ります。Figma で **File > Save local copy** を選びます（所有者がコピーを無効にしていなければ、閲覧権限で十分です）。その `.fig` を CLI またはローカル MCP サーバー経由でエージェントに渡せば、画面の検索、レイアウトやテキストの確認、アセットの抽出ができます。あるいは、フレームの設計データ、アセット、参照レンダリングを一つの zip にまとめて、どのエージェントやメンバーにも渡せます。

| こんな問題があるなら | figlens でできること |
|---|---|
| 有料シートでも「Rate limit exceeded, try again tomorrow」が出る | Figma API の呼び出し、トークン、シートなしで、保存したファイルを必要なだけ読む |
| エージェントがクラウドや CI で動く | Figma アプリやブラウザーセッションなしで、ネイティブ CLI から読む |
| 寸法の説明やアセットの書き出しを繰り返している | 構造化されたレイアウト、文字組み、展開済みインスタンス、バインド変数、元のアセットを渡す |
| 別のエージェントやメンバーにも同じ画面が必要 | 設計データ、アセット、参照レンダリング、説明を含むフレームの bundle を共有する |

デザイナーは引き続き Figma を使い、エージェントはプロジェクトの技術スタックで作業します。figlens がデザインのコンテキストを提供し、コーディングエージェントが実装を書きます。

<details>
<summary>.fig 形式は非公開です。Figma が形式を変えたらどうなりますか？</summary>

figlens は形式を推測しません。各リリースは同じファイルの Figma 自身による JSON、PNG、SVG 書き出しと照合しているため（[結果](#how-close-is-it-to-figma)）、形式の変更は計測されたリグレッションとして表面化し、気づかれないまま壊れることはありません。すでに保存したコピーは、それを読んだ figlens のバージョンで引き続き使えます。変更後の Figma から新しく保存したファイルだけが figlens の更新を必要とし、`figlens update` で取得できます。

</details>

<a id="from-a-local-fig-to-working-html"></a>

## ローカルの .fig から動く HTML へ

[![Pi がローカル Figma ファイルから操作可能な画面を作り、ターミナルの横にプレビューを表示する](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[動画を見る](assets/demo-8x.mp4)** · **8 倍速**の 45 秒タイムラプス。Orca 上の実際の Pi セッションで、ローカルデザインを読み、HTML/CSS を作成し、ライブプレビューで共有ダイアログと入力欄を操作します。HTML の画面には bundle 内の SVG アイコンを 19 個使っています。

| この例 | 測定結果 |
|---|---|
| ローカル `.fig` ファイル | 57.5 MiB |
| 選択したフレーム | 153 ノード、25 SVG アセット、欠落なし |
| bundle の抽出 | ウォームキャッシュで 5 回実行した中央値 **0.47 秒** |

測定対象は `figlens bundle` であり、**AI の実装時間は含みません**。ARM64 Docker コンテナー内の figlens 0.1.2 で測定しました。[コマンド、各回の結果、録画の説明](wiki/accuracy.md#local-design-to-html-demo)を参照してください。Pi がコードを書き、figlens が設計データを渡します。AI プロバイダーへの接続には引き続きネットワークを使います。

<details>
<summary>デザインの出典とデモの制約</summary>

「List views」フレームの出典は [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude) です。元の Anthropic フォントが同梱されていないため、システムの sans と Georgia を使用しています。Share はローカルダイアログを開き、返信はページ内に保持されますが再読み込みで消えます。録画は実際の Orca ウィンドウの連続スクリーンショットから作成しており、通常の動画収録より動きが滑らかではありません。

</details>

<a id="quick-start"></a>

## クイックスタート

<a id="1-open-your-design-locally"></a>

### 1. デザインをローカルで開く

Figma で **File > Save local copy** を選び、`design.fig` として保存します。次を実行してください。

```sh
npx @runkids/figlens preview design.fig
```

ターミナルに表示されたローカル URL を開きます。ページを閲覧し、実装したい画面を選択してください。その場で余白、文字組み、変数、アセットを確認できます。

Node.js 18+ が必要です。初回実行時にネイティブバイナリをダウンロード・検証し、キャッシュします。[単体インストール](#installation)では Node.js、Rust、Docker は不要です。Ctrl+C でビューアーを停止します。

<a id="2-give-the-screen-to-your-agent"></a>

### 2. 画面をエージェントに渡す

ビューアーから選択したフレームの bundle をダウンロードし、zip と次の指示をコーディングエージェントに渡します。

> この bundle を専用ディレクトリに展開し、context.md を読んでください。このプロジェクトの技術スタックと規約に沿って画面を実装してください。付属のレイアウト値とアセットを使用し、references/screen.png と比較して結果を確認してください。欠落アセットや利用できないフォントを報告してください。

bundle は構造、アセット、参照画像をまとめて含むため、エージェントは通常のファイルツールで読めます。画面の検索や確認を直接任せるには、[CLI または MCP サーバーを接続](#for-ai-agents)してください。[Pi では画面名を指定して依頼できます](#pi-extension)。

<a id="3-check-the-implementation"></a>

### 3. 実装を確認する

実装のスクリーンショットをビューアーに読み込みます。デザインと並べるか、透明度を調整した重ね合わせで違いを確認できます。画像はブラウザー内に留まります。これは目視での確認であり、figlens は実装の精度スコアを算出しません。

**`.fig` はスナップショットです。** デザインを変更したら、新しいローカルコピーを保存してください。Figma URL はそのコピー内のノードを選択できますが、デザインのダウンロードはできません。インストール後のファイル読み取りはオフラインで動作します。AI プロバイダーやプロジェクトの依存関係にはネットワークが必要な場合があります。

<details>
<summary>ターミナルを使いたい場合：フレームを探して直接 bundle を作成できます。</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

`12:345` をツリーや検索結果の ID に置き換えます。ノードには ID（`12:345`）、URL 形式（`12-345`）、または `?node-id=` 付きの Figma URL を使えます。`find` でレイヤー名やテキストを検索し、`inspect` で展開済みインスタンスの子要素とレイアウト／テキストの要約を確認します。詳細は[コマンド](#commands)を参照してください。

</details>

<a id="browse-and-hand-off-a-design"></a>

## デザインの閲覧と受け渡し

[![MCP Apps のデザインをローカルで確認：レイヤーツリー、レイアウト、コンポーネントのプロパティ](assets/preview-inspector.png)](assets/pi-demo.mp4)

ローカルビューアーで、Figma セッションなしに保存済みのデザインを調べられます。

- ページとレイヤーの閲覧、名前とテキストの検索、距離の測定、レイアウト、文字組み、変数、コンポーネントのプロパティ、CSS の確認。
- 選択したレイヤーを SVG、PNG、JPG、WebP、AVIF で書き出す、または元の画像フィルをダウンロード。
- レイヤー ID、構造化コンテキスト、実装指示のコピー。インスタンスのサブレイヤーを含む正確な選択範囲の bundle をダウンロード。

相手が閲覧できる形でデザインを渡すには、単独で開ける HTML スナップショットを書き出します。

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

サーバーもネットワークも不要で開け、設計データ、画像、スナップショット範囲の bundle を含みます。アセットの閲覧権限がある人にのみ共有してください。子要素の選択は確認するコンテキストを変えますが、埋め込み bundle の範囲は元のスナップショットのままです。

ライブビューアーはローカルファイルを監視し、保存成功後に更新します。不完全な保存や保存失敗時には古い表示であることを示します。HTML スナップショットは更新されません。操作、キャッシュ管理、ブラウザーエージェントによる自動化、制約は[プレビューガイド](wiki/preview.md)を参照してください。

<a id="pi-extension"></a>

## Pi 拡張機能

**Pi に `.fig` を渡し、作りたい画面を伝えます。** パッケージには CLI、MCP、プレビュー、拡張機能、エージェント用 skill が含まれます。

```sh
pi install npm:@runkids/figlens
```

Pi を再起動するか `/reload` を実行し、普段どおり依頼します。

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi は画面を見つけてレイアウトとアセットを読み、プロジェクトを調べて既存の技術スタックで実装します。フレーム ID を知る必要も `/figlens open` の実行もありません。名前が曖昧なら少数の候補を提示できます。MCP は自動起動し、グローバルな CLI インストールは不要です。

![Pi に画面を依頼し、デザインを見つけ、既存の技術スタックで実装して結果を確認する](assets/pi-workflow.svg)

**任意のコンポーネント再利用：** 再利用できる対応関係を保存するよう Pi に依頼し、`/figlens review` でプロジェクト内の未承認提案を承認・編集します。決定は `.figlens/component-mappings.md` に保存されます。バリアントや記録済みコードの変更には再レビューが必要です。対応関係がなくても画面は実装できます。

[![ローカルデザインを調べ、Pi で既存の HTML コンポーネントを確認し、次のフレームで再利用する](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[任意のマッピング手順を見る](assets/pi-demo.mp4)** · v0.3.0 の 20 秒の編集済み録画です。MCP Apps のデザインを確認し、事前に用意した既存 HTML との対応を承認して、新しいセッションで再利用します。モデルによる生成は表示も計測もしていません。[手順・コマンド・制約](pi/README.md) · [録画と検証の説明](wiki/pi.md#demo-and-verification)。

Pi 1.3.0+ が必要です。npm 拡張機能は同梱の CLI ランチャーを使います。MCP の自動起動を含む初回起動時に、対応するネイティブバイナリをダウンロード・検証します。Homebrew とリリースアーカイブにはネイティブ CLI とビューアーが含まれます。開発時は[ソースからのセットアップ](pi/README.md#install-once)を参照してください。

<a id="the-design-bundle"></a>

## デザイン bundle

一つのフレームを一つの zip に。実装用の構造、利用するアセット、比較するための画像が入っています。

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

一部だけ必要なら、JSON の書き出しやノードの直接レンダリングができます。

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

SVG 書き出しはフレーム、グループ、アイコン、正確なインスタンスのサブレイヤーに対応します。選択したサブツリーが単独の SVG になり、元の画像フィルを埋め込み、テキストはグリフの輪郭になります。bundle の展開は不要です。SVG の寸法は元のままで、`--scale` と `--quality` はラスター形式にのみ適用されます。MCP 経由のラスター書き出しは `maxSize`（既定で長辺 1600 px、0 で無効）に収まるよう縮小され、インライン画像が大きくなりません。

<a id="how-close-is-it-to-figma"></a>

## Figma との一致度は？

figlens は Figma 自身の JSON、PNG、SVG 書き出しと比較しています。**826 個の実運用フレーム**と **5 個の Community ファイル**で記録された結果です。

| 検証項目 | 結果 |
|---|---|
| ノード構造、名前、表示状態、境界、テキスト | 実運用フレーム 771 / 826 で一致 |
| 元の画像フィル | 2,608 / 2,608 がバイト単位で一致 |
| SVG アセット | 5,014 / 5,197 がピクセル差分のしきい値以内 |
| Community フレームの bundle | 306 / 306 で欠落アセットなし |

これらは対象ファイルの測定結果で、すべてのデザインを保証するものではありません。日付、しきい値、レンダリング結果、方法は[精度レポート](wiki/accuracy.md)を参照してください。

**既知の制約：** constraints、layoutAlign/layoutGrow、レイアウトグリッド、コンポーネントのプロパティ定義はまだ出力されません。背景ぼかしと一部の画像調整（コントラスト、色温度、色合い、ハイライト、シャドウ）は描画せず、角度・ひし形グラデーションは放射状として描画します。保存されたグリフ輪郭を使うため描画にフォントファイルは不要ですが、上書きテキストに Figma の最後のレイアウトが残ることがあります。実装用のフォントファイルは含まれません。

<a id="for-ai-agents"></a>

## AI エージェント向け

エージェント用 skill から CLI を使うか、同じバイナリを MCP サーバーとして接続します。

<a id="agent-skill"></a>

### エージェント用 skill

[figlens skill](skills/figlens/SKILL.md) はフレームの検索、bundle の分割読み取り、数値に基づく実装をエージェントに教えます。

[Skills CLI](https://github.com/vercel-labs/skills) でインストールします。

```sh
npx skills add runkids/figlens --skill figlens
```

Claude Code、Codex、Cursor、Gemini CLI などの skills をまとめて管理・更新するには [skillshare](https://github.com/runkids/skillshare) を使えます。

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` で最新に保てます。`skills/figlens/` をエージェントの skills ディレクトリ（Claude Code では `~/.claude/skills/`）へコピーする方法もあります。リリースアーカイブにも含まれます。

<a id="mcp-server"></a>

### MCP サーバー

[skillshare](https://github.com/runkids/skillshare) で MCP サーバーと skills をまとめて管理できます。figlens を一度定義し、接続設定を Claude Code、Codex、Cursor に同期します。

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

他のクライアントや既存設定の取り込みは [Skillshare MCP ガイド](https://skillshare.runkids.cc/docs/reference/commands/mcp/)を参照してください。

Claude Code に直接設定する場合：

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

他の MCP クライアント：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

単体でインストールした `figlens` を使う場合：

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

サーバーを特定のフォルダーに限定するには `--root` を付けます: `"args": ["mcp", "--root", "/path/to/designs"]`。ツール呼び出しが指定する `file` と出力パスはすべていずれかのルート配下である必要があります。フォルダーを増やすには `--root` を繰り返します。引数がツールのスキーマから外れる呼び出しも拒否されます。旧来の引数変換ではどの引数もパスになり得るためです。

サーバーはローカルの stdio で動作し、`tree`、`find`、`inspect`、`nodes`、`variables`、`bundle`、`export` を公開します。ツールには絶対パスを渡してください。相対パスはクライアントが figlens を起動したディレクトリを基準に解決されます。

まず `find` と `inspect` でフレームを選択します。`export` はファイルを保存してメタデータを返します。ラスター形式は MCP 画像コンテンツも返すため、エージェントはローカルファイルを開かずにデザインを見られます。SVG はメタデータのみです。PNG はクライアントの対応が最も広く、大きなフレームでは `scale` を下げてください。`find`、`inspect`、`bundle`、`export` は構造化 JSON も返します。

<a id="installation"></a>

## インストール

Node.js 18+ で `npx @runkids/figlens` を使うか、単体の CLI をインストールします。

| プラットフォーム | コマンド |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

リリースバイナリは macOS（Apple silicon、Intel）、Linux（arm64、x86_64、静的 musl）、Windows（x86_64、ARM64 はエミュレーション）に対応します。単体インストールは `figlens update` で更新できます。

<a id="commands"></a>

## コマンド

| コマンド | 出力 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | ノード ID・種類・名前。字下げテキスト（エージェントのコンテキストを節約）、`--json` なら入れ子の JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 大文字小文字を区別せず名前とテキストの部分一致検索。パス、サイズ、プレビュー。既定 20 件（1–200）、`truncated` は追加結果あり |
| `inspect <file> <node> [--depth N]` | インスタンス展開済みのレイアウト／テキスト要約。既定の深さ 1（0–10）、最大 200 ノード。全データは `nodes` |
| `nodes <file> <node>` | REST 形式 JSON：展開済みインスタンスと上書き、自動レイアウト、テキストスタイル、塗り、効果、バインド変数、コンポーネントのプロパティ、描画境界 |
| `variables <file> [node]` | フレームのバインド変数。ノード指定なしなら全ローカル変数 |
| `bundle <file> <node> <out.zip>` | 単一フレームの bundle。`{nodeId, path, warnings}` を返す |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | 一度のデコードで複数 bundle を並列作成。フレームごとに上記または `{nodeId, error}` の JSON を 1 行出力 |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG、PNG、JPG、WebP、AVIF。`--max-kb` は非可逆品質を下げてサイズに合わせる。PNG/SVG は超過時に失敗。`{path, width, height, bytes, quality}` を返す |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | ローカルの読み取り専用ビューアー。`--out` はノード指定が必要で、単独のオフラインスナップショットを作成 |
| `compare <file> <nodes.json> <node>` | 同じフレームの REST 書き出しとの差分 |
| `node <file> <node>` | 未加工の `.fig` ノード一つ |
| `mcp` | stdio によるローカル MCP サーバー |
| `update` | 最新リリースでバイナリを置き換え |

`find` は保存済みレイヤーを検索し、ID、祖先パス、ローカルサイズ、テキストのプレビューを返します。`inspect` はインスタンスを展開し、出力を 200 ノードとテキストごとに 200 文字に制限します。省略は `truncated` と `textTruncated` で示されます。CLI の各呼び出しはファイルをデコードします。MCP サーバーは、ディスク上のファイルが変わるまで、解析済みのファイルを呼び出し間で保持します。

`<file>` には `gs://bucket/object` も使えます。`gcloud` のログイン情報でダウンロードしてキャッシュします。使い方は `figlens --help` を実行してください。

<a id="help-figlens-grow"></a>

## figlens を応援する

figlens が次のデザインの実装に役立ったら、[スターを付けて](https://github.com/runkids/figlens)他の人にも見つけてもらえるとうれしいです。うまく扱えないデザインがあれば、実行したコマンドと期待した結果を添えて [issue を開いてください](https://github.com/runkids/figlens/issues)。

開発時の Rust は Docker 内でのみ実行します：`docker compose run --rm dev cargo test`。`make devc` は、この checkout のソースを `figlens` として実行するシェルを開きます。`make help` でコンテナーのターゲットを表示できます。VS Code の Reopen in Container も同じサービスを使います。規約は [AGENTS.md](AGENTS.md)、技術的な話題は [wiki](wiki/README.md) を参照してください。

<a id="license"></a>

## ライセンス

[MIT](LICENSE)。figlens は独立したプロジェクトで、Figma との提携、承認、スポンサー関係はありません。`.fig` 形式は未文書化で変更される可能性があります。figlens は自分の Figma アカウントから保存したファイルを読み取ります。
