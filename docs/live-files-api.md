# live-files API(前端对接文档)

本地文件监听 + HTTP/SSE 服务。前端用它**浏览一组目录下的文本/代码文件**,并在文件变动时**实时刷新**。

- **Base URL**(默认):`http://localhost:8080`
- 共 3 个接口:`GET /api/files`、`GET /api/file`、`GET /api/events`(SSE)。
- 另有 `GET /`:内置调试页面,联调用,不算正式 API。

---

## 几个先理解的概念

- **root**:服务端配置的「命名根目录」,如 `workspace`、`skills`。前端只看到名字,**看不到宿主机真实路径**。
- **逻辑路径(path)**:统一格式 `root名/相对路径`,正斜杠分隔,例如 `workspace/notes/todo.md`。文件树里每个文件节点的 `path` 可直接拿去调 `/api/file`。
- **只支持文本/代码文件**(md、json、py 等),`content` 一定是 UTF-8 文本字符串,**不会有二进制/图片**。
- **时间格式**:`modifiedAt` 为 ISO 8601 字符串(如 `"2026-05-27T10:00:00Z"`),可能为 `null`。

---

## TypeScript 类型

```typescript
// ---- /api/files 响应 ----
interface FilesResponse {
  roots: RootTree[];
}

interface RootTree {
  root: string;        // root 逻辑名
  tree: FileNode[];
}

type FileNode = FileNodeFile | FileNodeDir;   // 用 type 字段判别

interface FileNodeFile {
  name: string;
  type: "file";
  path: string;                  // 逻辑路径,直接传给 /api/file
  size: number;                  // 文件字节数
  modifiedAt?: string | null;    // ISO 8601
}

interface FileNodeDir {
  name: string;
  type: "dir";
  children: FileNode[];          // 递归
}

// ---- /api/file 响应 ----
interface FileContent {
  path: string;
  content: string;               // UTF-8 文本
  size: number;                  // 文件字节数(非字符数)
  modifiedAt?: string | null;
  etag?: string;
}

// ---- /api/events 的 data 负载 ----
interface ChangeEvent {
  kind: "created" | "modified" | "deleted" | "renamed";
  path: string;
  from?: string;                 // 仅 renamed:旧路径
  modifiedAt?: string | null;
}

// ---- 错误响应 ----
interface ApiError {
  error: string;
  message?: string;
}
```

---

## 1. `GET /api/files` — 列文件树

列出各 root 下的文件(树形)。默认返回所有 root;传 `?root=` 只看某一个。

**Query 参数**

| 参数 | 必填 | 说明 |
|------|------|------|
| `root` | 否 | 只看某个 root,如 `workspace`。省略=全部 root(`roots` 数组含多项) |

**请求示例**

```typescript
const res = await fetch("http://localhost:8080/api/files");
const data: FilesResponse = await res.json();
```

**响应示例(200)**

```json
{
  "roots": [
    {
      "root": "workspace",
      "tree": [
        {
          "name": "notes",
          "type": "dir",
          "children": [
            { "name": "todo.md", "type": "file", "path": "workspace/notes/todo.md",
              "size": 1024, "modifiedAt": "2026-05-27T10:00:00Z" }
          ]
        },
        { "name": "readme.md", "type": "file", "path": "workspace/readme.md",
          "size": 512, "modifiedAt": "2026-05-27T09:30:00Z" }
      ]
    },
    {
      "root": "skills",
      "tree": [
        { "name": "search.md", "type": "file", "path": "skills/search.md",
          "size": 2048, "modifiedAt": "2026-05-26T18:00:00Z" }
      ]
    }
  ]
}
```

**渲染树的小提示**:遍历时用 `node.type` 判别——`"dir"` 递归 `node.children`,`"file"` 是叶子,点击时拿 `node.path` 去调 `/api/file`。

**错误**:`404` — 传了不存在的 `root`。

---

## 2. `GET /api/file` — 读文件内容

