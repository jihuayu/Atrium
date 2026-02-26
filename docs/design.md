# xtalk - GitHub Issues API 兼容评论系统后端

## 变更记录

| 版本 | 变更内容 | 影响文件 |
|------|---------|---------|
| v0.3 | 新增共享路由层：`router.rs` + `handlers/` 目录，两端共用路由和 handler 逻辑；平台层（`platform/worker/`、`platform/server/`）瘦身为纯类型适配器，删除各自的 `routes.rs`；新增 `matchit`、`bytes` 依赖 | `src/router.rs`（新增）、`src/handlers/`（新增）、`platform/worker/mod.rs`、`platform/server/mod.rs`、`Cargo.toml` |
| v0.2 | `users` 表新增 `email` 字段；`comments` 表将 8 个 reaction 整数列合并为单个 `reactions TEXT`（JSON），创建/删除 reaction 时在 Rust 侧读写 JSON 而非 SQL 算术更新 | `migrations/0001_initial_schema.sql`、`src/services/reaction.rs`、`src/auth.rs` |
| v0.1 | 初始设计：单 crate + feature flags（`worker`/`server`）、Database/HttpClient trait、D1/SQLite 双实现、LRU 评论缓存（server 专属）、完整 Schema 和 API 端点 | 全部文件 |

---

## 1. 概述

xtalk 是一个用 Rust 编写的后端服务，提供与 GitHub Issues REST API 兼容的接口，旨在替代基于 GitHub Issues 的评论系统（Gitalk、Utterances 等）。现有前端**无需修改**，只需将 API 基地址指向 xtalk 即可完成切换。

### 为什么需要 xtalk？

| 问题 | GitHub Issues 方案 | xtalk |
|------|-------------------|-------|
| API Rate Limit | 60次/h（未认证） | 无限制（自有服务） |
| 数据主权 | 存储在 GitHub | 自有数据库 |
| 灵活性 | 受限于 GitHub 功能 | 可自由扩展 |
| 隐私 | 评论公开在仓库 Issues | 独立存储 |

### 核心策略

前端继续发送 `Authorization: token/Bearer {github_token}` 头部，xtalk 用该 token 调用 GitHub API 验证用户身份并缓存，然后使用该身份在本地数据库中执行操作。

## 2. 双部署架构

通过 Cargo feature flags 在**同一个 crate** 中切换编译目标：

```bash
# 容器部署：编译原生二进制
cargo build --release --features server --bin xtalk-server

# Workers 部署：通过 wrangler 编译 WASM
cd deploy/worker && npx wrangler deploy
```

### Feature 对照

```
                        共享层 (始终编译)
  ┌──────────────────────────────────────────────────┐
  │  types.rs    db.rs(trait)    auth.rs(trait)       │
  │  services/    fmt/    error.rs    markdown.rs     │
  └────────────────────────┬─────────────────────────┘
                           │
              ┌────────────┴────────────┐
              │                         │
        feature="worker"          feature="server"
        ┌─────────────────┐    ┌──────────────────┐
        │ D1 Database impl│    │ SqlitePool impl  │
        │ Fetch HttpClient│    │ reqwest Client   │
        │ worker::Router  │    │ axum::Router     │
        │ #[event(fetch)] │    │ main() + tokio   │
        │ → WASM cdylib   │    │ → native binary  │
        └─────────────────┘    └──────────────────┘
```

| 组件 | `--features worker` | `--features server` |
|------|---------------------|---------------------|
| HTTP 框架 | `worker` crate (Router) | `axum` |
| 数据库 | Cloudflare D1 | SQLite (`sqlx`) |
| HTTP 客户端 | `worker::Fetch` | `reqwest` |
| 运行时 | Workers WASM runtime | `tokio` |
| 产物 | WASM cdylib | native binary |

## 3. 项目结构

