//! live-files HTTP/SSE 服务器入口:解析 CLI/配置文件,启动 Watcher,
//! 注册三个 API 路由(及可选的调试前端),监听端口提供 HTTP 服务。

mod api;
#[cfg(feature = "debug-frontend")]
mod assets;
mod config_file;
mod sse;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, Router};
use clap::Parser;
use live_files_core::{Config, Root, Watcher};

pub(crate) type AppState = Arc<Watcher>;

#[derive(Parser)]
#[command(version, about = "live-files HTTP/SSE server")]
struct Cli {
    /// config.toml 路径。CLI/环境变量可覆盖其中的设置。
    #[arg(long, env = "LF_CONFIG", default_value = "config.toml")]
    config: PathBuf,

    /// root 目录,格式"名称=路径"(逗号分隔或传递多次)。
    /// 示例: --roots workspace=/data/workspace,skills=/data/skills
    /// 传入后会覆盖 config.toml 中的 [[roots]]。
    #[arg(long, env = "LF_ROOTS", value_delimiter = ',')]
    roots: Vec<String>,

    /// 要包含的文件扩展名,逗号分隔。覆盖 config.toml 中的 include_ext。
    #[arg(long, env = "LF_INCLUDE_EXT")]
    include_ext: Option<String>,

    /// 监听端口。覆盖 config.toml 中的 port。
    #[arg(long, env = "LF_PORT")]
    port: Option<u16>,

    /// 轮询间隔(毫秒),0=关闭轮询。覆盖 config.toml 中的 poll_interval_ms。
    #[arg(long, env = "LF_POLL_INTERVAL_MS")]
    poll_interval_ms: Option<u64>,
}

fn build_config(cli: Cli) -> anyhow::Result<(Config, u16)> {
    let file = config_file::load(&cli.config)?;

    // Roots 优先级:CLI/环境变量 → config.toml
    let roots: Vec<Root> = if !cli.roots.is_empty() {
        cli.roots
            .iter()
            .map(|s| {
                let (name, path) = s.split_once('=').ok_or_else(|| {
                    anyhow::anyhow!("invalid root (expected name=path): {s}")
                })?;
                Ok(Root {
                    name: name.trim().to_string(),
                    path: PathBuf::from(path.trim()),
                    read_only: true,
                    expose: vec![],
                })
            })
            .collect::<anyhow::Result<_>>()?
    } else if !file.roots.is_empty() {
        file.roots
            .into_iter()
            .map(|r| Root {
                name: r.name,
                path: r.path,
                read_only: r.read_only,
                expose: r.expose,
            })
            .collect()
    } else {
        anyhow::bail!(
            "no roots configured — use --roots or define [[roots]] in config.toml"
        )
    };

    let port = cli.port.or(file.port).unwrap_or(8080);

    let include_extensions = if let Some(ext) = cli.include_ext {
        ext.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if let Some(exts) = file.include_ext {
        exts
    } else {
        vec!["md".to_string()]
    };

    let poll_interval = match cli.poll_interval_ms.or(file.poll_interval_ms) {
        Some(0) => None,
        Some(ms) => Some(Duration::from_millis(ms)),
        None => Some(Duration::from_secs(3)),
    };

    let defaults = Config::default();
    let config = Config {
        roots,
        include_extensions,
        include_globs: file.include_globs.unwrap_or(defaults.include_globs),
        ignore_globs: file.ignore_globs.unwrap_or(defaults.ignore_globs),
        poll_interval,
        debounce: file
            .debounce_ms
            .map(Duration::from_millis)
            .unwrap_or(defaults.debounce),
        max_file_bytes: file.max_file_bytes.unwrap_or(defaults.max_file_bytes),
        ..defaults
    };

    Ok((config, port))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (config, port) = build_config(cli)?;
    let watcher: AppState = Arc::new(Watcher::new(config)?);

    let app = Router::new()
        .route("/api/files", get(api::files))
        .route("/api/file", get(api::file))
        .route("/api/events", get(sse::events))
        .with_state(Arc::clone(&watcher));

    #[cfg(feature = "debug-frontend")]
    let app = app.fallback(assets::serve);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("listening on http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
