# xtalk E2E 测试方案

## 变更记录

| 版本 | 变更内容 |
|------|---------|
| v0.1 | 初始设计：认证绕过、TestApp 脚手架、测试用例覆盖、Python 测试脚本 |

---

## 1. 概述

xtalk 有两种部署模式（server / worker），共享相同的路由和业务逻辑。E2E 测试需要：

1. **同一份测试代码**跑在两种模式下，通过环境变量切换目标
2. **认证绕过**必须对两种模式都有效——server 可以用 Rust feature 编译隔离，但 Worker WASM 编译参数固定，必须有运行时机制
3. 测试代码是纯 HTTP 黑盒测试，不感知底层是哪种模式

---

## 2. 认证绕过机制

### 2.1 设计

```
客户端发送：Authorization: testuser {secret}:{id}:{login}:{email}

服务端：
  1. 检查 XTALK_TEST_BYPASS_SECRET 环境变量是否设置
  2. 提取 token 中的 secret 部分
  3. 比对匹配 → 直接使用 {id}:{login}:{email} 作为身份
  4. 不匹配或环境变量未设置 → 继续正常认证流程（不报错，不泄露）
```

### 2.2 两端安全保证

- **server 构建**：`#[cfg(feature = "test-utils")]` 双重保险，生产构建（无此 feature）物理上不包含 bypass 代码
- **worker 构建**：bypass 代码始终编译进去，但 `XTALK_TEST_BYPASS_SECRET` 在 Cloudflare 生产 Worker 中从不配置，运行时检查失败则静默跳过

### 2.3 实现：`src/auth.rs`

```rust
/// 两端都编译，但 server 额外有 feature gate
#[cfg(any(feature = "test-utils", feature = "worker"))]
pub fn try_test_bypass(
    auth_header: &str,
    bypass_secret: Option<&str>,  // 从 AppContext 传入
) -> Option<AuthUser> {
    let bypass_secret = bypass_secret?;             // 未配置 → 直接返回 None
    let rest = auth_header.strip_prefix("testuser ")?;
    let mut parts = rest.splitn(4, ':');
    let secret = parts.next()?;
    if secret != bypass_secret { return None; }     // secret 不匹配 → 返回 None
    let id: i64 = parts.next()?.parse().ok()?;
    let login = parts.next()?.to_string();
    let email = parts.next().unwrap_or("").to_string();
    Some(AuthUser {
        id,
        login,
        email,
        avatar_url: format!("https://avatars.githubusercontent.com/u/{}?v=4", id),
        r#type: "User".to_string(),
        site_admin: false,
    })
}
```

### 2.4 `AppContext` 新增字段

```rust
pub struct AppContext<'a> {
    // ... 现有字段 ...
    pub test_bypass_secret: Option<&'a str>,
}
```

### 2.5 平台适配器注入点（`platform/*/mod.rs`）

```rust
// 认证分支最前面，优先于 github/xtalk 路径
if let Some(h) = &app_req.auth_header {
    if let Some(test_user) = auth::try_test_bypass(h, ctx.test_bypass_secret) {
        upsert_user_if_absent(&state.db, &test_user).await?;
        let ctx = ctx.with_user(Some(&test_user));
        return ROUTER.handle(app_req, &ctx).await;
    }
}
```

---

## 3. 服务端导出接口（供测试调用）

### 3.1 `platform/server/mod.rs` 导出 `build_app()`

当前 `build_app()` 已经是 `pub async fn`，返回 `Result<axum::Router>`。测试直接复用此函数，无需新增导出：

```rust
// 已有签名，不变
pub async fn build_app(
    database_url: &str,
    base_url: String,
    token_cache_ttl: i64,
    cache_max_issues: u64,
    cache_ttl_secs: u64,
    jwt_secret: Vec<u8>,
    google_client_id: Option<String>,
    apple_app_id: Option<String>,
) -> Result<Router>
```

### 3.2 测试中如何启动 server