```
xtalk/
├── Cargo.toml                          # feature 定义、依赖声明
├── migrations/
│   └── 0001_initial_schema.sql         # D1 与 SQLite 共享
│
├── src/
│   ├── lib.rs                          # 公共导出 + #[cfg(feature="worker")] 入口
│   │
│   │  ── 共享层 ──
│   ├── types.rs                        # 领域模型 + GitHub API 响应结构体
│   ├── error.rs                        # ApiError（GitHub 兼容错误格式）
│   ├── db.rs                           # Database trait 定义
│   ├── auth.rs                         # token 解析、哈希、HttpClient trait
│   ├── markdown.rs                     # pulldown-cmark 封装
│   │
│   ├── router.rs                       # AppRequest / AppResponse + 路由分发（matchit）
│   │
│   ├── handlers/                       # 路由处理逻辑（两端共用，取代 platform/*/routes.rs）
│   │   ├── mod.rs
│   │   ├── issues.rs
│   │   ├── comments.rs
│   │   ├── reactions.rs
│   │   ├── labels.rs
│   │   └── search.rs
│   │
│   ├── services/                       # 纯业务逻辑，依赖 db trait
│   │   ├── mod.rs
│   │   ├── repo.rs                     # 仓库自动创建
│   │   ├── issue.rs                    # Issue CRUD + per-repo 编号
│   │   ├── comment.rs                  # Comment CRUD + count 维护
│   │   ├── reaction.rs                 # Reaction 增删
│   │   ├── label.rs                    # Label CRUD
│   │   └── search.rs                   # 搜索查询解析器
│   │
│   ├── fmt/                            # DB row → GitHub API JSON
│   │   ├── mod.rs
│   │   ├── issue.rs
│   │   ├── comment.rs
│   │   ├── user.rs
│   │   └── pagination.rs              # Link header 构建
│   │
│   │  ── 平台层 (feature-gated，仅做适配) ──
│   ├── platform/
│   │   ├── mod.rs                      # cfg 分发
│   │   ├── worker/                     # #[cfg(feature = "worker")]
│   │   │   ├── mod.rs                  # #[event(fetch)]：worker::Request → AppRequest → worker::Response
│   │   │   ├── d1.rs                   # Database trait 的 D1 实现
│   │   │   └── http.rs                 # HttpClient trait 的 worker::Fetch 实现
│   │   └── server/                     # #[cfg(feature = "server")]
│   │       ├── mod.rs                  # axum 单一 catch-all handler → AppRequest → axum Response
│   │       ├── sqlite.rs              # Database trait 的 sqlx::SqlitePool 实现
│   │       ├── http.rs                 # HttpClient trait 的 reqwest 实现
│   │       └── cache.rs               # LRU 评论缓存（moka）
│   │
│   └── bin/
│       └── server.rs                   # server feature 的 main()
│
├── deploy/
│   └── worker/
│       └── wrangler.toml               # Workers 部署配置
│
├── Dockerfile                          # 容器部署
└── docs/
    └── design.md                       # 本文档
```

## 4. Cargo.toml

```toml
[package]
name = "xtalk"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "xtalk-server"
path = "src/bin/server.rs"
required-features = ["server"]

[features]
default = []
worker = ["dep:worker"]
server = ["dep:axum", "dep:sqlx", "dep:tokio", "dep:reqwest", "dep:tower-http", "dep:moka"]

[dependencies]
# ── 共享（始终编译）──
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
pulldown-cmark = { version = "0.12", default-features = false }
chrono = { version = "0.4", features = ["serde"], default-features = false }
async-trait = "0.1"

# ── worker feature ──
worker = { version = "0.4", features = ["d1"], optional = true }

# ── server feature ──
axum = { version = "0.8", optional = true }
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio"], optional = true }
tokio = { version = "1", features = ["full"], optional = true }
reqwest = { version = "0.12", features = ["json"], optional = true }
tower-http = { version = "0.6", features = ["cors"], optional = true }
moka = { version = "0.12", features = ["future"], optional = true }
```

## 5. 核心抽象层

### 5.1 Database Trait

```rust
// src/db.rs

/// 数据库参数值，兼容 D1 和 SQLite
pub enum DbValue {
    Null,
    Integer(i64),
    Text(String),
}

/// 统一数据库接口
/// ?Send：Workers 是单线程，不需要 Send bound
#[async_trait(?Send)]
pub trait Database {
    /// 执行写操作，返回影响行数
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64>;

    /// 查询单行（可选）
    async fn query_opt<T: DeserializeOwned>(
        &self, sql: &str, params: &[DbValue]
    ) -> Result<Option<T>>;

    /// 查询多行
    async fn query_all<T: DeserializeOwned>(
        &self, sql: &str, params: &[DbValue]
    ) -> Result<Vec<T>>;

    /// 批量执行（事务语义），用于 issue 编号的原子分配
    async fn batch(&self, stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()>;
}
```

**关键约束**：所有 SQL 查询统一使用 `?1, ?2` 有序参数语法——这是 D1 和 SQLite 的最大公约数（D1 不支持命名参数）。

### 5.2 HttpClient Trait

```rust
// src/auth.rs

/// GitHub API 客户端抽象
#[async_trait(?Send)]
pub trait HttpClient {
    /// 用 token 调用 GET https://api.github.com/user
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser>;
}
```

- **worker feature**: 使用 `worker::Fetch` 发起请求
- **server feature**: 使用 `reqwest::Client` 发起请求

### 5.3 AppContext

```rust
// src/lib.rs

/// 传递给所有 service 函数的上下文
pub struct AppContext<'a> {
    pub db: &'a dyn Database,
    pub http: &'a dyn HttpClient,
    pub base_url: &'a str,
    pub user: Option<&'a GitHubUser>,
}
```

Service 函数只依赖 `AppContext`，对底层平台完全无感知：

```rust
// src/services/issue.rs
pub async fn create_issue(
    ctx: &AppContext<'_>,
    owner: &str,
    repo: &str,
    input: &CreateIssueInput,
) -> Result<IssueResponse> {
    let user = ctx.user.ok_or(ApiError::unauthorized())?;
    // ... 业务逻辑，调用 ctx.db
}
```

### 5.4 共享路由层（router.rs）

这是新增的关键抽象——在 `xtalk-core` 中定义一个平台无关的路由器，让 worker 和 server 共用所有路由和 handler 逻辑。

#### 数据结构

