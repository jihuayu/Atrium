# xtalk 鎵╁睍璁捐锛歂ative API + 澶?Provider 璁よ瘉

## 鍙樻洿璁板綍

| 鐗堟湰 | 鍙樻洿鍐呭 |
|------|---------|
| v0.1 | 鍒濆璁捐锛歂ative `/api/v1/` 鎺ュ彛銆丟oogle/Apple 璁よ瘉銆亁talk JWT銆佸 Provider 鐢ㄦ埛鍏宠仈 |

---

## 1. 姒傝堪

鍦ㄧ幇鏈?GitHub Issues 鍏煎鎺ュ彛鐨勫熀纭€涓婏紝鏂板涓ゅぇ鑳藉姏锛?

1. **`/api/v1/` 鑷湁鎺ュ彛** 鈥?涓撲负璇勮绯荤粺璁捐锛屾父鏍囧垎椤点€佺畝娲佸搷搴旀牸寮忋€佹棤 GitHub 鐗规湁瀛楁
2. **澶?Provider 璁よ瘉** 鈥?鍦?GitHub token 閫忎紶涔嬪鏀寔 Google OAuth銆丄pple Sign-In锛涜嚜鏈夋帴鍙ｄ娇鐢?xtalk 绛惧彂鐨?JWT

**鏍稿績绾︽潫**锛?
- 涓ゅ鎺ュ彛鍏变韩鍚屼竴浠芥暟鎹簱锛坄issues`銆乣comments`銆乣reactions` 绛夎〃涓嶅彉锛?
- GitHub 鍏煎鎺ュ彛淇濇寔瀹屽叏鍚戝悗鍏煎锛岄浂鐮村潖鎬у彉鏇?
- 鎵€鏈夋柊浠ｇ爜閬靛惊鐜版湁 `Database` trait 鎶借薄锛屽悓鏃舵敮鎸?D1 鍜?SQLite

---

## 2. DB Schema 鍙樻洿

### 2.1 杩佺Щ绛栫暐

`users.id` 鐩墠绛変簬 GitHub user ID锛堝閮?ID 鐩存帴浣滀富閿級銆傚 Provider 涓嬮渶瑕佽嚜绠＄悊鐨勮嚜澧炰富閿€?

**鍏抽敭鐐?*锛氳縼绉绘椂淇濈暀鐜版湁琛岀殑 ID 鍊间笉鍙橈紝鎵€鏈?FK锛坮epos/issues/comments 涓殑 `user_id`锛夋棤闇€淇敼銆係QLite/D1 鐨?`AUTOINCREMENT` 鍦ㄦ彃鍏ヤ簡鎸囧畾 ID 鐨勮鍚庯紝浼氫粠 `max(id)+1` 缁х画锛屾柊鐢ㄦ埛鑾峰緱鍏ㄦ柊 ID銆?

### 2.2 `migrations/0002_multi_provider_auth.sql`

```sql
-- Step 1: 閲嶅缓 users 琛ㄤ负鑷 PK锛堜繚鐣欑幇鏈夎鐨?ID 鍊硷級
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

-- Step 2: Provider 韬唤鍏宠仈琛紙澶氬涓€ 鈫?users锛?
CREATE TABLE user_identities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK(provider IN ('github','google','apple')),
    provider_user_id TEXT NOT NULL,   -- GitHub: 鏁存暟 ID 杞?text锛汫oogle/Apple: JWT sub
    email            TEXT NOT NULL DEFAULT '',
    avatar_url       TEXT NOT NULL DEFAULT '',
    cached_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX idx_user_identities_user  ON user_identities(user_id);
CREATE INDEX idx_user_identities_email ON user_identities(email);

-- Step 3: 杩佺Щ宸叉湁 GitHub 韬唤
INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at)
SELECT id, 'github', CAST(id AS TEXT), email, avatar_url, cached_at FROM users_v1;

-- Step 4: token_cache 澧炲姞 provider 鍒楋紙澶嶅悎涓婚敭锛?
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

