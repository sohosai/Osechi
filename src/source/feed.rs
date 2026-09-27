//! 取得処理(バックグラウンドのスレッドやOSのコールバック)からエンジンスレッドへデータを渡す経路。
//!
//! ライブ用途では遅延が積み上がるより最新に追いつく方が大事なので、容量を超えたら
//! 古いものから捨てる。受信側の [`Feed`] が drop されると送信側の [`Producer`] は
//! それを検知でき、取得処理はそこで終了する。
//!
//! 失敗はデータとは別に「直近のエラー」として持つ。データと同じキューに入れると、
//! 満杯時に捨ててよいのか・順序に意味があるのかが曖昧になるため。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use crate::error::{Error, Result};

struct Queue<T> {
    items: VecDeque<T>,
    capacity: usize,
    error: Option<Error>,
}

type Shared<T> = Mutex<Queue<T>>;

fn lock<T>(shared: &Shared<T>) -> MutexGuard<'_, Queue<T>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 受け取る側(エンジンスレッド)の受信口。drop すると取得処理に停止を伝え、保持していた資源も解放する。
pub struct Feed<T> {
    shared: Arc<Shared<T>>,
    _guard: Option<Box<dyn Send>>,
}

impl<T> Feed<T> {
    /// 最大 `capacity` 個を溜める経路を作る。
    pub fn new(capacity: usize) -> (Producer<T>, Self) {
        let shared = Arc::new(Mutex::new(Queue {
            items: VecDeque::with_capacity(capacity),
            capacity,
            error: None,
        }));
        let producer = Producer(Arc::downgrade(&shared));
        let feed = Self {
            shared,
            _guard: None,
        };
        (producer, feed)
    }

    /// 取得処理 `run` を専用スレッドで動かし、その受信口を返す。
    /// `run` は [`Producer::is_open`] や [`Producer::send`] で受信側が無くなったのを知ったら戻ればよい。
    pub fn spawn(capacity: usize, run: impl FnOnce(Producer<T>) + Send + 'static) -> Self
    where
        T: Send + 'static,
    {
        let (producer, feed) = Self::new(capacity);
        thread::spawn(move || run(producer));
        feed
    }

    /// この受信口が drop されるまで生かしておく資源(OSの音声ストリームなど)を持たせる。
    pub fn with_guard(mut self, guard: impl Send + 'static) -> Self {
        self._guard = Some(Box::new(guard));
        self
    }

    /// 届いているデータを古い順に全て取り出す。ブロックしない。
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| lock(&self.shared).items.pop_front())
    }

    /// 直近の取得が失敗していれば、その理由(原因まで連結したもの)。
    pub fn error(&self) -> Option<String> {
        lock(&self.shared)
            .error
            .as_ref()
            .map(|err| format!("{err:#}"))
    }
}

/// 取得処理側の送信口。
pub struct Producer<T>(Weak<Shared<T>>);

impl<T> Clone for Producer<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Producer<T> {
    /// 取得結果を送る。成功ならデータを積み(満杯なら最古を捨てる)直近のエラーを消し、
    /// 失敗ならエラーとして記録する。受信側が既に無ければ `false` を返す。
    pub fn send(&self, item: Result<T>) -> bool {
        let Some(shared) = self.0.upgrade() else {
            return false;
        };
        let mut queue = lock(&shared);
        match item {
            Ok(item) => {
                if queue.items.len() >= queue.capacity {
                    queue.items.pop_front();
                }
                queue.items.push_back(item);
                queue.error = None;
            }
            Err(err) => queue.error = Some(err),
        }
        true
    }

    /// 受信側がまだ存在するか。
    pub fn is_open(&self) -> bool {
        self.0.strong_count() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_oldest_when_full() {
        let (producer, feed) = Feed::new(2);
        for i in 0..4 {
            producer.send(Ok(i));
        }
        assert_eq!(feed.try_iter().collect::<Vec<_>>(), [2, 3]);
        assert_eq!(feed.try_iter().count(), 0);
    }

    #[test]
    fn error_is_kept_until_next_success() {
        let (producer, feed) = Feed::<i32>::new(2);
        producer.send(Err(Error::new("device lost")));
        assert_eq!(feed.error().as_deref(), Some("device lost"));
        assert_eq!(feed.error().as_deref(), Some("device lost"));

        producer.send(Ok(1));
        assert_eq!(feed.error(), None);
    }

    #[test]
    fn producer_notices_dropped_feed() {
        let (producer, feed) = Feed::new(2);
        assert!(producer.is_open());
        assert!(producer.send(Ok(1)));

        drop(feed);
        assert!(!producer.is_open());
        assert!(!producer.send(Ok(2)));
    }

    #[test]
    fn guard_lives_as_long_as_feed() {
        let guard = Arc::new(());
        let (_producer, feed) = Feed::<i32>::new(1);
        let feed = feed.with_guard(Arc::clone(&guard));
        assert_eq!(Arc::strong_count(&guard), 2);
        drop(feed);
        assert_eq!(Arc::strong_count(&guard), 1);
    }
}