```rust
// src/router.rs
use std::collections::HashMap;

/// 平台无关的请求表示
pub struct AppRequest {
    pub method: String,                      // "GET" / "POST" / "PATCH" / "DELETE"
    pub path: String,                        // 原始路径，如 "/repos/user/blog/issues/1"
    pub path_params: HashMap<String, String>, // 由路由器匹配后填充
    pub query: HashMap<String, String>,      // 查询参数
    pub auth_header: Option<String>,         // Authorization 头原文
    pub accept: Option<String>,              // Accept 头
    pub body: bytes::Bytes,                  // 请求体
}

/// 平台无关的响应表示
pub struct AppResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,      // (name, value)
    pub body: bytes::Bytes,
}

impl AppResponse {
    pub fn json<T: serde::Serialize>(status: u16, value: &T) -> Self { ... }
    pub fn no_content() -> Self { /* 204 */ }
    pub fn with_header(mut self, k: &str, v: &str) -> Self { ... }
}
```

#### 路由表

```rust
// src/router.rs

use matchit::Router as Matchit;

/// 路由名称枚举，matchit 存储此值作为路由标识
#[derive(Clone)]
enum Route {
    ListIssues, CreateIssue, GetIssue, UpdateIssue, DeleteIssue,
    ListComments, CreateComment, GetComment, UpdateComment, DeleteComment,
    ListReactions, CreateReaction, DeleteReaction,
    ListLabels, CreateLabel,
    SearchIssues,
}

pub struct Router {
    get:    Matchit<Route>,
    post:   Matchit<Route>,
    patch:  Matchit<Route>,
    delete: Matchit<Route>,
}

impl Router {
    pub fn new() -> Self {
        let mut r = Self { ... };

        // 注意：含字面量 "comments" 的路由必须先于 ":number" 注册
        r.get.insert("/repos/:owner/:repo/issues/comments/:id", Route::GetComment).unwrap();
        r.get.insert("/repos/:owner/:repo/issues/:number/comments", Route::ListComments).unwrap();
        r.get.insert("/repos/:owner/:repo/issues/:number", Route::GetIssue).unwrap();
        r.get.insert("/repos/:owner/:repo/issues", Route::ListIssues).unwrap();
        r.get.insert("/repos/:owner/:repo/issues/comments/:id/reactions", Route::ListReactions).unwrap();
        r.get.insert("/repos/:owner/:repo/labels", Route::ListLabels).unwrap();
        r.get.insert("/search/issues", Route::SearchIssues).unwrap();

        r.post.insert("/repos/:owner/:repo/issues", Route::CreateIssue).unwrap();
        r.post.insert("/repos/:owner/:repo/issues/:number/comments", Route::CreateComment).unwrap();
        r.post.insert("/repos/:owner/:repo/issues/comments/:id/reactions", Route::CreateReaction).unwrap();
        r.post.insert("/repos/:owner/:repo/labels", Route::CreateLabel).unwrap();

        r.patch.insert("/repos/:owner/:repo/issues/:number", Route::UpdateIssue).unwrap();
        r.patch.insert("/repos/:owner/:repo/issues/comments/:id", Route::UpdateComment).unwrap();

        r.delete.insert("/repos/:owner/:repo/issues/:number", Route::DeleteIssue).unwrap();
        r.delete.insert("/repos/:owner/:repo/issues/comments/:id", Route::DeleteComment).unwrap();
        r.delete.insert("/repos/:owner/:repo/issues/comments/:id/reactions/:rid", Route::DeleteReaction).unwrap();

        r
    }
}
```

#### 分发入口

```rust
// src/router.rs

impl Router {
    pub async fn handle(&self, mut req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
        let table = match req.method.as_str() {
            "GET"    => &self.get,
            "POST"   => &self.post,
            "PATCH"  => &self.patch,
            "DELETE" => &self.delete,
            "OPTIONS" => return AppResponse::no_content().with_header("Allow", "GET,POST,PATCH,DELETE"),
            _ => return AppResponse::json(405, &json!({"message": "Method Not Allowed"})),
        };

        let matched = match table.at(&req.path) {
            Ok(m) => m,
            Err(_) => return AppResponse::json(404, &json!({"message": "Not Found"})),
        };

        // 将路径参数填入请求
        req.path_params = matched.params.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        // 分发到对应 handler
        match matched.value {
            Route::ListIssues     => handlers::issues::list(req, ctx).await,
            Route::CreateIssue    => handlers::issues::create(req, ctx).await,
            Route::GetIssue       => handlers::issues::get(req, ctx).await,
            Route::UpdateIssue    => handlers::issues::update(req, ctx).await,
            Route::DeleteIssue    => handlers::issues::delete(req, ctx).await,
            Route::ListComments   => handlers::comments::list(req, ctx).await,
            Route::CreateComment  => handlers::comments::create(req, ctx).await,
            // ...
        }
    }
}
```

#### 新增依赖

```toml
# 共享层，不属于任何 feature
matchit = "0.8"
bytes = "1"
```

#### 平台层变为纯适配器

**Worker**（`platform/worker/mod.rs`）：

