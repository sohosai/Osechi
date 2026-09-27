//! 番組出力(RTMP配信)の設定と状態のウインドウ。メニューバーには配信状態の表示を出す。

use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use super::theme;
use crate::app::App;
use crate::output::rtmp::{Settings, State, Status, Target};

/// 映像ビットレートの既定値(Mbps)。edamame の 1080p(copy)の宣言帯域 12Mbps に収まる値。
const DEFAULT_BITRATE_MBPS: u32 = 8;

/// 配信設定の入力中の内容。
pub(crate) struct RtmpForm {
    pub url: String,
    bitrate_mbps: u32,
    error: Option<String>,
}

impl Default for RtmpForm {
    fn default() -> Self {
        Self {
            url: String::new(),
            bitrate_mbps: DEFAULT_BITRATE_MBPS,
            error: None,
        }
    }
}

impl RtmpForm {
    fn settings(&self) -> Result<Settings, String> {
        let target: Target = self.url.parse().map_err(|err| format!("{err}"))?;
        Ok(Settings {
            target,
            video_bitrate: self.bitrate_mbps * 1_000_000,
        })
    }
}

/// 開いていれば配信ウインドウを描く。
pub(super) fn show(app: &mut App, ctx: &egui::Context) {
    let mut open = app.ui.output_open;
    egui::Window::new("Output")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| rtmp_panel(app, ui));
    app.ui.output_open = open;
}

fn rtmp_panel(app: &mut App, ui: &mut egui::Ui) {
    let running = app.rtmp.is_running();
    ui.label(RichText::new("RTMP").strong());
    ui.add_space(4.0);

    egui::Grid::new("rtmp_settings")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            let form = &mut app.ui.rtmp;
            ui.label("URL");
            ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut form.url)
                    .hint_text("rtmp://host:1935/live/<channel>")
                    .desired_width(300.0),
            );
            ui.end_row();

            ui.label("Video Bitrate");
            ui.add_enabled(
                !running,
                egui::DragValue::new(&mut form.bitrate_mbps)
                    .range(1..=20)
                    .suffix(" Mbps"),
            );
            ui.end_row();
        });
    ui.label(
        RichText::new("1920x1080 30fps H.264 / AAC 128kbps, keyframe every 2s")
            .small()
            .weak(),
    );

    if let Some(err) = &app.ui.rtmp.error {
        ui.add_space(4.0);
        ui.colored_label(theme::ACCENT_PROGRAM, err);
    }

    ui.add_space(6.0);
    if running {
        if ui.button("Stop").clicked() {
            app.rtmp.stop();
        }
    } else if ui.button("Start").clicked() {
        match app.ui.rtmp.settings() {
            Ok(settings) => {
                app.ui.rtmp.error = None;
                app.rtmp.start(settings, app.program.clone());
            }
            Err(err) => app.ui.rtmp.error = Some(err),
        }
    }

    if running {
        ui.add_space(8.0);
        ui.separator();
        status_grid(ui, &app.rtmp.status());
    }
}

fn status_grid(ui: &mut egui::Ui, status: &Status) {
    egui::Grid::new("rtmp_status")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(RichText::new(label).weak());
                ui.label(RichText::new(value).monospace());
                ui.end_row();
            };
            row("State", status.state.to_string());
            row(
                "Uptime",
                status
                    .live_since
                    .map_or_else(|| "-".to_string(), |since| format_duration(since.elapsed())),
            );
            row("Bitrate", format!("{:.2} Mbps", status.bitrate / 1e6));
            row("Encode", format!("{:.1} fps", status.fps));
            row("Dropped Frames", status.dropped_frames.to_string());
            row("Audio Gaps", status.audio_gaps.to_string());
            row("Reconnects", status.reconnects.to_string());
        });
    if let Some(err) = &status.error {
        ui.add_space(4.0);
        ui.colored_label(theme::ACCENT_PROGRAM, err);
    }
}

/// メニューバーに出す配信状態(配信していなければ何も出さない)。
pub(super) fn indicator(app: &App, ui: &mut egui::Ui) {
    if !app.rtmp.is_running() {
        return;
    }
    let status = app.rtmp.status();
    let (color, text) = match status.state {
        State::Live => (
            theme::ACCENT_PROGRAM,
            format!(
                "● LIVE {}  {:.1} Mbps",
                status
                    .live_since
                    .map_or_else(String::new, |since| format_duration(since.elapsed())),
                status.bitrate / 1e6
            ),
        ),
        State::Connecting => (Color32::YELLOW, "● CONNECTING".to_string()),
        State::Retrying => (Color32::YELLOW, "● RETRYING".to_string()),
        State::Idle => (ui.visuals().weak_text_color(), "● OFF".to_string()),
    };
    ui.label(RichText::new(text).color(color).monospace());
}

/// `01:02:03` の形にする。
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_uptime() {
        assert_eq!(format_duration(Duration::from_secs(3723)), "01:02:03");
    }

    #[test]
    fn form_validates_url() {
        let mut form = RtmpForm::default();
        assert!(form.settings().is_err());
        form.url = "rtmp://127.0.0.1:1935/live/1A".to_string();
        let settings = form.settings().unwrap();
        assert_eq!(settings.video_bitrate, DEFAULT_BITRATE_MBPS * 1_000_000);
    }
}
