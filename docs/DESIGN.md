# live-files 设计方案

> 一个平台无关的「本地文件监听 + HTTP/SSE 暴露」服务。核心目标:把一组目录下的文件(默认 Markdown)暴露给远程/本地客户端浏览,并在文件发生变动时实时推送给客户端刷新。

---

## 1. 背景与目标

### 1.1 起源场景
- OpenClaw 跑在 Docker 容器里,其 `workspace` 和 `skills` 目录下有大量 `.md` 文件。
- 有一个网页端客户端,需要:
  1. 看到这两个目录下的所有 `.md` 文件(文件树 + 内容预览)。
  2. 当 OpenClaw 这边文件有变动时,客户端能**实时更新**。

### 1.2 复用目标
不止服务于 OpenClaw,这套服务要能复用到其他场景,例如:
- **Windows 上的 Markdown 编辑器**:监听某个 agent 对 `.md` 文件的修改并实时反映到编辑器界面。

因此设计上**不绑定** OpenClaw、不绑定 Docker、不写死 `workspace/skills` 两个目录、不写死只看 `.md`。

### 1.3 核心能力
| 能力 | 说明 |
|------|------|
| 目录扫描 | 列出一组「命名根目录(roots)」下符合过滤规则的文件,树形结构 |
| 文件读取 | 按路径读取单个文件内容(带路径安全校验) |
| 可见性白名单 | 可按 root 配置**纯白名单**(支持文件与目录),只对外开放指定内容;一份白名单对所有接口生效 |
| 实时监听 | 监听文件的 create / modify / delete / rename |
| 实时推送 | 通过 SSE 把变动事件推送给所有连接的客户端 |
| 轮询兜底 | 在 inotify/原生监听失效的环境下,用定时 mtime 扫描保证最终一致 |
| 跨平台 | 同一份代码在 Linux(Docker)和 Windows 原生均可运行 |
| 可嵌入 | 核心逻辑做成库,既能独立部署也能被其他程序内嵌 |

---

## 2. 总体架构

### 2.1 分层:`lib`(core)+ `bin`(server)
把「扫描 + 监听 + 事件广播 + 读取」做成一个**库 crate**,HTTP/SSE 服务只是它的一个**薄壳 binary**。

```
live-files-core (lib)              ← 核心:roots、扫描、notify监听、轮询兜底、事件流、读取
   ├── live-files-server (bin)           ← Docker/远程部署:axum + REST + SSE(调试前端为可选 feature,默认关闭)
   └── md-editor (Windows app,未来) ← 直接 link 这个 lib,进程内订阅事件,无需 HTTP
```

这样两种复用方式都打开:
- **OpenClaw 场景**:用 `live-files-server` 这个 binary,远程客户端走 HTTP/SSE。
- **Windows md 编辑器场景**:
  1. 编辑器**内嵌 core 库**,进程内直接订阅文件事件 —— 零网络、最低延迟。
  2. 或仍跑 `live-files-server` 在 localhost,编辑器走 HTTP/SSE(适合 Tauri/Electron 这类 web 技术栈)。

### 2.2 运行时数据流

```
        ┌─────────────────────────────────────────────┐
        │              live-files-core                │
        │                                              │
   FS   │   ┌──────────┐                               │
 events │   │ notify   │──┐                            │
 ──────────▶│ watcher  │  │                            │
        │   └──────────┘  │   ┌──────────────┐         │
        │                 ├──▶│  debounce    │         │
        │   ┌──────────┐  │   │  + 去重 + 归一 │         │
   mtime│   │ polling  │──┘   └──────┬───────┘         │
 ──────────▶│ fallback │             │                 │
        │   └──────────┘             ▼                 │
        │                  ┌──────────────────┐        │
        │                  │ broadcast channel │       │
        │                  └─────────┬─────────┘        │
        └────────────────────────────┼─────────────────┘
                                      │ 订阅
              ┌───────────────────────┼───────────────────────┐
              ▼                       ▼                       ▼
        SSE 连接 A              SSE 连接 B              进程内订阅者
        (浏览器)               (另一个客户端)          (内嵌的编辑器)
```