```rust
#[event(fetch)]
async fn main(req: worker::Request, env: Env, _ctx: worker::Context) -> worker::Result<worker::Response> {
    let app_req = to_app_request(req).await?;    // worker::Request → AppRequest
    let db  = D1Database::new(env.d1("DB")?);
    let http = WorkerHttpClient;
    let user = auth::resolve_user(&db, &http, &app_req.auth_header, ...).await.ok().flatten();
    let ctx = AppContext { db: &db, http: &http, base_url: &base_url, user: user.as_ref() };

    let app_resp = ROUTER.handle(app_req, &ctx).await;

    to_worker_response(app_resp)                 // AppResponse → worker::Response
}
```

**Server**（`platform/server/mod.rs`）：

```rust
// axum 注册一个 catch-all，所有请求都走 Router::handle
async fn handler(State(state): State<AppState>, req: axum::http::Request<Body>) -> impl IntoResponse {
    let app_req = to_app_request(req).await;     // axum Request → AppRequest
    let user = auth::resolve_user(...).await.ok().flatten();
    let ctx = AppContext { db: &state.db, http: &state.http, base_url: &state.base_url, user: user.as_ref() };

    let app_resp = state.router.handle(app_req, &ctx).await;

    to_axum_response(app_resp)                   // AppResponse → axum Response
}

// axum 路由只有一条
let app = axum::Router::new()
    .fallback(handler)
    .layer(CorsLayer::permissive())
    .with_state(state);
```

### 5.5 层级职责总结

```
src/handlers/      ← 所有路由 handler（两端共用）
   ↓ 调用
src/services/      ← 业务逻辑（两端共用）
   ↓ 依赖
src/db.rs          ← Database trait（接口）
   ↑ 实现
platform/worker/d1.rs      (worker feature)
platform/server/sqlite.rs  (server feature)

src/router.rs      ← 路由表 + 分发（两端共用）
   ↑ 调用
platform/worker/mod.rs  → to_app_request / to_worker_response
platform/server/mod.rs  → to_app_request / to_axum_response
```

## 6. 数据库 Schema

```sql
-- migrations/0001_initial_schema.sql
-- D1 和 SQLite 完全兼容

CREATE TABLE users (
    id INTEGER PRIMARY KEY,             -- GitHub user ID（直接作为主键）
    login TEXT NOT NULL UNIQUE,
    email TEXT DEFAULT '',               -- GitHub 用户邮箱（可能为空）
    avatar_url TEXT NOT NULL DEFAULT '',
    type TEXT NOT NULL DEFAULT 'User',   -- 'User' | 'Organization'
    site_admin INTEGER NOT NULL DEFAULT 0,
    cached_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE token_cache (
    token_hash TEXT PRIMARY KEY,         -- SHA-256 hex（不存储原始 token）
    user_id INTEGER NOT NULL REFERENCES users(id),
    cached_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL             -- 默认 TTL 1 小时
);

CREATE TABLE repos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    admin_user_id INTEGER,              -- 仓库管理员的 GitHub user ID
    issue_counter INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner, name)
);

CREATE TABLE issues (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repos(id),
    number INTEGER NOT NULL,             -- per-repo 自增编号
    title TEXT NOT NULL,
    body TEXT,
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','closed')),
    state_reason TEXT,
    locked INTEGER NOT NULL DEFAULT 0,
    user_id INTEGER NOT NULL REFERENCES users(id),
    comment_count INTEGER NOT NULL DEFAULT 0,  -- 反范式化，避免 COUNT 查询
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    closed_at TEXT,
    deleted_at TEXT,                      -- 软删除
    UNIQUE(repo_id, number)
);

CREATE TABLE comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repos(id),
    issue_id INTEGER NOT NULL REFERENCES issues(id),
    body TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at TEXT,
    -- 冗余 reaction 计数，JSON 格式，避免列出评论时 JOIN 聚合
    -- 示例: {"plus_one":1,"heart":2,"total":3}
    -- 缺省键视为 0
    reactions TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE labels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repos(id),
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    color TEXT NOT NULL DEFAULT 'ededed', -- 6位 hex（不含 #）
    UNIQUE(repo_id, name)
);

CREATE TABLE issue_labels (
    issue_id INTEGER NOT NULL REFERENCES issues(id),
    label_id INTEGER NOT NULL REFERENCES labels(id),
    PRIMARY KEY (issue_id, label_id)
);

CREATE TABLE reactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comments(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    content TEXT NOT NULL CHECK(
        content IN ('+1','-1','laugh','confused','heart','hooray','rocket','eyes')
    ),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(comment_id, user_id, content) -- 每人每条评论每种反应只能一次
);

-- 索引
CREATE INDEX idx_issues_repo_state ON issues(repo_id, state, deleted_at);
CREATE INDEX idx_issues_repo_number ON issues(repo_id, number);
CREATE INDEX idx_comments_issue ON comments(issue_id, deleted_at);
CREATE INDEX idx_reactions_comment ON reactions(comment_id);
CREATE INDEX idx_token_expires ON token_cache(expires_at);
```

### Schema 设计要点

