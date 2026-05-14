use crate::source::video::traits::{VideoSource, VideoSourceId};
use eframe::egui;

/// 指定されたIDのソース名を取得する
pub fn get_source_name(id: &VideoSourceId, sources: &[Box<dyn VideoSource>]) -> String {
    sources
        .iter()
        .find(|s| s.id() == *id)
        .map(|s| s.name())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// ドロップダウン（コンボボックス）でビデオソースを選択する
pub fn draw_source_combo_box(
    ui: &mut egui::Ui,
    id_salt: &str,
    current_id: &mut Option<VideoSourceId>,
    sources: &[Box<dyn VideoSource>],
) {
    let selected_text = current_id
        .as_ref()
        .map(|id| get_source_name(id, sources))
        .unwrap_or_else(|| "None".to_string());

    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            let mut is_none = current_id.is_none();
            if ui.selectable_value(&mut is_none, true, "None").clicked() {
                *current_id = None;
            }

            for source in sources {
                let mut is_selected = current_id.as_ref() == Some(&source.id());
                if ui
                    .selectable_value(&mut is_selected, true, source.name())
                    .clicked()
                {
                    *current_id = Some(source.id());
                }
            }
        });
}

/// メニューバー内のラジオボタンでビデオソースを選択する
pub fn draw_source_radio_menu(
    ui: &mut egui::Ui,
    title: &str,
    current_id: &mut Option<VideoSourceId>,
    sources: &[Box<dyn VideoSource>],
) {
    ui.menu_button(title, |ui| {
        let mut is_none = current_id.is_none();
        if ui.radio_value(&mut is_none, true, "None").clicked() {
            *current_id = None;
        }

        for source in sources {
            let mut is_selected = current_id.as_ref() == Some(&source.id());
            if ui
                .radio_value(&mut is_selected, true, source.name())
                .clicked()
            {
                *current_id = Some(source.id());
            }
        }
    });
}
