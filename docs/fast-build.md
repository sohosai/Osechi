# 高速なビルドに関する情報

Osechiは様々なクレートを組み合わせている都合上、ビルド処理が重くなりがちです。そのため、手元のPCで可能な限り高速にビルドや実行を行うための設定方法をここにまとめます。

## 開発環境についてのポリシー

このリポジトリでは、「すべての標準的なOSにおいて、Rust (`cargo`) のデフォルト構成をインストールしていれば、リポジトリをCloneして即座に開発を始められること」を最重要視しています。

そのため、追加で必要になる高速化の設定などはリポジトリ側で強制せず、ここで情報として提供します。ご自身のPC環境のスペックやストレージ容量に合わせて、適宜設定を行ってください。

---

# 各個人のPCで設定する項目 (推奨)

これらはPC全体に適用されるため、ユーザーディレクトリ以下の `~/.cargo/config.toml` （Windowsの場合は `%USERPROFILE%\.cargo\config.toml`）に記述することをおすすめします。

**※注意:** リポジトリ直下に `.cargo/config.toml` を作成してこのプロジェクトだけに適用することも可能ですが、環境依存の設定となるため、誤ってGitHubにPushしないようにしてください（本リポジトリの `.gitignore` には除外設定を追加済みです）。

## 1. `sccache` を用いたキャッシュの活用

[sccache](https://github.com/mozilla/sccache) を使用することで、Rustのコンパイル結果をキャッシュし、2回目以降のビルド時間を大幅に短縮できます。

**インストール:**
```sh
cargo install sccache
```

**設定:**
`.cargo/config.toml` に以下を追記して、Cargoがデフォルトでsccacheを使用するようにします。
```toml
[build]
rustc-wrapper = "sccache"
```

**キャッシュサイズの変更:**
デフォルトのキャッシュサイズは10GBですが、ドライブの容量に余裕がある場合は、環境変数 `SCCACHE_CACHE_SIZE` を設定することで上限を増やすことができます。
*   Windows (PowerShell) の例: `$env:SCCACHE_CACHE_SIZE="50G"`（永続化する場合はシステムの環境変数設定から追加してください）
*   macOS / Linux の例: `export SCCACHE_CACHE_SIZE="50G"`（`~/.bashrc` や `~/.zshrc` などに追記）

## 2. ビルドのジョブ数 (並列処理数) の指定

CargoがRustをビルドする際の並列処理数を指定できます。
ご自身のPCの**論理コア数**を超えて設定しても逆にパフォーマンスが落ちる可能性があるため、環境に合わせて数値を調整してください。

```toml
[build]
jobs = 10
```

## 3. 高速なリンカーの設定

ビルドの最終段階である「リンク」の時間を短縮するため、デフォルトよりも高速なリンカーを使用することを推奨します。OSごとに使用するツールや設定が異なります。

*   **Windowsの場合:**
    Rust 1.71以降標準で同梱されている `rust-lld` を使用します。追加のインストールは不要です。
    ```toml
    [target.x86_64-pc-windows-msvc]
    linker = "rust-lld.exe"
    ```

*   **macOSの場合:**
    `zld` や `lld` の使用を推奨します。（例としてHomebrewで `zld` をインストールする場合: `brew install michaeleisel/zld/zld`）
    ```toml
    [target.x86_64-apple-darwin]
    rustflags = ["-C", "link-arg=-fuse-ld=zld"]

    [target.aarch64-apple-darwin]
    rustflags = ["-C", "link-arg=-fuse-ld=zld"]
    ```

*   **Linuxの場合:**
    非常に高速なリンカーである [mold](https://github.com/rui314/mold) の使用を推奨します。（事前に `sudo apt install mold` などでインストールが必要です）
    ```toml
    [target.x86_64-unknown-linux-gnu]
    rustflags = ["-C", "link-arg=-fuse-ld=mold"]
    ```
---

# このリポジトリで設定済みの項目

本リポジトリをCloneした時点で、以下の設定は既に適用されています。

## `Cargo.toml` の設定

開発時（`[profile.dev]`）において、外部クレート（依存関係）のみ最適化レベル（`opt-level`）を上げるなどの工夫が行われています。これにより、開発時のビルド速度と実行時パフォーマンスのバランスを取っています。

## `vscode` における設定 (`.vscode/settings.json`)

エディタ側でも効率的に動くように、Rust Analyzer向けの専用設定が含まれています。

### `rust-analyzer.cargo.targetDir`

*   **設定理由:** 専用のターゲットディレクトリを使用する設定です。
*   **効果:** ターミナルで実行する手動の `cargo build` と、VS Code裏で動く Rust Analyzer が出力ディレクトリを共有しなくなります。これにより、ビルド時のファイルロック（排他制御）の競合を防ぎ、スムーズにコード補完とコンパイルを行えます。

### `rust-analyzer.procMacro.enable`

*   **設定理由:** 手続き的マクロ (Procedural Macros) をエディタ上で展開・評価する機能に関する設定です。デフォルトでは動作を軽くするために無効化 (`false`) されています。
*   **効果:** この設定を `true` に変更して有効化すると、マクロを使ったコードの補完やエラーチェックが正確に行われるようになります。（※ただし、コードの記述時に著しくPCが重くなる場合があります）