-- Step 5: Refresh token 瀛樺偍
-- server 閮ㄧ讲锛氬瓨 DB 鏀寔鍚婇攢锛泈orker 閮ㄧ讲锛氭棤鐘舵€?JWT锛屾琛ㄤ笉浣跨敤
CREATE TABLE sessions (
    refresh_token_hash TEXT PRIMARY KEY,   -- SHA-256 of refresh token
    user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at         TEXT NOT NULL,
    revoked_at         TEXT               -- NULL = 鏈夋晥
);
CREATE INDEX idx_sessions_user    ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- Step 6: JWKS 缂撳瓨锛圵orker 鏃犳寔涔呭唴瀛橈紝瀛?D1锛泂erver 鐢?moka 鍐呭瓨缂撳瓨锛?
CREATE TABLE jwks_cache (
    provider   TEXT PRIMARY KEY,  -- 'google' | 'apple'
    jwks_json  TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

-- Cleanup
DROP TABLE users_v1;
DROP TABLE token_cache_v1;
```

### 2.3 鏈€缁堣〃缁撴瀯鎬昏

```
users                          鈫?鑷 PK锛宲rovider 鏃犲叧
  id, login, email, avatar_url, type, site_admin, cached_at

user_identities                鈫?姣忎釜 provider 韬唤涓€琛?
  id, user_id 鈫?users.id
  provider ('github'|'google'|'apple')
  provider_user_id             鈫?涓嶅悓 provider 鐨勫閮?ID
  email, avatar_url, cached_at
  UNIQUE(provider, provider_user_id)

token_cache                    鈫?澶嶅悎 PK (token_hash, provider)
  token_hash, provider, user_id, cached_at, expires_at

sessions                       鈫?server 涓撶敤锛宺efresh token 瀛樺偍
  refresh_token_hash, user_id, created_at, expires_at, revoked_at

jwks_cache                     鈫?Worker 涓撶敤锛孏oogle/Apple 鍏挜缂撳瓨
  provider, jwks_json, expires_at

-- 鍏朵綑琛ㄤ笉鍙橈紙repos, issues, comments, labels, reactions 绛夛級
```

---

## 3. 璁よ瘉鏋舵瀯

### 3.1 Provider 璁よ瘉娴佺▼姒傝

```
瀹㈡埛绔?                      xtalk                         澶栭儴 Provider
  鈹?                           鈹?                               鈹?
  鈹?POST /api/v1/auth/github   鈹?                               鈹?
  鈹?  { "token": "ghp_..." }   鈹?                               鈹?
  鈹?                           鈹傗攢鈹€ GET /user (GitHub) 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈫掆攤
  鈹?                           鈹?                               鈹?
  鈹?POST /api/v1/auth/google   鈹?                               鈹?
  鈹?  { "token": "eyJ..." }    鈹傗攢鈹€ 楠?JWKS 绛惧悕锛堟湰鍦帮級         鈹?
  鈹?                           鈹?  锛堜粎棣栨鎷夊彇 JWKS 鏃跺缃戯級    鈹?
  鈹?                           鈹?                               鈹?
  鈹?POST /api/v1/auth/apple    鈹?                               鈹?
  鈹?  { "token": "eyJ..." }    鈹傗攢鈹€ 楠?JWKS 绛惧悕锛堟湰鍦帮級         鈹?
  鈹?                           鈹?                               鈹?
  鈹?                           鈹?resolve_or_create_user()       鈹?
  鈹?                           鈹?issue_xtalk_jwt()              鈹?
  鈹?                           鈹?                               鈹?
  鈹傗啇鈹€ { access_token,          鈹?                               鈹?
  鈹?    refresh_token, user }  鈹?                               鈹?
  鈹?                           鈹?                               鈹?
  鈹?GET /api/v1/...            鈹?                               鈹?
  鈹? Authorization: Bearer JWT 鈹?                               鈹?
  鈹?                           鈹傗攢鈹€ verify_jwt() 鏈湴楠岀        鈹?
  鈹?                           鈹?  (鏃犲缃戣皟鐢?                  鈹?
  鈹傗啇鈹€ response                 鈹?                               鈹?
```

### 3.2 璺緞鍓嶇紑椹卞姩鐨勮璇佸垎鏀?

鍦ㄥ钩鍙伴€傞厤鍣紙`platform/*/mod.rs`锛変腑锛屾寜璇锋眰璺緞鍐冲畾鐢ㄥ摢濂楄璇侊細

```rust
let user = if path.starts_with("/api/v1/auth/") {
    // 璁よ瘉浜ゆ崲绔偣锛宧andler 鍐呴儴鑷澶勭悊锛屾棤闇€棰勮璇?
    None
} else if path.starts_with("/api/v1/") {
    // Native API锛氶獙璇?xtalk 绛惧彂鐨?JWT
    resolve_xtalk_jwt_user(auth_header, jwt_secret, db).await?
} else {
    // GitHub 鍏煎璺緞锛?repos/鈥? /search/鈥︼級锛氬師鏈?GitHub token 閫忎紶
    resolve_github_user(auth_header, db, http, ttl).await?
};
```

### 3.3 GitHub 鍏煎璺緞锛堟渶灏忔敼鍔級

`resolve_github_user()` 鍐?SQL 浠呭鍔?`AND tc.provider = 'github'`锛?

```sql
-- token_cache 鏌ヨ锛堝姞 provider 杩囨护锛?
SELECT u.* FROM token_cache tc
JOIN users u ON tc.user_id = u.id
WHERE tc.token_hash = ?1 AND tc.provider = 'github' AND tc.expires_at > datetime('now')

-- upsert锛圤N CONFLICT 鏀逛负澶嶅悎閿級
INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at)
VALUES (?1, 'github', ?2, datetime('now'), ...)
ON CONFLICT(token_hash, provider) DO UPDATE SET ...
```

鍏朵綑閫昏緫锛堣皟 GitHub `/user`銆乽psert users锛夊畬鍏ㄤ笉鍙樸€?

### 3.4 Google / Apple JWKS 楠岀

涓よ€呭潎涓?JWT锛岄獙绛炬祦绋嬶細

1. 瑙ｇ爜 JWT header 鍙?`kid`
2. 浠庣紦瀛樺彇 JWKS锛圵orker: `jwks_cache` D1 琛紱server: moka锛?
3. 缂撳瓨 miss 鈫?`HttpClient::get_jwks(url)` 鎷夊彇锛屾寜 `Cache-Control: max-age` 璁?TTL
4. 鎸?`kid` 鎵惧叕閽ワ細Google 鐢?RSA-2048锛坄rsa` crate锛夛紝Apple 鐢?P-256锛坄p256` crate锛?
5. 楠岀 + 鏍￠獙 `exp`銆乣iss`銆乣aud`锛坄aud` 浠庣幆澧冨彉閲忚鍙栵級
6. 鎻愬彇 `sub`銆乣email`銆乣email_verified`銆乣picture`

| Provider | JWKS URL | `iss` | `aud` 閰嶇疆椤?|
|----------|----------|-------|-------------|
| Google | `https://www.googleapis.com/oauth2/v3/certs` | `https://accounts.google.com` | `XTALK_GOOGLE_CLIENT_ID` |
| Apple | `https://appleid.apple.com/auth/keys` | `https://appleid.apple.com` | `XTALK_APPLE_APP_ID` |

**鏈厤缃?= 璇ユ笭閬撹嚜鍔ㄥ叧闂?*銆俙AppContext` 涓搴斿瓧娈电被鍨嬩负 `Option<&'a str>`锛岀幆澧冨彉閲忎笉瀛樺湪鎴栦负绌烘椂浼?`None`锛宧andler 鍏ュ彛澶勭洿鎺ヨ繑鍥?`501`锛?

```rust
// src/handlers/api/auth.rs
pub async fn auth_google(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let Some(client_id) = ctx.google_client_id else {
        return AppResponse::json(501, &json!({
            "error": "not_configured",
            "message": "Google login is not enabled on this server"
        }));
    };
    // ... 姝ｅ父楠岀娴佺▼
}
```

| 鐜鍙橀噺鐘舵€?| `AppContext` 瀛楁 | 琛屼负 |
|-------------|-----------------|------|
| 鏈缃?| `None` | `501 Not Configured` |
| 璁剧疆涓虹┖瀛楃涓?| `None` | `501 Not Configured` |
| 姝ｅ父鍊?| `Some("...")` | 姝ｅ父楠岀 |

GitHub 娓犻亾鏃犻渶 client_id锛坱oken 閫忎紶鍒?GitHub API 楠岃瘉锛夛紝濮嬬粓寮€鍚€?

鏂板渚濊禆锛堢函 Rust锛學ASM 鍏煎锛屽姞鍏ュ叡浜眰锛夛細

```toml
hmac = "0.12"
rsa  = { version = "0.9", default-features = false, features = ["sha2"] }
p256 = { version = "0.13", default-features = false, features = ["ecdsa"] }
```

### 3.5 xtalk JWT 璁捐

**鏍煎紡**锛圚S256锛屽绉扮鍚嶏紝WASM 鍏煎锛夛細

```
Header:  { "alg": "HS256", "typ": "JWT" }
Payload: { "sub": "42", "login": "alice", "iss": "xtalk",
           "iat": 1700000000, "exp": 1700003600, "jti": "uuid" }
```

| Token 绫诲瀷 | TTL | 瀛樺偍 |
|-----------|-----|------|
| Access token | 1 灏忔椂 | 涓嶅瓨 DB锛坰tateless锛?|
| Refresh token | 30 澶?| server: `sessions` 琛紙鍙悐閿€锛夛紱worker: 闀挎湡 JWT锛堟棤鐘舵€侊級 |

**瀵嗛挜閰嶇疆**锛?
- server: 鐜鍙橀噺 `XTALK_JWT_SECRET`锛堚墺32 瀛楄妭锛宐ase64锛?
- worker: wrangler secret `JWT_SECRET`

**瀹炵幇**锛坄src/jwt.rs`锛屼粎鐢?`hmac` + `sha2` + `base64`锛屼笉渚濊禆 `ring` / `jsonwebtoken`锛夛細

```rust
pub fn sign_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String>;
pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<JwtClaims>;
```

### 3.6 鐢ㄦ埛璐﹀彿鍏宠仈锛堟寜 email 鑷姩鍚堝苟锛?

```
resolve_or_create_user(db, provider, provider_user_id, email, ...):

  1. SELECT user_id FROM user_identities
     WHERE provider = ?1 AND provider_user_id = ?2
     鈫?鍛戒腑锛氱洿鎺ヨ繑鍥炶 user_id锛堟渶蹇矾寰勶級

  2. 鑻?email 涓嶄负绌?涓斾笉鍚?'privaterelay.appleid.com'锛?
     SELECT id FROM users WHERE email = ?
     鈫?鍛戒腑锛欼NSERT INTO user_identities 灏嗘柊 provider 鍏宠仈鍒板凡鏈夌敤鎴?
             锛堜笉鍚?provider 鍚岄偖绠?= 鍚屼竴涓汉锛?

  3. 鍚﹀垯锛?
     INSERT INTO users (login, email, avatar_url, ...)
     INSERT INTO user_identities (user_id, provider, provider_user_id, ...)
     杩斿洖鏂板缓鐨?user_id
```

Apple 绉佷汉涓户閭锛坄@privaterelay.appleid.com`锛変笉鍙備笌璺?provider 鍏宠仈锛岄伩鍏嶈鍚堝苟銆?

---

## 4. Native API 绔偣

### 4.1 Auth

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `POST` | `/api/v1/auth/github` | 鏃?| 浜ゆ崲 GitHub token 鈫?xtalk JWT |
| `POST` | `/api/v1/auth/google` | 鏃?| 浜ゆ崲 Google ID token 鈫?xtalk JWT |
| `POST` | `/api/v1/auth/apple` | 鏃?| 浜ゆ崲 Apple identity token 鈫?xtalk JWT |
| `POST` | `/api/v1/auth/refresh` | refresh JWT | 缁湡 |
| `DELETE` | `/api/v1/auth/session` | JWT | 吊销该用户全部 sessions |
| `GET` | `/api/v1/auth/me` | JWT | 鑾峰彇褰撳墠鐢ㄦ埛 |

**璇锋眰浣?*锛歚{ "token": "provider_issued_token" }`

**鍝嶅簲**锛?
```json
{
  "access_token": "eyJ...",
  "refresh_token": "eyJ...",
  "expires_in": 3600,
  "token_type": "Bearer",
  "user": { "id": 42, "login": "alice", "avatar_url": "...", "email": "..." }
}
```

### 4.2 Threads锛堝鐢?issues 琛級

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/threads` | 鍙€?| 鍒楀嚭锛堟父鏍囧垎椤碉級 |
| `POST` | `/api/v1/repos/:owner/:repo/threads` | 蹇呴』 | 鍒涘缓 |
| `GET` | `/api/v1/repos/:owner/:repo/threads/:number` | 鍙€?| 鑾峰彇 |
| `PATCH` | `/api/v1/repos/:owner/:repo/threads/:number` | 蹇呴』 | 鏇存柊锛堜綔鑰?admin锛?|
| `DELETE` | `/api/v1/repos/:owner/:repo/threads/:number` | admin | 鍒犻櫎 |

**GET 鏌ヨ鍙傛暟**锛歚state`锛坥pen/closed/all锛夈€乣limit`锛坢ax 100锛夈€乣cursor`銆乣direction`锛坅sc/desc锛?

### 4.3 Comments

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/threads/:number/comments` | 鍙€?| 鍒楀嚭锛堟父鏍囧垎椤碉級 |
| `POST` | `/api/v1/repos/:owner/:repo/threads/:number/comments` | 蹇呴』 | 鍒涘缓 |
| `GET` | `/api/v1/repos/:owner/:repo/comments/:id` | 鍙€?| 鑾峰彇 |
| `PATCH` | `/api/v1/repos/:owner/:repo/comments/:id` | 蹇呴』 | 鏇存柊 |
| `DELETE` | `/api/v1/repos/:owner/:repo/comments/:id` | 蹇呴』 | 鍒犻櫎 |

**GET 鏌ヨ鍙傛暟**锛歚limit`銆乣cursor`銆乣order`锛坅sc/desc锛?

### 4.4 Reactions

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `POST` | `/api/v1/repos/:owner/:repo/comments/:id/reactions` | 蹇呴』 | 娣诲姞 |
| `DELETE` | `/api/v1/repos/:owner/:repo/comments/:id/reactions/:content` | 蹇呴』 | 鍒犻櫎锛堢敤 `+1` 绛変綔璺緞鍙傛暟锛?|

### 4.5 Labels

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/labels` | 鍙€?| 鍒楀嚭 |
| `POST` | `/api/v1/repos/:owner/:repo/labels` | admin | 鍒涘缓 |
| `DELETE` | `/api/v1/repos/:owner/:repo/labels/:name` | admin | 鍒犻櫎 |

### 4.6 Export锛堟暟鎹鍑猴級

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `GET` | `/api/v1/repos/:owner/:repo/export` | admin | 瀵煎嚭瀹屾暣浠撳簱鏁版嵁锛坱hreads + comments + reactions + labels锛?|

**鏌ヨ鍙傛暟**锛歚format`锛坄json`锛堥粯璁わ級/ `csv`锛夈€乣since`锛圛SO 8601锛屼粎瀵煎嚭姝ゆ椂闂翠箣鍚庣殑鏁版嵁锛?

**JSON 鍝嶅簲**锛堟祦寮忚緭鍑猴紝澶т粨搴撲笉浼?OOM锛夛細

```json
{
  "repo": { "owner": "user", "name": "blog" },
  "exported_at": "2025-01-15T08:00:00Z",
  "labels": [
    { "id": 1, "name": "bug", "color": "d73a4a" }
  ],
  "threads": [
    {
      "number": 1,
      "title": "Hello",
      "body": "...",
      "state": "open",
      "author": { "id": 42, "login": "alice" },
      "labels": ["bug"],
      "created_at": "...",
      "updated_at": "...",
      "comments": [
        {
          "id": 1,
          "body": "...",
          "author": { "id": 43, "login": "bob" },
          "reactions": { "+1": 2, "heart": 1 },
          "created_at": "...",
          "updated_at": "..."
        }
      ]
    }
  ]
}
```

**CSV 鏍煎紡**锛氭寜 thread 灞曞紑锛屾瘡琛屼竴鏉?comment锛堝惈 thread 鍏冩暟鎹垪锛夛紝閫傚悎瀵煎叆鐢靛瓙琛ㄦ牸銆?

**璁捐瑕佺偣**锛?
- Admin 鏉冮檺锛岄槻姝㈡櫘閫氱敤鎴锋壒閲忔媺鍙栨暟鎹?
- 鏁版嵁搴撲晶鎸?thread number 鍗囧簭閫愭潯鏌ヨ锛屾嫾娴佸紡 JSON 鍐欏嚭锛屽唴瀛樺崰鐢ㄤ笌鎬婚噺鏃犲叧
- `since` 鍙傛暟鐢ㄤ簬澧為噺澶囦唤锛歚WHERE updated_at >= ?1`

### 4.7 Admin

| Method | Path | Auth | 璇存槑 |
|--------|------|------|------|
| `POST` | `/api/v1/repos` | JWT | 显式创建 `_global/{name}` 仓库并将 admin 设为当前用户 |
| `GET` | `/api/v1/repos/:owner/:repo` | admin | 鑾峰彇浠撳簱璁剧疆 |
| `PATCH` | `/api/v1/repos/:owner/:repo` | admin | 鏇存柊浠撳簱璁剧疆 |

### 4.8 鍝嶅簲鏍煎紡

**鍒楄〃鍝嶅簲**锛堢粺涓€鍖呰锛夛細
```json
{
  "data": [ ... ],
  "pagination": { "next_cursor": "eyJpZCI6NDJ9", "has_more": true }
}
```

**Thread 瀵硅薄**锛堟棤 GitHub 鐗规湁瀛楁锛夛細
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

**閿欒鍝嶅簲**锛堜笌 compat 璺緞涓嶅悓鏍煎紡锛夛細
```json
{ "error": "unauthorized", "message": "Authentication required" }
```

### 4.9 娓告爣鍒嗛〉

娓告爣 = `base64url( {"id": last_seen_id} )`锛孲QL 浣跨敤 `WHERE id > ?1 ORDER BY id ASC LIMIT ?2`锛屾棤 OFFSET锛屽苟鍙戞彃鍏ュ畨鍏ㄣ€?

---

## 5. 浠ｇ爜缁撴瀯鍙樻洿

### 5.1 鏂板鏂囦欢

| 鏂囦欢 | 鑱岃矗 |
|------|------|
| `src/jwt.rs` | HS256 JWT 绛惧彂/楠岀锛坄hmac` + `sha2` + `base64`锛學ASM 鍏煎锛?|
| `src/jwks.rs` | Google/Apple JWKS 鎷夊彇 + RS256/ES256 楠岀锛沗JwksCache` trait |
| `src/services/auth.rs` | `resolve_or_create_user()`銆乣issue_xtalk_jwt()`銆乣refresh_jwt()` |
| `src/services/session.rs` | `create_session()`銆乣revoke_session()`锛坰erver 鐢級 |
| `src/services/cursor.rs` | 娓告爣缂栬В鐮?`encode_cursor()` / `decode_cursor()` |
| `src/handlers/api/auth.rs` | POST auth/github\|google\|apple, refresh, me, session delete |
| `src/handlers/api/threads.rs` | Thread CRUD handler |
| `src/handlers/api/comments.rs` | Comment CRUD handler |
| `src/handlers/api/reactions.rs` | Reaction add/remove handler |
| `src/handlers/api/labels.rs` | Label CRUD handler |
| `src/handlers/api/admin.rs` | 浠撳簱璁剧疆 handler |
| `src/handlers/api/export.rs` | 鏁版嵁瀵煎嚭 handler锛圝SON/CSV 娴佸紡杈撳嚭锛?|
| `src/fmt/api.rs` | DB row 鈫?native API JSON 鏍煎紡 |
| `migrations/0002_multi_provider_auth.sql` | Schema 杩佺Щ |

### 5.2 淇敼鏂囦欢

| 鏂囦欢 | 鍏抽敭鏀瑰姩 |
|------|---------|
| `src/auth.rs` | `resolve_user()` 鏀瑰悕 `resolve_github_user()`锛孲QL 鍔?`AND provider='github'`锛涙柊澧?`resolve_xtalk_jwt_user()` |
| `src/types.rs` | 鍔?`pub type AuthUser = GitHubUser`锛涘姞 `JwtClaims`銆乣NativeThreadResponse`銆乣NativeCommentResponse`銆乣CursorPage<T>`銆乣AuthTokenResponse`銆乣ProviderUser` |
| `src/lib.rs` | `AppContext` 鍔?`jwt_secret: &'a [u8]`銆乣google_client_id: Option<&'a str>`銆乣apple_app_id: Option<&'a str>` |
| `src/error.rs` | 鍔?`to_native_response()` 杩斿洖 `{"error":"鈥?,"message":"鈥?}` |
| `src/router.rs` | `Route` 鏋氫妇鍔?~30 涓?`Api*` 鍙樹綋锛涙敞鍐屾墍鏈?`/api/v1/` 璺敱锛沝ispatch 鍔犳柊鍒嗘敮 |
| `platform/worker/mod.rs` | `WorkerState` 鍔?JWT 閰嶇疆锛涙寜璺緞鍓嶇紑閫夎璇佹柟寮?|
| `platform/server/mod.rs` | `AppState` 鍔?JWT 閰嶇疆锛涙寜璺緞鍓嶇紑閫夎璇佹柟寮忥紱JWKS 鐢?moka 缂撳瓨 |
| `Cargo.toml` | 鍏变韩灞傚姞 `hmac`銆乣rsa`銆乣p256` |
| `deploy/worker/wrangler.toml` | 鍔?secrets: `JWT_SECRET`銆乣GOOGLE_CLIENT_ID`銆乣APPLE_APP_ID` |

---

## 6. 瀹炵幇椤哄簭

| 姝ラ | 鏂囦欢 |
|------|------|
| 1 | `migrations/0002_multi_provider_auth.sql` |
| 2 | `src/jwt.rs` |
| 3 | `src/types.rs`锛堟柊澧炵被鍨嬶級 |
| 4 | `src/auth.rs`锛堟媶鍒?github/xtalk 璁よ瘉锛屼慨 SQL锛?|
| 5 | `src/jwks.rs`锛圙oogle/Apple 楠岀锛?|
| 6 | `src/services/auth.rs`锛堢敤鎴峰叧鑱?+ JWT 绛惧彂锛?|
| 7 | `src/services/session.rs` + `cursor.rs` |
| 8 | `src/fmt/api.rs`锛坣ative 鍝嶅簲鏍煎紡锛?|
| 9 | `src/handlers/api/`锛堝叏閮?handler锛?|
| 10 | `src/router.rs`锛堟敞鍐屾柊璺敱锛?|
| 11 | `src/lib.rs`锛堟墿灞?AppContext锛?|
| 12 | `platform/*/mod.rs`锛堣矾寰勫墠缂€璁よ瘉鍒嗘敮锛?|
| 13 | `Cargo.toml` + `wrangler.toml` |

---

## 7. 閰嶇疆鍙傝€?

### 7.1 Cloudflare Workers锛坄deploy/worker/wrangler.toml`锛?

```toml
[vars]
BASE_URL = "https://xtalk.yourdomain.com"
TOKEN_CACHE_TTL = "3600"

# JWT_SECRET 蹇呴』閰嶇疆锛屽惁鍒?native API 鏃犳硶绛惧彂 token锛堚墺32瀛楄妭锛屽缓璁敤 openssl rand -base64 32 鐢熸垚锛?
# GOOGLE_CLIENT_ID / APPLE_APP_ID 涓嶅～鍒欏搴旂櫥褰曟笭閬撹嚜鍔ㄥ叧闂紝杩斿洖 501

# 閫氳繃 wrangler secret put JWT_SECRET 璁剧疆锛堟晱鎰熷€间笉鍐?toml 鏄庢枃锛?
# 鍙€?
# wrangler secret put GOOGLE_CLIENT_ID
# wrangler secret put APPLE_APP_ID
```

### 7.2 瀹瑰櫒閮ㄧ讲锛堢幆澧冨彉閲忥級

| 鍙橀噺 | 蹇呭～ | 璇存槑 |
|------|------|------|
| `XTALK_JWT_SECRET` | **鏄?* | HS256 绛惧悕瀵嗛挜锛屸墺32 瀛楄妭锛宐ase64 缂栫爜 |
| `XTALK_GOOGLE_CLIENT_ID` | 鍚?| 涓嶅～鍒?`/api/v1/auth/google` 杩斿洖 `501` |
| `XTALK_APPLE_APP_ID` | 鍚?| 涓嶅～鍒?`/api/v1/auth/apple` 杩斿洖 `501` |
| `XTALK_BASE_URL` | **鏄?* | 鏈嶅姟瀵瑰鍦板潃 |
| `XTALK_DATABASE_URL` | **鏄?* | `sqlite:///data/xtalk.db` |
| `XTALK_TOKEN_CACHE_TTL` | 鍚?| GitHub token 缂撳瓨绉掓暟锛岄粯璁?`3600` |
| `XTALK_CACHE_TTL` | 鍚?| 璇勮 LRU 缂撳瓨绉掓暟锛岄粯璁?`60` |

### 7.3 娓犻亾鍚敤鐘舵€佷竴瑙?

| 娓犻亾 | 绔偣 | 鍚敤鏉′欢 |
|------|------|---------|
| GitHub | `/api/v1/auth/github` | 濮嬬粓鍚敤锛坱oken 鐩存帴閫忎紶楠岃瘉锛屾棤闇€閰嶇疆锛?|
| Google | `/api/v1/auth/google` | 闇€閰嶇疆 `GOOGLE_CLIENT_ID` |
| Apple | `/api/v1/auth/apple` | 闇€閰嶇疆 `APPLE_APP_ID` |

鏈厤缃椂璁块棶瀵瑰簲绔偣鍝嶅簲锛?

```json
HTTP 501
{ "error": "not_configured", "message": "Google login is not enabled on this server" }
```

---

## 8. 楠岃瘉鏂规

```bash
# 搴旂敤杩佺Щ
wrangler d1 execute xtalk-db --local --file=migrations/0002_multi_provider_auth.sql
# 鎴?server
cargo run --features server &

# GitHub 鎹?xtalk JWT
curl -X POST localhost:3000/api/v1/auth/github \
  -H "Content-Type: application/json" \
  -d '{"token":"ghp_xxx"}'
# 鈫?{ "access_token": "eyJ...", "user": {...} }

# 鏈厤缃?Google 鏃剁殑棰勬湡鍝嶅簲
curl -X POST localhost:3000/api/v1/auth/google \
  -H "Content-Type: application/json" \
  -d '{"token":"google_id_token"}'
# 鈫?501 { "error": "not_configured", "message": "Google login is not enabled on this server" }

# 鐢?xtalk JWT 璁块棶 native API
ACCESS_TOKEN="eyJ..."
curl localhost:3000/api/v1/repos/user/blog/threads \
  -H "Authorization: Bearer $ACCESS_TOKEN"
# 鈫?{ "data": [...], "pagination": {...} }

# 娓告爣缈婚〉
CURSOR="eyJpZCI6NDJ9"
curl "localhost:3000/api/v1/repos/user/blog/threads/1/comments?cursor=$CURSOR&limit=10"

# GitHub 鍏煎璺緞涓嶅彈褰卞搷
curl -H "Authorization: token ghp_xxx" localhost:3000/repos/user/blog/issues
# 鈫?鍘熸湁 GitHub 鍏煎鍝嶅簲锛屽畬鍏ㄤ笉鍙?
```



