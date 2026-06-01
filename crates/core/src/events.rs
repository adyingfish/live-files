//! 文件变动事件类型,经 SSE(`/api/events`)推送给客户端。

use std::time::SystemTime;

use serde::Serialize;

/// 变动种类。序列化为小写字符串(`created`/`modified`/`deleted`/`renamed`)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// 单条文件变动事件。事件只说明「哪个文件变了」,不含文件内容。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeEvent {
    pub kind: ChangeKind,
    /// 逻辑路径「root 名/相对路径」。
    pub path: String,
    /// 仅 rename 时出现,表示旧路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::time_fmt::serialize_opt"
    )]
    pub modified_at: Option<SystemTime>,
}