```rust
// tests/common/mod.rs — TestApp::spawn_server()
async fn spawn_server() -> Self {
    let db_file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let db_url = format!("sqlite:{}?mode=rwc", db_file.display());

    // build_app 内部会执行 migrations
    let app = xtalk::platform::server::build_app(
        &db_url,
        "http://localhost".into(),
        3600,         // token_cache_ttl
        1000,         // cache_max_issues
        60,           // cache_ttl_secs
        b"test-jwt-secret-at-least-32-bytes!!".to_vec(),
        None,         // google_client_id
        None,         // apple_app_id
    ).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Self {
        base_url: format!("http://{}", addr),
        bypass_secret: secret.to_string(),
        _guard: TestGuard::InProcess {
            _handle: handle.abort_handle(),
            _db_file: db_file,
        },
    }
}
```

---

## 4. TestApp：统一测试脚手架

### 4.1 核心设计：通过环境变量切换目标

```
XTALK_TEST_BASE_URL 未设置
  → 自动 spawn axum server（使用临时 SQLite）
  → 测试结束后自动清理

XTALK_TEST_BASE_URL=http://localhost:8788
  → 连接外部已运行的 wrangler dev
  → XTALK_TEST_BYPASS_SECRET 必须同时设置，且与 wrangler dev 的配置一致
```

### 4.2 `tests/common/mod.rs`

```rust
pub struct TestApp {
    pub base_url: String,
    pub bypass_secret: String,
    _guard: TestGuard,
}

enum TestGuard {
    /// server 模式：持有 axum server 句柄 + 临时 DB 文件
    InProcess {
        _handle:  tokio::task::AbortHandle,
        _db_file: tempfile::TempPath,
    },
    /// worker 模式：外部进程，测试只负责发请求
    External,
}

impl TestApp {
    /// 所有测试调用此方法，自动感知模式
    pub async fn start() -> Self {
        match std::env::var("XTALK_TEST_BASE_URL") {
            Ok(url) => {
                let secret = std::env::var("XTALK_TEST_BYPASS_SECRET")
                    .expect("XTALK_TEST_BYPASS_SECRET must be set for external target");
                Self { base_url: url, bypass_secret: secret, _guard: TestGuard::External }
            }
            Err(_) => Self::spawn_server().await,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    // ── 身份帮助方法 ──────────────────────────────
    pub fn as_user(&self, id: i64, login: &str) -> AuthClient {
        AuthClient::new(format!(
            "testuser {}:{}:{}:{}@test.com",
            self.bypass_secret, id, login, login
        ))
    }
    pub fn as_admin(&self) -> AuthClient { self.as_user(1, "admin") }
    pub fn as_alice(&self) -> AuthClient { self.as_user(2, "alice") }
    pub fn as_bob(&self)   -> AuthClient { self.as_user(3, "bob") }
    pub fn as_anon(&self)  -> AuthClient { AuthClient::new_anon() }
}
```

### 4.3 `AuthClient`

```rust
pub struct AuthClient {
    client: reqwest::Client,
    auth:   Option<String>,
}

impl AuthClient {
    pub fn new(auth_header: String) -> Self {
        Self { client: reqwest::Client::new(), auth: Some(auth_header) }
    }
    pub fn new_anon() -> Self {
        Self { client: reqwest::Client::new(), auth: None }
    }

    pub fn get   (&self, url: &str) -> reqwest::RequestBuilder { self.req(|c,u| c.get(u),    url) }
    pub fn post  (&self, url: &str) -> reqwest::RequestBuilder { self.req(|c,u| c.post(u),   url) }
    pub fn patch (&self, url: &str) -> reqwest::RequestBuilder { self.req(|c,u| c.patch(u),  url) }
    pub fn delete(&self, url: &str) -> reqwest::RequestBuilder { self.req(|c,u| c.delete(u), url) }

    fn req(&self, f: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder, url: &str)
        -> reqwest::RequestBuilder
    {
        let b = f(&self.client, url);
        match &self.auth {
            Some(h) => b.header("Authorization", h),
            None    => b,
        }
    }
}
```

---

## 5. 测试文件结构

```
tests/
├── common/
│   ├── mod.rs          # TestApp、AuthClient
│   └── fixtures.rs     # seed_repo / seed_issue / seed_comment
├── e2e/
│   ├── compat_issues.rs
│   ├── compat_comments.rs
│   ├── compat_reactions.rs
│   ├── compat_search.rs
│   ├── native_auth.rs
│   ├── native_threads.rs
│   ├── native_comments.rs
│   ├── native_reactions.rs
│   └── native_export.rs
└── integration_test.rs  # mod 声明入口
```

---

