//! 画面。
//!
//! 1ファイル1パネルで、各パネルは `pub(super) fn show(app: &mut App, ui: &mut egui::Ui)` を持つ。
//! 複数のパネルで使う部品は [`widget`] に `egui::Widget` として、配色・フォントは [`theme`] に置く。

mod errors;
mod menu;
mod mixer;
mod multiview;
mod output;
mod record;
mod sources;
pub mod theme;
mod widget;

use std::collections::HashSet;

use eframe::egui;

use crate::app::App;
use crate::mixer::Pick;
use crate::source::SourceId;

/// ドラッグ&ドロップで運ぶもの。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Drag {
    Video(SourceId),
    /// 音声ソースと、そのどのチャンネルをミキサーに流すか
    Audio(SourceId, Pick),
}

/// UIだけが持つ状態。
pub struct State {
    /// マルチビューの各セルにラベルを出すか
    pub show_labels: bool,
    /// 「Add AES67 Source」ダイアログの入力中の内容(閉じていれば `None`)
    aes67_form: Option<sources::Aes67Form>,
    /// ソース一覧でチャンネルごとの行を広げている音声ソース。
    /// 何チャンネル届いているかを見られるよう、エンジンスレッドはミキサーに入っていなくても開いておく。
    pub(crate) expanded_audio: HashSet<SourceId>,
    /// 配信ウインドウを開いているか
    output_open: bool,
    /// 配信設定の入力中の内容
    pub(crate) rtmp: output::RtmpForm,
    /// 録画ウインドウを開いているか
    record_open: bool,
    /// 録画設定の入力中の内容
    recording: record::RecordForm,
}

impl Default for State {
    fn default() -> Self {
        Self {
            show_labels: true,
            aes67_form: None,
            expanded_audio: HashSet::new(),
            output_open: false,
            rtmp: output::RtmpForm::default(),
            record_open: false,
            recording: record::RecordForm::default(),
        }
    }
}

/// 全パネルを描画する。egui は先に置いたパネルから外周を確保するので、この順番が配置を決める
/// (上: メニュー、左: ソース一覧、下: ミキサー、残り: マルチビュー)。
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    menu::show(app, ui);
    errors::show(app, ui);
    sources::show(app, ui);
    mixer::show(app, ui);
    multiview::show(app, ui);
    output::show(app, ui.ctx());
    record::show(app, ui.ctx());
}
