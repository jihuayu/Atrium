# xtalk 扩展设计：Native API + 多 Provider 认证

## 变更记录

| 版本 | 变更内容 |
|------|---------|
| v0.1 | 初始设计：Native `/api/v1/` 接口、Google/Apple 认证、xtalk JWT、多 Provider 用户关联 |

---

## 1. 概述

在现有 GitHub Issues 兼容接口的基础上，新增两大能力：

1. **`/api/v1/` 自有接口** — 专为评论系统设计，游标分页、简洁响应格式、无 GitHub 特有字段
2. **多 Provider 认证** — 在 GitHub token 透传之外支持 Google OAuth、Apple Sign-In；自有接口使用 xtalk 签发的 JWT

**核心约束**：
- 两套接口共享同一份数据库（`issues`、`comments`、`reactions` 等表不变）
- GitHub 兼容接口保持完全向后兼容，零破坏性变更
- 所有新代码遵循现有 `Database` trait 抽象，同时支持 D1 和 SQLite

---

## 2. DB Schema 变更

### 2.1 迁移策略

`users.id` 目前等于 GitHub user ID（外部 ID 直接作主键）。多 Provider 下需要自管理的自增主键。

**关键点**：迁移时保留现有行的 ID 值不变，所有 FK（repos/issues/comments 中的 `user_id`）无需修改。SQLite/D1 的 `AUTOINCREMENT` 在插入了指定 ID 的行后，会从 `max(id)+1` 继续，新用户获得全新 ID。

### 2.2 `migrations/0002_multi_provider_auth.sql`

```sql
-- Step 1: 重建 users 表为自增 PK（保留现有行的 ID 值）
ALTER TABLE users RENAME TO users_v1;

CREATE TABLE users (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    login      TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL DEFAULT '',
    avatar_url TEXT NOT NULL DEFAULT '',
    type       TEXT NOT NULL DEFAULT 'User',
    site_admin INTEGER NOT NULL DEFAULT 0,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO users SELECT id, login, email, avatar_url, type, site_admin, cached_at FROM users_v1;

-- Step 2: Provider 身份关联表（多对一 → users）
CREATE TABLE user_identities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK(provider IN ('github','google','apple')),
    provider_user_id TEXT NOT NULL,   -- GitHub: 整数 ID 转 text；Google/Apple: JWT sub
    email            TEXT NOT NULL DEFAULT '',
    avatar_url       TEXT NOT NULL DEFAULT '',
    cached_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX idx_user_identities_user  ON user_identities(user_id);
CREATE INDEX idx_user_identities_email ON user_identities(email);

-- Step 3: 迁移已有 GitHub 身份
INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at)
SELECT id, 'github', CAST(id AS TEXT), email, avatar_url, cached_at FROM users_v1;

-- Step 4: token_cache 增加 provider 列（复合主键）
ALTER TABLE token_cache RENAME TO token_cache_v1;

CREATE TABLE token_cache (
    token_hash TEXT NOT NULL,
    provider   TEXT NOT NULL DEFAULT 'github'
               CHECK(provider IN ('github','google','apple','xtalk')),
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    PRIMARY KEY (token_hash, provider)
);
CREATE INDEX idx_token_cache_expires ON token_cache(expires_at);

INSERT INTO token_cache
SELECT token_hash, 'github', user_id, cached_at, expires_at FROM token_cache_v1;

-- Step 5: Refresh token 存储
-- server 部署：存 DB 支持吊销；worker 部署：无状态 JWT，此表不使用
CREATE TABLE sessions (
    refresh_token_hash TEXT PRIMARY KEY,   -- SHA-256 of refresh token
    user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at         TEXT NOT NULL,
    revoked_at         TEXT               -- NULL = 有效
);
CREATE INDEX idx_sessions_user    ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- Step 6: JWKS 缓存（Worker 无持久内存，存 D1；server 用 moka 内存缓存）
CREATE TABLE jwks_cache (
    provider   TEXT PRIMARY KEY,  -- 'google' | 'apple'
    jwks_json  TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

-- Cleanup
DROP TABLE users_v1;
DROP TABLE token_cache_v1;
```

### 2.3 最终表结构总览

```
users                          ← 自增 PK，provider 无关
  id, login, email, avatar_url, type, site_admin, cached_at

user_identities                ← 每个 provider 身份一行
  id, user_id → users.id
  provider ('github'|'google'|'apple')
  provider_user_id             ← 不同 provider 的外部 ID
  email, avatar_url, cached_at
  UNIQUE(provider, provider_user_id)

token_cache                    ← 复合 PK (token_hash, provider)
  token_hash, provider, user_id, cached_at, expires_at

sessions                       ← server 专用，refresh token 存储
  refresh_token_hash, user_id, created_at, expires_at, revoked_at

jwks_cache                     ← Worker 专用，Google/Apple 公钥缓存
  provider, jwks_json, expires_at

-- 其余表不变（repos, issues, comments, labels, reactions 等）
```

