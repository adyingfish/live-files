//! `SystemTime` 字段的 JSON 序列化:统一输出 RFC 3339 (ISO 8601) 字符串
//! (如 `2026-05-29T10:20:30Z`),取代 serde 默认的 `{secs_since_epoch,..}` 结构。
//! 见 docs/openclaw-finder-api.md:`modifiedAt` 约定为 ISO 8601。

use std::time::SystemTime;

use serde::{ser::Error as _, Serializer};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// 序列化 `Option<SystemTime>`:`Some` → RFC 3339 字符串,`None` → null。
pub(crate) fn serialize_opt<S: Serializer>(
    value: &Option<SystemTime>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(t) => {
            let s = OffsetDateTime::from(*t)
                .format(&Rfc3339)
                .map_err(S::Error::custom)?;
            serializer.serialize_str(&s)
        }
        None => serializer.serialize_none(),
    }
}