## 6. 测试用例覆盖（两端通用）

### GitHub Compat Issues
```
✓ GET  /repos/:o/:r/issues  未认证 200 + 数组
✓ POST /repos/:o/:r/issues  未认证 401 / 认证后 201
✓ POST 缺少 title           422
✓ GET  /:number 存在 200 / 不存在 404
✓ PATCH 非作者 403 / 作者 200
✓ PATCH state=closed → closed_at 非 null
✓ DELETE 非作者 403 / 删除后 GET 404
```

### GitHub Compat Comments
```
✓ GET 评论列表含 Link header 分页
✓ POST 后 issue.comment_count +1
✓ DELETE 后 comment_count -1
✓ PATCH/DELETE 非作者 403
```

### Reactions
```
✓ POST 新建 201 / 重复 200（内容不变）
✓ reactions JSON 计数正确
✓ DELETE 他人 reaction → 403
```

### Search
```
✓ q=repo:test/blog          返回该 repo issues
✓ q=...+label:bug           标签过滤
✓ q=...+is:closed           状态过滤
✓ q=hello                   自由文本搜索
```

### Native Auth
```
✓ GET /api/v1/auth/me 无 token → 401
✓ GET /api/v1/auth/me testuser bypass → 200 + user
✓ POST /api/v1/auth/google 未配置 → 501
✓ bypass 密钥错误 → 401（不泄露"密钥错误"信息）
```

### Native Threads / Comments
```
✓ 响应体无 node_id / locked / state_reason（GitHub 专有字段）
✓ 用 author 而非 user
✓ 游标分页：limit=2 时 has_more=true
✓ next_cursor 翻页无重叠无遗漏
✓ order=desc 顺序正确
```

### Native Export（数据导出）
```
✓ GET  /api/v1/repos/:o/:r/export 非 admin → 403
✓ GET  /api/v1/repos/:o/:r/export admin → 200 + 完整 JSON
✓ 导出数据包含 repo / labels / threads / comments / reactions
✓ 导出中 comments 嵌套在对应 thread 内
✓ ?format=csv → Content-Type: text/csv，每行一条 comment
✓ ?since=ISO8601 → 仅返回该时间之后更新的数据
✓ 空仓库导出 → 200 + threads: []
```

---

## 7. Cargo.toml 变更

```toml
[features]
test-utils = []  # server 构建的编译期保险；生产构建绝对不包含

[dev-dependencies]
reqwest  = { version = "0.12", features = ["json"] }
tokio    = { version = "1",    features = ["full"] }
tempfile = "3"
```

---

## 8. 运行方式

```bash
# ── server 模式（自动 spawn，无需外部进程）──
python scripts/test.py server
# 等价于: cargo test --features "server,test-utils" --test integration_test

# ── worker 模式（自动创建/初始化/删除 D1）──
# 需要: npx wrangler login（或 CLOUDFLARE_API_TOKEN 环境变量）
python scripts/test.py worker

# ── 全量（server + worker）──
python scripts/test.py all

# ── 只跑某一组 ──
python scripts/test.py server compat_issues
python scripts/test.py worker native_export
```

---

## 9. 测试脚本（`scripts/test.py`）

### 9.1 `deploy/worker/wrangler.test.toml.template`

提交到 git 的模板文件，脚本运行时替换占位符（基于实际 `wrangler.toml` 结构，去掉 `[build]` 避免重复构建）：

```toml
# Test environment — generated by scripts/test.py, do not edit manually
name = "xtalk-test"
main = "../../build/worker/shim.mjs"
compatibility_date = "2024-09-23"

[vars]
BASE_URL = "http://localhost:__TEST_PORT__"
TOKEN_CACHE_TTL = "3600"
GOOGLE_CLIENT_ID = ""
APPLE_APP_ID = ""
XTALK_TEST_BYPASS_SECRET = "__TEST_BYPASS_SECRET__"
JWT_SECRET = "__TEST_JWT_SECRET__"

[[d1_databases]]
binding = "DB"
database_name = "__TEST_DB_NAME__"
database_id = "__TEST_DB_ID__"
```

### 9.2 `scripts/test.py`