| 设计决策 | 理由 |
|---------|------|
| `issue_counter` 在 repos 表 | 原子递增实现 per-repo 编号（UPDATE + SELECT in batch） |
| `comment_count` 反范式化 | GitHub API 在 issue 对象中返回评论数，避免每次 COUNT |
| `deleted_at` 软删除 | 支持撤销，审计追踪 |
| `token_hash` 存哈希不存原文 | 安全：即使数据库泄漏也不暴露用户 token |
| `UNIQUE(comment_id, user_id, content)` | 匹配 GitHub 行为：每人每条评论每种 reaction 只能一个 |

## 7. API 端点详情

### 7.1 Issues

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/repos/:owner/:repo/issues` | 可选 | 列出 issues |
| `POST` | `/repos/:owner/:repo/issues` | 必须 | 创建 issue |
| `GET` | `/repos/:owner/:repo/issues/:number` | 可选 | 获取单个 issue |
| `PATCH` | `/repos/:owner/:repo/issues/:number` | 必须 | 更新 issue |

**GET /repos/:owner/:repo/issues 查询参数**：

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `state` | string | `open` | `open` / `closed` / `all` |
| `labels` | string | - | 逗号分隔的标签名 |
| `sort` | string | `created` | `created` / `updated` / `comments` |
| `direction` | string | `desc` | `asc` / `desc` |
| `since` | string | - | ISO 8601 时间戳，只返回此后更新的 |
| `per_page` | int | 30 | 每页数量（max 100） |
| `page` | int | 1 | 页码 |
| `creator` | string | - | 按创建者 login 过滤 |

**POST /repos/:owner/:repo/issues 请求体**：

```json
{
    "title": "string (必须)",
    "body": "string (可选)",
    "labels": ["string"] // 可选，标签名数组
}
```

### 7.2 Comments

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/repos/:owner/:repo/issues/:n/comments` | 可选 | 列出某 issue 的评论 |
| `POST` | `/repos/:owner/:repo/issues/:n/comments` | 必须 | 创建评论 |
| `GET` | `/repos/:owner/:repo/issues/comments/:id` | 可选 | 获取单条评论 |
| `PATCH` | `/repos/:owner/:repo/issues/comments/:id` | 必须 | 编辑评论（仅作者/admin） |
| `DELETE` | `/repos/:owner/:repo/issues/comments/:id` | 必须 | 删除评论（仅作者/admin） |

**评论列表查询参数**：`per_page`, `page`, `since`

**创建/更新评论请求体**：

```json
{ "body": "string (必须)" }
```

### 7.3 Reactions

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `…/comments/:id/reactions` | 可选 | 列出反应 |
| `POST` | `…/comments/:id/reactions` | 必须 | 添加反应（已存在→200，新建→201） |
| `DELETE` | `…/comments/:id/reactions/:rid` | 必须 | 删除自己的反应 |

**创建反应请求体**：

```json
{ "content": "+1" }
```

可选值：`+1`, `-1`, `laugh`, `confused`, `heart`, `hooray`, `rocket`, `eyes`

### 7.4 Search

| Method | Path | 说明 |
|--------|------|------|
| `GET` | `/search/issues?q=...` | 搜索 issues |

**`q` 参数支持的限定符**：

| 限定符 | 示例 | 说明 |
|--------|------|------|
| `repo:` | `repo:user/blog` | 按仓库过滤 |
| `label:` | `label:bug` | 按标签过滤 |
| `is:` | `is:open` / `is:closed` | 按状态过滤 |
| 自由文本 | `hello world` | 在 title 和 body 中搜索（LIKE） |

其他参数：`sort`, `order`, `per_page`, `page`

### 7.5 Labels

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/repos/:owner/:repo/labels` | 可选 | 列出标签 |
| `POST` | `/repos/:owner/:repo/labels` | 必须 | 创建标签 |

## 8. 认证流程

```
┌──────────┐     Authorization: token ghp_xxx      ┌──────────┐
│  前端     │ ──────────────────────────────────────→ │  xtalk   │
│ (Gitalk)  │                                        │          │
└──────────┘                                        └────┬─────┘
                                                         │
                              ┌───────────────────────────┤
                              │                           │
                         缓存命中?                     缓存未命中
                              │                           │
                         ┌────▼────┐              ┌───────▼───────┐
                         │token_   │              │ GET            │
                         │cache 表 │              │ api.github.com │
                         │(SHA-256)│              │ /user          │
                         └────┬────┘              └───────┬───────┘
                              │                           │
                              └───────────┬───────────────┘
                                          │
                                    ┌─────▼─────┐
                                    │ GitHubUser │
                                    │ 身份确认    │
                                    └───────────┘
```

详细步骤：

1. 前端发送 `Authorization: token {github_token}` 或 `Authorization: Bearer {token}`
2. 解析 token → `sha2::Sha256` 哈希
3. 查 `token_cache` 表，如缓存命中且未过期 → 返回关联的 `users` 记录
4. 缓存未命中 → 通过 `HttpClient` trait 调用 `GET https://api.github.com/user`
5. Upsert `users` 表 + 写入 `token_cache`（TTL 默认 1h）
6. 未认证请求可读取公开数据，写操作返回 401

