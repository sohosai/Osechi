pub mod app;

/// 開発・デザイン確認用のコマンドラインオプション(スクリーンショット等)
pub mod dev;

/// Osechi全体のエラーをまとめたEnum
pub mod error;

/// カメラ,ログ,ウインドウなどの初期化処理をまとめたモジュール
pub mod init;

/// オーディオミキサーの実音声合成(ダウンミックス・リサンプル・出力)をまとめたモジュール
pub mod mixer;

/// 映像や音声などの入力をまとめたモジュール
pub mod source;

/// UIのパーツをまとめたモジュール
pub mod ui;
