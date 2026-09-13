use crate::source::{audio, video};

/// 指定されたIDの映像ソース名を取得する
pub fn video_source_name(id: &video::SourceId, sources: &[video::Descriptor]) -> String {
    sources
        .iter()
        .find(|d| d.id == *id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// 指定されたIDの音声ソース名を取得する
pub fn audio_source_name(id: &audio::SourceId, sources: &[audio::Descriptor]) -> String {
    sources
        .iter()
        .find(|d| d.id == *id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "Unknown".to_string())
}