---

## 3. 实时推送方案:SSE vs WebSocket

针对本场景(**服务端单向推送「文件变了」的通知**)的对比:

| 维度 | SSE | WebSocket |
|------|-----|-----------|
| 通信方向 | 单向 server→client | 双向 |
| 协议 | 普通 HTTP,`text/event-stream` | 独立协议,需 HTTP Upgrade 握手 |
| 自动重连 | 浏览器原生(`EventSource` 自带,带 `Last-Event-ID`) | 需自己写重连 |
| 客户端代码 | `new EventSource(url)` 几行 | 需管理连接状态/心跳/重连 |
| 经过代理/Nginx | 偶尔需关 buffering,基本无障碍 | 需配置 `Upgrade` 头 |
| 二进制 | 不支持(纯文本) | 支持 |
| 服务端实现(axum) | 一个 `Sse` 响应 + broadcast,简单 | 需处理握手/帧/双向循环 |
| 浏览器同域连接数 | HTTP/1.1 下 6 个;HTTP/2 无限制 | 无此限制 |

**决策:采用 SSE。** 需求是典型的单向通知,客户端收到「某文件变了」后自行重新拉取内容,不需要反向写入或低延迟双向交互。SSE 实现更简单、浏览器原生重连、对调试前端友好。若未来需要客户端反向编辑、协同光标等,再升级到 WebSocket。

---

## 4. API 设计

所有路径对外统一用正斜杠 `/`,且为「root 名 + root 内相对路径」的形式,**不暴露宿主机绝对路径**。

### 4.1 `GET /api/files`
列出各 root 下符合过滤规则的文件(树形)。默认返回**所有 root**;可选 `?root=workspace` 只看某个 root(此时 `roots` 数组只含一个元素)。
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
- 顶层 `roots` 是数组,每个元素为一个 root 的 `{ root, tree }`,对应库接口 `list()` 返回的 `Vec<FileTree>`。
- 客户端从这里即可得到「有哪些 root」及各自的文件(无需单独的 roots 接口)。
- `size`(文件字节数)和 `modifiedAt` 只出现在文件节点(`type: file`)上,目录节点(`type: dir`)没有。

### 4.2 `GET /api/file?path=workspace/notes/todo.md`
读取单个文件内容。
```json
{
  "path": "workspace/notes/todo.md",
  "content": "# Todo\n- ...",
  "size": 1024,
  "modifiedAt": "2026-05-27T10:00:00Z",
  "etag": "\"<hash-or-mtime>\""
}
```
- 支持 `If-None-Match`,命中返回 `304`,减少重复传输。
- `size` 为文件在磁盘上的字节数(`meta.len()`),与 `content` 的字符数不同(UTF-8 下多字节字符会让两者不等)。
- 仅支持文本/代码类文件(md、json、py 等),`content` 按 UTF-8 文本读取;**不支持二进制**(图片、PDF、附件),见 §12。
- 超过大小上限(`max_file_bytes`)的文件返回 `413`,或仅返回元数据。

### 4.3 `GET /api/events`(SSE)
事件流。客户端用 `EventSource` 连接,收到事件后按需重新拉 `/api/files` 或 `/api/file`。

事件格式(`data` 为 JSON):
```
event: change
id: 1716800000123
data: {"kind":"modified","path":"workspace/notes/todo.md","modifiedAt":"2026-05-27T10:00:00Z"}

event: change
data: {"kind":"created","path":"skills/new.md"}

event: change
data: {"kind":"deleted","path":"workspace/old.md"}

: keep-alive 注释行(心跳,防代理断连)
```
- `kind`:`created` | `modified` | `deleted` | `renamed`。
- `renamed` 额外带 `from` / `to`。
- 连接建立时可先推一条 `event: snapshot`(可选),让客户端知道当前全量,避免错过历史。
- 服务端定期发送注释行(`:`)做心跳。

### 4.4 调试前端 `GET /`(可选,仅调试)
一个**单页**调试界面(纯静态),功能:
- 左侧文件树(调 `/api/files`)。
- 右侧 Markdown 预览(调 `/api/file`,前端渲染)。
- 顶部连接 `/api/events`,收到变动时:对应文件高亮 + 自动刷新当前打开的文件。
- 仅用于本地联调验证,**不是正式客户端**。

