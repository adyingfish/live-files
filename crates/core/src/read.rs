//! 单个文件内容读取,含可见性检查、大小限制与 ETag 生成。

use std::time::SystemTime;

use serde::Serialize;

use crate::{Config, Error, Result, path, scan, visibility};

/// 单个文件的读取结果,对应 `GET /api/file` 的响应体。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: String,
    /// 文件在磁盘上的字节数(非 UTF-8 字符数)。
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<SystemTime>,
    /// ETag 值(带引号),如 `"\"1234567890\""`,用于 HTTP `If-None-Match` 条件请求。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// 读取指定逻辑路径的文件内容。执行流程:
/// 1. 解析逻辑路径 → root 名 + 相对路径
/// 2. `resolve`(拒绝 `..`)→ `canonicalize` → `assert_within`(防符号链接绕过)
/// 3. 可见性检查(§5.5)
/// 4. 大小上限检查 → UTF-8 读入 → 生成 ETag
pub fn read(config: &Config, logical_path: &str) -> Result<FileContent> {
    let (root_name, relative) = path::split_logical(logical_path)
        .ok_or_else(|| Error::NotFound(logical_path.to_string()))?;

    let root = config
        .roots
        .iter()
        .find(|r| r.name == root_name)
        .ok_or_else(|| Error::RootNotFound(root_name.to_string()))?;

    let physical = path::resolve(root, relative)?;

    let canonical = physical
        .canonicalize()
        .map_err(|_| Error::NotFound(logical_path.to_string()))?;

    let canonical_root = root.path.canonicalize()?;
    path::assert_within(&canonical_root, &canonical)?;

    // 可见性闸门(§5.5):读内容前先判定,防止猜路径绕过文件树。
    let relative_str = canonical
        .strip_prefix(&canonical_root)
        .map_err(|_| Error::PathTraversal(logical_path.to_string()))?
        .to_string_lossy()
        .replace('\\', "/");

    let ignore_set = scan::build_glob_set(&config.ignore_globs)?;
    let include_glob_set = if config.include_globs.is_empty() {
        None
    } else {
        Some(scan::build_glob_set(&config.include_globs)?)
    };

    if !visibility::is_exposed(
        &root.expose,
        &relative_str,
        &config.include_extensions,
        &ignore_set,
        include_glob_set.as_ref(),
    ) {
        return Err(Error::NotFound(logical_path.to_string()));
    }

    let meta = std::fs::metadata(&canonical)
        .map_err(|_| Error::NotFound(logical_path.to_string()))?;

    if !meta.is_file() {
        return Err(Error::NotAFile(logical_path.to_string()));
    }

    let size = meta.len();
    if size > config.max_file_bytes {
        return Err(Error::TooLarge { size, limit: config.max_file_bytes });
    }

    let content = std::fs::read_to_string(&canonical)?;

    let modified_at = meta.modified().ok();
    let etag = modified_at.and_then(|t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| format!("\"{}\"", d.as_nanos()))
    });

    Ok(FileContent {
        path: logical_path.to_string(),
        content,
        size,
        modified_at,
        etag,
    })
}
