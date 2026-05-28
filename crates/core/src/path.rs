//! 逻辑路径 ↔ 物理路径转换及目录穿越防护。

use std::path::{Path, PathBuf};

use crate::{Error, Result, config::Root};

/// `"rootname/relative/path"` → `("rootname", "relative/path")`
pub fn split_logical(logical: &str) -> Option<(&str, &str)> {
    logical.split_once('/')
}

/// 将相对路径拼接到 root 的 canonical 路径下,拒绝包含 `..` 的穿越尝试。
/// 调用方在拿到物理路径后还需调用 [`assert_within`] 做二次校验。
pub fn resolve(root: &Root, relative: &str) -> Result<PathBuf> {
    if relative.contains("..") {
        return Err(Error::PathTraversal(relative.to_string()));
    }
    let canonical_root = root.path.canonicalize()?;
    Ok(canonical_root.join(relative))
}

/// 确保已 canonicalize 的 `path` 确实在 `canonical_root` 子树内。
/// 这一步防止符号链接指向 root 目录之外的路径。
pub fn assert_within(canonical_root: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(canonical_root) {
        return Err(Error::PathTraversal(path.display().to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_root(path: &str) -> Root {
        Root { name: "test".into(), path: PathBuf::from(path), read_only: true, expose: vec![] }
    }

    #[test]
    fn split_valid() {
        assert_eq!(split_logical("ws/notes/todo.md"), Some(("ws", "notes/todo.md")));
    }

    #[test]
    fn split_no_slash() {
        assert_eq!(split_logical("workspace"), None);
    }

    #[test]
    fn resolve_rejects_dotdot() {
        let root = dummy_root("/tmp");
        assert!(matches!(resolve(&root, "../etc/passwd"), Err(Error::PathTraversal(_))));
    }

    #[test]
    fn resolve_rejects_embedded_dotdot() {
        let root = dummy_root("/tmp");
        assert!(matches!(resolve(&root, "notes/../../etc/passwd"), Err(Error::PathTraversal(_))));
    }

    #[test]
    fn assert_within_rejects_outside() {
        let root = PathBuf::from("/tmp/test");
        let outside = PathBuf::from("/etc/passwd");
        assert!(matches!(assert_within(&root, &outside), Err(Error::PathTraversal(_))));
    }

    #[test]
    fn assert_within_accepts_inside() {
        let root = PathBuf::from("/tmp/test");
        let inside = PathBuf::from("/tmp/test/subdir/file.md");
        assert!(assert_within(&root, &inside).is_ok());
    }

    #[test]
    fn assert_within_rejects_sibling() {
        // /tmp/test-evil 不能被误判为在 /tmp/test 内
        let root = PathBuf::from("/tmp/test");
        let sibling = PathBuf::from("/tmp/test-evil/file.md");
        assert!(matches!(assert_within(&root, &sibling), Err(Error::PathTraversal(_))));
    }
}
