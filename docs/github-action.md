# GitHub Actions による CI/CD の仕様

Osechi リポジトリでは、コードの品質担保およびリリース作業の自動化のために、GitHub Actions を使用した複数のワークフローを設定しています。

## 1. Fast Test (fast-test.yml)
PRで、コードの基本的な品質を高速に検証するためのワークフローです。

- **トリガー**: main ブランチへの Pull Request 作成・更新時
  - ※ただし、docs/**, **.md ファイル, LICENSE, .gitignore への変更のみの場合は無関係なため実行をスキップします。
- **実行環境**: Ubuntu (Linux) のみ
- **実行されるステップ**:
  1. cargo fmt: コードのフォーマットが適切かチェックします (--check)。
  2. cargo clippy: Linterによる静的解析を行い、警告をすべてエラーとして捕捉して未然に防ぎます (-D warnings)。
  3. cargo test: ユニットテスト等のテストを実行します。

## 2. Full Test (full-test.yml)
Fast Test よりも網羅的な検証を行うためのワークフローです。複数OS環境でのビルドやテストが正しく通るかを確認します。PRのレビュー担当者が全てのOSで動作することを確認したい場合に実行します。

> [!IMPORTANT]
> OsechiはWindows,Mac,Linuxをサポートします。どれかのOSで動作しないコードは許可されません。

- **トリガー**:
  - 手動実行 (workflow_dispatch) のみ
- **実行環境**: Ubuntu, Windows, macOS の3プラットフォーム (matrixビルド)
- **実行されるステップ**:
  1. cargo build: 全ての特徴(--all-features)を有効にした状態でリリースビルドを行います。
  2. cargo test: 各OSでテストを実行します。
  3. cargo clippy: 各OSごとに Linter 警告がないか確認します。

## 3. Release Build (release.yml)
アプリケーションの新しいバージョンをリリースするためのワークフローです。バージョン情報を自動で更新し、インストール用の実行ファイルを各OS向けにビルドして配布できるようにします。

- **トリガー**: 手動実行 (workflow_dispatch) のみ
- **入力項目**: 
  - **バージョンの更新桁 (bump)**: major, minor, patch から選択可能。デフォルトは patch。
- **機能と流れ**:
  1. **Bump Version ジョブ (Ubuntu)**:
     - cargo-edit (cargo set-version) を使って Cargo.toml のバージョン指定を自動で書き換えます。
     - バージョン変更をコミットし、Gitタグ (例: v0.1.3) を自動生成してリポジトリに Push・同期します。
  2. **Build Release ジョブ (各OS)**:
     - Bump Version ジョブの正常完了後、最新のタグ・コミットをチェックアウトします。
     - Ubuntu, Windows, macOS の3つのOS向けに cargo build --release によって最適化されたバイナリを作成します。
     - 生成された実行ファイル (Osechi または Osechi.exe) は、GitHub Actionsの Artifacts（アーティファクト）としてアップロードされます。
     - アップロードされたファイル（Osechi-Windows,Osechi-macOS, Osechi-Linux）は、GitHub のアクション実行結果ページから Zip 形式でダウンロードしてそのまま利用・配布することができます。