---

## 3. 认证架构

### 3.1 Provider 认证流程概览

```
客户端                       xtalk                         外部 Provider
  │                            │                                │
  │ POST /api/v1/auth/github   │                                │
  │   { "token": "ghp_..." }   │                                │
  │                            │── GET /user (GitHub) ─────────→│
  │                            │                                │
  │ POST /api/v1/auth/google   │                                │
  │   { "token": "eyJ..." }    │── 验 JWKS 签名（本地）         │
  │                            │   （仅首次拉取 JWKS 时外网）    │
  │                            │                                │
  │ POST /api/v1/auth/apple    │                                │
  │   { "token": "eyJ..." }    │── 验 JWKS 签名（本地）         │
  │                            │                                │
  │                            │ resolve_or_create_user()       │
  │                            │ issue_xtalk_jwt()              │
  │                            │                                │
  │←─ { access_token,          │                                │
  │     refresh_token, user }  │                                │
  │                            │                                │
  │ GET /api/v1/...            │                                │
  │  Authorization: Bearer JWT │                                │
  │                            │── verify_jwt() 本地验签        │
  │                            │   (无外网调用)                  │
  │←─ response                 │                                │
```

### 3.2 路径前缀驱动的认证分支

在平台适配器（`platform/*/mod.rs`）中，按请求路径决定用哪套认证：

```rust
let user = if path.starts_with("/api/v1/auth/") {
    // 认证交换端点，handler 内部自行处理，无需预认证
    None
} else if path.starts_with("/api/v1/") {
    // Native API：验证 xtalk 签发的 JWT
    resolve_xtalk_jwt_user(auth_header, jwt_secret, db).await?
} else {
    // GitHub 兼容路径（/repos/…, /search/…）：原有 GitHub token 透传
    resolve_github_user(auth_header, db, http, ttl).await?
};
```

### 3.3 GitHub 兼容路径（最小改动）

`resolve_github_user()` 内 SQL 仅增加 `AND tc.provider = 'github'`：

```sql
-- token_cache 查询（加 provider 过滤）
SELECT u.* FROM token_cache tc
JOIN users u ON tc.user_id = u.id
WHERE tc.token_hash = ?1 AND tc.provider = 'github' AND tc.expires_at > datetime('now')

-- upsert（ON CONFLICT 改为复合键）
INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at)
VALUES (?1, 'github', ?2, datetime('now'), ...)
ON CONFLICT(token_hash, provider) DO UPDATE SET ...
```

其余逻辑（调 GitHub `/user`、upsert users）完全不变。

### 3.4 Google / Apple JWKS 验签

两者均为 JWT，验签流程：

1. 解码 JWT header 取 `kid`
2. 从缓存取 JWKS（Worker: `jwks_cache` D1 表；server: moka）
3. 缓存 miss → `HttpClient::get_jwks(url)` 拉取，按 `Cache-Control: max-age` 设 TTL
4. 按 `kid` 找公钥：Google 用 RSA-2048（`rsa` crate），Apple 用 P-256（`p256` crate）
5. 验签 + 校验 `exp`、`iss`、`aud`（`aud` 从环境变量读取）
6. 提取 `sub`、`email`、`email_verified`、`picture`

| Provider | JWKS URL | `iss` | `aud` 配置项 |
|----------|----------|-------|-------------|
| Google | `https://www.googleapis.com/oauth2/v3/certs` | `https://accounts.google.com` | `XTALK_GOOGLE_CLIENT_ID` |
| Apple | `https://appleid.apple.com/auth/keys` | `https://appleid.apple.com` | `XTALK_APPLE_APP_ID` |

**未配置 = 该渠道自动关闭**。`AppContext` 中对应字段类型为 `Option<&'a str>`，环境变量不存在或为空时传 `None`，handler 入口处直接返回 `501`：

```rust
// src/handlers/api/auth.rs
pub async fn auth_google(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let Some(client_id) = ctx.google_client_id else {
        return AppResponse::json(501, &json!({
            "error": "not_configured",
            "message": "Google login is not enabled on this server"
        }));
    };
    // ... 正常验签流程
}
```

