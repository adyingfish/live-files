//! 基于 notify+debouncer 的实时文件监听。
//!
//! 每个配置的 root 以递归模式被监听;事件经去抖后转为 [`ChangeEvent`]
//! 并通过 broadcast channel 推送给所有订阅者。

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{RecommendedWatcher, Watcher as _};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap,
};
use tokio::sync::broadcast;

use crate::{Config, Result, events::{ChangeEvent, ChangeKind}, visibility};

/// 去抖监听器的持有句柄,drop 时自动停止。
pub type WatchHandle = Debouncer<RecommendedWatcher, FileIdMap>;

/// root 的内部表示:预存 canonical 路径以避免每次匹配时重新计算。
struct CanonicalRoot {
    name: String,
    canonical: PathBuf,
    expose: Vec<String>,
}

/// 启动 notify 监听,在独立线程中运行去抖回调,事件通过 `tx` 广播。
pub fn start(config: &Config, tx: broadcast::Sender<ChangeEvent>) -> Result<WatchHandle> {
    let canonical_roots: Vec<CanonicalRoot> = config
        .roots
        .iter()
        .filter_map(|r| {
            r.path.canonicalize().ok().map(|p| CanonicalRoot {
                name: r.name.clone(),
                canonical: p,
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

    let mut debouncer = new_debouncer(config.debounce, None, move |result| {
        dispatch(
            result,
            &canonical_roots,
            &tx,
            &include_extensions,
            &ignore_set,
            include_glob_set.as_ref(),
        );
    })?;

    for root in &config.roots {
        debouncer.watcher().watch(&root.path, notify::RecursiveMode::Recursive)?;
    }

    Ok(debouncer)
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

fn dispatch(
    result: DebounceEventResult,
    canonical_roots: &[CanonicalRoot],
    tx: &broadcast::Sender<ChangeEvent>,
    include_extensions: &[String],
    ignore_set: &GlobSet,
    include_glob_set: Option<&GlobSet>,
) {
    let events = match result {
        Ok(events) => events,
        Err(errors) => {
            for e in errors {
                eprintln!("watch error: {e}");
            }
            return;
        }
    };

    for event in &events {
        if let Some(change) = to_change_event(event, canonical_roots) {
            if is_visible(&change.path, canonical_roots, include_extensions, ignore_set, include_glob_set) {
                let _ = tx.send(change);
            }
        }
    }
}

fn is_visible(
    logical_path: &str,
    canonical_roots: &[CanonicalRoot],
    include_extensions: &[String],
    ignore_set: &GlobSet,
    include_glob_set: Option<&GlobSet>,
) -> bool {
    let Some(slash) = logical_path.find('/') else { return false };
    let root_name = &logical_path[..slash];
    let relative = &logical_path[slash + 1..];
    let Some(cr) = canonical_roots.iter().find(|r| r.name == root_name) else { return false };
    visibility::is_exposed(&cr.expose, relative, include_extensions, ignore_set, include_glob_set)
}

/// notify 事件 → [`ChangeEvent`],同时将物理路径转换为逻辑路径。
/// 仅处理内容相关的变动(Create/Remove/Modify/Rename),元数据变更不推送。
fn to_change_event(
    event: &DebouncedEvent,
    canonical_roots: &[CanonicalRoot],
) -> Option<ChangeEvent> {
    use notify::EventKind::*;
    use notify::event::{ModifyKind, RenameMode};

    let (kind, path, from) = match event.kind {
        Create(_) => {
            let p = logical(event.paths.first()?, canonical_roots)?;
            (ChangeKind::Created, p, None)
        }
        Remove(_) => {
            let p = logical(event.paths.first()?, canonical_roots)?;
            (ChangeKind::Deleted, p, None)
        }
        // debouncer-full 在 rename 时将 from+to 放在同一事件,paths = [from, to]
        Modify(ModifyKind::Name(RenameMode::Both)) => {
            let from = logical(event.paths.first()?, canonical_roots)?;
            let to = logical(event.paths.get(1)?, canonical_roots)?;
            (ChangeKind::Renamed, to, Some(from))
        }
        // 只收到 From(To 丢失)→按删除处理
        Modify(ModifyKind::Name(RenameMode::From)) => {
            let p = logical(event.paths.first()?, canonical_roots)?;
            (ChangeKind::Deleted, p, None)
        }
        // 只收到 To(From 丢失)→按创建处理
        Modify(ModifyKind::Name(RenameMode::To)) => {
            let p = logical(event.paths.first()?, canonical_roots)?;
            (ChangeKind::Created, p, None)
        }
        Modify(ModifyKind::Data(_) | ModifyKind::Any | ModifyKind::Other) => {
            let p = logical(event.paths.first()?, canonical_roots)?;
            (ChangeKind::Modified, p, None)
        }
        // 仅元数据/Access/其他 → 不推送
        _ => return None,
    };

    Some(ChangeEvent { kind, path, from, modified_at: None })
}

/// 物理路径 → 逻辑路径「root 名/相对路径」。不匹配任何 root 时返回 `None`。
fn logical(physical: &Path, canonical_roots: &[CanonicalRoot]) -> Option<String> {
    for cr in canonical_roots {
        if let Ok(rel) = physical.strip_prefix(&cr.canonical) {
            return Some(format!("{}/{}", cr.name, rel.to_string_lossy().replace('\\', "/")));
        }
    }
    None
}
