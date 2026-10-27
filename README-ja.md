<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">coding agent が Figma ファイルを読む。<br>Figma アプリも、ログインも、API 制限も不要。</h1>

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
  <a href="#start">はじめに</a> · <a href="#demo">デモ</a> · <a href="#why">理由</a> · <a href="#bundle">エージェントが受け取るもの</a> · <a href="#pi">Pi</a> · <a href="#limitations">制限</a>
</p>

**課題。** 画面の実装にはレイアウト値、テキスト、素材が必要です。リモートホストや CI では、実行中のデスクトップアプリに依存せず、コードと一緒にデザイン情報へアクセスできる必要があります。

**解決策。** Figma で **File > Save local copy** を選び、`design.fig` をコードの隣に置きます。figlens がローカルで解析し、レイアウト、テキスト、スタイル、variables、アイコン、画像、任意の frame の参照レンダリングをエージェントに渡します。ネイティブバイナリ 1 つで、Figma へのネットワーク呼び出しはゼロです。

<a id="start"></a>

## はじめに

**デザインファイルがまだない場合：** [小さな公開サンプルでセットアップを確認](examples/first-success/README.md)できます。使用するバイナリ、frame の検索、生成した bundle の内容を検証します。モデルは不要です。

`design.fig` をプロジェクトに置き、使うエージェントを選びます。

**Claude Code**（Node.js 18+、macOS/Linux）：

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

新しいセッションを開き、`/mcp` を確認します。画面を普段の言葉で依頼してください。MCP には計画・実装・画像準備のガイドが組み込まれており、skill を別途インストールする必要はありません。

**Pi**（1.3.0+）：1 つのパッケージに CLI、MCP、viewer、skill が含まれます。

```sh
pi install npm:@runkids/figlens
```

**任意の MCP クライアント**（Cursor、Codex、Gemini CLI など）：

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

[スタンドアロン版のインストール](docs/README-ja.md#install)

あとは画面を依頼します。

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

エージェントは frame を見つけ、デザインデータ、素材、参照画像を bundle にまとめ、あなたのスタックで画面を実装します。共有ワークフローがフォントと素材の確認を案内し、不足は報告しつつ、記録した代替手段でレイアウト実装を続けます。変更したファイルとブラウザでの比較を見せるよう依頼してください。

<a id="demo"></a>

## デモ：Pi が .fig から React の画面を作る

[![Pi plans from a local .fig, builds a React screen and checks it beside a live browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

GIF は編集済み動画をさらに 3 倍速でループ再生します。クリックすると 78 秒の動画全編を見られます。

**[78 秒の動画](assets/taskflow-demo.mp4)。** Pi が `/figlens plan` を実行し、ローカルの `.fig` からダッシュボードの frame を読み、React と Tailwind CSS で作り、動作中の UI を確認して修正します。編集済みのタイムラプスであり、ベンチマークでもピクセル単位の一致の保証でもありません。デザイン：[Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [収録メモ](wiki/accuracy.md#taskflow-react-and-tailwind-demo)。

**[.fig から編集可能な HTML/CSS へ](examples/task-list/README.md)：** 条件を揃えたタスクリストの検証では、figlens を使った 6 回すべての試行で、表示される 19 箇所のテキストを実際の DOM テキストとして、4 つのアイコンも保持しました。Docker のネットワークを無効にした状態でも CLI と MCP の抽出に成功しました。設定手順、測定値、独立した画像比較は事例ページに掲載しています。

<a id="why"></a>

## 公式 Figma MCP ではなく figlens を使う理由

デザインをファイルとして持ち運びたいなら **figlens** を使います。リモートエージェント、CI、オフライン作業、エージェント間の引き継ぎ、無制限の繰り返し読み取りです。エクスポート手順なしでエージェントがライブのデザインを見る必要があるなら、**公式 Figma MCP** を使います。

`.fig` の読み取りはオフラインで動作します。figlens のインストールと更新、`gs://` 入力、モデルプロバイダー、プロジェクトの依存関係にはネットワークが必要です。

[詳しい比較](docs/README-ja.md#why)

<a id="bundle"></a>

## エージェントが受け取るもの

1 つの frame を 1 つの zip に。実装の土台となる構造、使う素材、照合用の画像です。

[Bundle の内容と使用例](docs/README-ja.md#bundle) · [Figma にどこまで近いか](docs/README-ja.md#accuracy)

## ドキュメント

- <a id="pi"></a>[Pi 拡張](docs/README-ja.md#pi)
- <a id="viewer"></a>[Figma なしでデザインを閲覧する](docs/README-ja.md#viewer)
- <a id="mcp-and-skill"></a>[MCP サーバーとエージェント skill](docs/README-ja.md#mcp-and-skill)
- <a id="commands"></a>[CLI リファレンス](docs/README-ja.md#commands)

<a id="limitations"></a>

## 制限

- **スナップショットであり、ライブ同期ではありません。** デザインが変わったら新しい `.fig` をエクスポートしてください。figlens は Figma の URL からデザインをダウンロードできません。
- **非公開のフォーマット。** figlens はファイルに埋め込まれたスキーマをデコードします。新しい Figma のエクスポートには figlens の更新が必要な場合があります。
- **フォントとテキスト：** フォントファイルは含まれません。オーバーライドされたテキストは Figma の保存済みレイアウトを保持することがあります。利用許諾のあるプロジェクトのフォントを使い、結果を比較してください。
- **対応範囲：** [レンダリングの改善](https://github.com/runkids/figlens/issues/36)と[ハンドオフのセマンティクス](https://github.com/runkids/figlens/issues/37)は issue で追跡しています。[現在のセマンティクス対応範囲](wiki/semantics.md)も参照してください。
- **出力を確認してください。** bundle の `warnings`、`missing-assets.json`、`context.md` 内の未解決の variables を読み、実装をデザインと比較してください。
- 既知の可視レンダリング制限はノードの `_figlens.renderDiagnostics`、件数制限のある `inspect` 要約、bundle の `warnings` に表示されます。`missing-assets.json` は素材の失敗のみを記録します。警告がなくてもピクセル一致は保証されません。

## figlens を育てる

figlens がデザインをコードにする助けになったなら、[リポジトリに star](https://github.com/runkids/figlens) を付けて、他の人が見つけやすくしてください。読み取れないファイルに出会いましたか？ コマンドと期待した結果を添えて [issue を作成](https://github.com/runkids/figlens/issues)してください。

開発では Rust を Docker 内でのみ実行します：`docker compose run --rm dev cargo test`。[AGENTS.md](AGENTS.md) と [wiki](wiki/README.md) を参照してください。

## ライセンス

[MIT](LICENSE)。figlens は独立したプロジェクトであり、Figma と提携しておらず、Figma の承認やスポンサーを受けていません。`.fig` フォーマットは非公開であり、変更される可能性があります。アクセスと共有の権限があるファイルと素材のみを使用してください。
