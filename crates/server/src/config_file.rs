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
    /// supervisor 上报服务的鉴权/地址配置,缺省则不向 supervisor 转发。
    pub supervisor: Option<SupervisorConfig>,
}

/// supervisor 文件变化上报接口的配置。文件变动会被转发到
/// `{base_url}{events_path}`,并带 `Authorization: Bearer <access_token>`。
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SupervisorConfig {
    /// supervisor 服务基址,例如 `https://supervisor.example.com`。
    pub base_url: String,
    /// 上报路径,默认 `/api/v1/finder/events`。
    #[serde(default = "default_events_path")]
    pub events_path: String,
    /// 调用上报接口的 API access token,作为 Bearer 凭证。
    pub access_token: String,
    /// 是否启用上报,默认 true。设为 false 可在保留配置的同时临时关闭转发。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_events_path() -> String {
    "/api/v1/finder/events".to_string()
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
