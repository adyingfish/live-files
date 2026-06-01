//! 递归扫描 root 目录生成文件树,对外提供 [`list`] 入口。
//!
//! 文件树节点按 `type` 字段区分文件与目录;文件节点含 `path`/`size`/`modifiedAt`,
//! 目录节点仅含 `children`。

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::{Config, Error, Result, config::Root, visibility};

/// 单个 root 的文件树,对应 `GET /api/files` 响应中 `roots[].root` + `roots[].tree`。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTree {
    pub root: String,
    pub tree: Vec<FileNode>,
}

/// 文件树中的单个节点。通过 `type` 字段区分文件(`file`)与目录(`dir`)。
///
/// 文件节点含 `path`/`size`/`modifiedAt`;目录节点含 `children`。两类字段互斥出现。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::time_fmt::serialize_opt"
    )]
    pub modified_at: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Dir,
}

/// 列出文件树。`root_name` 为 `None` 时返回所有 root,否则只返回指定 root。
pub fn list(config: &Config, root_name: Option<&str>) -> Result<Vec<FileTree>> {
    let roots: Vec<&Root> = match root_name {
        Some(name) => {
            let r = config
                .roots
                .iter()
                .find(|r| r.name == name)
                .ok_or_else(|| Error::RootNotFound(name.to_string()))?;
            vec![r]
        }
        None => config.roots.iter().collect(),
    };

    roots.iter().map(|r| scan_root(config, r)).collect()
}

pub(crate) fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p)
            .map_err(|e| Error::InvalidGlob(format!("{p}: {e}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| Error::InvalidGlob(format!("(build): {e}")))
}

fn scan_root(config: &Config, root: &Root) -> Result<FileTree> {
    let canonical_root = root.path.canonicalize()?;
    let ignore_set = build_glob_set(&config.ignore_globs)?;
    let include_glob_set = if config.include_globs.is_empty() {
        None
    } else {
        Some(build_glob_set(&config.include_globs)?)
    };

    let nodes = scan_dir(
        root,
        &canonical_root,
        &canonical_root,
        config,
        &ignore_set,
        include_glob_set.as_ref(),
    )?;
    Ok(FileTree { root: root.name.clone(), tree: nodes })
}

fn scan_dir(
    root: &Root,
    dir: &Path,
    canonical_root: &Path,
    config: &Config,
    ignore_set: &GlobSet,
    include_glob_set: Option<&GlobSet>,
) -> Result<Vec<FileNode>> {
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());

    let mut nodes = Vec::new();

    for entry in entries {
        let path = entry.path();
        // DirEntry::metadata() 不跟随符号链接(lstat 语义),符号链接自然跳过
        let meta = entry.metadata()?;

        let relative = path
            .strip_prefix(canonical_root)
            .map_err(|_| Error::PathTraversal(path.display().to_string()))?;
        let relative_str = relative.to_string_lossy().replace('\\', "/");

        let name = entry.file_name().to_string_lossy().into_owned();

        if meta.is_dir() {
            // 被忽略的目录跳过,避免无谓递归
            if ignore_set.is_match(&relative_str) {
                continue;
            }
            let children = scan_dir(
                root,
                &path,
                canonical_root,
                config,
                ignore_set,
                include_glob_set,
            )?;
            nodes.push(FileNode {
                name,
                kind: NodeKind::Dir,
                path: None,
                size: None,
                modified_at: None,
                children: Some(children),
            });
        } else if meta.is_file() {
            // 统一可见性闸门(§5.5):白名单或全局过滤规则
            if visibility::is_exposed(
                &root.expose,
                &relative_str,
                &config.include_extensions,
                ignore_set,
                include_glob_set,
            ) {
                nodes.push(FileNode {
                    name,
                    kind: NodeKind::File,
                    path: Some(format!("{}/{}", root.name, relative_str)),
                    size: Some(meta.len()),
                    modified_at: meta.modified().ok(),
                    children: None,
                });
            }
        }
    }

    Ok(nodes)
}
