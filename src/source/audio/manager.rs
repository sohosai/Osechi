use std::collections::HashSet;

use crate::{
    error::AppError,
    source::audio::input_device,
    source::audio::sap::SapListener,
    source::audio::{Descriptor, SourceId, SourceKind, Stream},
};

/// 音声ソースの検出と管理を行うマネージャー。
///
/// スキャンによって検出されたソース・手動で追加されたソース(AES67など)・
/// SAPで自動検出されたソースをまとめて [`Descriptor`] の一覧として保持し、
/// 指定されたソースの [`Stream`] を生成する。
pub struct SourceManager {
    sources: Vec<Descriptor>,
    /// SAP自動検出によって `sources` に追加されたIDの集合。
    /// タイムアウトで取り除いてよいのはこの集合に含まれるものだけで、
    /// 手動追加されたソースを誤って消さないようにするために区別する。
    sap_added_ids: HashSet<SourceId>,
    sap_listener: Option<SapListener>,
    // Todo: 将来的にここに伝送, システム音声, ファイル再生, etc...が足されていく
}

impl SourceManager {
    // `new()` はSAPリスナーの起動という副作用を持つため、`Default::default()`
    // だと気づかずSAP自動検出が無効なマネージャーを作ってしまう恐れがある。
    // 意図的に `Default` は実装しない。
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            sap_added_ids: HashSet::new(),
            sap_listener: SapListener::start(),
        }
    }

    /// 音声入力デバイスを全てスキャンする。
    /// AES67など手動で追加したソースはスキャン対象外なので保持したままにする。
    pub fn input_device_scan(&mut self) {
        self.sources
            .retain(|d| !matches!(d.kind, SourceKind::InputDevice { .. }));
        self.sources.extend(input_device::scan());
    }

    /// SAPで新たに検出されたAES67フローを追加し、再アナウンスが無くなって
    /// タイムアウトしたものを取り除く。毎フレーム呼び出すことを想定している。
    pub fn sync_sap_discoveries(&mut self) {
        let Some(listener) = &self.sap_listener else {
            return;
        };
        let discovered = listener.discovered();
        let discovered_ids: HashSet<SourceId> = discovered.iter().map(|d| d.id.clone()).collect();

        // このマネージャー自身がSAP検出で追加したもののうち、
        // もう検出されなくなったものだけを取り除く(手動追加分は対象外)
        let expired: Vec<SourceId> = self
            .sap_added_ids
            .iter()
            .filter(|id| !discovered_ids.contains(id))
            .cloned()
            .collect();
        for id in expired {
            self.sources.retain(|d| d.id != id);
            self.sap_added_ids.remove(&id);
        }

        for descriptor in discovered {
            if self.sources.iter().any(|d| d.id == descriptor.id) {
                continue;
            }
            self.sap_added_ids.insert(descriptor.id.clone());
            self.sources.push(descriptor);
        }
    }

    /// 指定したIDがSAPで自動検出されたものかどうかを返す
    /// (UI側で削除ボタンの表示可否を判断するために使う)。
    pub fn is_sap_discovered(&self, source_id: &SourceId) -> bool {
        self.sap_added_ids.contains(source_id)
    }

    /// 手動設定のAES67フローを追加する。同じIDのフローが既にあれば何もしない。
    pub fn add_aes67_flow(&mut self, descriptor: Descriptor) {
        if self.sources.iter().any(|d| d.id == descriptor.id) {
            return;
        }
        self.sources.push(descriptor);
    }

    /// 指定したIDの音声ソースを取り除く。
    pub fn remove(&mut self, source_id: &SourceId) {
        self.sources.retain(|d| d.id != *source_id);
        self.sap_added_ids.remove(source_id);
    }

    /// 最新の音声ソース一覧(スキャンされたもの・手動追加されたもの・
    /// SAPで自動検出されたもの全て)を返す
    pub fn list(&self) -> &[Descriptor] {
        &self.sources
    }

    /// 指定されたIDの音声ソースを開き、[`Stream`] を取得する
    pub fn open(&self, source_id: &SourceId) -> Result<Box<dyn Stream>, AppError> {
        let descriptor = self
            .sources
            .iter()
            .find(|d| d.id == *source_id)
            .ok_or_else(|| AppError::Other(format!("Audio source {:?} not found", source_id)))?;
        descriptor.open()
    }
}
