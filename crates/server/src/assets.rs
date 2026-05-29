//! 调试前端静态资源服务(仅 `debug-frontend` feature 启用)。
//! 将 `tests/frontend/dist/` 嵌入二进制,unknown paths 回退到 index.html(SPA fallback)。

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// 通过 `rust-embed` 将 `tests/frontend/dist/` 编译时嵌入二进制。
/// 路径相对 crate 根(crates/server),故用 `../../` 回到仓库根。
#[derive(RustEmbed)]
#[folder = "../../tests/frontend/dist/"]
struct Assets;

/// 回退路由:路径对应到嵌入文件则返回,否则返回 index.html(SPA fallback)。
pub async fn serve(uri: Uri) -> impl IntoResponse {
    let raw = uri.path().trim_start_matches('/');
    let path = if raw.is_empty() { "index.html" } else { raw };

    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(file.data))
                .unwrap()
        }
        // SPA fallback:未知路径返回 index.html
        None => match Assets::get("index.html") {
            Some(file) => Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(file.data))
                .unwrap(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    }
}
