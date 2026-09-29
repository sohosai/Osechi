//! いま開いているソースの集合。

use std::collections::{HashMap, HashSet};

use super::{Catalog, Feed, Open, SourceId};
use crate::error::{Context, Result};

/// いま開いているソースの集合。
///
/// 使われている(Previewに出ている・ミキサーに入っている等)ソースだけを開いた状態に保つ。
/// 開くのに失敗したソースはそのエラーを持ったまま、使われなくなるまで再試行しない。
pub struct Live<T> {
    feeds: HashMap<SourceId, Result<Feed<T>>>,
}

impl<T> Default for Live<T> {
    fn default() -> Self {
        Self {
            feeds: HashMap::new(),
        }
    }
}

impl<T> Live<T> {
    /// `wanted` のソースだけが開いた状態になるよう、不要なものを閉じ、足りないものを `catalog` から開く。
    pub fn sync<'a, K>(
        &mut self,
        catalog: &Catalog<K>,
        wanted: impl IntoIterator<Item = &'a SourceId>,
    ) where
        K: Open<Output = T>,
    {
        let wanted: HashSet<&SourceId> = wanted.into_iter().collect();
        self.feeds.retain(|id, _| wanted.contains(id));

        for id in wanted {
            if !self.feeds.contains_key(id) {
                let feed = catalog
                    .get(id)
                    .context("source not found")
                    .and_then(|source| source.kind.open())
                    .context("open failed");
                self.feeds.insert(id.clone(), feed);
            }
        }
    }

    pub fn contains(&self, id: &SourceId) -> bool {
        self.feeds.contains_key(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &SourceId> {
        self.feeds.keys()
    }

    /// `id` のソースに届いているデータを古い順に全て取り出す。
    pub fn drain(&self, id: &SourceId) -> impl Iterator<Item = T> + '_ {
        self.feeds
            .get(id)
            .and_then(|feed| feed.as_ref().ok())
            .into_iter()
            .flat_map(Feed::try_iter)
    }

    /// 開いている全てのソースについて、届いているデータを古い順に全て取り出す。
    pub fn drain_all(&self) -> HashMap<SourceId, Vec<T>> {
        self.ids()
            .map(|id| (id.clone(), self.drain(id).collect()))
            .collect()
    }

    /// 開けなかった、または直近の取得に失敗したソースとその理由。
    pub fn errors(&self) -> impl Iterator<Item = (&SourceId, String)> {
        self.feeds.iter().filter_map(|(id, feed)| match feed {
            Ok(feed) => feed.error().map(|err| (id, err)),
            Err(err) => Some((id, format!("{err:#}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::source::{Origin, Source};

    /// 開くと自分の番号を1つ流すテスト用の種別。負の番号は開くのに失敗する。
    #[derive(Debug, Clone)]
    struct Test(i32);

    impl Open for Test {
        type Output = i32;

        fn open(&self) -> Result<Feed<i32>> {
            if self.0 < 0 {
                return Err(Error::new("no device"));
            }
            let (producer, feed) = Feed::new(4);
            producer.send(Ok(self.0));
            Ok(feed)
        }
    }

    fn catalog(numbers: &[i32]) -> Catalog<Test> {
        let mut catalog = Catalog::default();
        for &n in numbers {
            catalog.insert(Source {
                id: id(n),
                name: n.to_string(),
                kind: Test(n),
                origin: Origin::Scanned,
            });
        }
        catalog
    }

    fn id(n: i32) -> SourceId {
        SourceId::new("test", n)
    }

    #[test]
    fn opens_wanted_and_closes_others() {
        let catalog = catalog(&[1, 2]);
        let mut live = Live::default();

        live.sync(&catalog, [&id(1), &id(2)]);
        assert_eq!(live.drain(&id(1)).collect::<Vec<_>>(), [1]);
        assert_eq!(live.drain(&id(2)).collect::<Vec<_>>(), [2]);

        live.sync(&catalog, [&id(2)]);
        assert!(!live.contains(&id(1)));
        assert!(live.contains(&id(2)));
    }

    #[test]
    fn keeps_open_feeds_across_syncs() {
        let catalog = catalog(&[1]);
        let mut live = Live::default();
        live.sync(&catalog, [&id(1)]);
        assert_eq!(live.drain(&id(1)).count(), 1);

        // 開き直していれば番号がもう一度流れてくる
        live.sync(&catalog, [&id(1)]);
        assert_eq!(live.drain(&id(1)).count(), 0);
    }

    #[test]
    fn drain_all_takes_every_open_source_once() {
        let catalog = catalog(&[1, 2]);
        let mut live = Live::default();
        live.sync(&catalog, [&id(1), &id(2)]);

        let drained = live.drain_all();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[&id(1)], [1]);
        assert_eq!(drained[&id(2)], [2]);
        assert!(live.drain_all().values().all(Vec::is_empty));
    }

    #[test]
    fn reports_open_failures() {
        let catalog = catalog(&[-1]);
        let mut live = Live::default();
        live.sync(&catalog, [&id(-1), &id(9)]);

        let mut errors: Vec<String> = live.errors().map(|(_, err)| err).collect();
        errors.sort();
        assert_eq!(
            errors,
            ["open failed: no device", "open failed: source not found"]
        );
        assert_eq!(live.drain(&id(-1)).count(), 0);
    }
}