| 环境变量状态 | `AppContext` 字段 | 行为 |
|-------------|-----------------|------|
| 未设置 | `None` | `501 Not Configured` |
| 设置为空字符串 | `None` | `501 Not Configured` |
| 正常值 | `Some("...")` | 正常验签 |

GitHub 渠道无需 client_id（token 透传到 GitHub API 验证），始终开启。

新增依赖（纯 Rust，WASM 兼容，加入共享层）：

```toml
hmac = "0.12"
rsa  = { version = "0.9", default-features = false, features = ["sha2"] }
p256 = { version = "0.13", default-features = false, features = ["ecdsa"] }
```

### 3.5 xtalk JWT 设计

**格式**（HS256，对称签名，WASM 兼容）：

```
Header:  { "alg": "HS256", "typ": "JWT" }
Payload: { "sub": "42", "login": "alice", "iss": "xtalk",
           "iat": 1700000000, "exp": 1700003600, "jti": "uuid" }
```

| Token 类型 | TTL | 存储 |
|-----------|-----|------|
| Access token | 1 小时 | 不存 DB（stateless） |
| Refresh token | 30 天 | server: `sessions` 表（可吊销）；worker: 长期 JWT（无状态） |

**密钥配置**：
- server: 环境变量 `XTALK_JWT_SECRET`（≥32 字节，base64）
- worker: wrangler secret `JWT_SECRET`

**实现**（`src/jwt.rs`，仅用 `hmac` + `sha2` + `base64`，不依赖 `ring` / `jsonwebtoken`）：

```rust
pub fn sign_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String>;
pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<JwtClaims>;
```

### 3.6 用户账号关联（按 email 自动合并）

```
resolve_or_create_user(db, provider, provider_user_id, email, ...):

  1. SELECT user_id FROM user_identities
     WHERE provider = ?1 AND provider_user_id = ?2
     → 命中：直接返回该 user_id（最快路径）

  2. 若 email 不为空 且不含 'privaterelay.appleid.com'：
     SELECT id FROM users WHERE email = ?
     → 命中：INSERT INTO user_identities 将新 provider 关联到已有用户
             （不同 provider 同邮箱 = 同一个人）

  3. 否则：
     INSERT INTO users (login, email, avatar_url, ...)
     INSERT INTO user_identities (user_id, provider, provider_user_id, ...)
     返回新建的 user_id
```

Apple 私人中继邮箱（`@privaterelay.appleid.com`）不参与跨 provider 关联，避免误合并。

---

## 4. Native API 端点

### 4.1 Auth

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `POST` | `/api/v1/auth/github` | 无 | 交换 GitHub token → xtalk JWT |
| `POST` | `/api/v1/auth/google` | 无 | 交换 Google ID token → xtalk JWT |
| `POST` | `/api/v1/auth/apple` | 无 | 交换 Apple identity token → xtalk JWT |
| `POST` | `/api/v1/auth/refresh` | refresh JWT | 续期 |
| `DELETE` | `/api/v1/auth/session` | JWT | 吊销当前 session |
| `GET` | `/api/v1/auth/me` | JWT | 获取当前用户 |

**请求体**：`{ "token": "provider_issued_token" }`

**响应**：
```json
{
  "access_token": "eyJ...",
  "refresh_token": "eyJ...",
  "expires_in": 3600,
  "token_type": "Bearer",
  "user": { "id": 42, "login": "alice", "avatar_url": "...", "email": "..." }
}
```

### 4.2 Threads（复用 issues 表）

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/threads` | 可选 | 列出（游标分页） |
| `POST` | `/api/v1/repos/:owner/:repo/threads` | 必须 | 创建 |
| `GET` | `/api/v1/repos/:owner/:repo/threads/:number` | 可选 | 获取 |
| `PATCH` | `/api/v1/repos/:owner/:repo/threads/:number` | 必须 | 更新（作者/admin） |
| `DELETE` | `/api/v1/repos/:owner/:repo/threads/:number` | admin | 删除 |

**GET 查询参数**：`state`（open/closed/all）、`limit`（max 100）、`cursor`、`direction`（asc/desc）

### 4.3 Comments

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/threads/:number/comments` | 可选 | 列出（游标分页） |
| `POST` | `/api/v1/repos/:owner/:repo/threads/:number/comments` | 必须 | 创建 |
| `GET` | `/api/v1/repos/:owner/:repo/comments/:id` | 可选 | 获取 |
| `PATCH` | `/api/v1/repos/:owner/:repo/comments/:id` | 必须 | 更新 |
| `DELETE` | `/api/v1/repos/:owner/:repo/comments/:id` | 必须 | 删除 |