```rust
// src/auth.rs

pub fn parse_token(header: &str) -> Option<&str> {
    let header = header.trim();
    if let Some(t) = header.strip_prefix("token ") {
        Some(t.trim())
    } else if let Some(t) = header.strip_prefix("Bearer ") {
        Some(t.trim())
    } else {
        None
    }
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

pub async fn resolve_user(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    cache_ttl_secs: i64,
) -> Result<GitHubUser> {
    let token_hash = hash_token(token);

    // 1. 查缓存
    if let Some(cached) = db.query_opt::<CachedUser>(
        "SELECT u.* FROM token_cache tc \
         JOIN users u ON tc.user_id = u.id \
         WHERE tc.token_hash = ?1 AND tc.expires_at > datetime('now')",
        &[DbValue::Text(token_hash.clone())],
    ).await? {
        return Ok(cached.into());
    }

    // 2. 调 GitHub API
    let gh_user = http.get_github_user(token).await?;

    // 3. Upsert user + cache token
    db.batch(vec![
        (
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET \
               login=excluded.login, email=excluded.email, \
               avatar_url=excluded.avatar_url, \
               type=excluded.type, cached_at=datetime('now')",
            vec![
                DbValue::Integer(gh_user.id),
                DbValue::Text(gh_user.login.clone()),
                DbValue::Text(gh_user.email.clone().unwrap_or_default()),
                DbValue::Text(gh_user.avatar_url.clone()),
                DbValue::Text(gh_user.r#type.clone()),
                DbValue::Integer(gh_user.site_admin as i64),
            ],
        ),
        (
            "INSERT INTO token_cache (token_hash, user_id, cached_at, expires_at) \
             VALUES (?1, ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds')) \
             ON CONFLICT(token_hash) DO UPDATE SET \
               user_id=excluded.user_id, cached_at=datetime('now'), \
               expires_at=excluded.expires_at",
            vec![
                DbValue::Text(token_hash),
                DbValue::Integer(gh_user.id),
                DbValue::Integer(cache_ttl_secs),
            ],
        ),
    ]).await?;

    Ok(gh_user.into())
}
```

## 9. 响应格式

### Issue 对象

```json
{
    "id": 1,
    "node_id": "eHRhbGs6SXNzdWU6MQ==",
    "number": 1,
    "title": "页面标题",
    "body": "评论内容（原始 Markdown）",
    "body_html": "<p>评论内容（渲染后 HTML）</p>",
    "state": "open",
    "locked": false,
    "user": {
        "login": "octocat",
        "id": 1,
        "avatar_url": "https://avatars.githubusercontent.com/u/1?v=4",
        "html_url": "https://github.com/octocat",
        "type": "User"
    },
    "labels": [
        {
            "id": 1,
            "name": "bug",
            "color": "d73a4a",
            "description": ""
        }
    ],
    "comments": 5,
    "created_at": "2024-01-01T00:00:00Z",
    "updated_at": "2024-01-02T00:00:00Z",
    "closed_at": null,
    "author_association": "NONE",
    "reactions": {
        "url": "https://xtalk.example.com/repos/user/blog/issues/1/reactions",
        "total_count": 0,
        "+1": 0, "-1": 0, "laugh": 0, "confused": 0,
        "heart": 0, "hooray": 0, "rocket": 0, "eyes": 0
    },
    "url": "https://xtalk.example.com/repos/user/blog/issues/1",
    "html_url": "https://xtalk.example.com/repos/user/blog/issues/1",
    "comments_url": "https://xtalk.example.com/repos/user/blog/issues/1/comments"
}
```

### Comment 对象

```json
{
    "id": 42,
    "node_id": "eHRhbGs6Q29tbWVudDo0Mg==",
    "body": "评论内容",
    "body_html": "<p>评论内容</p>",
    "user": { "login": "octocat", "id": 1, "avatar_url": "...", "type": "User" },
    "created_at": "2024-01-01T00:00:00Z",
    "updated_at": "2024-01-01T00:00:00Z",
    "html_url": "https://xtalk.example.com/repos/user/blog/issues/1#comment-42",
    "issue_url": "https://xtalk.example.com/repos/user/blog/issues/1",
    "author_association": "NONE",
    "reactions": {
        "url": ".../comments/42/reactions",
        "total_count": 2,
        "+1": 1, "-1": 0, "laugh": 1, "confused": 0,
        "heart": 0, "hooray": 0, "rocket": 0, "eyes": 0
    }
}
```

### Accept Header 支持

| Accept 值 | 返回字段 |
|-----------|---------|
| `application/json`（默认） | `body`（原始 Markdown） |
| `application/vnd.github.v3.raw+json` | `body` |
| `application/vnd.github.v3.html+json` | `body_html` |
| `application/vnd.github.v3.full+json` | `body` + `body_html` |

Markdown → HTML 使用 `pulldown-cmark`（纯 Rust，WASM 兼容）。

### 分页

列表端点通过 `Link` header 返回分页信息：