**编译期开关 + 不进 Docker**:此前端通过 `debug-frontend` feature 在编译期内嵌进 binary,**默认关闭**。
- 本地开发:`cargo run --features debug-frontend`,`GET /` 可用。
- 生产 / Docker 构建:**不启用**该 feature,前端资源不被打包,binary 更小,`GET /` 不存在(只保留 `/api/*`)。详见 §7.1。

### 4.5 向 supervisor 上报文件变动(可选)

除自有 SSE(§4.3)外,文件变动事件还可**转发到 supervisor 的上报接口**,由 supervisor 再广播给它自己的 SSE 客户端(`finder_configs_changed`)。两者并存:**自有 SSE 行为完全不变**,转发只是额外多挂一个订阅者。

- **对接接口**:`POST {base_url}/api/v1/finder/events`(详见 `docs/openclaw-finder-api.md`)。
- **鉴权**:`Authorization: Bearer <access_token>`(supervisor 侧的 API access token)。
- **请求体**:直接是 `ChangeEvent`(§5.3)的序列化,即 `{kind, path, from?, modifiedAt?}`,字段与上报接口一一对应,无需额外转换。
- **实现**:server 端额外起一个后台任务订阅**同一个 broadcast 通道**,每条事件 POST 出去。失败/非 2xx 仅记日志,不重试、不阻塞自有 SSE。
- **配置**:`config.toml` 的 `[supervisor]` 段,缺省(无该段)则完全不转发;`enabled = false` 可保留配置临时关闭。

配置示例:
```toml
[supervisor]
base_url = "https://supervisor.example.com"
access_token = "<api_access_token>"
events_path = "/api/v1/finder/events"  # 可选,默认即此值
enabled = true                          # 可选,默认 true
```

> 注意:转发的 `path` 是 live-files 的逻辑路径「root 名/相对路径」(如 `docs/foo.md`)。上报接口当前对 `path` 原样透传、不做枚举校验。

---

## 5. 核心模块设计(`live-files-core`)

### 5.1 配置模型
```rust
struct Root {
    name: String,        // 对外暴露的逻辑名,如 "workspace"
    path: PathBuf,        // 宿主机实际路径(canonicalized)
    read_only: bool,
    expose: Vec<String>,  // 纯白名单:空=回退全局过滤规则;非空=只放行这些条目(支持文件与目录)。详见 §5.5
}

struct Config {
    roots: Vec<Root>,
    include_extensions: Vec<String>,  // 默认 ["md"];空表示全部
    include_globs: Vec<String>,       // 可选,更细粒度过滤
    ignore_globs: Vec<String>,        // 如 ["**/.git/**","**/node_modules/**"]
    poll_interval: Option<Duration>,  // None=关闭轮询兜底;默认 Some(3s)
    debounce: Duration,               // 事件去抖,默认 300ms
    max_file_bytes: u64,              // 读取/内容返回上限
    follow_symlinks: bool,            // 默认 false,避免越界
}
```
配置来源支持三种(优先级 CLI > env > 配置文件):
- CLI 参数(`clap`)
- 环境变量(Docker 注入,如 `LF_ROOTS=workspace=/data/workspace,skills=/data/skills`)
- 配置文件(`config.toml`)

### 5.2 监听:`notify` + 轮询兜底
- 用 `notify`(配合 `notify-debouncer-full`)监听**目录**(不是单个文件),关注 create/modify/delete/rename。
  - 监听目录而非文件,可正确处理「写临时文件 → rename 覆盖」的原子保存(否则 watch 挂在旧 inode 上会漏事件)。
- 同时起一个低频任务(默认每 3s)扫描各 root 的文件 mtime/size 快照做 diff,作为**兜底**。
- 两路事件都先经过**去抖 + 去重 + 路径归一化**,再投递到同一个 `tokio::sync::broadcast`。
- inotify/原生监听可用时走它(低延迟);失效环境靠轮询保证最终一致。可分别开关。

