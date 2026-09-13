use nokhwa::{native_api_backend, query};
use xcap::Monitor;

use crate::{
    error::AppError,
    source::video::{Descriptor, SourceId, SourceKind, Stream},
};

/// 映像ソースの検出と管理を行うマネージャー。
///
/// スキャンによって検出されたソースの一覧を [`Descriptor`] として保持し、
/// 指定されたソースの [`Stream`] を生成する。
#[derive(Default)]
pub struct SourceManager {
    sources: Vec<Descriptor>,
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// 映像ソースを全てスキャンする(WEBカメラ・画面共有)
    pub fn scan(&mut self) {
        self.sources = scan_web_cameras()
            .into_iter()
            .chain(scan_screens())
            .collect();
    }

    /// 最新のスキャン結果の映像ソース一覧を返す
    pub fn list(&self) -> &[Descriptor] {
        &self.sources
    }

    /// 指定されたIDの映像ソースを開き、[`Stream`] を取得する
    pub fn open(&self, source_id: &SourceId) -> Result<Box<dyn Stream>, AppError> {
        let descriptor = self
            .sources
            .iter()
            .find(|d| d.id == *source_id)
            .ok_or_else(|| AppError::Other(format!("Video source {:?} not found", source_id)))?;
        descriptor.open()
    }
}

fn scan_web_cameras() -> Vec<Descriptor> {
    let backend = native_api_backend().unwrap_or(nokhwa::utils::ApiBackend::Auto);
    let cameras = query(backend).unwrap_or_default();

    cameras
        .into_iter()
        .map(|info| {
            let id_str = format!("{}_{}", info.human_name(), info.index());
            Descriptor {
                id: SourceId::WebCamera(id_str),
                name: info.human_name().to_string(),
                kind: SourceKind::WebCamera {
                    index: info.index().clone(),
                },
            }
        })
        .collect()
}

fn scan_screens() -> Vec<Descriptor> {
    let Ok(monitors) = Monitor::all() else {
        return Vec::new();
    };

    monitors
        .into_iter()
        .filter_map(|monitor| {
            let id = monitor.id().ok()?;
            let name = monitor.name().unwrap_or_else(|_| format!("Screen {}", id));
            Some(Descriptor {
                id: SourceId::ScreenCapture(id),
                name,
                kind: SourceKind::ScreenCapture { monitor_id: id },
            })
        })
        .collect()
}