```
Link: <https://xtalk.example.com/repos/user/blog/issues?page=2&per_page=30>; rel="next",
      <https://xtalk.example.com/repos/user/blog/issues?page=5&per_page=30>; rel="last"
```

### 错误响应

所有错误使用 GitHub 兼容格式：

```json
{
    "message": "Validation Failed",
    "errors": [
        { "resource": "Issue", "field": "title", "code": "missing_field" }
    ],
    "documentation_url": "https://docs.github.com/rest"
}
```

| 状态码 | 场景 |
|--------|------|
| 400 | 请求体格式错误 |
| 401 | 未认证访问写端点 |
| 403 | 无权限（编辑他人评论） |
| 404 | 仓库/Issue/评论不存在 |
| 422 | 验证失败（缺少必填字段） |

## 10. 关键实现细节

### 10.1 Per-repo Issue 编号

使用 batch 操作保证原子性：

```rust
// 1. 递增计数器
// 2. 读取新值
// 3. 插入 issue
db.batch(vec![
    ("UPDATE repos SET issue_counter = issue_counter + 1 WHERE id = ?1",
     vec![DbValue::Integer(repo_id)]),
    ("SELECT issue_counter FROM repos WHERE id = ?1",
     vec![DbValue::Integer(repo_id)]),
]).await?;
// D1 batch 和 SQLite transaction 都能保证原子性
```

### 10.2 路由冲突处理

`/repos/:owner/:repo/issues/comments/:id` 与 `/repos/:owner/:repo/issues/:number` 存在路径歧义。解决方案：在两个框架中都**先注册**含 `comments` 字面量的路由，确保优先匹配。

### 10.3 仓库自动创建

首次访问不存在的 `owner/repo` 时自动创建仓库记录，第一个通过认证的用户成为 admin。

### 10.4 author_association

Phase 1 简化实现：
- 仓库 admin → `OWNER`
- 其他用户 → `NONE`

### 10.5 评论列表的 Reaction 数据

Reaction 计数以 JSON 冗余在 comments 表的 `reactions` 字段中，列出评论时无需 JOIN：

```sql
SELECT c.*, u.login, u.avatar_url, u.email
FROM comments c
JOIN users u ON c.user_id = u.id
WHERE c.issue_id = ?1 AND c.deleted_at IS NULL
ORDER BY c.created_at ASC
LIMIT ?2 OFFSET ?3
```

`reactions` 字段格式：`{"plus_one":1,"heart":2,"total":3}`，缺省键视为 0。

创建/删除 reaction 时，在 Rust 侧读出 JSON → 修改计数 → 写回：

```rust
// 在 reaction service 中（伪码）
let mut counts: ReactionCounts = serde_json::from_str(&comment.reactions)?;
counts.increment("heart");  // +1 并更新 total
db.execute(
    "UPDATE comments SET reactions = ?1 WHERE id = ?2",
    &[DbValue::Text(serde_json::to_string(&counts)?), DbValue::Integer(comment_id)],
).await?;
```

## 11. 部署配置

### Workers 部署

```toml
# deploy/worker/wrangler.toml
name = "xtalk"
main = "../../target/wasm32-unknown-unknown/release/xtalk.wasm"
compatibility_date = "2024-09-23"

[build]
command = "cargo build --release --features worker --target wasm32-unknown-unknown"

[vars]
BASE_URL = "https://xtalk.yourdomain.com"
TOKEN_CACHE_TTL = "3600"

[[d1_databases]]
binding = "DB"
database_name = "xtalk-db"
database_id = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
```

### 容器部署

```dockerfile
# Dockerfile
FROM rust:1.82-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features server --bin xtalk-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/xtalk-server /usr/local/bin/
EXPOSE 3000
CMD ["xtalk-server"]
```

```bash
# 环境变量
XTALK_BASE_URL=https://xtalk.yourdomain.com
XTALK_DATABASE_URL=sqlite:///data/xtalk.db
XTALK_TOKEN_CACHE_TTL=3600
XTALK_LISTEN=0.0.0.0:3000
```

## 12. 实现顺序

| 阶段 | 内容 | 产出文件 |
|------|------|---------|
| 1 | 项目脚手架 | `Cargo.toml`, 目录结构 |
| 2 | 数据库 Schema | `migrations/0001_initial_schema.sql` |
| 3 | 核心类型 | `types.rs`, `error.rs` |
| 4 | 数据库抽象 | `db.rs` |
| 5 | 认证逻辑 | `auth.rs` |
| 6 | 响应格式化 | `fmt/` 目录 |
| 7 | 业务逻辑 | `services/` 目录 |
| 8 | **Server 平台**（先做，方便调试） | `platform/server/`, `bin/server.rs` |
| 9 | Worker 平台 | `platform/worker/` |
| 10 | 测试 & 集成验证 | 测试文件 |

## 13. Server 部署 LRU 缓存

仅 `--features server` 编译时生效，Worker 部署不涉及（无持久进程内存）。

### 缓存什么

评论列表是读多写少的热点：每次页面加载都会拉取，但评论不会频繁变动。因此缓存两类数据：

