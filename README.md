# Atrium

Atrium 是一个轻量后端服务，用来代理 GitHub Issues 作为评论存储引擎，并暴露与 GitHub Issues 兼容的 API，方便 `utterances`、`gitalk` 一类前端评论组件无缝接入。

## 背景

很多博客评论系统直接把评论写入 GitHub 仓库的 Issues。这样虽然方便，但很不优雅。

Atrium 的目标是：

- 保持 GitHub Issues API 兼容，尽量不改前端接入层
- 把评论数据落到你自己的数据库
- 支持独立后端部署和 Cloudflare Worker 部署

## 功能特性

- GitHub Issues API 兼容接口（issues / comments / reactions / search）
- Native API（JWT）用于更完整的站内能力
- 多 Provider 认证（GitHub / Google / Apple）
- 本地 SQLite 或 Cloudflare D1
- 轻量、低运维成本

## 部署方式

### 1. 独立服务（Server）

```bash
cargo run --features server --bin atrium-server
```

常用环境变量（新旧前缀兼容）：

- `ATRIUM_BASE_URL`（兼容 `XTALK_BASE_URL`）
- `ATRIUM_DATABASE_URL`（兼容 `XTALK_DATABASE_URL`）
- `ATRIUM_JWT_SECRET`（兼容 `XTALK_JWT_SECRET`）
- `ATRIUM_LISTEN`（兼容 `XTALK_LISTEN`）

### 2. Cloudflare Worker

```bash
cargo install worker-build
worker-build --release --features worker
```

Worker 配置见 `deploy/worker/wrangler.toml`。

## 与前端评论库接入

以 `utterances` 为例，将其 API 基地址指向 Atrium 服务地址即可。Atrium 会保持 GitHub 风格接口，减少前端改造成本。

## 测试与覆盖率

运行测试：

```bash
cargo test --features "server,test-utils" --no-fail-fast
```

统计覆盖率（建议关注库代码）：

```bash
cargo llvm-cov --features "server,test-utils" --lib --summary-only
```

## 许可证

本项目使用 `Apache-2.0`，许可证全文见 `LICENSE`。
