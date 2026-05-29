# Finder / OpenClaw API 对接文档

## 公共约定

- Base Path: `/api`
- 请求格式：`Content-Type: application/json`
- 鉴权：接口走 API 现有 `Authenticator` middleware，请求头需携带 API access token。

```http
Authorization: Bearer <api_access_token>
Content-Type: application/json
```

错误响应格式：

```json
{
  "code": 6002,
  "message": "Bad Request"
}
```

## 1. OpenClaw 上报 Finder 文件变化事件

### Endpoint

```http
POST /api/v1/finder/events
```

### 说明

OpenClaw 在 Finder 文件发生变化时调用该接口。API 接收后会广播 `finder_configs_changed` 事件给 Supervisor SSE 在线客户端。

### Request Body

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `kind` | string | 是 | 文件变化类型，建议值：`created`、`modified`、`deleted`、`renamed`。当前后端原样透传，不做枚举校验 |
| `path` | string | 是 | 当前文件路径。后端会 trim，空白字符串会返回 `400` |
| `from` | string | 否 | rename 前路径。`kind = "renamed"` 时必填且不能是空白字符串 |
| `modifiedAt` | string/null | 否 | 文件修改时间，建议 ISO 8601 格式。后端会映射为 SSE payload 的 `modified_at` |

### Request Example

```json
{
  "kind": "modified",
  "path": "/workspace/finder/config.json",
  "modifiedAt": "2026-05-29T10:20:30Z"
}
```

`renamed` 示例：

```json
{
  "kind": "renamed",
  "path": "/workspace/finder/new.json",
  "from": "/workspace/finder/old.json",
  "modifiedAt": "2026-05-29T10:20:30Z"
}
```

### Success Response

```http
HTTP/1.1 200 OK
```

无响应体。成功只表示 API 已接收并发布事件，不保证离线客户端可收到。

### SSE Event

Supervisor SSE 输出标准 SSE frame：

```text
event: finder_configs_changed
data: {"kind":"modified","path":"/workspace/finder/config.json","modified_at":"2026-05-29T10:20:30Z"}

```

`renamed` 示例：

```text
event: finder_configs_changed
data: {"kind":"renamed","path":"/workspace/finder/new.json","from":"/workspace/finder/old.json","modified_at":"2026-05-29T10:20:30Z"}

```

浏览器端消费示例：

```js
source.addEventListener("finder_configs_changed", (event) => {
  const payload = JSON.parse(event.data);
});
```

## 2. Validate supervisor JWT for Finder/OpenClaw SSO

### Endpoint

```http
POST /api/v1/finder/auth/validate
```

### 说明

用于 OpenClaw/Finder 校验 Supervisor JWT 是否有效。

注意：

- Header 中的 `Authorization: Bearer <api_access_token>` 是调用 API 的 access token。
- Body 中的 `token` 是待校验的 Supervisor JWT。

### Request Body

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `token` | string | 是 | 待校验的 Supervisor JWT |

### Request Example

```json
{
  "token": "<supervisor_jwt>"
}
```

### Success Response: Valid Token

```json
{
  "valid": true,
  "administrator_id": 123,
  "role_id": 456,
  "is_super": false,
  "issued_at": 1710000000,
  "expires_at": 1710604800
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `valid` | boolean | 是否有效 |
| `administrator_id` | number | Supervisor 管理员 ID |
| `role_id` | number | 角色 ID |
| `is_super` | boolean | 是否超级管理员 |
| `issued_at` | number | JWT 签发时间，Unix 秒 |
| `expires_at` | number | JWT 过期时间，Unix 秒 |

### Success Response: Invalid Token

无效、过期或校验未通过时返回：

```json
{
  "valid": false
}
```

HTTP 状态码仍为 `200 OK`。

## 常见错误

| HTTP 状态码 | code | 场景 |
| --- | --- | --- |
| `400` | `6002` | JSON 解析失败、`path` 为空、`renamed` 缺少有效 `from` |
| `401` | `6001` | 缺少或无效的 API `Authorization` |
| `500` | `2000` | 事件总线发布失败、下游服务异常等内部错误 |
