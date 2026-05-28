//! SSE(Server-Sent Events)端点:将 broadcast channel 的 [`ChangeEvent`]
//! 转为 `text/event-stream` 格式持续推送给客户端,含 15s 心跳。

use std::convert::Infallible;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::Stream;
use tokio::sync::broadcast;

use live_files_core::ChangeEvent;

use crate::AppState;

/// GET /api/events — SSE 事件流,订阅文件变动。15s 心跳保活。
pub async fn events(
    State(watcher): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = watcher.subscribe();
    Sse::new(to_sse_stream(rx)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn to_sse_stream(
    rx: broadcast::Receiver<ChangeEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    let id = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis().to_string())
                        .unwrap_or_default();
                    return Some((Ok(Event::default().event("change").id(id).data(data)), rx));
                }
                // 消费者太慢，部分事件被丢弃，继续等下一个
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}