**GET 查询参数**：`limit`、`cursor`、`order`（asc/desc）

### 4.4 Reactions

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `POST` | `/api/v1/repos/:owner/:repo/comments/:id/reactions` | 必须 | 添加 |
| `DELETE` | `/api/v1/repos/:owner/:repo/comments/:id/reactions/:content` | 必须 | 删除（用 `+1` 等作路径参数） |

### 4.5 Labels

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/labels` | 可选 | 列出 |
| `POST` | `/api/v1/repos/:owner/:repo/labels` | admin | 创建 |
| `DELETE` | `/api/v1/repos/:owner/:repo/labels/:name` | admin | 删除 |

### 4.6 Admin

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo` | admin | 获取仓库设置 |
| `PATCH` | `/api/v1/repos/:owner/:repo` | admin | 更新仓库设置 |

### 4.7 响应格式

**列表响应**（统一包装）：
```json
{
  "data": [ ... ],
  "pagination": { "next_cursor": "eyJpZCI6NDJ9", "has_more": true }
}
```

**Thread 对象**（无 GitHub 特有字段）：
```json
{
  "id": 1, "number": 1, "title": "...", "body": "...", "body_html": "<p>...</p>",
  "state": "open", "comment_count": 5,
  "author": { "id": 42, "login": "alice", "avatar_url": "..." },
  "labels": [{ "id": 1, "name": "bug", "color": "d73a4a" }],
  "reactions": { "+1": 2, "heart": 1, "total": 3 },
  "created_at": "...", "updated_at": "..."
}
```

**错误响应**（与 compat 路径不同格式）：
```json
{ "error": "unauthorized", "message": "Authentication required" }
```

### 4.8 游标分页

游标 = `base64url( {"id": last_seen_id} )`，SQL 使用 `WHERE id > ?1 ORDER BY id ASC LIMIT ?2`，无 OFFSET，并发插入安全。

---

## 5. 代码结构变更

### 5.1 新增文件

| 文件 | 职责 |
|------|------|
| `src/jwt.rs` | HS256 JWT 签发/验签（`hmac` + `sha2` + `base64`，WASM 兼容） |
| `src/jwks.rs` | Google/Apple JWKS 拉取 + RS256/ES256 验签；`JwksCache` trait |
| `src/services/auth.rs` | `resolve_or_create_user()`、`issue_xtalk_jwt()`、`refresh_jwt()` |
| `src/services/session.rs` | `create_session()`、`revoke_session()`（server 用） |
| `src/services/cursor.rs` | 游标编解码 `encode_cursor()` / `decode_cursor()` |
| `src/handlers/api/auth.rs` | POST auth/github\|google\|apple, refresh, me, session delete |
| `src/handlers/api/threads.rs` | Thread CRUD handler |
| `src/handlers/api/comments.rs` | Comment CRUD handler |
| `src/handlers/api/reactions.rs` | Reaction add/remove handler |
| `src/handlers/api/labels.rs` | Label CRUD handler |
| `src/handlers/api/admin.rs` | 仓库设置 handler |
| `src/fmt/api.rs` | DB row → native API JSON 格式 |
| `migrations/0002_multi_provider_auth.sql` | Schema 迁移 |

### 5.2 修改文件

| 文件 | 关键改动 |
|------|---------|
| `src/auth.rs` | `resolve_user()` 改名 `resolve_github_user()`，SQL 加 `AND provider='github'`；新增 `resolve_xtalk_jwt_user()` |
| `src/types.rs` | 加 `pub type AuthUser = GitHubUser`；加 `JwtClaims`、`NativeThreadResponse`、`NativeCommentResponse`、`CursorPage<T>`、`AuthTokenResponse`、`ProviderUser` |
| `src/lib.rs` | `AppContext` 加 `jwt_secret: &'a [u8]`、`google_client_id: Option<&'a str>`、`apple_app_id: Option<&'a str>` |
| `src/error.rs` | 加 `to_native_response()` 返回 `{"error":"…","message":"…"}` |
| `src/router.rs` | `Route` 枚举加 ~30 个 `Api*` 变体；注册所有 `/api/v1/` 路由；dispatch 加新分支 |
| `platform/worker/mod.rs` | `WorkerState` 加 JWT 配置；按路径前缀选认证方式 |
| `platform/server/mod.rs` | `AppState` 加 JWT 配置；按路径前缀选认证方式；JWKS 用 moka 缓存 |
| `Cargo.toml` | 共享层加 `hmac`、`rsa`、`p256` |
| `deploy/worker/wrangler.toml` | 加 secrets: `JWT_SECRET`、`GOOGLE_CLIENT_ID`、`APPLE_APP_ID` |

