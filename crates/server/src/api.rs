//! REST 接口处理函数:`GET /api/files`、`GET /api/file`。

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use live_files_core::{Error as CoreError, FileTree};

use crate::AppState;

/// 统一的错误转换层:将 core 库错误映射到对应的 HTTP 状态码。
pub struct AppError(CoreError);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            CoreError::RootNotFound(_) | CoreError::NotFound(_) => StatusCode::NOT_FOUND,
            CoreError::PathTraversal(_) => StatusCode::FORBIDDEN,
            CoreError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            CoreError::NotAFile(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.0.to_string()).into_response()
    }
}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        AppError(e)
    }
}

// ---- GET /api/files?root=<name> ----

/// `/api/files` 响应体:`roots` 数组,每项是一个 root 的完整文件树。
#[derive(Serialize)]
struct FilesResponse {
    roots: Vec<FileTree>,
}

#[derive(Deserialize)]
pub struct FilesQuery {
    root: Option<String>,
}

/// GET /api/files?root=<name> — 列出文件树
pub async fn files(
    State(watcher): State<AppState>,
    Query(q): Query<FilesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let roots = watcher.list(q.root.as_deref())?;
    Ok(Json(FilesResponse { roots }))
}

// ---- GET /api/file?path=<logical> ----

#[derive(Deserialize)]
pub struct FileQuery {
    path: String,
}

/// GET /api/file?path=<logical> — 读取单个文件内容,支持 ETag 条件请求
pub async fn file(
    State(watcher): State<AppState>,
    Query(q): Query<FileQuery>,
    headers: HeaderMap,
) -> Result<Response<Body>, AppError> {
    let content = watcher.read(&q.path)?;

    let etag = content.etag.clone();

    // If-None-Match:ETag 命中时返回 304,省流量
    if let (Some(etag_val), Some(inm)) = (
        &etag,
        headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()),
    ) {
        if etag_val.as_str() == inm {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let mut resp = Json(content).into_response();
    if let Some(etag_val) = etag {
        if let Ok(val) = etag_val.parse() {
            resp.headers_mut().insert(header::ETAG, val);
        }
    }
    Ok(resp)
}
