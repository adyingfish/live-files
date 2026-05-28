//! 统一的对外可见性闸门(§5.5):所有入口(list、read、events)都经此判断。
//!
//! 白名单模式(`expose` 非空):只放行精确匹配或目录前缀命中的条目,跳过全局过滤规则。
//! 全局模式(`expose` 为空):依次按 ignore_globs → include_extensions → include_globs 过滤。

use std::path::Path;

use globset::GlobSet;

/// 判断 `relative`(root 内相对路径,如 `"notes/todo.md"`)是否对外可见。
///
/// 白名单模式(`expose` 非空):只放行精确匹配或目录前缀命中的条目,跳过全局过滤规则。
/// 全局模式(`expose` 为空):依次按 ignore_globs → include_extensions → include_globs 过滤。
pub(crate) fn is_exposed(
    expose: &[String],
    relative: &str,
    include_extensions: &[String],
    ignore_set: &GlobSet,
    include_glob_set: Option<&GlobSet>,
) -> bool {
    if !expose.is_empty() {
        return expose.iter().any(|entry| {
            relative == entry.as_str() || relative.starts_with(&format!("{}/", entry))
        });
    }
    if ignore_set.is_match(relative) {
        return false;
    }
    if !include_extensions.is_empty() {
        let ext = Path::new(relative)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !include_extensions.contains(&ext) {
            return false;
        }
    }
    if let Some(iset) = include_glob_set {
        if !iset.is_match(relative) {
            return false;
        }
    }
    true
}
