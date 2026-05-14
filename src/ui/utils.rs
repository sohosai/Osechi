use crate::source::video::{Descriptor, SourceId};
use eframe::egui;

/// 指定されたIDのソース名を取得する
pub fn get_source_name(id: &SourceId, sources: &[Descriptor]) -> String {
    sources
        .iter()
        .find(|d| d.id == *id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// ドロップダウン（コンボボックス）でビデオソースを選択する
pub fn draw_source_combo_box(
    ui: &mut egui::Ui,
    id_salt: &str,
    current_id: &mut Option<SourceId>,
    sources: &[Descriptor],
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

            for desc in sources {
                let mut is_selected = current_id.as_ref() == Some(&desc.id);
                if ui
                    .selectable_value(&mut is_selected, true, &desc.name)
                    .clicked()
                {
                    *current_id = Some(desc.id.clone());
                }
            }
        });
}

/// メニューバー内のラジオボタンでビデオソースを選択する
pub fn draw_source_radio_menu(
    ui: &mut egui::Ui,
    title: &str,
    current_id: &mut Option<SourceId>,
    sources: &[Descriptor],
) {
    ui.menu_button(title, |ui| {
        let mut is_none = current_id.is_none();
        if ui.radio_value(&mut is_none, true, "None").clicked() {
            *current_id = None;
        }

        for desc in sources {
            let mut is_selected = current_id.as_ref() == Some(&desc.id);
            if ui.radio_value(&mut is_selected, true, &desc.name).clicked() {
                *current_id = Some(desc.id.clone());
            }
        }
    });
}
