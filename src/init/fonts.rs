use std::sync::Arc;

use eframe::egui;

/// OSにインストールされているフォントを探して優先度の低いフォールバックとして
/// 追加し、日本語などのCJK文字(音声デバイス名などOS側から渡ってくる文字列)が
/// 豆腐(□)にならないようにする。見つからない場合は既定のフォントのままになる。
pub fn install_cjk_fallback(ctx: &egui::Context) {
    let Some((bytes, index)) = find_cjk_font() else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    let font_name = "cjk_fallback".to_owned();

    let mut font_data = egui::FontData::from_owned(bytes);
    font_data.index = index;
    fonts
        .font_data
        .insert(font_name.clone(), Arc::new(font_data));

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(font_name.clone());
    }

    ctx.set_fonts(fonts);
}

/// 候補パスを順に探し、最初に見つかったフォントのバイト列と
/// (collectionの場合の)フォントフェイスのインデックスを返す。
fn find_cjk_font() -> Option<(Vec<u8>, u32)> {
    const CANDIDATES: &[(&str, u32)] = &[
        // Windows
        (r"C:\Windows\Fonts\YuGothM.ttc", 0),
        (r"C:\Windows\Fonts\meiryo.ttc", 0),
        (r"C:\Windows\Fonts\msgothic.ttc", 0),
        // macOS
        ("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc", 0),
        ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
        // Linux
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
    ];

    CANDIDATES
        .iter()
        .find_map(|(path, index)| std::fs::read(path).ok().map(|bytes| (bytes, *index)))
}