### 5.3 事件类型
```rust
enum ChangeKind { Created, Modified, Deleted, Renamed }

struct ChangeEvent {
    kind: ChangeKind,
    path: String,                 // "root名/相对路径"
    from: Option<String>,         // renamed 时的旧路径
    modified_at: Option<SystemTime>,
}
```

### 5.4 公开接口(库 API)
```rust
impl Watcher {
    fn new(config: Config) -> Result<Self>;
    fn list(&self, root: Option<&str>) -> Result<Vec<FileTree>>;  // None=所有 root;Some=只该 root(长度 1)
    fn read(&self, logical_path: &str) -> Result<FileContent>;
    fn subscribe(&self) -> broadcast::Receiver<ChangeEvent>;  // 进程内订阅
}
```
`live-files-server` binary 只是把 `list/read` 包成 REST、把 `subscribe` 包成 SSE。Windows 编辑器可直接 `subscribe()`。

### 5.5 对外可见性:白名单闸门(allowlist)

「对外开放哪些文件」必须**只有一个真相来源**:一个统一的可见性判断函数,所有会暴露文件存在性或内容的入口都必须先过它。绝不在单个接口里各写一套过滤——否则容易出现「list 看不到、read 却读得到」这类闸门不一致的安全洞。

```rust
/// 该逻辑路径是否对外可见。综合 root 的 expose 白名单与全局过滤规则。
/// 注意:此判断叠加在路径安全(§6)之上,路径安全永远先行且独立生效。
fn is_exposed(config: &Config, root: &Root, relative: &str) -> bool;
```

#### 必须过闸的入口(共 3 处)

| 入口 | 作用 | 不过滤的后果 |
|------|------|--------------|
| `list` / `GET /api/files` | 决定文件树里出现哪些文件 | 白名单外文件被列出 |
| `read` / `GET /api/file` | 决定能读哪个文件内容 | **猜路径即可读到**(最严重) |
| `subscribe` / `GET /api/events`(SSE) | 决定推不推某文件的变动事件 | 白名单外文件一改,事件里的 `path` 就泄露其存在 |

事件路径特别容易遗漏:SSE 推「哪个文件变了」,若不过滤,白名单外文件被修改时其路径会随事件泄露——等于白名单在事件流上开了口子。归一化后、投进 broadcast 前必须丢弃不可见路径的事件。

#### 语义:纯白名单(per-root)

- `expose` **为空**:回退到全局过滤规则(`include_extensions` / `include_globs` / `ignore_globs`),即默认行为。
- `expose` **非空**:该 root 进入**纯白名单**模式——**只有命中白名单的条目才放行,全局的扩展名/include/ignore 规则对该 root 不再参与判断**。语义最简单、最不易出错,适合「只对外开放固定几个文件」的安全场景。

#### 条目支持文件与目录

`expose` 的每一条都是一个相对 root 的路径条目:

| 条目类型 | 例子 | 放行范围 |
|----------|------|----------|
| 文件 | `readme.md`、`notes/todo.md` | 精确放行该文件 |
| 目录 | `notes`、`docs/api` | 放行该目录**及其下所有内容(递归)** |

> 实现可用 globset:目录条目展开为「该目录本身 + `条目/**`」两条匹配即可;文件条目为精确匹配。具体留待开发阶段,本文档只约定语义。

示例:`workspace` 只对外开放 7 项(几个文件 + 一整个子目录):
```rust
Root {
    name: "workspace".into(),
    path: "/data/workspace".into(),
    read_only: true,
    expose: vec![
        "readme.md".into(),
        "notes/todo.md".into(),
        "docs".into(),        // 整个 docs 目录递归放行
        // ...
    ],
}
```

#### 与路径安全的关系

白名单是**额外一层**,叠加在 §6 的路径安全之上,不替代它。无论 `expose` 怎么配,`..` 拒绝、`canonicalize`、within-root 前缀校验、`follow_symlinks=false` 始终先行且独立生效。顺序为:先过路径安全 → 再过 `is_exposed`。

---

## 6. 路径安全(必做)