```python
#!/usr/bin/env python3
"""
scripts/test.py — xtalk test runner

Usage:
    python scripts/test.py server              # in-process axum + temp SQLite
    python scripts/test.py worker              # wrangler dev + temp D1
    python scripts/test.py all                 # both (default)
    python scripts/test.py worker compat_issues  # filter tests
"""

import argparse, json, os, re, secrets, subprocess, sys, tempfile, time
import urllib.request
from pathlib import Path

SCRIPT_DIR     = Path(__file__).parent.resolve()
PROJECT_ROOT   = SCRIPT_DIR.parent
WORKER_DIR     = PROJECT_ROOT / "deploy" / "worker"
MIGRATIONS_DIR = PROJECT_ROOT / "migrations"


def banner(msg: str) -> None:
    print(f"\n{'═' * 50}\n  {msg}\n{'═' * 50}")


def run(cmd: list, **kwargs) -> None:
    print(f"  $ {' '.join(str(c) for c in cmd)}")
    subprocess.run(cmd, check=True, **kwargs)


# ── Server mode ───────────────────────────────────────────────────────────────

def run_server_tests(extra: list[str]) -> None:
    banner("Running SERVER tests")
    run(
        ["cargo", "test", "--features", "server,test-utils",
         "--test", "integration_test"] + extra,
        cwd=PROJECT_ROOT,
    )


# ── Worker mode ───────────────────────────────────────────────────────────────

def run_worker_tests(extra: list[str]) -> None:
    banner("Running WORKER tests")

    test_db_name  = f"xtalk-test-{int(time.time())}"
    test_port     = 8788
    bypass_secret = secrets.token_hex(16)
    jwt_secret    = secrets.token_urlsafe(32)

    test_db_id:    str | None              = None
    wrangler_proc: subprocess.Popen | None = None
    temp_cfg_path: str | None              = None

    def cleanup() -> None:
        print("\n--- Cleanup ---")
        if wrangler_proc and wrangler_proc.poll() is None:
            print(f"Stopping wrangler dev (PID {wrangler_proc.pid})…")
            wrangler_proc.terminate()
            try:
                wrangler_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                wrangler_proc.kill()
        if test_db_id:
            print(f"Deleting D1 database: {test_db_name}")
            subprocess.run(
                ["npx", "wrangler", "d1", "delete", test_db_name, "--yes"],
                cwd=WORKER_DIR, check=False,
            )
        if temp_cfg_path:
            Path(temp_cfg_path).unlink(missing_ok=True)

    try:
        # ── Step 1: Create D1 test database ──────────────────────
        print(f"\n[1/5] Creating D1 database: {test_db_name}")
        result = subprocess.run(
            ["npx", "wrangler", "d1", "create", test_db_name, "--json"],
            cwd=WORKER_DIR, capture_output=True, text=True,
        )
        if result.returncode == 0:
            try:
                data = json.loads(result.stdout)
                test_db_id = (
                    data.get("uuid")
                    or (data.get("result") or {}).get("uuid")
                )
            except (json.JSONDecodeError, AttributeError):
                pass
        if not test_db_id:
            r2 = subprocess.run(
                ["npx", "wrangler", "d1", "create", test_db_name],
                cwd=WORKER_DIR, capture_output=True, text=True, check=True,
            )
            m = re.search(
                r'database_id\s*=\s*"([^"]+)"', r2.stdout + r2.stderr
            )
            if not m:
                print("ERROR: cannot parse DB ID"); sys.exit(1)
            test_db_id = m.group(1)
        print(f"  DB ID: {test_db_id}")

        # ── Step 2: Apply all migrations ─────────────────────────
        print("\n[2/5] Applying migrations…")
        for migration in sorted(MIGRATIONS_DIR.glob("*.sql")):
            print(f"  {migration.name}")
            run(
                ["npx", "wrangler", "d1", "execute", test_db_name,
                 "--file", str(migration), "--remote"],
                cwd=WORKER_DIR,
            )

        # ── Step 3: Generate temp wrangler config ─────────────────
        print("\n[3/5] Generating test wrangler config…")
        template = (WORKER_DIR / "wrangler.test.toml.template").read_text()
        config = (
            template
            .replace("__TEST_DB_NAME__",       test_db_name)
            .replace("__TEST_DB_ID__",         test_db_id)
            .replace("__TEST_PORT__",          str(test_port))
            .replace("__TEST_BYPASS_SECRET__", bypass_secret)
            .replace("__TEST_JWT_SECRET__",    jwt_secret)
        )
        fd, temp_cfg_path = tempfile.mkstemp(
            suffix=".toml", prefix="wrangler-test-"
        )
        os.write(fd, config.encode())
        os.close(fd)

        # ── Step 4: Start wrangler dev ────────────────────────────
        print(f"\n[4/5] Starting wrangler dev on :{test_port}…")
        wrangler_proc = subprocess.Popen(
            ["npx", "wrangler", "dev",
             "--config", temp_cfg_path,
             "--port",   str(test_port),
             "--log-level", "error"],
            cwd=WORKER_DIR,
        )
        for _ in range(30):
            try:
                urllib.request.urlopen(
                    f"http://localhost:{test_port}/", timeout=1
                )
                break
            except Exception:
                time.sleep(1)
        else:
            print("ERROR: wrangler dev did not start within 30 s")
            sys.exit(1)
        print("  wrangler dev ready")

        # ── Step 5: Run integration tests ─────────────────────────
        print("\n[5/5] Running integration tests…")
        env = os.environ.copy()
        env["XTALK_TEST_BASE_URL"]      = f"http://localhost:{test_port}"
        env["XTALK_TEST_BYPASS_SECRET"] = bypass_secret
        run(
            ["cargo", "test", "--test", "integration_test"] + extra,
            cwd=PROJECT_ROOT, env=env,
        )
    finally:
        cleanup()


# ── Entry point ───────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="xtalk test runner")
    parser.add_argument(
        "mode", nargs="?", default="all",
        choices=["server", "worker", "all"],
    )
    parser.add_argument("extra", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    if args.mode == "server":
        run_server_tests(args.extra)
    elif args.mode == "worker":
        run_worker_tests(args.extra)
    else:
        run_server_tests([])
        run_worker_tests([])
        print("\n✅ All tests passed (server + worker)")


if __name__ == "__main__":
    main()
```

