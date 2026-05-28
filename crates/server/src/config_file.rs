//! config.toml 解析:所有字段均为可选,缺失时退回 CLI/环境变量或硬编码默认值。

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// config.toml 的反序列化目标。所有字段可选,缺失值由 CLI/环境变量/硬编码默认值回退。
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub port: Option<u16>,
    pub include_ext: Option<Vec<String>>,
    pub include_globs: Option<Vec<String>>,
    pub ignore_globs: Option<Vec<String>>,
    pub poll_interval_ms: Option<u64>,
    pub debounce_ms: Option<u64>,
    pub max_file_bytes: Option<u64>,
    #[serde(default)]
    pub roots: Vec<RootEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootEntry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub read_only: bool,
    #[serde(default)]
    pub expose: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// 加载并解析 config.toml。文件不存在时返回空配置(全部字段取默认值)。
pub fn load(path: &Path) -> anyhow::Result<ConfigFile> {
    if !path.exists() {
        return Ok(ConfigFile::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid config {}: {e}", path.display()))
}
