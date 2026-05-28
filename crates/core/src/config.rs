//! 配置模型:运行期的 [`Config`] 与每个根目录的 [`Root`]。
//!
//! server crate 负责从 config.toml / CLI / 环境变量构造出这些结构再传入 core。

use std::path::PathBuf;
use std::time::Duration;

/// 一个对外暴露的「命名根目录」。
pub struct Root {
    /// 对外逻辑名(如 `workspace`),客户端只看到它,看不到宿主机真实路径。
    pub name: String,
    pub path: PathBuf,
    pub read_only: bool,
    /// 按 root 的白名单。空 = 套用全局过滤规则;非空 = 只暴露这些条目。
    /// 每个条目是相对该 root 的路径:文件(精确匹配)或目录(前缀匹配)。详见 §5.5。
    pub expose: Vec<String>,
}

/// 运行期总配置。
pub struct Config {
    pub roots: Vec<Root>,
    /// 空 = 不限制扩展名
    pub include_extensions: Vec<String>,
    pub include_globs: Vec<String>,
    pub ignore_globs: Vec<String>,
    /// `None` = 关闭轮询兜底,只靠 notify 实时监听。
    pub poll_interval: Option<Duration>,
    /// notify 事件去抖窗口,合并短时间内对同一文件的连续变动。
    pub debounce: Duration,
    /// 单个文件读取上限,超过则 [`read`](crate::Watcher::read) 返回 `TooLarge`(HTTP 413)。
    pub max_file_bytes: u64,
    pub follow_symlinks: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            include_extensions: vec!["md".to_string()],
            include_globs: Vec::new(),
            ignore_globs: vec![
                "**/.git/**".to_string(),
                ".git/**".to_string(),
                "**/node_modules/**".to_string(),
            ],
            poll_interval: Some(Duration::from_secs(3)),
            debounce: Duration::from_millis(300),
            max_file_bytes: 10 * 1024 * 1024,
            follow_symlinks: false,
        }
    }
}