### 9.3 Worker 测试生命周期

```
scripts/test.py worker
  │
  ├─ [1/5] wrangler d1 create xtalk-test-{ts} --json
  │         → 解析 database_id
  │
  ├─ [2/5] wrangler d1 execute ... --file 0001_*.sql --remote
  │         wrangler d1 execute ... --file 0002_*.sql --remote
  │
  ├─ [3/5] 模板替换 → 生成临时 wrangler.toml
  │
  ├─ [4/5] wrangler dev --config {tmp} --port 8788
  │         └─ 轮询 30s 等待就绪
  │
  ├─ [5/5] XTALK_TEST_BASE_URL=... cargo test --test integration_test
  │
  └─ cleanup（finally）
      ├─ kill wrangler dev
      ├─ wrangler d1 delete xtalk-test-{ts} --yes
      └─ rm 临时 toml
```

### 9.4 前提条件

- Python 3.10+
- wrangler v3+（支持 `d1 create --json`）
- `npx wrangler login` 已完成认证（CI 中使用 `CLOUDFLARE_API_TOKEN`）
- Worker 已构建（`build/worker/shim.mjs` 存在），测试模板不含 `[build]` 以避免重复构建

---

## 10. 需要修改的文件

| 文件 | 改动 |
|------|------|
| `Cargo.toml` | 加 `test-utils` feature；加 dev-dependencies |
| `src/auth.rs` | 加 `try_test_bypass()`（`cfg(any(test-utils, worker))`） |
| `src/lib.rs` | `AppContext` 加 `test_bypass_secret: Option<&'a str>` |
| `platform/server/mod.rs` | bypass 注入（`build_app()` 已是 pub，无需额外导出） |
| `platform/worker/mod.rs` | bypass 注入（读 env binding） |
| `tests/common/mod.rs` | 新增 TestApp、AuthClient |
| `tests/common/fixtures.rs` | 新增 seed 函数 |
| `tests/e2e/*.rs` | 新增各端点测试（含 `native_export.rs`） |
| `tests/integration_test.rs` | 新增 mod 入口 |
| `scripts/test.py` | 新增 Python 测试脚本 |
| `deploy/worker/wrangler.test.toml.template` | 新增 wrangler 测试配置模板 |
| `deploy/worker/wrangler.toml` | 开发/CI 环境加 `XTALK_TEST_BYPASS_SECRET` 注释说明 |