`/api/file?path=...` 和库的 `read()` 必须防目录穿越:
1. 解析 `path` 为 `root名` + `相对路径`。
2. 在该 root 的 canonical 根上拼接,再 `canonicalize`。
3. 校验结果仍以该 root 的 canonical 根为前缀;否则拒绝(`403`)。
4. 默认 `follow_symlinks=false`,防止软链指向 root 之外。
5. 可见性判断(扩展名/glob/ignore + per-root 白名单)统一走 §5.5 的 `is_exposed` 闸门,在 list、read、events 三处入口都生效,避免泄露非预期文件。

---

## 7. 部署

### 7.1 OpenClaw 场景:单独容器(sidecar)+ 共享 volume(推荐)

```
┌─────────────────┐     ┌──────────────────┐
│  OpenClaw 容器   │     │ live-files-server 容器 │
│                 │     │  (axum + notify)  │
└────────┬────────┘     └─────────┬────────┘
         │   挂载同一 volume(只读)  │
         └──────────┬──────────────┘
                    ▼
       named volume / 宿主目录(workspace + skills)
```

`docker-compose.yml`(示意):
```yaml
services:
  openclaw:
    image: openclaw:latest
    volumes:
      - claw-data:/data            # workspace、skills 落在这里

  live-files-server:
    build: .
    environment:
      LF_ROOTS: "workspace=/data/workspace,skills=/data/skills"
      LF_INCLUDE_EXT: "md"
      LF_POLL_INTERVAL_MS: "3000"
    volumes:
      - claw-data:/data:ro         # 只读挂载,live-files-server 不写文件
    ports:
      - "8080:8080"

volumes:
  claw-data:
```

`Dockerfile`(多阶段,产物为静态小镜像):
```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
# 不带 --features debug-frontend:生产构建不内嵌调试前端
RUN cargo build --release --bin live-files-server

FROM debian:bookworm-slim
COPY --from=build /app/target/release/live-files-server /usr/local/bin/
EXPOSE 8080
ENTRYPOINT ["live-files-server"]
```

> **调试前端不进 Docker**:`debug-frontend` 默认关闭,生产构建不传该 feature,因此镜像里**不含**前端静态资源,`GET /` 不存在,只暴露 `/api/*`(见 §4.4)。这样镜像更小、对外面积更窄。需要在容器里联调时,再单独构建带 feature 的镜像即可。

**优点**:职责清晰、独立重启、互不拖累、镜像小。
**前提**:见下文 inotify 注意事项。

### 7.2 Windows 原生场景
- 方式一:md 编辑器**内嵌 `live-files-core` 库**,进程内 `subscribe()`,无需 HTTP。
- 方式二:把 `live-files-server` 当 localhost 服务跑,编辑器走 HTTP/SSE(适合 Tauri/Electron)。
- Windows 用 `ReadDirectoryChangesW`,能感知**本机任意进程**对目录的修改 —— agent 改文件、编辑器收到事件,可靠且无跨内核问题。

---

## 8. 文件监听的可靠性边界(inotify / ReadDirectoryChangesW 的限制)

监听依赖操作系统机制,只能感知「同一内核、同一文件系统视图」内的改动。以下情况**原生监听可能不触发**,需轮询兜底:

| 场景 | 原生监听 | 说明 |
|------|----------|------|
| Linux 宿主 + 同宿主容器共享 volume/bind mount | ✅ 可靠 | 共享宿主内核,生产最常见 |
| Docker Desktop(Mac/Win)bind mount | ⚠️ 可能失效 | 改动在虚拟机外,容器内 inotify 挂在虚拟机内核,跨边界不翻译 |
| NFS / SMB / 网络盘 | ❌ 不支持 | inotify/RDC 不支持网络文件系统,另一台机器的改动本机内核不知道 |
| WSL2 跨边界(Win 本体 ↔ WSL2 内文件) | ⚠️ 可能失效 | 同样是跨虚拟机边界 |
| 原子写(写临时文件 + rename 覆盖) | ⚠️ 易漏 | 监听单文件会挂在旧 inode;**改为监听目录**即可解决 |