| 缓存项 | key | value |
|--------|-----|-------|
| 某 issue 的评论列表（分页） | `(issue_id, page, per_page)` | `(Vec<CommentRow>, total_count)` |
| 单条评论 | `comment_id` | `CommentRow` |

Issue 列表不缓存——它的过滤参数组合多、失效频繁，收益低。

### 依赖

```toml
# server feature 专属
moka = { version = "0.12", features = ["future"], optional = true }
```

`moka` 是异步原生的 LRU/TinyLFU 缓存库，支持按容量和 TTL 双重淘汰。

### 数据结构

```rust
// src/platform/server/cache.rs

use moka::future::Cache;

#[derive(Clone)]
pub struct CommentCache {
    /// 评论列表缓存，key = (issue_id, page, per_page)
    list: Cache<(i64, i64, i64), (Vec<CommentRow>, i64)>,
    /// 单条评论缓存，key = comment_id
    single: Cache<i64, CommentRow>,
}

impl CommentCache {
    pub fn new(max_capacity: u64, ttl_secs: u64) -> Self {
        let ttl = std::time::Duration::from_secs(ttl_secs);
        Self {
            list: Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(ttl)
                .build(),
            single: Cache::builder()
                .max_capacity(max_capacity * 10)
                .time_to_live(ttl)
                .build(),
        }
    }

    pub async fn get_list(&self, issue_id: i64, page: i64, per_page: i64)
        -> Option<(Vec<CommentRow>, i64)>
    {
        self.list.get(&(issue_id, page, per_page)).await
    }

    pub async fn set_list(&self, issue_id: i64, page: i64, per_page: i64,
        rows: Vec<CommentRow>, total: i64)
    {
        self.list.insert((issue_id, page, per_page), (rows, total)).await;
    }

    pub async fn get_single(&self, comment_id: i64) -> Option<CommentRow> {
        self.single.get(&comment_id).await
    }

    pub async fn set_single(&self, comment: CommentRow) {
        self.single.insert(comment.id, comment).await;
    }

    /// 某 issue 下有评论变动时，让该 issue 所有列表缓存失效
    /// moka 不支持按前缀批量删除，用 invalidate_entries_if 按 issue_id 扫描
    pub async fn invalidate_issue(&self, issue_id: i64) {
        self.list
            .invalidate_entries_if(move |k, _| k.0 == issue_id)
            .await
            .ok();
    }

    pub async fn invalidate_comment(&self, comment_id: i64) {
        self.single.invalidate(&comment_id).await;
    }
}
```

### 挂载到 AppState

```rust
// src/platform/server/mod.rs

pub struct AppState {
    pub db: Arc<SqliteDatabase>,
    pub http: Arc<ReqwestHttpClient>,
    pub cache: CommentCache,
    pub base_url: String,
    pub token_cache_ttl: i64,
}
```

### 缓存读写时机

```
GET …/issues/:n/comments
  └─ 先查 cache.get_list(issue_id, page, per_page)
       命中 → 直接返回
       未命中 → 查 DB → cache.set_list(...) → 返回

GET …/issues/comments/:id
  └─ 先查 cache.get_single(comment_id)
       命中 → 直接返回
       未命中 → 查 DB → cache.set_single(...) → 返回

POST  …/issues/:n/comments     （新建评论）
PATCH …/issues/comments/:id    （编辑评论）
DELETE …/issues/comments/:id   （删除评论）
POST/DELETE …/comments/:id/reactions  （reaction 变动）
  └─ 写入 DB → cache.invalidate_issue(issue_id)
                + cache.invalidate_comment(comment_id)
```

### 配置（环境变量）

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `XTALK_CACHE_MAX_ISSUES` | `256` | 最多缓存多少个 issue 的列表 |
| `XTALK_CACHE_TTL` | `60` | 缓存 TTL（秒），到期自动失效 |

TTL 兜底保证即使失效逻辑有疏漏，缓存最多 60 秒后也会自动过期。

### Worker 部署对比

Worker 是无状态的，每个请求都是独立的 isolate，没有进程内共享内存。如需缓存可使用 Cloudflare KV 或 Cache API，但这属于 Phase 2 范畴，当前 Worker 版直接走 D1。

## 14. 验证方案

```bash
# ── 容器版 ──
cargo run --features server --bin xtalk-server

# 创建 issue
curl -X POST http://localhost:3000/repos/user/blog/issues \
  -H "Authorization: token ghp_xxx" \
  -H "Content-Type: application/json" \
  -d '{"title":"Hello","body":"First post","labels":["comment"]}'

# 列出 issues
curl http://localhost:3000/repos/user/blog/issues

# 创建评论
curl -X POST http://localhost:3000/repos/user/blog/issues/1/comments \
  -H "Authorization: token ghp_xxx" \
  -H "Content-Type: application/json" \
  -d '{"body":"Nice post!"}'

# ── Workers 版 ──
cd deploy/worker
npx wrangler d1 execute xtalk-db --local \
  --file=../../migrations/0001_initial_schema.sql
npx wrangler dev

# ── 前端兼容性验证 ──
# 将 Gitalk/Utterances 的 API 基地址指向本地服务
# 验证评论加载、创建、反应等功能正常工作
```
