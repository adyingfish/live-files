//! 轮询兜底:在 notify 失效的环境(Docker Desktop、NFS 等)下,
//! 通过定时扫描文件 mtime/size 快照做 diff 来检测文件变动。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::{Config, dispatch::Dispatcher, events::{ChangeEvent, ChangeKind}, visibility};

/// 逻辑路径 → (mtime, 字节数)
type Snapshot = HashMap<String, (SystemTime, u64)>;

struct RootInfo {
    name: String,
    canonical: PathBuf,
    expose: Vec<String>,
}

/// drop 时设置停止标志,轮询线程在下一次 sleep 后退出。
pub struct PollHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for PollHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// 启动轮询线程,按 `config.poll_interval` 间隔扫描文件系统,
/// 将差异事件经 `dispatcher` 去重后广播。返回的 [`PollHandle`] drop 时停止线程。
pub fn start(config: &Config, dispatcher: Arc<Dispatcher>) -> PollHandle {
    let interval = config.poll_interval.expect("poll::start called with poll_interval=None");

    let roots: Vec<RootInfo> = config
        .roots
        .iter()
        .filter_map(|r| {
            r.path.canonicalize().ok().map(|canonical| RootInfo {
                name: r.name.clone(),
                canonical,
                expose: r.expose.clone(),
            })
        })
        .collect();

    let include_extensions = config.include_extensions.clone();
    let ignore_set = build_glob_set(&config.ignore_globs);
    let include_glob_set: Option<GlobSet> = if config.include_globs.is_empty() {
        None
    } else {
        Some(build_glob_set(&config.include_globs))
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let result = std::thread::Builder::new()
        .name("live-files-poll".into())
        .spawn(move || {
            let mut prev = take_snapshot(&roots, &include_extensions, &ignore_set, include_glob_set.as_ref());

            loop {
                std::thread::sleep(interval);
                if stop_flag.load(Ordering::Acquire) {
                    break;
                }
                let curr = take_snapshot(&roots, &include_extensions, &ignore_set, include_glob_set.as_ref());
                for event in diff(&prev, &curr) {
                    dispatcher.send(event);
                }
                prev = curr;
            }
        });

    if let Err(e) = result {
        eprintln!("warning: failed to start poll thread: {e}");
    }

    PollHandle { stop }
}

fn take_snapshot(
    roots: &[RootInfo],
    exts: &[String],
    ignore: &GlobSet,
    include_glob_set: Option<&GlobSet>,
) -> Snapshot {
    let mut snap = HashMap::new();
    for root in roots {
        collect(&root.canonical, &root.canonical, root, exts, ignore, include_glob_set, &mut snap);
    }
    snap
}

fn collect(
    dir: &Path,
    root_path: &Path,
    root_info: &RootInfo,
    exts: &[String],
    ignore: &GlobSet,
    include_glob_set: Option<&GlobSet>,
    snap: &mut Snapshot,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };

        let Ok(relative) = path.strip_prefix(root_path) else { continue };
        let rel_str = relative.to_string_lossy().replace('\\', "/");

        if meta.is_dir() {
            // 被忽略的目录跳过,避免无谓递归
            if ignore.is_match(&rel_str) {
                continue;
            }
            collect(&path, root_path, root_info, exts, ignore, include_glob_set, snap);
        } else if meta.is_file() {
            // 统一可见性闸门(§5.5)
            if visibility::is_exposed(&root_info.expose, &rel_str, exts, ignore, include_glob_set) {
                if let Ok(mtime) = meta.modified() {
                    snap.insert(format!("{}/{}", root_info.name, rel_str), (mtime, meta.len()));
                }
            }
        }
    }
}

/// 对比两次快照,返回差异事件:mtime 或 size 变化→Modified,
/// 新增→Created, 消失→Deleted。
pub(crate) fn diff(prev: &Snapshot, curr: &Snapshot) -> Vec<ChangeEvent> {
    let mut events = Vec::new();

    for (path, &(mtime, size)) in curr {
        match prev.get(path) {
            None => events.push(ChangeEvent {
                kind: ChangeKind::Created,
                path: path.clone(),
                from: None,
                modified_at: Some(mtime),
            }),
            Some(&(prev_mtime, prev_size)) if mtime != prev_mtime || size != prev_size => {
                events.push(ChangeEvent {
                    kind: ChangeKind::Modified,
                    path: path.clone(),
                    from: None,
                    modified_at: Some(mtime),
                });
            }
            _ => {}
        }
    }

    for path in prev.keys() {
        if !curr.contains_key(path) {
            events.push(ChangeEvent {
                kind: ChangeKind::Deleted,
                path: path.clone(),
                from: None,
                modified_at: None,
            });
        }
    }

    events
}

fn build_glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(glob) = Glob::new(p) {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(entries: &[(&str, u64, u64)]) -> Snapshot {
        entries
            .iter()
            .map(|&(path, secs, size)| {
                let mtime = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
                (path.to_string(), (mtime, size))
            })
            .collect()
    }

    #[test]
    fn diff_created() {
        let events = diff(&snap(&[]), &snap(&[("ws/a.md", 100, 10)]));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, ChangeKind::Created));
        assert_eq!(events[0].path, "ws/a.md");
    }

    #[test]
    fn diff_modified_mtime() {
        let events = diff(
            &snap(&[("ws/a.md", 100, 10)]),
            &snap(&[("ws/a.md", 200, 10)]),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, ChangeKind::Modified));
    }

    #[test]
    fn diff_modified_size() {
        let events = diff(
            &snap(&[("ws/a.md", 100, 10)]),
            &snap(&[("ws/a.md", 100, 20)]),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, ChangeKind::Modified));
    }

    #[test]
    fn diff_deleted() {
        let events = diff(&snap(&[("ws/a.md", 100, 10)]), &snap(&[]));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, ChangeKind::Deleted));
        assert_eq!(events[0].path, "ws/a.md");
    }

    #[test]
    fn diff_unchanged() {
        let events = diff(
            &snap(&[("ws/a.md", 100, 10)]),
            &snap(&[("ws/a.md", 100, 10)]),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn diff_mixed() {
        // a.md 修改, b.md 删除, c.md 创建
        let prev = snap(&[("ws/a.md", 100, 10), ("ws/b.md", 100, 20)]);
        let curr = snap(&[("ws/a.md", 200, 10), ("ws/c.md", 100, 5)]);
        let events = diff(&prev, &curr);
        assert_eq!(events.len(), 3);
    }
}