**应对**:始终监听目录而非单文件;始终开启轮询兜底(可配间隔);两路事件合流。这样无论部署在哪种环境都不会出现「客户端不刷新」。

---

## 9. 技术栈

| 用途 | 选型 |
|------|------|
| 异步运行时 | `tokio` |
| HTTP 框架 | `axum` |
| 文件监听 | `notify` + `notify-debouncer-full` |
| 事件广播 | `tokio::sync::broadcast` |
| 序列化 | `serde` / `serde_json` |
| 配置/CLI | `clap` + 环境变量 + `toml` |
| glob 过滤 | `globset` |
| 日志 | `tracing` / `tracing-subscriber` |
| 错误处理 | `anyhow`(bin)/ `thiserror`(lib) |
| 调试前端 | 内嵌静态单页(`rust-embed`),`debug-frontend` feature 编译期开关,默认关闭、不进生产/Docker |

---

## 10. 仓库结构(规划)

```
live-files/
├── Cargo.toml                 # workspace 根
├── DESIGN.md                  # 本文档
├── crates/
│   ├── core/                  # live-files-core 库
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── config.rs      # Root / Config
│   │   │   ├── scan.rs        # 目录扫描 + 过滤 + 文件树
│   │   │   ├── watch.rs       # notify 监听
│   │   │   ├── poll.rs        # 轮询兜底
│   │   │   ├── events.rs      # ChangeEvent / 去抖去重 / broadcast
│   │   │   ├── read.rs        # 读取 + 路径安全
│   │   │   └── path.rs        # 逻辑路径 <-> 物理路径 + 校验
│   │   └── Cargo.toml
│   └── server/                # live-files-server 二进制
│       ├── src/
│       │   ├── main.rs
│       │   ├── api.rs         # REST handlers
│       │   ├── sse.rs         # SSE handler
│       │   ├── config_file.rs # config.toml 解析(含 [supervisor])
│       │   ├── supervisor.rs  # 文件变动转发到 supervisor 上报接口
│       │   └── assets.rs      # 调试前端静态资源服务(嵌入 tests/frontend/dist/)
│       └── Cargo.toml
├── tests/                     # 调试/联调资产(根 .gitignore 整体忽略)
│   ├── frontend/              # 调试前端(Vite/React,debug-frontend 嵌入其 dist/)
│   └── mock-supervisor/       # mock Supervisor 后端(finder_configs_changed SSE 联调)
├── Dockerfile
└── docker-compose.yml
```

---

## 11. 里程碑

1. **M1 — core 基础**:Config/Root、目录扫描 + 过滤、文件树、安全读取。单元测试覆盖路径穿越。
2. **M2 — 监听**:notify 监听目录 + 去抖,事件归一化进 broadcast。
3. **M3 — 轮询兜底**:mtime 快照 diff,与 M2 合流;可配开关。
4. **M4 — server**:axum 暴露 `/api/files`、`/api/file`、`/api/events`(SSE)。
5. **M5 — 调试前端**:`debug-frontend` feature 下内嵌单页,文件树 + 预览 + 实时刷新;默认关闭,生产/Docker 不打包。
6. **M6 — 部署**:Dockerfile + docker-compose,Linux 宿主联调验证 inotify。
7. **M7 — 复用验证**:在 Windows 上以库方式 `subscribe()` 跑通最小示例(为 md 编辑器铺路)。

---

## 12. 待确认 / 可选项

- **文件类型范围(已定)**:只支持文本/代码类文件(md、json、py、toml、yaml 等),通过 `include_extensions` 配置(默认 `["md"]`)。**不支持**二进制(图片、PDF、附件等)——因此 `content` 恒为 UTF-8 文本字符串,无需 base64 或独立的 raw 字节接口。
- 是否需要鉴权(token / 局域网限制)?当前假设部署在可信网络;如需公网暴露,加一层 token 校验。
- SSE 连接建立时是否要推全量 `snapshot`?默认不推,客户端连上后自行拉一次 `/api/files`;若客户端希望「连上即同步」可开启。
- 是否需要文件内容的增量 diff 推送?当前只推「变了哪个文件」,内容由客户端按需重新拉取(更简单、更省服务端)。