---

## 6. 实现顺序

| 步骤 | 文件 |
|------|------|
| 1 | `migrations/0002_multi_provider_auth.sql` |
| 2 | `src/jwt.rs` |
| 3 | `src/types.rs`（新增类型） |
| 4 | `src/auth.rs`（拆分 github/xtalk 认证，修 SQL） |
| 5 | `src/jwks.rs`（Google/Apple 验签） |
| 6 | `src/services/auth.rs`（用户关联 + JWT 签发） |
| 7 | `src/services/session.rs` + `cursor.rs` |
| 8 | `src/fmt/api.rs`（native 响应格式） |
| 9 | `src/handlers/api/`（全部 handler） |
| 10 | `src/router.rs`（注册新路由） |
| 11 | `src/lib.rs`（扩展 AppContext） |
| 12 | `platform/*/mod.rs`（路径前缀认证分支） |
| 13 | `Cargo.toml` + `wrangler.toml` |

---

## 7. 配置参考

### 7.1 Cloudflare Workers（`deploy/worker/wrangler.toml`）

```toml
[vars]
BASE_URL = "https://xtalk.yourdomain.com"
TOKEN_CACHE_TTL = "3600"

# JWT_SECRET 必须配置，否则 native API 无法签发 token（≥32字节，建议用 openssl rand -base64 32 生成）
# GOOGLE_CLIENT_ID / APPLE_APP_ID 不填则对应登录渠道自动关闭，返回 501

# 通过 wrangler secret put JWT_SECRET 设置（敏感值不写 toml 明文）
# 可选:
# wrangler secret put GOOGLE_CLIENT_ID
# wrangler secret put APPLE_APP_ID
```

### 7.2 容器部署（环境变量）

| 变量 | 必填 | 说明 |
|------|------|------|
| `XTALK_JWT_SECRET` | **是** | HS256 签名密钥，≥32 字节，base64 编码 |
| `XTALK_GOOGLE_CLIENT_ID` | 否 | 不填则 `/api/v1/auth/google` 返回 `501` |
| `XTALK_APPLE_APP_ID` | 否 | 不填则 `/api/v1/auth/apple` 返回 `501` |
| `XTALK_BASE_URL` | **是** | 服务对外地址 |
| `XTALK_DATABASE_URL` | **是** | `sqlite:///data/xtalk.db` |
| `XTALK_TOKEN_CACHE_TTL` | 否 | GitHub token 缓存秒数，默认 `3600` |
| `XTALK_CACHE_TTL` | 否 | 评论 LRU 缓存秒数，默认 `60` |

### 7.3 渠道启用状态一览

| 渠道 | 端点 | 启用条件 |
|------|------|---------|
| GitHub | `/api/v1/auth/github` | 始终启用（token 直接透传验证，无需配置） |
| Google | `/api/v1/auth/google` | 需配置 `GOOGLE_CLIENT_ID` |
| Apple | `/api/v1/auth/apple` | 需配置 `APPLE_APP_ID` |

未配置时访问对应端点响应：

```json
HTTP 501
{ "error": "not_configured", "message": "Google login is not enabled on this server" }
```

---

## 8. 验证方案

```bash
# 应用迁移
wrangler d1 execute xtalk-db --local --file=migrations/0002_multi_provider_auth.sql
# 或 server
cargo run --features server &

# GitHub 换 xtalk JWT
curl -X POST localhost:3000/api/v1/auth/github \
  -H "Content-Type: application/json" \
  -d '{"token":"ghp_xxx"}'
# → { "access_token": "eyJ...", "user": {...} }

# 未配置 Google 时的预期响应
curl -X POST localhost:3000/api/v1/auth/google \
  -H "Content-Type: application/json" \
  -d '{"token":"google_id_token"}'
# → 501 { "error": "not_configured", "message": "Google login is not enabled on this server" }

# 用 xtalk JWT 访问 native API
ACCESS_TOKEN="eyJ..."
curl localhost:3000/api/v1/repos/user/blog/threads \
  -H "Authorization: Bearer $ACCESS_TOKEN"
# → { "data": [...], "pagination": {...} }

# 游标翻页
CURSOR="eyJpZCI6NDJ9"
curl "localhost:3000/api/v1/repos/user/blog/threads/1/comments?cursor=$CURSOR&limit=10"

# GitHub 兼容路径不受影响
curl -H "Authorization: token ghp_xxx" localhost:3000/repos/user/blog/issues
# → 原有 GitHub 兼容响应，完全不变
```
