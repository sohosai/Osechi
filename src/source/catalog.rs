//! 利用可能なソースの一覧。

use super::{Origin, Source, SourceId};

/// 利用可能なソースの一覧。IDの重複は持たない。
#[derive(Debug, Clone)]
pub struct Catalog<K> {
    sources: Vec<Source<K>>,
}

impl<K> Default for Catalog<K> {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
        }
    }
}

impl<K> Catalog<K> {
    pub fn get(&self, id: &SourceId) -> Option<&Source<K>> {
        self.sources.iter().find(|source| source.id == *id)
    }

    /// 表示名。一覧に無ければ `"Unknown"`。
    pub fn name(&self, id: &SourceId) -> &str {
        self.get(id).map_or("Unknown", |source| &source.name)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Source<K>> {
        self.sources.iter()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// 追加する。同じIDが既にあれば何もしない。
    pub fn insert(&mut self, source: Source<K>) {
        if self.get(&source.id).is_none() {
            self.sources.push(source);
        }
    }

    pub fn remove(&mut self, id: &SourceId) {
        self.sources.retain(|source| source.id != *id);
    }

    /// `origin` 由来のソースを `fresh` の内容に入れ替える。
    ///
    /// `fresh` に無くなったものは消し、あるものは位置を保ったまま更新し、新しいものは末尾に足す。
    /// 別の由来で同じIDが既に載っている場合はそちらを優先して上書きしない
    /// (手動追加したAES67フローが、同じフローのSAP告知で置き換わらないように)。
    pub fn sync(&mut self, origin: Origin, fresh: impl IntoIterator<Item = Source<K>>) {
        let fresh: Vec<Source<K>> = fresh.into_iter().collect();
        self.sources
            .retain(|source| source.origin != origin || fresh.iter().any(|f| f.id == source.id));

        for mut source in fresh {
            source.origin = origin;
            match self.sources.iter_mut().find(|s| s.id == source.id) {
                Some(existing) if existing.origin == origin => *existing = source,
                Some(_) => {}
                None => self.sources.push(source),
            }
        }
    }
}

impl<'a, K> IntoIterator for &'a Catalog<K> {
    type Item = &'a Source<K>;
    type IntoIter = std::slice::Iter<'a, Source<K>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(key: &str, origin: Origin) -> Source<u32> {
        Source {
            id: SourceId::new("test", key),
            name: key.to_string(),
            kind: 0,
            origin,
        }
    }

    fn keys(catalog: &Catalog<u32>) -> Vec<&str> {
        catalog.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn insert_ignores_duplicate_id() {
        let mut catalog = Catalog::default();
        catalog.insert(source("a", Origin::Manual));
        catalog.insert(source("a", Origin::Scanned));
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.iter().next().unwrap().origin, Origin::Manual);
    }

    #[test]
    fn sync_replaces_only_its_own_origin() {
        let mut catalog = Catalog::default();
        catalog.insert(source("manual", Origin::Manual));
        catalog.sync(
            Origin::Scanned,
            [source("a", Origin::Scanned), source("b", Origin::Scanned)],
        );
        catalog.sync(
            Origin::Scanned,
            [source("b", Origin::Scanned), source("c", Origin::Scanned)],
        );
        assert_eq!(keys(&catalog), ["manual", "b", "c"]);
    }

    #[test]
    fn sync_updates_in_place() {
        let mut catalog = Catalog::default();
        catalog.sync(Origin::Discovered, [source("a", Origin::Discovered)]);
        catalog.insert(source("m", Origin::Manual));
        let mut renamed = source("a", Origin::Discovered);
        renamed.kind = 7;
        catalog.sync(Origin::Discovered, [renamed]);
        assert_eq!(keys(&catalog), ["a", "m"]);
        assert_eq!(catalog.iter().next().unwrap().kind, 7);
    }

    #[test]
    fn sync_does_not_take_over_other_origin() {
        let mut catalog = Catalog::default();
        catalog.insert(source("a", Origin::Manual));
        catalog.sync(Origin::Discovered, [source("a", Origin::Discovered)]);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.iter().next().unwrap().origin, Origin::Manual);

        // 告知が途絶えても手動追加分は残る
        catalog.sync(Origin::Discovered, []);
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn name_falls_back_to_unknown() {
        let catalog = Catalog::<u32>::default();
        assert_eq!(catalog.name(&SourceId::new("test", "x")), "Unknown");
    }
}
