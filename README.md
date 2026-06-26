# Atrium

Atrium 是一个 Rust + SQLite 的原生评论后端，当前 `master` 面向 Railway 部署。对外接口以原 Cloudflare Worker 版本的 native website/page/comment API 为准，旧的 GitHub Issues / utterances 兼容接口不再作为 `master` 的公开契约。

## 功能特性

- 原生站点、页面、评论模型：`websites` / `pages` / `comments`
- SQLite 持久化，启动时自动创建数据库目录并执行迁移
- Railway 友好：自动读取 `PORT`，默认监听 `0.0.0.0:$PORT`
- HttpOnly Cookie + Bearer JWT 双模式认证
- Account Center cookie introspection 登录桥接
- 站点管理员、超级管理员、封禁、评论删除/编辑权限
- 评论 reaction、快速 Referer 页面识别、发现配置查询

## 本地运行

```bash
export ATRIUM_JWT_SECRET="change-me-to-at-least-16-bytes"
export ATRIUM_DATABASE_URL="sqlite:///tmp/atrium.db"
cargo run --features server --bin atrium-server
```

访问 `http://localhost:3000/` 可看到服务入口说明。

## Railway 部署

Railway 可以直接使用仓库里的 `Dockerfile` 构建服务。建议挂载一个 volume 到 `/data`，并使用 SQLite 文件路径作为数据库地址。

必填环境变量：

- `ATRIUM_JWT_SECRET`：至少 16 字节；可以是普通字符串、标准 base64 或 URL-safe base64
- `ATRIUM_DATABASE_URL`：建议 `sqlite:///data/atrium.db`

常用环境变量：

- `ATRIUM_BASE_URL`：服务公网地址，例如 `https://atrium.example.com`
- `ATRIUM_CORS_ORIGIN`：允许携带 cookie 的前端 origin
- `ACCOUNT_BASE_URL` 或 `ACCOUNT_ISSUER`：Account Center 地址
- `ACCOUNT_AUDIENCE`：调用 Account Center introspection 时使用的 audience
- `ACCOUNT_INTERNAL_SECRET`：调用 Account Center introspection 的内部密钥
- `ATRIUM_SUPER_ADMIN_ACCOUNT_IDS`：逗号分隔的超级管理员账号标识、邮箱或登录名
- `ATRIUM_DISCOVERY_PUBLIC_JWK`：发现协议返回的公开 JWK JSON
- `ATRIUM_DISCOVERY_KEY_ID`：发现协议 key id

Railway 会提供 `PORT`，无需手动设置 `ATRIUM_LISTEN`。如果本地或其他平台需要固定监听地址，可设置：

```bash
ATRIUM_LISTEN=0.0.0.0:3000
```

## 主要接口

认证：

- `POST /api/v1/auth/account`
- `GET /api/v1/auth/account/authorize`
- `GET /api/v1/auth/account/callback`
- `POST /api/v1/auth/refresh`
- `DELETE /api/v1/auth/session`
- `GET /api/v1/auth/me`

站点与页面：

- `POST /api/v1/websites`
- `GET /api/v1/websites`
- `GET /api/v1/websites/{websiteKey}`
- `PATCH /api/v1/websites/{websiteKey}`
- `GET /api/v1/websites/{websiteKey}/admins`
- `POST /api/v1/websites/{websiteKey}/admins`
- `DELETE /api/v1/websites/{websiteKey}/admins/{userId}`
- `PUT /api/v1/websites/{websiteKey}/pages/{pageKey}`
- `GET /api/v1/websites/{websiteKey}/pages`
- `GET /api/v1/websites/{websiteKey}/pages/{pageKey}`

评论：

- `GET /api/v1/websites/{websiteKey}/pages/{pageKey}/comments`
- `POST /api/v1/websites/{websiteKey}/pages/{pageKey}/comments`
- `PATCH /api/v1/websites/{websiteKey}/comments/{commentId}`
- `DELETE /api/v1/websites/{websiteKey}/comments/{commentId}`
- `PUT /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}`
- `DELETE /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}`
- `GET /api/v1/comments/current`
- `POST /api/v1/comments/current`

管理与发现：

- `GET /api/v1/websites/{websiteKey}/admin/comments`
- `GET /api/v1/websites/{websiteKey}/bans`
- `POST /api/v1/websites/{websiteKey}/bans`
- `DELETE /api/v1/websites/{websiteKey}/bans/{userId}`
- `GET /api/v1/discovery/public-key`
- `GET /docs/discovery`

## 测试

默认测试目标是 Rust server + SQLite：

```bash
python3 scripts/test.py
```

等价于：

```bash
cargo test --features "server,test-utils" --no-fail-fast
```

## 许可证

本项目使用 `Apache-2.0`，许可证全文见 `LICENSE`。
