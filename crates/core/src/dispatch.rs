//! 去重投递:notify 监听与轮询兜底两路事件都经此发往同一 broadcast。
//! 在 `window` 时间窗内,相同 `(kind, path)` 的事件只投递一次,用于抑制
//! 两个检测器对同一次文件变动的重复上报(见 DESIGN §5.2「去抖 + 去重」)。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::events::{ChangeEvent, ChangeKind};

pub(crate) struct Dispatcher {
    tx: broadcast::Sender<ChangeEvent>,
    window: Duration,
    /// `(kind, path)` → 最近一次投递时间。
    seen: Mutex<HashMap<(u8, String), Instant>>,
}

impl Dispatcher {
    pub fn new(tx: broadcast::Sender<ChangeEvent>, window: Duration) -> Self {
        Self {
            tx,
            window,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// 投递一条事件;若 `window` 内已投递过相同 `(kind, path)` 则丢弃。
    pub fn send(&self, event: ChangeEvent) {
        let now = Instant::now();
        let key = (kind_tag(&event.kind), event.path.clone());

        let mut seen = self.seen.lock().unwrap();
        // 顺手清理过期项,避免 map 无界增长;retain 后留下的都在窗内。
        seen.retain(|_, &mut t| now.duration_since(t) < self.window);
        if seen.contains_key(&key) {
            return; // 窗内重复,丢弃
        }
        seen.insert(key, now);
        drop(seen);

        let _ = self.tx.send(event);
    }
}

fn kind_tag(kind: &ChangeKind) -> u8 {
    match kind {
        ChangeKind::Created => 0,
        ChangeKind::Modified => 1,
        ChangeKind::Deleted => 2,
        ChangeKind::Renamed => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(path: &str) -> ChangeEvent {
        ChangeEvent {
            kind: ChangeKind::Modified,
            path: path.into(),
            from: None,
            modified_at: None,
        }
    }

    #[test]
    fn suppresses_duplicate_within_window() {
        let (tx, mut rx) = broadcast::channel(16);
        let d = Dispatcher::new(tx, Duration::from_secs(5));
        d.send(ev("docs/a.md"));
        d.send(ev("docs/a.md")); // 窗内重复
        assert_eq!(rx.try_recv().unwrap().path, "docs/a.md");
        assert!(rx.try_recv().is_err()); // 第二条被丢弃
    }

    #[test]
    fn passes_distinct_paths() {
        let (tx, mut rx) = broadcast::channel(16);
        let d = Dispatcher::new(tx, Duration::from_secs(5));
        d.send(ev("docs/a.md"));
        d.send(ev("docs/b.md"));
        assert_eq!(rx.try_recv().unwrap().path, "docs/a.md");
        assert_eq!(rx.try_recv().unwrap().path, "docs/b.md");
    }

    #[test]
    fn passes_after_window() {
        let (tx, mut rx) = broadcast::channel(16);
        let d = Dispatcher::new(tx, Duration::from_millis(20));
        d.send(ev("docs/a.md"));
        let _ = rx.try_recv();
        std::thread::sleep(Duration::from_millis(40));
        d.send(ev("docs/a.md")); // 窗已过,应放行
        assert_eq!(rx.try_recv().unwrap().path, "docs/a.md");
    }
}
