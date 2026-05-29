//! 将文件变动事件转发到 supervisor 的上报接口
//! (`POST {base_url}{events_path}`,见 docs/openclaw-finder-api.md)。
//!
//! 订阅 watcher 的 broadcast 通道,每条 [`ChangeEvent`] 以
//! `{kind,path,from,modifiedAt}` 形式 POST 出去,并带
//! `Authorization: Bearer <access_token>`。自有 SSE(`/api/events`)不受影响。

use std::sync::Arc;

use reqwest::Client;
use tokio::sync::broadcast;

use live_files_core::{ChangeEvent, Watcher};

use crate::config_file::SupervisorConfig;

/// 启动后台转发任务,订阅 watcher 事件并上报到 supervisor。
pub fn spawn(watcher: &Arc<Watcher>, config: SupervisorConfig) {
    let url = format!(
        "{}{}",
        config.base_url.trim_end_matches('/'),
        config.events_path
    );
    let token = config.access_token;
    let mut rx = watcher.subscribe();
    let client = Client::new();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => forward(&client, &url, &token, &event).await,
                // 消费太慢丢了部分事件,记录后继续
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("supervisor forwarder lagged, dropped {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

async fn forward(client: &Client, url: &str, token: &str, event: &ChangeEvent) {
    match client.post(url).bearer_auth(token).json(event).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => eprintln!(
            "supervisor forward failed: {} for {}",
            r.status(),
            event.path
        ),
        Err(e) => eprintln!("supervisor forward error for {}: {e}", event.path),
    }
}
