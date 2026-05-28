//! live-files 核心库:文件扫描、读取、实时监听与对外可见性控制。
//!
//! 对外只暴露一个门面 [`Watcher`],提供三类能力:列文件树([`Watcher::list`])、
//! 读单个文件([`Watcher::read`])、订阅文件变动事件([`Watcher::subscribe`])。
//! server crate 仅把这些能力包装成 HTTP/SSE,核心逻辑全在这里。

mod config;
pub mod events;
mod path;
mod poll;
mod read;
mod scan;
mod visibility;
mod watch;

pub use config::{Config, Root};
pub use events::{ChangeEvent, ChangeKind};
pub use read::FileContent;
pub use scan::{FileNode, FileTree, NodeKind};

pub type Result<T> = std::result::Result<T, Error>;

/// 库的统一错误类型;server 层据此映射到对应的 HTTP 状态码。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("root not found: {0}")]
    RootNotFound(String),
    #[error("path traversal: {0}")]
    PathTraversal(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("file too large: {size} bytes (limit {limit})")]
    TooLarge { size: u64, limit: u64 },
    #[error("not a regular file: {0}")]
    NotAFile(String),
    #[error("invalid glob: {0}")]
    InvalidGlob(String),
    #[error("watch error: {0}")]
    Watch(#[from] notify::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 库门面:持有配置、事件广播通道,以及 notify 与轮询两个监听句柄。
///
/// 两个监听句柄用 `_` 前缀字段持有——只为保活,不直接访问;一旦 `Watcher`
/// 被 drop,监听线程随之停止。
pub struct Watcher {
    config: Config,
    tx: tokio::sync::broadcast::Sender<ChangeEvent>,
    // Mutex 使 Debouncer（含 JoinHandle，!Sync）满足 Arc<Watcher>: Sync
    _watch: std::sync::Mutex<watch::WatchHandle>,
    _poll: Option<poll::PollHandle>,
}

impl Watcher {
    /// 构造并立即启动监听:notify 实时监听始终开启;轮询兜底仅在
    /// `config.poll_interval` 为 `Some` 时启动。
    pub fn new(config: Config) -> Result<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        let handle = watch::start(&config, tx.clone())?;
        let poll = config.poll_interval.map(|_| poll::start(&config, tx.clone()));
        Ok(Self { config, tx, _watch: std::sync::Mutex::new(handle), _poll: poll })
    }

    /// 列出文件树。`root` 为 `None` 时返回所有 root,否则只返回指定 root。
    pub fn list(&self, root: Option<&str>) -> Result<Vec<FileTree>> {
        scan::list(&self.config, root)
    }

    /// 读取单个文件内容,`logical_path` 为逻辑路径「root 名/相对路径」。
    pub fn read(&self, logical_path: &str) -> Result<FileContent> {
        read::read(&self.config, logical_path)
    }

    /// 订阅文件变动事件,供 SSE 等长连接消费(每个订阅者各持一个接收端)。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ChangeEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Watcher) {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.md"), "# Hello").unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        fs::write(dir.path().join("notes/todo.md"), "- Task 1").unwrap();
        fs::write(dir.path().join("secret.txt"), "secret").unwrap();

        let config = Config {
            roots: vec![Root {
                name: "test".into(),
                path: dir.path().to_path_buf(),
                read_only: true,
                expose: vec![],
            }],
            ..Config::default()
        };
        (dir, Watcher::new(config).unwrap())
    }

    #[test]
    fn list_returns_md_files_only() {
        let (_dir, watcher) = setup();
        let trees = watcher.list(None).unwrap();
        assert_eq!(trees.len(), 1);
        assert_eq!(count_files(&trees[0].tree), 2); // readme.md + notes/todo.md
    }

    #[test]
    fn list_specific_root() {
        let (_dir, watcher) = setup();
        let trees = watcher.list(Some("test")).unwrap();
        assert_eq!(trees.len(), 1);
    }

    #[test]
    fn list_unknown_root() {
        let (_dir, watcher) = setup();
        assert!(matches!(
            watcher.list(Some("nonexistent")),
            Err(Error::RootNotFound(_))
        ));
    }

    #[test]
    fn read_valid_file() {
        let (_dir, watcher) = setup();
        let content = watcher.read("test/readme.md").unwrap();
        assert_eq!(content.content, "# Hello");
        assert_eq!(content.path, "test/readme.md");
    }

    #[test]
    fn read_nested_file() {
        let (_dir, watcher) = setup();
        let content = watcher.read("test/notes/todo.md").unwrap();
        assert_eq!(content.content, "- Task 1");
    }

    #[test]
    fn read_rejects_dotdot() {
        let (_dir, watcher) = setup();
        // "../etc/passwd" 这种穿越形式
        assert!(matches!(
            watcher.read("test/../etc/passwd"),
            Err(Error::PathTraversal(_))
        ));
    }

    #[test]
    fn read_rejects_embedded_dotdot() {
        let (_dir, watcher) = setup();
        assert!(matches!(
            watcher.read("test/notes/../../etc/passwd"),
            Err(Error::PathTraversal(_))
        ));
    }

    #[test]
    fn read_unknown_root() {
        let (_dir, watcher) = setup();
        assert!(matches!(
            watcher.read("nonexistent/file.md"),
            Err(Error::RootNotFound(_))
        ));
    }

    #[test]
    fn read_nonexistent_file() {
        let (_dir, watcher) = setup();
        assert!(matches!(
            watcher.read("test/ghost.md"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn read_enforces_size_limit() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("big.md"), "x".repeat(100)).unwrap();
        let config = Config {
            roots: vec![Root {
                name: "r".into(),
                path: dir.path().to_path_buf(),
                read_only: true,
                expose: vec![],
            }],
            max_file_bytes: 10,
            ..Config::default()
        };
        let watcher = Watcher::new(config).unwrap();
        assert!(matches!(watcher.read("r/big.md"), Err(Error::TooLarge { .. })));
    }

    fn count_files(nodes: &[FileNode]) -> usize {
        nodes
            .iter()
            .map(|n| match &n.children {
                Some(children) => count_files(children),
                None => 1,
            })
            .sum()
    }
}