**Query 参数**

| 参数 | 必填 | 说明 |
|------|------|------|
| `path` | 是 | 逻辑路径,如 `workspace/notes/todo.md` |

**请求头(可选)**

| 头 | 说明 |
|----|------|
| `If-None-Match` | 上次拿到的 `etag`;内容没变则返回 `304`,省流量 |

**请求示例**

```typescript
const path = "workspace/notes/todo.md";
const res = await fetch(
  `http://localhost:8080/api/file?path=${encodeURIComponent(path)}`
);
const file: FileContent = await res.json();
// file.content 就是文本,可直接渲染(md 自行走 markdown 渲染器)
```

> `path` 记得 `encodeURIComponent`,避免子路径里的 `/` 和特殊字符出问题。

**响应示例(200)**

```json
{
  "path": "workspace/notes/todo.md",
  "content": "# Todo\n- ...",
  "size": 1024,
  "modifiedAt": "2026-05-27T10:00:00Z",
  "etag": "\"<hash-or-mtime>\""
}
```

响应头带 `ETag`,下次请求可放进 `If-None-Match` 做条件请求。

**错误码**

| 状态 | 含义 |
|------|------|
| `304` | 命中 `If-None-Match`,内容未变,无响应体 |
| `400` | 缺少或非法的 `path` |
| `403` | 路径穿越,或文件不在可见范围(白名单/过滤规则挡掉) |
| `404` | root 或文件不存在 |
| `413` | 文件超过大小上限 |

---

## 3. `GET /api/events` — 实时变动(SSE)

文件一变,服务端推一条事件过来。**事件只告诉你「哪个文件变了」,不含内容**——前端收到后按需重新拉 `/api/files` 或 `/api/file`。

用浏览器原生 `EventSource`,自带断线重连:

```typescript
const es = new EventSource("http://localhost:8080/api/events");

es.addEventListener("change", (e) => {
  const evt: ChangeEvent = JSON.parse(e.data);
  switch (evt.kind) {
    case "created":
    case "deleted":
    case "renamed":
      // 结构变了 → 重新拉文件树
      refreshTree();
      break;
    case "modified":
      // 若当前正打开的就是这个文件 → 重新拉内容
      if (evt.path === currentOpenPath) reloadFile(evt.path);
      break;
  }
});

es.onerror = () => {
  // EventSource 会自动重连,通常无需手动处理;按需提示「连接中断」
};
```

**事件原始格式**(SSE 文本流,浏览器会帮你解析,无需手写):

```
event: change
id: 1716800000123
data: {"kind":"modified","path":"workspace/notes/todo.md","modifiedAt":"2026-05-27T10:00:00Z"}

event: change
data: {"kind":"created","path":"skills/new.md"}

: keep-alive
```

要点:

- 事件类型固定为 `change`,所以监听 `addEventListener("change", ...)`(或用 `es.onmessage` 视服务端是否带 `event:` 字段而定,建议用具名 `change`)。
- `evt.kind`:`created` / `modified` / `deleted` / `renamed`;`renamed` 额外带 `from`(旧路径)。
- 以 `:` 开头的是心跳注释行,浏览器自动忽略,前端不用管。
- 断线后 `EventSource` 自动带 `Last-Event-ID` 重连,服务端从断点续推。

---

## 错误响应格式

非 2xx/3xx 时,响应体为 JSON:

```json
{ "error": "PathTraversal", "message": "..." }
```

```typescript
if (!res.ok) {
  const err: ApiError = await res.json();
  // err.error 是类型标识,err.message 是可读描述
}
```

---

## 典型前端流程

1. 连上 `GET /api/events`(`EventSource`),挂好监听。
2. 拉 `GET /api/files` 渲染左侧文件树。
3. 用户点文件 → `GET /api/file?path=...` 渲染右侧内容。
4. 收到 `change` 事件 → 结构变更刷新树、当前文件变更重拉内容。
