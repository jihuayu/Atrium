use std::collections::{HashMap, HashSet};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rsa::{BigUint, Oaep, RsaPrivateKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    AppContext, Result,
    auth::parse_token,
    cookies,
    db::{self, DbValue},
    error::ApiError,
    jwt, markdown,
    services::cursor::{decode_cursor, encode_cursor},
    types::{AuthTokenResponse, GitHubUser, JwtClaims, NativeUser},
};

const ACCOUNT_PROFILE_REFRESH_TTL_SECONDS: i64 = 24 * 60 * 60;
const ACCOUNT_SSO_COOKIE: &str = "__Secure-jihuayu_sso";
const DISCOVERY_PATH: &str = "/.well-known/atrium.json";
const ENCRYPTED_FIELD_PREFIX: &str = "enc:jwe:";
const DISCOVERY_JWE_ALG: &str = "RSA-OAEP-256";
const DISCOVERY_JWE_ENC: &str = "A256GCM";
const TXT_RECORD_PREFIX: &str = "atrium-site=";
const PUBLIC_COMMENT_VISIBILITY_FILTER: &str = "(c.deleted_at IS NULL OR (c.parent_comment_id IS NULL AND EXISTS (SELECT 1 FROM comments child WHERE child.website_id = c.website_id AND child.page_id = c.page_id AND child.parent_comment_id = c.id)))";
const COMMENT_ROW_COLUMNS: &str = "c.id, c.website_id, c.page_id, c.parent_comment_id, c.body, c.user_id, c.created_at, c.updated_at, c.deleted_at, c.reactions, COALESCE(NULLIF(u.display_name, ''), u.login) AS login, COALESCE(NULLIF(u.display_name, ''), u.login) AS display_name, u.email, u.avatar_url, EXISTS(SELECT 1 FROM website_admins wa WHERE wa.website_id = c.website_id AND wa.user_id = c.user_id) AS author_is_website_admin, EXISTS(SELECT 1 FROM website_bans wb WHERE wb.website_id = c.website_id AND wb.user_id = c.user_id AND wb.unbanned_at IS NULL) AS author_is_banned";
const COMMENT_ROW_FROM: &str = "FROM comments c JOIN users u ON u.id = c.user_id";
const ALLOWED_REACTIONS: &[&str] = &[
    "like", "dislike", "heart", "laugh", "hooray", "confused", "rocket", "eyes",
];

#[derive(Debug, Deserialize)]
struct UserRow {
    id: i64,
    login: String,
    #[serde(default)]
    display_name: String,
    email: String,
    avatar_url: String,
    user_type: String,
    site_admin: i64,
    cached_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebsiteRow {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageRow {
    pub id: i64,
    pub website_id: i64,
    pub key: String,
    pub title: String,
    pub url: String,
    pub normalized_url: String,
    pub metadata: Option<String>,
    pub comment_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CommentRow {
    id: i64,
    page_id: i64,
    parent_comment_id: Option<i64>,
    body: String,
    user_id: i64,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    reactions: String,
    login: String,
    display_name: String,
    avatar_url: String,
    author_is_website_admin: i64,
    author_is_banned: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ReactionCounts {
    like: i64,
    dislike: i64,
    heart: i64,
    laugh: i64,
    hooray: i64,
    confused: i64,
    rocket: i64,
    eyes: i64,
    total: i64,
}

struct CommentResponseContext {
    website: WebsiteRow,
    page: Option<PageRow>,
    actor_id: Option<i64>,
    actor_can_moderate: bool,
}

#[derive(Debug, Deserialize)]
struct AccountSessionUser {
    sub: String,
    #[serde(rename = "displayName")]
    display_name_camel: Option<String>,
    display_name: Option<String>,
    name: Option<String>,
    email: Option<String>,
    #[serde(rename = "avatarUrl")]
    avatar_url_camel: Option<String>,
    avatar_url: Option<String>,
    picture: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountIntrospectionResponse {
    active: Option<bool>,
    user: Option<AccountSessionUser>,
    message: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
struct DiscoveryMetadata {
    origin: String,
    website_key: String,
    name: String,
    admin_emails: Vec<String>,
    #[allow(dead_code)]
    contact_email: Option<String>,
    source: String,
}

#[derive(Debug, Clone)]
struct DiscoveryFailure {
    status: &'static str,
    source: Option<String>,
    error: String,
}

pub async fn upsert_auth_user(ctx: &AppContext<'_>, user: &GitHubUser) -> Result<()> {
    let display_name = display_name(user);
    ctx.db
        .execute(
            "INSERT INTO users (id, login, display_name, email, avatar_url, type, site_admin, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET \
             login = excluded.login, display_name = excluded.display_name, email = excluded.email, \
             avatar_url = excluded.avatar_url, type = excluded.type, site_admin = excluded.site_admin, cached_at = datetime('now')",
            &[
                DbValue::Integer(user.id),
                DbValue::Text(user.login.clone()),
                DbValue::Text(display_name),
                DbValue::Text(user.email.clone()),
                DbValue::Text(user.avatar_url.clone()),
                DbValue::Text(user.r#type.clone()),
                DbValue::Integer(user.site_admin as i64),
            ],
        )
        .await?;

    if let Some(account_sub) = user.account_sub.as_deref() {
        ctx.db
            .execute(
                "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
                 VALUES (?1, 'account', ?2, ?3, ?4, datetime('now')) \
                 ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
                 user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
                &[
                    DbValue::Integer(user.id),
                    DbValue::Text(account_sub.to_string()),
                    DbValue::Text(user.email.clone()),
                    DbValue::Text(user.avatar_url.clone()),
                ],
            )
            .await?;
    }

    claim_pending_website_admins(ctx, user).await
}

pub async fn resolve_native_request_user(
    ctx: &AppContext<'_>,
    auth_header: Option<&str>,
    cookie_header: Option<&str>,
) -> Result<Option<GitHubUser>> {
    let token = auth_header
        .and_then(parse_token)
        .map(str::to_string)
        .or_else(|| {
            cookie_header.and_then(|header| {
                cookies::cookie_value(header, cookies::ACCESS_COOKIE).map(str::to_string)
            })
        });

    if let Some(token) = token {
        let user = resolve_atrium_jwt_user(ctx, &token).await?;
        if is_account_profile_refresh_due(&user) {
            return Ok(resolve_account_cookie_user(ctx, cookie_header)
                .await?
                .or(Some(user)));
        }
        return Ok(Some(user));
    }

    resolve_account_cookie_user(ctx, cookie_header).await
}

pub async fn resolve_atrium_jwt_user(ctx: &AppContext<'_>, token: &str) -> Result<GitHubUser> {
    let claims = jwt::verify_jwt(token, ctx.jwt_secret)?;
    if claims.token_type == "refresh" {
        return Err(ApiError::unauthorized());
    }
    let user_id = claims
        .sub
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;
    let row = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, display_name, email, avatar_url, type AS user_type, site_admin, cached_at FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    Ok(user_from_row(row, None))
}

pub async fn resolve_account_cookie_user(
    ctx: &AppContext<'_>,
    cookie_header: Option<&str>,
) -> Result<Option<GitHubUser>> {
    let Some(cookie_header) = cookie_header else {
        return Ok(None);
    };
    if !cookie_header
        .split(';')
        .any(|part| part.trim().starts_with(&format!("{}=", ACCOUNT_SSO_COOKIE)))
    {
        return Ok(None);
    }

    let account = introspect_account_cookie(ctx, cookie_header).await?;
    let Some(account) = account else {
        return Ok(None);
    };
    resolve_or_create_account_user(ctx, &account)
        .await
        .map(Some)
}

async fn introspect_account_cookie(
    ctx: &AppContext<'_>,
    cookie_header: &str,
) -> Result<Option<AccountSessionUser>> {
    let response = ctx
        .http
        .post_account_introspect(
            &account_base_url(ctx),
            cookie_header,
            ctx.account_internal_secret,
            &account_audience(ctx),
        )
        .await?;
    let payload: AccountIntrospectionResponse =
        serde_json::from_slice(&response.body).unwrap_or(AccountIntrospectionResponse {
            active: Some(false),
            user: None,
            message: None,
            error: None,
        });
    if !(200..300).contains(&response.status) {
        let message = payload
            .message
            .or(payload.error)
            .unwrap_or_else(|| "Account session introspection failed".to_string());
        return Err(ApiError::new(
            if response.status == 401 || response.status == 403 {
                response.status
            } else {
                502
            },
            message,
        ));
    }
    if payload.active.unwrap_or(false) {
        if let Some(user) = payload.user {
            if !user.sub.trim().is_empty() {
                return Ok(Some(user));
            }
        }
    }
    Ok(None)
}

async fn resolve_or_create_account_user(
    ctx: &AppContext<'_>,
    account: &AccountSessionUser,
) -> Result<GitHubUser> {
    let login = format!("account-{}", account.sub);
    let display = first_present([
        account.display_name_camel.as_deref(),
        account.display_name.as_deref(),
        account.name.as_deref(),
        account.email.as_deref(),
    ])
    .unwrap_or_else(|| login.clone());
    let email = account.email.clone().unwrap_or_default();
    let avatar = first_present([
        account.avatar_url_camel.as_deref(),
        account.avatar_url.as_deref(),
        account.picture.as_deref(),
        None,
    ])
    .unwrap_or_default();

    if let Some(identity) = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT u.id, u.login, u.display_name, u.email, u.avatar_url, u.type AS user_type, u.site_admin, u.cached_at \
         FROM user_identities ui JOIN users u ON u.id = ui.user_id \
         WHERE ui.provider = 'account' AND ui.provider_user_id = ?1",
        &[DbValue::Text(account.sub.clone())],
    )
    .await?
    {
        let allocated = allocate_login_for_user(ctx, &login, Some(identity.id)).await?;
        ctx.db
            .execute(
                "UPDATE users SET login = ?1, display_name = ?2, email = ?3, avatar_url = ?4, type = 'User', cached_at = datetime('now') WHERE id = ?5",
                &[
                    DbValue::Text(allocated.clone()),
                    DbValue::Text(display.clone()),
                    DbValue::Text(email.clone()),
                    DbValue::Text(avatar.clone()),
                    DbValue::Integer(identity.id),
                ],
            )
            .await?;
        ctx.db
            .execute(
                "UPDATE user_identities SET email = ?1, avatar_url = ?2, cached_at = datetime('now') WHERE provider = 'account' AND provider_user_id = ?3",
                &[
                    DbValue::Text(email.clone()),
                    DbValue::Text(avatar.clone()),
                    DbValue::Text(account.sub.clone()),
                ],
            )
            .await?;
        let user = GitHubUser {
            id: identity.id,
            login: allocated,
            display_name: display,
            email,
            avatar_url: avatar,
            r#type: "User".to_string(),
            site_admin: false,
            account_sub: Some(account.sub.clone()),
            cached_at: Some(chrono::Utc::now().to_rfc3339()),
        };
        claim_pending_website_admins(ctx, &user).await?;
        return Ok(user);
    }

    let mut user_id = None;
    if !email.is_empty() && !email.ends_with("privaterelay.appleid.com") {
        #[derive(Deserialize)]
        struct IdRow {
            id: i64,
        }
        user_id = db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE email = ?1",
            &[DbValue::Text(email.clone())],
        )
        .await?
        .map(|row| row.id);
    }

    let user_id = if let Some(id) = user_id {
        let allocated = allocate_login_for_user(ctx, &login, Some(id)).await?;
        ctx.db
            .execute(
                "UPDATE users SET login = ?1, display_name = ?2, email = ?3, avatar_url = ?4, type = 'User', cached_at = datetime('now') WHERE id = ?5",
                &[
                    DbValue::Text(allocated),
                    DbValue::Text(display.clone()),
                    DbValue::Text(email.clone()),
                    DbValue::Text(avatar.clone()),
                    DbValue::Integer(id),
                ],
            )
            .await?;
        id
    } else {
        let allocated = allocate_login_for_user(ctx, &login, None).await?;
        ctx.db
            .execute(
                "INSERT INTO users (login, display_name, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, 'User', 0, datetime('now'))",
                &[
                    DbValue::Text(allocated.clone()),
                    DbValue::Text(display.clone()),
                    DbValue::Text(email.clone()),
                    DbValue::Text(avatar.clone()),
                ],
            )
            .await?;
        #[derive(Deserialize)]
        struct IdRow {
            id: i64,
        }
        db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE login = ?1",
            &[DbValue::Text(allocated)],
        )
        .await?
        .ok_or_else(|| ApiError::internal("failed to create user"))?
        .id
    };

    ctx.db
        .execute(
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
             VALUES (?1, 'account', ?2, ?3, ?4, datetime('now')) \
             ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
            &[
                DbValue::Integer(user_id),
                DbValue::Text(account.sub.clone()),
                DbValue::Text(email),
                DbValue::Text(avatar),
            ],
        )
        .await?;

    let row = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, display_name, email, avatar_url, type AS user_type, site_admin, cached_at FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to load user"))?;
    let user = user_from_row(row, Some(account.sub.clone()));
    claim_pending_website_admins(ctx, &user).await?;
    Ok(user)
}

pub async fn issue_atrium_tokens(
    ctx: &AppContext<'_>,
    user: &GitHubUser,
) -> Result<AuthTokenResponse> {
    let now = chrono::Utc::now().timestamp();
    let access = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "atrium".to_string(),
        iat: now,
        exp: now + 3600,
        jti: format!("acc-{}-{}", user.id, now),
        token_type: "access".to_string(),
    };
    let refresh = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "atrium".to_string(),
        iat: now,
        exp: now + 30 * 24 * 3600,
        jti: format!("ref-{}-{}", user.id, now),
        token_type: "refresh".to_string(),
    };
    Ok(AuthTokenResponse {
        access_token: jwt::sign_jwt(&access, ctx.jwt_secret)?,
        refresh_token: jwt::sign_jwt(&refresh, ctx.jwt_secret)?,
        expires_in: 3600,
        token_type: "Bearer".to_string(),
        user: native_user(user),
    })
}

pub async fn refresh_atrium_tokens(
    ctx: &AppContext<'_>,
    refresh_token: &str,
) -> Result<AuthTokenResponse> {
    let claims = jwt::verify_jwt(refresh_token, ctx.jwt_secret)?;
    if claims.token_type != "refresh" {
        return Err(ApiError::unauthorized());
    }
    let user_id = claims
        .sub
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;
    let row = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, display_name, email, avatar_url, type AS user_type, site_admin, cached_at FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    issue_atrium_tokens(ctx, &user_from_row(row, None)).await
}

pub async fn is_super_admin(ctx: &AppContext<'_>) -> Result<bool> {
    let Some(actor) = ctx.user else {
        return Ok(false);
    };
    let ids = super_admin_ids(ctx);
    if ids.is_empty() {
        return Ok(false);
    }
    if actor
        .account_sub
        .as_deref()
        .is_some_and(|sub| ids.iter().any(|id| id == sub))
    {
        return Ok(true);
    }
    if !actor.email.is_empty()
        && ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(actor.email.as_str()))
    {
        return Ok(true);
    }
    #[derive(Deserialize)]
    struct Identity {
        provider_user_id: String,
        identity_email: String,
        user_email: String,
    }
    let rows = db::query_all::<Identity>(
        ctx.db,
        "SELECT ui.provider_user_id, ui.email AS identity_email, u.email AS user_email \
         FROM user_identities ui JOIN users u ON u.id = ui.user_id \
         WHERE ui.user_id = ?1 AND ui.provider = 'account'",
        &[DbValue::Integer(actor.id)],
    )
    .await?;
    Ok(rows.iter().any(|row| {
        ids.iter().any(|id| {
            id == &row.provider_user_id
                || id.eq_ignore_ascii_case(&row.identity_email)
                || id.eq_ignore_ascii_case(&row.user_email)
        })
    }))
}

pub async fn create_website(ctx: &AppContext<'_>, input: Value) -> Result<Value> {
    let actor = require_super_admin(ctx).await?;
    let key = normalize_key(input.get("key"), "Website", "key")?;
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&key)
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(ApiError::validation("Website", "name", "missing_field"));
    }
    if find_website(ctx, &key).await?.is_some() {
        return Err(ApiError::new(409, "Website already exists"));
    }
    let website = db::query_opt::<WebsiteRow>(
        ctx.db,
        "INSERT INTO websites (key, name, created_at, updated_at) VALUES (?1, ?2, datetime('now'), datetime('now')) RETURNING id, key, name, created_at, updated_at",
        &[DbValue::Text(key), DbValue::Text(name)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to create website"))?;
    replace_website_origins(ctx, website.id, input.get("origins")).await?;
    let mut admin_ids = vec![actor.id];
    if let Some(ids) = input.get("admin_user_ids").and_then(Value::as_array) {
        admin_ids.extend(ids.iter().filter_map(Value::as_i64));
    }
    add_website_admins(ctx, website.id, &admin_ids).await?;
    website_response(ctx, &website).await
}

pub async fn list_websites(ctx: &AppContext<'_>, query: &HashMap<String, String>) -> Result<Value> {
    let actor = require_user(ctx)?;
    let limit = list_limit(query);
    let cursor_id = query
        .get("cursor")
        .map(|value| decode_cursor(value))
        .transpose()?;
    let mut params = Vec::new();
    let mut filters = vec!["1 = 1".to_string()];
    if let Some(cursor_id) = cursor_id {
        filters.push(format!("w.id > ?{}", params.len() + 1));
        params.push(DbValue::Integer(cursor_id));
    }
    if !is_super_admin(ctx).await? {
        filters.push(format!(
            "EXISTS (SELECT 1 FROM website_admins wa WHERE wa.website_id = w.id AND wa.user_id = ?{})",
            params.len() + 1
        ));
        params.push(DbValue::Integer(actor.id));
    }
    params.push(DbValue::Integer(limit + 1));
    let sql = format!(
        "SELECT w.id, w.key, w.name, w.created_at, w.updated_at FROM websites w WHERE {} ORDER BY w.id ASC LIMIT ?{}",
        filters.join(" AND "),
        params.len()
    );
    let mut rows = db::query_all::<WebsiteRow>(ctx.db, &sql, &params).await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.pop();
    }
    let last_id = rows.last().map(|row| row.id);
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        data.push(website_response(ctx, &row).await?);
    }
    cursor_page(data, has_more, last_id)
}

pub async fn get_website_response(ctx: &AppContext<'_>, website_key: &str) -> Result<Value> {
    let website = get_website(ctx, website_key).await?;
    website_response(ctx, &website).await
}

pub async fn update_website(
    ctx: &AppContext<'_>,
    website_key: &str,
    input: Value,
) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    if input.get("name").is_some() {
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            return Err(ApiError::validation("Website", "name", "missing_field"));
        }
        ctx.db
            .execute(
                "UPDATE websites SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
                &[
                    DbValue::Text(name.to_string()),
                    DbValue::Integer(website.id),
                ],
            )
            .await?;
    }
    if input.get("origins").is_some() {
        replace_website_origins(ctx, website.id, input.get("origins")).await?;
    }
    get_website_response(ctx, website_key).await
}

pub async fn list_website_admins(ctx: &AppContext<'_>, website_key: &str) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    website_admins_response(ctx, website.id).await
}

pub async fn add_website_admin_by_input(
    ctx: &AppContext<'_>,
    website_key: &str,
    input: Value,
) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    let user_id = input
        .get("user_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| ApiError::validation("WebsiteAdmin", "user_id", "invalid"))?;
    add_website_admins(ctx, website.id, &[user_id]).await?;
    website_admins_response(ctx, website.id).await
}

pub async fn remove_website_admin(
    ctx: &AppContext<'_>,
    website_key: &str,
    user_id: i64,
) -> Result<()> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    #[derive(Deserialize)]
    struct CountRow {
        total: i64,
    }
    let total = db::query_opt::<CountRow>(
        ctx.db,
        "SELECT COUNT(*) AS total FROM website_admins WHERE website_id = ?1",
        &[DbValue::Integer(website.id)],
    )
    .await?
    .map(|row| row.total)
    .unwrap_or(0);
    if total <= 1 && !is_super_admin(ctx).await? {
        return Err(ApiError::forbidden("Cannot remove the last website admin"));
    }
    ctx.db
        .execute(
            "DELETE FROM website_admins WHERE website_id = ?1 AND user_id = ?2",
            &[DbValue::Integer(website.id), DbValue::Integer(user_id)],
        )
        .await?;
    Ok(())
}

pub async fn upsert_page(
    ctx: &AppContext<'_>,
    website_key: &str,
    page_key: &str,
    input: Value,
) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    let page = upsert_page_row(ctx, &website, page_key, &input).await?;
    Ok(page_response(&page, &website))
}

pub async fn get_page_response(
    ctx: &AppContext<'_>,
    website_key: &str,
    page_key: &str,
) -> Result<Value> {
    let website = get_website(ctx, website_key).await?;
    let page = get_page(ctx, website.id, page_key).await?;
    Ok(page_response(&page, &website))
}

pub async fn list_pages(
    ctx: &AppContext<'_>,
    website_key: &str,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    let limit = list_limit(query);
    let cursor_id = query
        .get("cursor")
        .map(|value| decode_cursor(value))
        .transpose()?;
    let mut params = vec![DbValue::Integer(website.id)];
    let mut filters = vec!["website_id = ?1".to_string()];
    if let Some(cursor_id) = cursor_id {
        filters.push("id > ?2".to_string());
        params.push(DbValue::Integer(cursor_id));
    }
    params.push(DbValue::Integer(limit + 1));
    let sql = format!(
        "SELECT id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at FROM pages WHERE {} ORDER BY id ASC LIMIT ?{}",
        filters.join(" AND "),
        params.len()
    );
    let mut rows = db::query_all::<PageRow>(ctx.db, &sql, &params).await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.pop();
    }
    let last_id = rows.last().map(|row| row.id);
    let data = rows
        .iter()
        .map(|page| page_response(page, &website))
        .collect();
    cursor_page(data, has_more, last_id)
}

pub async fn list_page_comments(
    ctx: &AppContext<'_>,
    website_key: &str,
    page_key: &str,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let website = get_website(ctx, website_key).await?;
    let page = get_page(ctx, website.id, page_key).await?;
    let parent = query.get("parent_id").map(String::as_str).unwrap_or("root");
    list_comments_for_page(ctx, &website, &page, parent, query).await
}

pub async fn create_page_comment(
    ctx: &AppContext<'_>,
    website_key: &str,
    page_key: &str,
    input: Value,
) -> Result<Value> {
    let website = get_website(ctx, website_key).await?;
    let page = get_page(ctx, website.id, page_key).await?;
    create_comment_for_page(ctx, &website, &page, input).await
}

pub async fn update_comment(
    ctx: &AppContext<'_>,
    website_key: &str,
    comment_id: i64,
    input: Value,
) -> Result<Value> {
    let actor = require_user(ctx)?.clone();
    let website = get_website(ctx, website_key).await?;
    require_not_banned(ctx, website.id).await?;
    let row = get_comment_row(ctx, website.id, comment_id).await?;
    if row.deleted_at.is_some() {
        return Err(ApiError::not_found("Comment"));
    }
    if row.user_id != actor.id {
        return Err(ApiError::forbidden(
            "You are not allowed to edit this comment",
        ));
    }
    let body = input.get("body").and_then(Value::as_str).unwrap_or("");
    if body.trim().is_empty() {
        return Err(ApiError::validation("Comment", "body", "missing_field"));
    }
    ctx.db
        .execute(
            "UPDATE comments SET body = ?1, updated_at = datetime('now') WHERE id = ?2 AND website_id = ?3",
            &[DbValue::Text(body.to_string()), DbValue::Integer(comment_id), DbValue::Integer(website.id)],
        )
        .await?;
    let row = get_comment_row(ctx, website.id, comment_id).await?;
    let response_ctx = comment_response_context(ctx, website, None).await?;
    Ok(comment_response(&row, &response_ctx))
}

pub async fn delete_comment(
    ctx: &AppContext<'_>,
    website_key: &str,
    comment_id: i64,
) -> Result<()> {
    let actor = require_user(ctx)?.clone();
    let website = get_website(ctx, website_key).await?;
    let row = get_comment_row(ctx, website.id, comment_id).await?;
    if row.deleted_at.is_some() {
        return Ok(());
    }
    if row.user_id != actor.id && !is_website_admin_or_super(ctx, website.id, actor.id).await? {
        return Err(ApiError::forbidden(
            "You are not allowed to delete this comment",
        ));
    }
    ctx.db
        .execute(
            "UPDATE comments SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND website_id = ?2",
            &[DbValue::Integer(comment_id), DbValue::Integer(website.id)],
        )
        .await?;
    ctx.db
        .execute(
            "UPDATE pages SET comment_count = CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END, updated_at = datetime('now') WHERE id = ?1",
            &[DbValue::Integer(row.page_id)],
        )
        .await?;
    Ok(())
}

pub async fn set_comment_reaction(
    ctx: &AppContext<'_>,
    website_key: &str,
    comment_id: i64,
    content: &str,
) -> Result<Value> {
    let website = get_website(ctx, website_key).await?;
    let counts = set_comment_reaction_for_website(ctx, &website, comment_id, content).await?;
    serde_json::to_value(counts).map_err(ApiError::from)
}

pub async fn delete_comment_reaction(
    ctx: &AppContext<'_>,
    website_key: &str,
    comment_id: i64,
    content: &str,
) -> Result<()> {
    let website = get_website(ctx, website_key).await?;
    delete_comment_reaction_for_website(ctx, &website, comment_id, content).await
}

pub async fn list_moderation_comments(
    ctx: &AppContext<'_>,
    website_key: &str,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    let limit = list_limit(query);
    let cursor_id = query
        .get("cursor")
        .map(|value| decode_cursor(value))
        .transpose()?;
    let mut filters = vec!["c.website_id = ?1".to_string()];
    let mut params = vec![DbValue::Integer(website.id)];
    if let Some(cursor_id) = cursor_id {
        filters.push(format!("c.id > ?{}", params.len() + 1));
        params.push(DbValue::Integer(cursor_id));
    }
    match query.get("status").map(String::as_str).unwrap_or("all") {
        "active" => filters.push("c.deleted_at IS NULL".to_string()),
        "deleted" => filters.push("c.deleted_at IS NOT NULL".to_string()),
        "all" => {}
        _ => return Err(ApiError::bad_request("invalid status")),
    }
    let mut from = COMMENT_ROW_FROM.to_string();
    if let Some(page_key) = query.get("page_key") {
        from.push_str(" JOIN pages p ON p.id = c.page_id");
        filters.push(format!("p.key = ?{}", params.len() + 1));
        params.push(DbValue::Text(page_key.clone()));
    }
    if let Some(author_id) = query.get("author_id").and_then(|v| v.parse::<i64>().ok()) {
        filters.push(format!("c.user_id = ?{}", params.len() + 1));
        params.push(DbValue::Integer(author_id));
    }
    params.push(DbValue::Integer(limit + 1));
    let sql = format!(
        "SELECT {} {} WHERE {} ORDER BY c.id ASC LIMIT ?{}",
        COMMENT_ROW_COLUMNS,
        from,
        filters.join(" AND "),
        params.len()
    );
    let mut rows = db::query_all::<CommentRow>(ctx.db, &sql, &params).await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.pop();
    }
    let last_id = rows.last().map(|row| row.id);
    let response_ctx = comment_response_context(ctx, website, None).await?;
    let data = rows
        .iter()
        .map(|row| comment_response(row, &response_ctx))
        .collect();
    cursor_page(data, has_more, last_id)
}

pub async fn ban_website_user(
    ctx: &AppContext<'_>,
    website_key: &str,
    input: Value,
) -> Result<Value> {
    let actor = require_user(ctx)?.clone();
    let website = require_website_admin_or_super(ctx, website_key).await?;
    let user_id = input
        .get("user_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| ApiError::validation("WebsiteBan", "user_id", "invalid"))?;
    #[derive(Deserialize)]
    struct ExistsRow {}
    if db::query_opt::<ExistsRow>(
        ctx.db,
        "SELECT id FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .is_none()
    {
        return Err(ApiError::validation("WebsiteBan", "user_id", "invalid"));
    }
    let reason = input
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    ctx.db
        .execute(
            "INSERT INTO website_bans (website_id, user_id, reason, banned_by_user_id, banned_at, unbanned_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'), NULL) \
             ON CONFLICT(website_id, user_id) DO UPDATE SET reason = excluded.reason, banned_by_user_id = excluded.banned_by_user_id, banned_at = datetime('now'), unbanned_at = NULL",
            &[
                DbValue::Integer(website.id),
                DbValue::Integer(user_id),
                opt_text(reason),
                DbValue::Integer(actor.id),
            ],
        )
        .await?;
    list_website_bans(ctx, website_key).await
}

pub async fn list_website_bans(ctx: &AppContext<'_>, website_key: &str) -> Result<Value> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    #[derive(Deserialize)]
    struct BanRow {
        id: i64,
        login: String,
        display_name: String,
        email: String,
        avatar_url: String,
        #[serde(rename = "type")]
        user_type: String,
        site_admin: i64,
        reason: Option<String>,
        banned_at: String,
    }
    let rows = db::query_all::<BanRow>(
        ctx.db,
        "SELECT u.id, wb.reason, wb.banned_at, u.login, u.display_name, u.email, u.avatar_url, u.type, u.site_admin \
         FROM website_bans wb JOIN users u ON u.id = wb.user_id \
         WHERE wb.website_id = ?1 AND wb.unbanned_at IS NULL ORDER BY wb.banned_at DESC, wb.user_id ASC",
        &[DbValue::Integer(website.id)],
    )
    .await?;
    Ok(json!({
        "data": rows.into_iter().map(|row| {
            let user = GitHubUser {
                id: row.id,
                login: row.login,
                display_name: row.display_name,
                email: row.email,
                avatar_url: row.avatar_url,
                r#type: row.user_type,
                site_admin: row.site_admin != 0,
                account_sub: None,
                cached_at: None,
            };
            json!({
                "user": public_user(&user, true),
                "reason": row.reason,
                "banned_at": to_iso(Some(&row.banned_at)),
            })
        }).collect::<Vec<_>>()
    }))
}

pub async fn unban_website_user(
    ctx: &AppContext<'_>,
    website_key: &str,
    user_id: i64,
) -> Result<()> {
    let website = require_website_admin_or_super(ctx, website_key).await?;
    ctx.db
        .execute(
            "UPDATE website_bans SET unbanned_at = datetime('now') WHERE website_id = ?1 AND user_id = ?2 AND unbanned_at IS NULL",
            &[DbValue::Integer(website.id), DbValue::Integer(user_id)],
        )
        .await?;
    Ok(())
}

pub async fn get_current_comments(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let (website, page) = resolve_current_page_for_read(ctx, referer).await?;
    let Some(page) = page else {
        return cursor_page(Vec::<Value>::new(), false, None);
    };
    list_comments_for_page(ctx, &website, &page, "root", query).await
}

pub async fn create_current_comment(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    input: Value,
) -> Result<Value> {
    let title = input.get("page_title").and_then(Value::as_str);
    let (website, page) = resolve_current_page_for_write(ctx, referer, title).await?;
    create_comment_for_page(ctx, &website, &page, input).await
}

pub async fn list_current_replies(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let comment_id = query
        .get("comment_id")
        .ok_or_else(|| ApiError::bad_request("missing comment_id"))?;
    let (website, page) = resolve_current_page_for_read(ctx, referer).await?;
    let Some(page) = page else {
        return cursor_page(Vec::<Value>::new(), false, None);
    };
    list_comments_for_page(ctx, &website, &page, comment_id, query).await
}

pub async fn set_current_reaction(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    comment_id: i64,
    content: &str,
) -> Result<Value> {
    let (website, page) = resolve_current_page_for_read(ctx, referer).await?;
    let page = page.ok_or_else(|| ApiError::not_found("Page"))?;
    ensure_comment_on_page(ctx, website.id, page.id, comment_id).await?;
    let counts = set_comment_reaction_for_website(ctx, &website, comment_id, content).await?;
    serde_json::to_value(counts).map_err(ApiError::from)
}

pub async fn delete_current_reaction(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    comment_id: i64,
    content: &str,
) -> Result<()> {
    let (website, page) = resolve_current_page_for_read(ctx, referer).await?;
    let page = page.ok_or_else(|| ApiError::not_found("Page"))?;
    ensure_comment_on_page(ctx, website.id, page.id, comment_id).await?;
    delete_comment_reaction_for_website(ctx, &website, comment_id, content).await
}

pub async fn discovery_public_key(ctx: &AppContext<'_>) -> Result<Value> {
    let raw = ctx
        .discovery_public_jwk
        .or(ctx.discovery_private_jwk)
        .ok_or_else(|| ApiError::internal("ATRIUM_DISCOVERY_PUBLIC_JWK is not configured"))?;
    let mut jwk = serde_json::from_str::<Map<String, Value>>(raw)
        .map_err(|_| ApiError::internal("ATRIUM_DISCOVERY_PUBLIC_JWK is invalid"))?;
    for key in ["d", "p", "q", "dp", "dq", "qi", "oth", "k", "priv"] {
        jwk.remove(key);
    }
    if !jwk.contains_key("kty") {
        return Err(ApiError::internal(
            "ATRIUM_DISCOVERY_PUBLIC_JWK must be a JWK",
        ));
    }
    let kid = ctx
        .discovery_key_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| jwk.get("kid").and_then(Value::as_str).map(str::to_string));
    if let Some(kid) = kid.clone() {
        jwk.insert("kid".to_string(), Value::String(kid));
    }
    jwk.insert(
        "alg".to_string(),
        Value::String(DISCOVERY_JWE_ALG.to_string()),
    );
    jwk.insert(
        "key_ops".to_string(),
        Value::Array(vec![Value::String("encrypt".to_string())]),
    );
    Ok(json!({
        "kid": kid,
        "alg": DISCOVERY_JWE_ALG,
        "enc": DISCOVERY_JWE_ENC,
        "jwk": Value::Object(jwk),
    }))
}

async fn create_comment_for_page(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    page: &PageRow,
    input: Value,
) -> Result<Value> {
    let actor = require_user(ctx)?.clone();
    require_not_banned(ctx, website.id).await?;
    let body = input.get("body").and_then(Value::as_str).unwrap_or("");
    if body.trim().is_empty() {
        return Err(ApiError::validation("Comment", "body", "missing_field"));
    }
    let parent_id = input.get("parent_id").and_then(Value::as_i64);
    if let Some(parent_id) = parent_id {
        ensure_active_comment_on_page(ctx, website.id, page.id, parent_id).await?;
    }
    #[derive(Deserialize)]
    struct IdRow {
        id: i64,
    }
    let row = db::query_opt::<IdRow>(
        ctx.db,
        "INSERT INTO comments (website_id, page_id, parent_comment_id, body, user_id, reactions, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, '{}', datetime('now'), datetime('now')) RETURNING id",
        &[
            DbValue::Integer(website.id),
            DbValue::Integer(page.id),
            opt_i64(parent_id),
            DbValue::Text(body.to_string()),
            DbValue::Integer(actor.id),
        ],
    )
    .await?
    .ok_or_else(|| ApiError::internal("comment insert failed"))?;
    ctx.db
        .execute(
            "UPDATE pages SET comment_count = comment_count + 1, updated_at = datetime('now') WHERE id = ?1",
            &[DbValue::Integer(page.id)],
        )
        .await?;
    let row = get_comment_row(ctx, website.id, row.id).await?;
    let response_ctx = comment_response_context(ctx, website.clone(), Some(page.clone())).await?;
    Ok(comment_response(&row, &response_ctx))
}

async fn list_comments_for_page(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    page: &PageRow,
    parent: &str,
    query: &HashMap<String, String>,
) -> Result<Value> {
    let limit = list_limit(query);
    let order = if query
        .get("order")
        .map(|v| v.eq_ignore_ascii_case("desc"))
        .unwrap_or(false)
    {
        "DESC"
    } else {
        "ASC"
    };
    let cursor_id = query
        .get("cursor")
        .map(|value| decode_cursor(value))
        .transpose()?;
    let flat_thread = query.get("thread").map(|v| v == "flat").unwrap_or(false);
    let response_ctx = comment_response_context(ctx, website.clone(), Some(page.clone())).await?;

    if parent != "root" {
        let parent_id = parent
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request("invalid parent_id"))?;
        ensure_comment_on_page(ctx, website.id, page.id, parent_id).await?;
        if flat_thread {
            let mut params = vec![
                DbValue::Integer(website.id),
                DbValue::Integer(page.id),
                DbValue::Integer(parent_id),
                DbValue::Integer(website.id),
                DbValue::Integer(page.id),
            ];
            let mut filters = vec![
                "c.website_id = ?1".to_string(),
                "c.page_id = ?2".to_string(),
                PUBLIC_COMMENT_VISIBILITY_FILTER.to_string(),
            ];
            if let Some(cursor_id) = cursor_id {
                filters.push(format!(
                    "c.id {} ?{}",
                    if order == "DESC" { "<" } else { ">" },
                    params.len() + 1
                ));
                params.push(DbValue::Integer(cursor_id));
            }
            params.push(DbValue::Integer(limit + 1));
            let sql = format!(
                "WITH RECURSIVE descendants(id) AS (\
                 SELECT id FROM comments WHERE website_id = ?1 AND page_id = ?2 AND parent_comment_id = ?3 \
                 UNION ALL \
                 SELECT c.id FROM comments c JOIN descendants d ON c.parent_comment_id = d.id WHERE c.website_id = ?4 AND c.page_id = ?5\
                 ) SELECT {} FROM comments c JOIN descendants d ON d.id = c.id JOIN users u ON u.id = c.user_id WHERE {} ORDER BY c.id {} LIMIT ?{}",
                COMMENT_ROW_COLUMNS,
                filters.join(" AND "),
                order,
                params.len()
            );
            return comment_cursor_response(ctx, &sql, &params, limit, &response_ctx).await;
        }
    }

    let mut params = vec![DbValue::Integer(website.id), DbValue::Integer(page.id)];
    let mut filters = vec![
        "c.website_id = ?1".to_string(),
        "c.page_id = ?2".to_string(),
        PUBLIC_COMMENT_VISIBILITY_FILTER.to_string(),
    ];
    if parent == "root" {
        filters.push("c.parent_comment_id IS NULL".to_string());
    } else {
        filters.push(format!("c.parent_comment_id = ?{}", params.len() + 1));
        params.push(DbValue::Integer(
            parent
                .parse::<i64>()
                .map_err(|_| ApiError::bad_request("invalid parent_id"))?,
        ));
    }
    if let Some(cursor_id) = cursor_id {
        filters.push(format!(
            "c.id {} ?{}",
            if order == "DESC" { "<" } else { ">" },
            params.len() + 1
        ));
        params.push(DbValue::Integer(cursor_id));
    }
    params.push(DbValue::Integer(limit + 1));
    let sql = format!(
        "SELECT {} {} WHERE {} ORDER BY c.id {} LIMIT ?{}",
        COMMENT_ROW_COLUMNS,
        COMMENT_ROW_FROM,
        filters.join(" AND "),
        order,
        params.len()
    );
    comment_cursor_response(ctx, &sql, &params, limit, &response_ctx).await
}

async fn comment_cursor_response(
    ctx: &AppContext<'_>,
    sql: &str,
    params: &[DbValue],
    limit: i64,
    response_ctx: &CommentResponseContext,
) -> Result<Value> {
    let mut rows = db::query_all::<CommentRow>(ctx.db, sql, params).await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.pop();
    }
    let last_id = rows.last().map(|row| row.id);
    let data = rows
        .iter()
        .map(|row| comment_response(row, response_ctx))
        .collect();
    cursor_page(data, has_more, last_id)
}

async fn set_comment_reaction_for_website(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    comment_id: i64,
    content: &str,
) -> Result<ReactionCounts> {
    let actor = require_user(ctx)?.clone();
    require_not_banned(ctx, website.id).await?;
    if !ALLOWED_REACTIONS.contains(&content) {
        return Err(ApiError::validation("Reaction", "content", "invalid"));
    }
    ensure_active_comment_in_website(ctx, website.id, comment_id).await?;
    ctx.db
        .execute(
            "INSERT INTO comment_reactions (comment_id, user_id, content, created_at) VALUES (?1, ?2, ?3, datetime('now')) ON CONFLICT(comment_id, user_id, content) DO NOTHING",
            &[DbValue::Integer(comment_id), DbValue::Integer(actor.id), DbValue::Text(content.to_string())],
        )
        .await?;
    rebuild_cached_reactions(ctx, comment_id).await
}

async fn delete_comment_reaction_for_website(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    comment_id: i64,
    content: &str,
) -> Result<()> {
    let actor = require_user(ctx)?.clone();
    require_not_banned(ctx, website.id).await?;
    if !ALLOWED_REACTIONS.contains(&content) {
        return Err(ApiError::validation("Reaction", "content", "invalid"));
    }
    ensure_active_comment_in_website(ctx, website.id, comment_id).await?;
    let affected = ctx
        .db
        .execute(
            "DELETE FROM comment_reactions WHERE comment_id = ?1 AND user_id = ?2 AND content = ?3",
            &[
                DbValue::Integer(comment_id),
                DbValue::Integer(actor.id),
                DbValue::Text(content.to_string()),
            ],
        )
        .await?;
    if affected > 0 {
        rebuild_cached_reactions(ctx, comment_id).await?;
    }
    Ok(())
}

async fn rebuild_cached_reactions(ctx: &AppContext<'_>, comment_id: i64) -> Result<ReactionCounts> {
    #[derive(Deserialize)]
    struct CountsRow {
        like: i64,
        dislike: i64,
        heart: i64,
        laugh: i64,
        hooray: i64,
        confused: i64,
        rocket: i64,
        eyes: i64,
    }
    let row = db::query_opt::<CountsRow>(
        ctx.db,
        "SELECT COALESCE(SUM(CASE WHEN content = 'like' THEN 1 ELSE 0 END), 0) AS like, \
         COALESCE(SUM(CASE WHEN content = 'dislike' THEN 1 ELSE 0 END), 0) AS dislike, \
         COALESCE(SUM(CASE WHEN content = 'heart' THEN 1 ELSE 0 END), 0) AS heart, \
         COALESCE(SUM(CASE WHEN content = 'laugh' THEN 1 ELSE 0 END), 0) AS laugh, \
         COALESCE(SUM(CASE WHEN content = 'hooray' THEN 1 ELSE 0 END), 0) AS hooray, \
         COALESCE(SUM(CASE WHEN content = 'confused' THEN 1 ELSE 0 END), 0) AS confused, \
         COALESCE(SUM(CASE WHEN content = 'rocket' THEN 1 ELSE 0 END), 0) AS rocket, \
         COALESCE(SUM(CASE WHEN content = 'eyes' THEN 1 ELSE 0 END), 0) AS eyes \
         FROM comment_reactions WHERE comment_id = ?1",
        &[DbValue::Integer(comment_id)],
    )
    .await?
    .unwrap_or(CountsRow {
        like: 0,
        dislike: 0,
        heart: 0,
        laugh: 0,
        hooray: 0,
        confused: 0,
        rocket: 0,
        eyes: 0,
    });
    let mut counts = ReactionCounts {
        like: row.like,
        dislike: row.dislike,
        heart: row.heart,
        laugh: row.laugh,
        hooray: row.hooray,
        confused: row.confused,
        rocket: row.rocket,
        eyes: row.eyes,
        total: 0,
    };
    counts.total = counts.like
        + counts.dislike
        + counts.heart
        + counts.laugh
        + counts.hooray
        + counts.confused
        + counts.rocket
        + counts.eyes;
    ctx.db
        .execute(
            "UPDATE comments SET reactions = ?1, updated_at = updated_at WHERE id = ?2",
            &[
                DbValue::Text(serde_json::to_string(&counts).map_err(ApiError::from)?),
                DbValue::Integer(comment_id),
            ],
        )
        .await?;
    Ok(counts)
}

async fn resolve_current_page_for_read(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
) -> Result<(WebsiteRow, Option<PageRow>)> {
    let referer = referer.ok_or_else(|| ApiError::bad_request("missing Referer header"))?;
    let (website, normalized_url) = resolve_website_by_referer(ctx, referer).await?;
    let page_key = page_key_from_url(&normalized_url);
    let page = find_page(ctx, website.id, &page_key).await?;
    Ok((website, page))
}

async fn resolve_current_page_for_write(
    ctx: &AppContext<'_>,
    referer: Option<&str>,
    title: Option<&str>,
) -> Result<(WebsiteRow, PageRow)> {
    let referer = referer.ok_or_else(|| ApiError::bad_request("missing Referer header"))?;
    let (website, normalized_url) = resolve_website_by_referer(ctx, referer).await?;
    let page_key = page_key_from_url(&normalized_url);
    if let Some(page) = find_page(ctx, website.id, &page_key).await? {
        return Ok((website, page));
    }
    let input = json!({
        "title": title.unwrap_or(&normalized_url),
        "url": normalized_url,
        "metadata": null,
    });
    let page = insert_page_row(ctx, &website, &page_key, &input).await?;
    Ok((website, page))
}

async fn resolve_website_by_referer(
    ctx: &AppContext<'_>,
    referer: &str,
) -> Result<(WebsiteRow, String)> {
    let normalized_url = normalize_page_url(referer)?;
    let origin = normalize_origin(&normalized_url)?;
    if let Some(row) = db::query_opt::<WebsiteRow>(
        ctx.db,
        "SELECT w.id, w.key, w.name, w.created_at, w.updated_at \
         FROM website_origins wo JOIN websites w ON w.id = wo.website_id \
         WHERE wo.origin = ?1 LIMIT 1",
        &[DbValue::Text(origin.clone())],
    )
    .await?
    {
        return Ok((row, normalized_url));
    }
    let website = discover_website_for_origin(ctx, &origin)
        .await?
        .ok_or_else(|| ApiError::new(404, "website_not_found"))?;
    Ok((website, normalized_url))
}

async fn discover_website_for_origin(
    ctx: &AppContext<'_>,
    origin: &str,
) -> Result<Option<WebsiteRow>> {
    if has_fresh_discovery_failure(ctx, origin).await? {
        return Ok(None);
    }
    let (metadata, failure) = discover_origin_metadata(ctx, origin).await?;
    let Some(metadata) = metadata else {
        let failure = failure.unwrap_or_else(|| DiscoveryFailure {
            status: "not_found",
            source: None,
            error: "discovery metadata not found".to_string(),
        });
        record_discovery_failure(
            ctx,
            origin,
            failure.status,
            failure.source.as_deref(),
            &failure.error,
        )
        .await?;
        return Ok(None);
    };
    if metadata.admin_emails.is_empty() {
        record_discovery_failure(
            ctx,
            origin,
            "invalid",
            Some(&metadata.source),
            "admin_emails is required",
        )
        .await?;
        return Ok(None);
    }
    if let Some(existing) = find_website(ctx, &metadata.website_key).await? {
        let origins = website_origins(ctx, existing.id).await?;
        if origins.iter().any(|item| item == &metadata.origin) {
            return Ok(Some(existing));
        }
        record_discovery_failure(
            ctx,
            origin,
            "conflict",
            Some(&metadata.source),
            "derived website key already exists for another origin",
        )
        .await?;
        return Ok(None);
    }
    let website = db::query_opt::<WebsiteRow>(
        ctx.db,
        "INSERT INTO websites (key, name, created_at, updated_at) VALUES (?1, ?2, datetime('now'), datetime('now')) RETURNING id, key, name, created_at, updated_at",
        &[DbValue::Text(metadata.website_key), DbValue::Text(metadata.name)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to create discovered website"))?;
    ctx.db
        .execute(
            "INSERT INTO website_origins (website_id, origin, created_at) VALUES (?1, ?2, datetime('now'))",
            &[DbValue::Integer(website.id), DbValue::Text(metadata.origin.clone())],
        )
        .await?;
    save_pending_website_admins(ctx, website.id, &metadata.admin_emails, &metadata.source).await?;
    claim_pending_website_admins_for_website(ctx, website.id).await?;
    record_discovery_success(ctx, origin, website.id, &metadata.source).await?;
    Ok(Some(website))
}

async fn discover_origin_metadata(
    ctx: &AppContext<'_>,
    origin: &str,
) -> Result<(Option<DiscoveryMetadata>, Option<DiscoveryFailure>)> {
    let origin_url = match Url::parse(origin) {
        Ok(url) => url,
        Err(_) => {
            return Ok((
                None,
                Some(DiscoveryFailure {
                    status: "invalid",
                    source: None,
                    error: "invalid origin".to_string(),
                }),
            ));
        }
    };
    if origin_url.scheme() != "https" {
        return Ok((
            None,
            Some(DiscoveryFailure {
                status: "not_found",
                source: None,
                error: "discovery requires https origin".to_string(),
            }),
        ));
    }
    let mut failures = Vec::<DiscoveryFailure>::new();
    if let Some(text) = mocked_well_known_text(ctx, origin) {
        if let Some(metadata) =
            parse_discovery_candidate(ctx, &text, origin, "well-known", &mut failures)
        {
            return Ok((Some(metadata), None));
        }
    } else {
        let url = format!("{}{}", origin.trim_end_matches('/'), DISCOVERY_PATH);
        match ctx.http.get_url(&url, "application/json").await {
            Ok(response) if response.status == 200 => {
                if let Ok(text) = String::from_utf8(response.body.to_vec()) {
                    if let Some(metadata) =
                        parse_discovery_candidate(ctx, &text, origin, "well-known", &mut failures)
                    {
                        return Ok((Some(metadata), None));
                    }
                }
            }
            Ok(response) if response.status == 404 || response.status == 410 => {}
            Ok(response) => failures.push(DiscoveryFailure {
                status: "error",
                source: Some("well-known".to_string()),
                error: format!("well-known returned {}", response.status),
            }),
            Err(error) => failures.push(DiscoveryFailure {
                status: "error",
                source: Some("well-known".to_string()),
                error: error.to_string(),
            }),
        }
    }

    let hostname = origin_url.host_str().unwrap_or_default();
    let payloads = if let Some(payloads) = mocked_dns_txt_payloads(ctx, hostname) {
        payloads
    } else {
        let url = format!(
            "https://cloudflare-dns.com/dns-query?name={}&type=TXT",
            urlencoding::encode(&format!("_atrium.{}", hostname))
        );
        match ctx.http.get_url(&url, "application/dns-json").await {
            Ok(response) if response.status == 200 => {
                let payload: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
                parse_atrium_txt_payloads_from_doh(&payload)
            }
            Ok(response) => {
                failures.push(DiscoveryFailure {
                    status: "error",
                    source: Some("dns-txt".to_string()),
                    error: format!("dns query returned {}", response.status),
                });
                Vec::new()
            }
            Err(error) => {
                failures.push(DiscoveryFailure {
                    status: "error",
                    source: Some("dns-txt".to_string()),
                    error: error.to_string(),
                });
                Vec::new()
            }
        }
    };
    for payload in payloads {
        if let Some(metadata) =
            parse_discovery_candidate(ctx, &payload, origin, "dns-txt", &mut failures)
        {
            return Ok((Some(metadata), None));
        }
    }
    let failure = failures
        .iter()
        .find(|item| item.status == "invalid")
        .cloned()
        .or_else(|| failures.iter().find(|item| item.status == "error").cloned())
        .unwrap_or_else(|| DiscoveryFailure {
            status: "not_found",
            source: None,
            error: "discovery metadata not found".to_string(),
        });
    Ok((None, Some(failure)))
}

fn parse_discovery_candidate(
    ctx: &AppContext<'_>,
    text: &str,
    expected_origin: &str,
    source: &str,
    failures: &mut Vec<DiscoveryFailure>,
) -> Option<DiscoveryMetadata> {
    match parse_discovery_document(ctx, text, expected_origin, source) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            failures.push(DiscoveryFailure {
                status: if error.status >= 500 {
                    "error"
                } else {
                    "invalid"
                },
                source: Some(source.to_string()),
                error: error.body.message,
            });
            None
        }
    }
}

fn parse_discovery_document(
    ctx: &AppContext<'_>,
    text: &str,
    expected_origin: &str,
    source: &str,
) -> Result<DiscoveryMetadata> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|_| ApiError::bad_request("invalid discovery document"))?;
    let obj = raw
        .as_object()
        .ok_or_else(|| ApiError::bad_request("document must be a JSON object"))?;
    let document = decrypt_flat_discovery_fields(ctx, obj)?;
    if document.get("atrium").and_then(Value::as_str) != Some("v1") {
        return Err(ApiError::bad_request("atrium must be v1"));
    }
    if document.contains_key("website_key") {
        return Err(ApiError::bad_request("website_key is not allowed"));
    }
    let origin = document
        .get("origin")
        .and_then(Value::as_str)
        .map(normalize_discovery_origin)
        .transpose()?
        .unwrap_or_else(|| expected_origin.to_string());
    if origin != expected_origin {
        return Err(ApiError::bad_request(
            "origin does not match referer origin",
        ));
    }
    let origin_url = Url::parse(expected_origin)
        .map_err(|_| ApiError::validation("Website", "origins", "invalid"))?;
    let website_key = normalize_key(
        Some(&Value::String(
            origin_url.host_str().unwrap_or_default().to_string(),
        )),
        "Website",
        "key",
    )?;
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&website_key)
        .trim()
        .to_string();
    if name.is_empty() || name.len() > 160 {
        return Err(ApiError::bad_request("name is invalid"));
    }
    let mut emails = Vec::new();
    if let Some(value) = document.get("admin_emails") {
        let values = value
            .as_array()
            .ok_or_else(|| ApiError::bad_request("admin_emails must be an array"))?;
        if values.len() > 20 {
            return Err(ApiError::bad_request("admin_emails has too many entries"));
        }
        let mut seen = HashSet::new();
        for value in values {
            let email = value
                .as_str()
                .ok_or_else(|| ApiError::bad_request("admin_emails must be a string"))?;
            let email = normalize_discovery_email(email, "admin_emails")?;
            if seen.insert(email.clone()) {
                emails.push(email);
            }
        }
    }
    let contact_email = document
        .get("contact_email")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| ApiError::bad_request("contact_email must be a string"))
                .and_then(|email| normalize_discovery_email(email, "contact_email"))
        })
        .transpose()?;
    Ok(DiscoveryMetadata {
        origin,
        website_key,
        name,
        admin_emails: emails,
        contact_email,
        source: source.to_string(),
    })
}

fn decrypt_flat_discovery_fields(
    ctx: &AppContext<'_>,
    obj: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut out = Map::with_capacity(obj.len());
    for (key, value) in obj {
        if let Some(raw) = value.as_str().and_then(|text| {
            text.strip_prefix(ENCRYPTED_FIELD_PREFIX)
                .map(str::to_string)
        }) {
            out.insert(key.clone(), decrypt_discovery_field(ctx, key, &raw)?);
        } else {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(out)
}

fn decrypt_discovery_field(ctx: &AppContext<'_>, field: &str, compact: &str) -> Result<Value> {
    let private_jwk = ctx
        .discovery_private_jwk
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("ATRIUM_DISCOVERY_PRIVATE_JWK is not configured"))?;

    decrypt_compact_discovery_jwe(private_jwk, ctx.discovery_key_id, compact)
        .map_err(|_| ApiError::bad_request(format!("{} could not be decrypted", field)))
}

fn decrypt_compact_discovery_jwe(
    private_jwk: &str,
    configured_kid: Option<&str>,
    compact: &str,
) -> std::result::Result<Value, String> {
    let parts = compact.split('.').collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err("compact JWE must have five parts".to_string());
    }

    let protected = b64url_decode(parts[0])?;
    let protected_header = serde_json::from_slice::<Value>(&protected)
        .map_err(|_| "protected header is not JSON".to_string())?;
    let protected_header = protected_header
        .as_object()
        .ok_or_else(|| "protected header must be an object".to_string())?;
    if protected_header.get("alg").and_then(Value::as_str) != Some(DISCOVERY_JWE_ALG) {
        return Err("unsupported JWE alg".to_string());
    }
    if protected_header.get("enc").and_then(Value::as_str) != Some(DISCOVERY_JWE_ENC) {
        return Err("unsupported JWE enc".to_string());
    }

    let private_jwk = serde_json::from_str::<Value>(private_jwk)
        .map_err(|_| "private JWK is not JSON".to_string())?;
    let private_jwk = private_jwk
        .as_object()
        .ok_or_else(|| "private JWK must be an object".to_string())?;
    let expected_kid = configured_kid
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| private_jwk.get("kid").and_then(Value::as_str));
    if let Some(expected_kid) = expected_kid {
        if protected_header.get("kid").and_then(Value::as_str) != Some(expected_kid) {
            return Err("JWE kid mismatch".to_string());
        }
    }

    let private_key = rsa_private_key_from_jwk(private_jwk)?;
    let encrypted_key = b64url_decode(parts[1])?;
    let cek = private_key
        .decrypt(Oaep::new::<Sha256>(), &encrypted_key)
        .map_err(|_| "failed to unwrap content encryption key".to_string())?;
    if cek.len() != 32 {
        return Err("A256GCM requires a 256-bit key".to_string());
    }

    let iv = b64url_decode(parts[2])?;
    if iv.len() != 12 {
        return Err("A256GCM requires a 96-bit IV".to_string());
    }
    let mut ciphertext_and_tag = b64url_decode(parts[3])?;
    let tag = b64url_decode(parts[4])?;
    if tag.len() != 16 {
        return Err("A256GCM requires a 128-bit tag".to_string());
    }
    ciphertext_and_tag.extend_from_slice(&tag);

    let cipher = Aes256Gcm::new_from_slice(&cek).map_err(|_| "invalid A256GCM key".to_string())?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: &ciphertext_and_tag,
                aad: parts[0].as_bytes(),
            },
        )
        .map_err(|_| "failed to decrypt ciphertext".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|_| "plaintext is not JSON".to_string())
}

fn rsa_private_key_from_jwk(
    jwk: &Map<String, Value>,
) -> std::result::Result<RsaPrivateKey, String> {
    if jwk.get("kty").and_then(Value::as_str) != Some("RSA") {
        return Err("private JWK must be RSA".to_string());
    }
    let n = jwk_biguint(jwk, "n")?;
    let e = jwk_biguint(jwk, "e")?;
    let d = jwk_biguint(jwk, "d")?;
    let primes = vec![jwk_biguint(jwk, "p")?, jwk_biguint(jwk, "q")?];
    RsaPrivateKey::from_components(n, e, d, primes)
        .map_err(|_| "invalid RSA private JWK".to_string())
}

fn jwk_biguint(jwk: &Map<String, Value>, name: &str) -> std::result::Result<BigUint, String> {
    let raw = jwk
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("private JWK missing {}", name))?;
    let bytes = b64url_decode(raw)?;
    if bytes.is_empty() {
        return Err(format!("private JWK {} is empty", name));
    }
    Ok(BigUint::from_bytes_be(&bytes))
}

fn b64url_decode(raw: &str) -> std::result::Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "invalid base64url".to_string())
}

fn normalize_discovery_email(raw: &str, field: &str) -> Result<String> {
    normalize_email(raw).ok_or_else(|| ApiError::bad_request(format!("{} is invalid", field)))
}

fn parse_atrium_txt_payloads_from_doh(payload: &Value) -> Vec<String> {
    payload
        .get("Answer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|answer| {
            if answer.get("type").and_then(Value::as_i64) != Some(16) {
                return None;
            }
            let data = answer.get("data")?.as_str()?;
            let joined = parse_dns_txt_data(data).join("");
            joined.strip_prefix(TXT_RECORD_PREFIX).map(str::to_string)
        })
        .collect()
}

fn parse_dns_txt_data(data: &str) -> Vec<String> {
    let trimmed = data.trim();
    if !trimmed.starts_with('"') {
        return vec![trimmed.to_string()];
    }
    let chars = trimmed.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if in_quote {
                parts.push(std::mem::take(&mut current));
            }
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote {
            if ch == '\\' {
                if i + 3 < chars.len()
                    && chars[i + 1].is_ascii_digit()
                    && chars[i + 2].is_ascii_digit()
                    && chars[i + 3].is_ascii_digit()
                {
                    let code = [chars[i + 1], chars[i + 2], chars[i + 3]]
                        .iter()
                        .collect::<String>()
                        .parse::<u32>()
                        .unwrap_or(0);
                    if let Some(decoded) = char::from_u32(code) {
                        current.push(decoded);
                    }
                    i += 4;
                    continue;
                }
                if i + 1 < chars.len() {
                    current.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
            }
            current.push(ch);
        }
        i += 1;
    }
    if in_quote && !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        vec![trimmed.to_string()]
    } else {
        parts
    }
}

async fn has_fresh_discovery_failure(ctx: &AppContext<'_>, origin: &str) -> Result<bool> {
    #[derive(Deserialize)]
    struct ExistsRow {}
    Ok(db::query_opt::<ExistsRow>(
        ctx.db,
        "SELECT 1 AS hit FROM website_discovery_cache WHERE origin = ?1 AND status != 'discovered' AND retry_after IS NOT NULL AND retry_after > datetime('now') LIMIT 1",
        &[DbValue::Text(origin.to_string())],
    )
    .await?
    .is_some())
}

async fn record_discovery_failure(
    ctx: &AppContext<'_>,
    origin: &str,
    status: &str,
    source: Option<&str>,
    error: &str,
) -> Result<()> {
    let retry_after = match status {
        "not_found" => "+6 hours",
        "error" => "+10 minutes",
        _ => "+1 hour",
    };
    ctx.db
        .execute(
            "INSERT INTO website_discovery_cache (origin, status, website_id, error, source, checked_at, retry_after) \
             VALUES (?1, ?2, NULL, ?3, ?4, datetime('now'), datetime('now', ?5)) \
             ON CONFLICT(origin) DO UPDATE SET status = excluded.status, website_id = NULL, error = excluded.error, source = excluded.source, checked_at = datetime('now'), retry_after = excluded.retry_after",
            &[
                DbValue::Text(origin.to_string()),
                DbValue::Text(status.to_string()),
                DbValue::Text(error.chars().take(240).collect()),
                opt_text(source.map(str::to_string)),
                DbValue::Text(retry_after.to_string()),
            ],
        )
        .await?;
    Ok(())
}

async fn record_discovery_success(
    ctx: &AppContext<'_>,
    origin: &str,
    website_id: i64,
    source: &str,
) -> Result<()> {
    ctx.db
        .execute(
            "INSERT INTO website_discovery_cache (origin, status, website_id, error, source, checked_at, retry_after) \
             VALUES (?1, 'discovered', ?2, NULL, ?3, datetime('now'), NULL) \
             ON CONFLICT(origin) DO UPDATE SET status = 'discovered', website_id = excluded.website_id, error = NULL, source = excluded.source, checked_at = datetime('now'), retry_after = NULL",
            &[DbValue::Text(origin.to_string()), DbValue::Integer(website_id), DbValue::Text(source.to_string())],
        )
        .await?;
    Ok(())
}

async fn website_response(ctx: &AppContext<'_>, website: &WebsiteRow) -> Result<Value> {
    Ok(json!({
        "id": website.id,
        "key": website.key,
        "name": website.name,
        "origins": website_origins(ctx, website.id).await?,
        "created_at": to_iso(Some(&website.created_at)),
        "updated_at": to_iso(Some(&website.updated_at)),
    }))
}

async fn website_origins(ctx: &AppContext<'_>, website_id: i64) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct OriginRow {
        origin: String,
    }
    Ok(db::query_all::<OriginRow>(
        ctx.db,
        "SELECT origin FROM website_origins WHERE website_id = ?1 ORDER BY origin ASC",
        &[DbValue::Integer(website_id)],
    )
    .await?
    .into_iter()
    .map(|row| row.origin)
    .collect())
}

fn page_response(page: &PageRow, website: &WebsiteRow) -> Value {
    json!({
        "id": page.id,
        "website_key": website.key,
        "key": page.key,
        "title": page.title,
        "url": page.url,
        "normalized_url": page.normalized_url,
        "metadata": page.metadata.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok()),
        "comment_count": page.comment_count,
        "created_at": to_iso(Some(&page.created_at)),
        "updated_at": to_iso(Some(&page.updated_at)),
    })
}

fn comment_response(row: &CommentRow, context: &CommentResponseContext) -> Value {
    let deleted = row.deleted_at.is_some();
    let reactions = if deleted {
        ReactionCounts::default()
    } else {
        parse_reaction_counts(&row.reactions)
    };
    let actor_owns_comment = context.actor_id == Some(row.user_id);
    let author_is_banned = row.author_is_banned == 1;
    let can_delete = !deleted && (actor_owns_comment || context.actor_can_moderate);
    let can_ban =
        !deleted && context.actor_can_moderate && !actor_owns_comment && !author_is_banned;
    let mut body = json!({
        "id": row.id,
        "website_key": context.website.key,
        "parent_id": row.parent_comment_id,
        "body": if deleted { "" } else { row.body.as_str() },
        "body_html": if deleted { String::new() } else { markdown::render_markdown(&row.body) },
        "author": {
            "id": row.user_id,
            "login": row.login,
            "display_name": if row.display_name.is_empty() { row.login.as_str() } else { row.display_name.as_str() },
            "avatar_url": row.avatar_url,
            "is_website_admin": row.author_is_website_admin == 1,
        },
        "reactions": reactions,
        "deleted": deleted,
        "can_delete": can_delete,
        "can_ban": can_ban,
        "created_at": to_iso(Some(&row.created_at)),
        "updated_at": to_iso(Some(&row.updated_at)),
        "deleted_at": to_iso(row.deleted_at.as_deref()),
    });
    if let Some(page) = &context.page {
        body["page_key"] = Value::String(page.key.clone());
    }
    body
}

async fn website_admins_response(ctx: &AppContext<'_>, website_id: i64) -> Result<Value> {
    #[derive(Deserialize)]
    struct AdminRow {
        id: i64,
        login: String,
        display_name: String,
        email: String,
        avatar_url: String,
        #[serde(rename = "type")]
        user_type: String,
        site_admin: i64,
        created_at: String,
    }
    let rows = db::query_all::<AdminRow>(
        ctx.db,
        "SELECT u.id, u.login, u.display_name, u.email, u.avatar_url, u.type, u.site_admin, wa.created_at \
         FROM website_admins wa JOIN users u ON u.id = wa.user_id \
         WHERE wa.website_id = ?1 ORDER BY wa.created_at ASC, u.id ASC",
        &[DbValue::Integer(website_id)],
    )
    .await?;
    Ok(json!({
        "data": rows.into_iter().map(|row| {
            let user = GitHubUser {
                id: row.id,
                login: row.login,
                display_name: row.display_name,
                email: row.email,
                avatar_url: row.avatar_url,
                r#type: row.user_type,
                site_admin: row.site_admin != 0,
                account_sub: None,
                cached_at: None,
            };
            json!({
                "user": public_user(&user, true),
                "created_at": to_iso(Some(&row.created_at)),
            })
        }).collect::<Vec<_>>()
    }))
}

async fn get_comment_row(
    ctx: &AppContext<'_>,
    website_id: i64,
    comment_id: i64,
) -> Result<CommentRow> {
    let sql = format!(
        "SELECT {} {} WHERE c.website_id = ?1 AND c.id = ?2",
        COMMENT_ROW_COLUMNS, COMMENT_ROW_FROM
    );
    db::query_opt::<CommentRow>(
        ctx.db,
        &sql,
        &[DbValue::Integer(website_id), DbValue::Integer(comment_id)],
    )
    .await?
    .ok_or_else(|| ApiError::not_found("Comment"))
}

async fn ensure_active_comment_in_website(
    ctx: &AppContext<'_>,
    website_id: i64,
    comment_id: i64,
) -> Result<()> {
    ensure_hit(
        ctx,
        "SELECT 1 AS hit FROM comments WHERE website_id = ?1 AND id = ?2 AND deleted_at IS NULL",
        &[DbValue::Integer(website_id), DbValue::Integer(comment_id)],
        "Comment",
    )
    .await
}

async fn ensure_comment_on_page(
    ctx: &AppContext<'_>,
    website_id: i64,
    page_id: i64,
    comment_id: i64,
) -> Result<()> {
    ensure_hit(
        ctx,
        "SELECT 1 AS hit FROM comments WHERE website_id = ?1 AND page_id = ?2 AND id = ?3",
        &[
            DbValue::Integer(website_id),
            DbValue::Integer(page_id),
            DbValue::Integer(comment_id),
        ],
        "Comment",
    )
    .await
}

async fn ensure_active_comment_on_page(
    ctx: &AppContext<'_>,
    website_id: i64,
    page_id: i64,
    comment_id: i64,
) -> Result<()> {
    ensure_hit(
        ctx,
        "SELECT 1 AS hit FROM comments WHERE website_id = ?1 AND page_id = ?2 AND id = ?3 AND deleted_at IS NULL",
        &[DbValue::Integer(website_id), DbValue::Integer(page_id), DbValue::Integer(comment_id)],
        "Comment",
    )
    .await
}

async fn ensure_hit(
    ctx: &AppContext<'_>,
    sql: &str,
    params: &[DbValue],
    resource: &str,
) -> Result<()> {
    #[derive(Deserialize)]
    struct ExistsRow {}
    if db::query_opt::<ExistsRow>(ctx.db, sql, params)
        .await?
        .is_some()
    {
        Ok(())
    } else {
        Err(ApiError::not_found(resource))
    }
}

async fn get_website(ctx: &AppContext<'_>, website_key: &str) -> Result<WebsiteRow> {
    find_website(ctx, website_key)
        .await?
        .ok_or_else(|| ApiError::not_found("Website"))
}

async fn find_website(ctx: &AppContext<'_>, website_key: &str) -> Result<Option<WebsiteRow>> {
    db::query_opt::<WebsiteRow>(
        ctx.db,
        "SELECT id, key, name, created_at, updated_at FROM websites WHERE key = ?1",
        &[DbValue::Text(website_key.to_string())],
    )
    .await
}

async fn get_page(ctx: &AppContext<'_>, website_id: i64, page_key: &str) -> Result<PageRow> {
    find_page(ctx, website_id, page_key)
        .await?
        .ok_or_else(|| ApiError::not_found("Page"))
}

async fn find_page(
    ctx: &AppContext<'_>,
    website_id: i64,
    page_key: &str,
) -> Result<Option<PageRow>> {
    db::query_opt::<PageRow>(
        ctx.db,
        "SELECT id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at FROM pages WHERE website_id = ?1 AND key = ?2",
        &[DbValue::Integer(website_id), DbValue::Text(page_key.to_string())],
    )
    .await
}

async fn upsert_page_row(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    page_key: &str,
    input: &Value,
) -> Result<PageRow> {
    let page_input = normalized_page_input(page_key, input)?;
    db::query_opt::<PageRow>(
        ctx.db,
        "INSERT INTO pages (website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, datetime('now'), datetime('now')) \
         ON CONFLICT(website_id, key) DO UPDATE SET title = excluded.title, url = excluded.url, normalized_url = excluded.normalized_url, metadata = excluded.metadata, updated_at = datetime('now') \
         RETURNING id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at",
        &[
            DbValue::Integer(website.id),
            DbValue::Text(page_input.key),
            DbValue::Text(page_input.title),
            DbValue::Text(page_input.raw_url),
            DbValue::Text(page_input.normalized_url),
            opt_text(page_input.metadata),
        ],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to upsert page"))
}

async fn insert_page_row(
    ctx: &AppContext<'_>,
    website: &WebsiteRow,
    page_key: &str,
    input: &Value,
) -> Result<PageRow> {
    let page_input = normalized_page_input(page_key, input)?;
    if let Some(row) = db::query_opt::<PageRow>(
        ctx.db,
        "INSERT INTO pages (website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, datetime('now'), datetime('now')) \
         ON CONFLICT(website_id, key) DO NOTHING \
         RETURNING id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at",
        &[
            DbValue::Integer(website.id),
            DbValue::Text(page_input.key.clone()),
            DbValue::Text(page_input.title),
            DbValue::Text(page_input.raw_url),
            DbValue::Text(page_input.normalized_url),
            opt_text(page_input.metadata),
        ],
    )
    .await?
    {
        return Ok(row);
    }
    find_page(ctx, website.id, &page_input.key)
        .await?
        .ok_or_else(|| ApiError::internal("failed to create page"))
}

struct NormalizedPageInput {
    key: String,
    raw_url: String,
    normalized_url: String,
    title: String,
    metadata: Option<String>,
}

fn normalized_page_input(page_key: &str, input: &Value) -> Result<NormalizedPageInput> {
    let key = normalize_key(Some(&Value::String(page_key.to_string())), "Page", "key")?;
    let raw_url = input
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if raw_url.is_empty() {
        return Err(ApiError::validation("Page", "url", "missing_field"));
    }
    let normalized_url = normalize_page_url(raw_url)?;
    let title = input
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&normalized_url)
        .trim()
        .to_string();
    if title.is_empty() {
        return Err(ApiError::validation("Page", "title", "missing_field"));
    }
    let metadata = match input.get("metadata") {
        Some(Value::Null) | None => None,
        Some(value) => Some(serde_json::to_string(value).map_err(ApiError::from)?),
    };
    Ok(NormalizedPageInput {
        key,
        raw_url: raw_url.to_string(),
        normalized_url,
        title,
        metadata,
    })
}

async fn require_website_admin_or_super(
    ctx: &AppContext<'_>,
    website_key: &str,
) -> Result<WebsiteRow> {
    let actor = require_user(ctx)?.clone();
    let website = get_website(ctx, website_key).await?;
    if !is_website_admin_or_super(ctx, website.id, actor.id).await? {
        return Err(ApiError::forbidden("Website admin required"));
    }
    Ok(website)
}

async fn is_website_admin_or_super(
    ctx: &AppContext<'_>,
    website_id: i64,
    user_id: i64,
) -> Result<bool> {
    if is_super_admin(ctx).await? {
        return Ok(true);
    }
    #[derive(Deserialize)]
    struct ExistsRow {}
    Ok(db::query_opt::<ExistsRow>(
        ctx.db,
        "SELECT 1 AS hit FROM website_admins WHERE website_id = ?1 AND user_id = ?2 LIMIT 1",
        &[DbValue::Integer(website_id), DbValue::Integer(user_id)],
    )
    .await?
    .is_some())
}

async fn comment_response_context(
    ctx: &AppContext<'_>,
    website: WebsiteRow,
    page: Option<PageRow>,
) -> Result<CommentResponseContext> {
    let actor = ctx.user;
    Ok(CommentResponseContext {
        actor_id: actor.map(|user| user.id),
        actor_can_moderate: match actor {
            Some(user) => is_website_admin_or_super(ctx, website.id, user.id).await?,
            None => false,
        },
        website,
        page,
    })
}

async fn require_not_banned(ctx: &AppContext<'_>, website_id: i64) -> Result<()> {
    let actor = require_user(ctx)?;
    #[derive(Deserialize)]
    struct ExistsRow {}
    if db::query_opt::<ExistsRow>(
        ctx.db,
        "SELECT 1 AS hit FROM website_bans WHERE website_id = ?1 AND user_id = ?2 AND unbanned_at IS NULL LIMIT 1",
        &[DbValue::Integer(website_id), DbValue::Integer(actor.id)],
    )
    .await?
    .is_some()
    {
        return Err(ApiError::forbidden("User is disabled for this website"));
    }
    Ok(())
}

async fn add_website_admins(ctx: &AppContext<'_>, website_id: i64, user_ids: &[i64]) -> Result<()> {
    let unique: Vec<i64> = user_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if unique.is_empty() {
        return Ok(());
    }
    let placeholders = placeholders(unique.len(), 1);
    let params = unique
        .iter()
        .copied()
        .map(DbValue::Integer)
        .collect::<Vec<_>>();
    #[derive(Deserialize)]
    struct ExistingUserRow {}
    let users = db::query_all::<ExistingUserRow>(
        ctx.db,
        &format!("SELECT id FROM users WHERE id IN ({})", placeholders),
        &params,
    )
    .await?;
    if users.len() != unique.len() {
        return Err(ApiError::validation("WebsiteAdmin", "user_id", "invalid"));
    }
    let stmts = unique
        .into_iter()
        .map(|user_id| {
            (
                "INSERT INTO website_admins (website_id, user_id, created_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(website_id, user_id) DO NOTHING",
                vec![DbValue::Integer(website_id), DbValue::Integer(user_id)],
            )
        })
        .collect();
    ctx.db.batch(stmts).await
}

async fn replace_website_origins(
    ctx: &AppContext<'_>,
    website_id: i64,
    raw_origins: Option<&Value>,
) -> Result<()> {
    let Some(raw_origins) = raw_origins else {
        return Ok(());
    };
    let origins = raw_origins
        .as_array()
        .ok_or_else(|| ApiError::validation("Website", "origins", "invalid"))?;
    let mut seen = HashSet::new();
    let mut stmts = vec![(
        "DELETE FROM website_origins WHERE website_id = ?1",
        vec![DbValue::Integer(website_id)],
    )];
    for origin in origins.iter().filter_map(Value::as_str) {
        let origin = normalize_origin(origin)?;
        if seen.insert(origin.clone()) {
            stmts.push((
                "INSERT INTO website_origins (website_id, origin, created_at) VALUES (?1, ?2, datetime('now'))",
                vec![DbValue::Integer(website_id), DbValue::Text(origin)],
            ));
        }
    }
    ctx.db.batch(stmts).await
}

async fn save_pending_website_admins(
    ctx: &AppContext<'_>,
    website_id: i64,
    emails: &[String],
    source: &str,
) -> Result<()> {
    if emails.is_empty() {
        return Ok(());
    }
    let stmts = emails
        .iter()
        .map(|email| {
            (
                "INSERT INTO website_pending_admins (website_id, email, source, created_at) VALUES (?1, ?2, ?3, datetime('now')) ON CONFLICT(website_id, email) DO NOTHING",
                vec![DbValue::Integer(website_id), DbValue::Text(email.clone()), DbValue::Text(source.to_string())],
            )
        })
        .collect();
    ctx.db.batch(stmts).await
}

async fn claim_pending_website_admins(ctx: &AppContext<'_>, user: &GitHubUser) -> Result<()> {
    let Some(email) = normalize_email(&user.email) else {
        return Ok(());
    };
    #[derive(Deserialize)]
    struct Pending {
        website_id: i64,
    }
    let rows = db::query_all::<Pending>(
        ctx.db,
        "SELECT website_id FROM website_pending_admins WHERE email = ?1 AND claimed_at IS NULL ORDER BY website_id ASC",
        &[DbValue::Text(email.clone())],
    )
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut stmts = Vec::with_capacity(rows.len() * 2);
    for row in rows {
        stmts.push((
            "INSERT INTO website_admins (website_id, user_id, created_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(website_id, user_id) DO NOTHING",
            vec![DbValue::Integer(row.website_id), DbValue::Integer(user.id)],
        ));
        stmts.push((
            "UPDATE website_pending_admins SET claimed_at = datetime('now'), claimed_user_id = ?1 WHERE website_id = ?2 AND email = ?3 AND claimed_at IS NULL",
            vec![DbValue::Integer(user.id), DbValue::Integer(row.website_id), DbValue::Text(email.clone())],
        ));
    }
    ctx.db.batch(stmts).await
}

async fn claim_pending_website_admins_for_website(
    ctx: &AppContext<'_>,
    website_id: i64,
) -> Result<()> {
    #[derive(Deserialize)]
    struct Claim {
        email: String,
        user_id: i64,
    }
    let rows = db::query_all::<Claim>(
        ctx.db,
        "SELECT wpa.email, u.id AS user_id FROM website_pending_admins wpa JOIN users u ON lower(u.email) = wpa.email WHERE wpa.website_id = ?1 AND wpa.claimed_at IS NULL ORDER BY u.id ASC",
        &[DbValue::Integer(website_id)],
    )
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut stmts = Vec::with_capacity(rows.len() * 2);
    for row in rows {
        stmts.push((
            "INSERT INTO website_admins (website_id, user_id, created_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(website_id, user_id) DO NOTHING",
            vec![DbValue::Integer(website_id), DbValue::Integer(row.user_id)],
        ));
        stmts.push((
            "UPDATE website_pending_admins SET claimed_at = datetime('now'), claimed_user_id = ?1 WHERE website_id = ?2 AND email = ?3 AND claimed_at IS NULL",
            vec![DbValue::Integer(row.user_id), DbValue::Integer(website_id), DbValue::Text(row.email)],
        ));
    }
    ctx.db.batch(stmts).await
}

fn require_user<'a>(ctx: &'a AppContext<'_>) -> Result<&'a GitHubUser> {
    ctx.user.ok_or_else(ApiError::unauthorized)
}

async fn require_super_admin(ctx: &AppContext<'_>) -> Result<GitHubUser> {
    let actor = require_user(ctx)?;
    if !is_super_admin(ctx).await? {
        return Err(ApiError::forbidden("Super admin required"));
    }
    Ok(actor.clone())
}

fn user_from_row(row: UserRow, account_sub: Option<String>) -> GitHubUser {
    GitHubUser {
        id: row.id,
        login: row.login,
        display_name: row.display_name,
        email: row.email,
        avatar_url: row.avatar_url,
        r#type: row.user_type,
        site_admin: row.site_admin != 0,
        account_sub,
        cached_at: row.cached_at,
    }
}

fn native_user(user: &GitHubUser) -> NativeUser {
    let display = display_name(user);
    NativeUser {
        id: user.id,
        login: display.clone(),
        display_name: display,
        avatar_url: user.avatar_url.clone(),
        email: user.email.clone(),
    }
}

pub fn public_user(user: &GitHubUser, include_email: bool) -> Value {
    let display = display_name(user);
    let mut value = json!({
        "id": user.id,
        "login": display,
        "display_name": display_name(user),
        "avatar_url": user.avatar_url,
    });
    if include_email {
        value["email"] = Value::String(user.email.clone());
    }
    value
}

fn display_name(user: &GitHubUser) -> String {
    if user.display_name.trim().is_empty() {
        user.login.clone()
    } else {
        user.display_name.clone()
    }
}

fn is_account_profile_refresh_due(user: &GitHubUser) -> bool {
    let Some(cached_at) = user_cached_at(user) else {
        return true;
    };
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(&cached_at) {
        return chrono::Utc::now().timestamp() - time.timestamp()
            >= ACCOUNT_PROFILE_REFRESH_TTL_SECONDS;
    }
    chrono::NaiveDateTime::parse_from_str(&cached_at, "%Y-%m-%d %H:%M:%S")
        .map(|time| {
            chrono::Utc::now().timestamp() - time.and_utc().timestamp()
                >= ACCOUNT_PROFILE_REFRESH_TTL_SECONDS
        })
        .unwrap_or(true)
}

fn user_cached_at(user: &GitHubUser) -> Option<String> {
    user.cached_at.clone()
}

async fn allocate_login_for_user(
    ctx: &AppContext<'_>,
    preferred: &str,
    current_user_id: Option<i64>,
) -> Result<String> {
    let base = slugify(preferred);
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };
    #[derive(Deserialize)]
    struct LoginRow {
        id: i64,
        login: String,
    }
    let rows = db::query_all::<LoginRow>(
        ctx.db,
        "SELECT id, login FROM users WHERE login = ?1 OR login GLOB ?2",
        &[
            DbValue::Text(base.clone()),
            DbValue::Text(format!("{}-*", base)),
        ],
    )
    .await?;
    let used = rows
        .into_iter()
        .filter(|row| current_user_id != Some(row.id))
        .map(|row| row.login)
        .collect::<HashSet<_>>();
    for index in 0..1000 {
        let candidate = if index == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, index)
        };
        if !used.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ApiError::internal("unable to allocate login"))
}

fn normalize_page_url(raw: &str) -> Result<String> {
    let mut url = Url::parse(raw).map_err(|_| ApiError::validation("Page", "url", "invalid"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ApiError::validation("Page", "url", "invalid"));
    }
    url.set_fragment(None);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.set_host(Some(&host))
        .map_err(|_| ApiError::validation("Page", "url", "invalid"))?;
    if (url.scheme() == "https" && url.port() == Some(443))
        || (url.scheme() == "http" && url.port() == Some(80))
    {
        url.set_port(None).ok();
    }
    let mut pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    pairs.sort();
    url.set_query(None);
    if !pairs.is_empty() {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(&key, &value);
        }
    }
    Ok(url.to_string())
}

fn page_key_from_url(normalized_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_url.as_bytes());
    let hex = hex::encode(hasher.finalize());
    format!("url-{}", &hex[..32])
}

fn normalize_origin(raw: &str) -> Result<String> {
    let mut url =
        Url::parse(raw).map_err(|_| ApiError::validation("Website", "origins", "invalid"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ApiError::validation("Website", "origins", "invalid"));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.set_host(Some(&host))
        .map_err(|_| ApiError::validation("Website", "origins", "invalid"))?;
    if (url.scheme() == "https" && url.port() == Some(443))
        || (url.scheme() == "http" && url.port() == Some(80))
    {
        url.set_port(None).ok();
    }
    Ok(url.origin().ascii_serialization())
}

fn normalize_discovery_origin(raw: &str) -> Result<String> {
    let origin = normalize_origin(raw)?;
    if !origin.starts_with("https://") {
        return Err(ApiError::bad_request("origin must be https"));
    }
    Ok(origin)
}

fn normalize_key(value: Option<&Value>, resource: &str, field: &str) -> Result<String> {
    let key = value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let valid = key.len() >= 2
        && key.len() <= 128
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && key.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' || c == '-'
        });
    if valid {
        Ok(key)
    } else {
        Err(ApiError::validation(resource, field, "invalid"))
    }
}

fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_ascii_lowercase();
    if email.is_empty()
        || email.len() > 254
        || !email.contains('@')
        || !email.contains('.')
        || email.contains(char::is_whitespace)
    {
        None
    } else {
        Some(email)
    }
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        let next = if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' {
            Some(ch)
        } else if ch == '-' {
            Some('-')
        } else {
            Some('-')
        };
        if let Some(ch) = next {
            if ch == '-' {
                if !last_dash && !out.is_empty() {
                    out.push(ch);
                }
                last_dash = true;
            } else {
                out.push(ch);
                last_dash = false;
            }
        }
        if out.len() >= 64 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn parse_reaction_counts(raw: &str) -> ReactionCounts {
    let mut counts = serde_json::from_str::<ReactionCounts>(raw).unwrap_or_default();
    counts.total = counts.like
        + counts.dislike
        + counts.heart
        + counts.laugh
        + counts.hooray
        + counts.confused
        + counts.rocket
        + counts.eyes;
    counts
}

fn cursor_page(data: Vec<Value>, has_more: bool, last_id: Option<i64>) -> Result<Value> {
    Ok(json!({
        "data": data,
        "pagination": {
            "next_cursor": if has_more { last_id.map(encode_cursor).transpose()? } else { None },
            "has_more": has_more,
        }
    }))
}

fn list_limit(query: &HashMap<String, String>) -> i64 {
    query
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20)
        .clamp(1, 100)
}

fn placeholders(count: usize, start: usize) -> String {
    (start..start + count)
        .map(|index| format!("?{}", index))
        .collect::<Vec<_>>()
        .join(", ")
}

fn to_iso(value: Option<&str>) -> Value {
    match value {
        None => Value::Null,
        Some(value) if value.contains('T') && value.ends_with('Z') => {
            Value::String(value.to_string())
        }
        Some(value) => Value::String(format!("{}Z", value.replace(' ', "T"))),
    }
}

fn opt_text(value: Option<String>) -> DbValue {
    value.map(DbValue::Text).unwrap_or(DbValue::Null)
}

fn opt_i64(value: Option<i64>) -> DbValue {
    value.map(DbValue::Integer).unwrap_or(DbValue::Null)
}

fn first_present(values: [Option<&str>; 4]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn super_admin_ids(ctx: &AppContext<'_>) -> Vec<String> {
    ctx.super_admin_account_ids
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn account_base_url(ctx: &AppContext<'_>) -> String {
    ctx.account_base_url
        .unwrap_or("https://account.jihuayu.com")
        .trim_end_matches('/')
        .to_string()
}

fn account_audience(ctx: &AppContext<'_>) -> String {
    ctx.account_audience.unwrap_or("atrium").to_string()
}

fn mocked_well_known_text(ctx: &AppContext<'_>, origin: &str) -> Option<String> {
    let raw = ctx.test_discovery_well_known?;
    let map = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
    map.get(origin).map(|value| {
        value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())
    })
}

fn mocked_dns_txt_payloads(ctx: &AppContext<'_>, hostname: &str) -> Option<Vec<String>> {
    let raw = ctx.test_discovery_dns_txt?;
    let map = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
    let value = map
        .get(hostname)
        .or_else(|| map.get(&format!("_atrium.{}", hostname)))?;
    let record = if let Some(array) = value.as_array() {
        array
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("")
    } else {
        value.as_str().unwrap_or_default().to_string()
    };
    Some(vec![
        record
            .strip_prefix(TXT_RECORD_PREFIX)
            .unwrap_or(&record)
            .to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::Database,
        types::GitHubApiUser,
    };
    use rand::rngs::OsRng;
    use rsa::{
        RsaPublicKey,
        traits::{PrivateKeyParts, PublicKeyParts},
    };
    use std::collections::HashMap;

    static NOOP_DB: NoopDb = NoopDb;
    static NOOP_HTTP: NoopHttp = NoopHttp;

    struct NoopDb;

    #[cfg_attr(feature = "server", async_trait::async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait::async_trait(?Send))]
    impl Database for NoopDb {
        async fn execute(&self, _sql: &str, _params: &[DbValue]) -> Result<u64> {
            Err(ApiError::internal("not used"))
        }

        async fn query_opt_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Option<Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn query_all_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Vec<Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn batch(&self, _stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()> {
            Err(ApiError::internal("not used"))
        }
    }

    struct NoopHttp;

    #[cfg_attr(feature = "server", async_trait::async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait::async_trait(?Send))]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }
    }

    fn test_context<'a>(private_jwk: &'a str, key_id: Option<&'a str>) -> AppContext<'a> {
        AppContext {
            db: &NOOP_DB,
            http: &NOOP_HTTP,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: None,
            apple_app_id: None,
            github_client_id: None,
            github_client_secret: None,
            account_base_url: None,
            account_audience: None,
            account_internal_secret: None,
            super_admin_account_ids: None,
            discovery_private_jwk: Some(private_jwk),
            discovery_public_jwk: None,
            discovery_key_id: key_id,
            test_discovery_well_known: None,
            test_discovery_dns_txt: None,
            stateful_sessions: false,
            test_bypass_secret: None,
        }
    }

    fn generated_private_jwk(kid: &str) -> (String, RsaPublicKey) {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public_key = RsaPublicKey::from(&private_key);
        let private_jwk = json!({
            "kty": "RSA",
            "kid": kid,
            "n": b64_biguint(public_key.n()),
            "e": b64_biguint(public_key.e()),
            "d": b64_biguint(private_key.d()),
            "p": b64_biguint(&private_key.primes()[0]),
            "q": b64_biguint(&private_key.primes()[1]),
        })
        .to_string();
        (private_jwk, public_key)
    }

    fn encrypt_discovery_value(public_key: &RsaPublicKey, kid: &str, value: &Value) -> String {
        let protected = json!({
            "alg": DISCOVERY_JWE_ALG,
            "enc": DISCOVERY_JWE_ENC,
            "kid": kid,
        });
        let protected =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&protected).expect("protected header"));
        let cek = [17_u8; 32];
        let iv = [23_u8; 12];
        let mut rng = OsRng;
        let encrypted_key = public_key
            .encrypt(&mut rng, Oaep::new::<Sha256>(), &cek)
            .expect("encrypt CEK");
        let cipher = Aes256Gcm::new_from_slice(&cek).expect("cipher");
        let plaintext = serde_json::to_vec(value).expect("plaintext JSON");
        let mut encrypted = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &plaintext,
                    aad: protected.as_bytes(),
                },
            )
            .expect("encrypt field");
        let tag = encrypted.split_off(encrypted.len() - 16);
        format!(
            "{}.{}.{}.{}.{}",
            protected,
            URL_SAFE_NO_PAD.encode(encrypted_key),
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(encrypted),
            URL_SAFE_NO_PAD.encode(tag)
        )
    }

    fn b64_biguint(value: &BigUint) -> String {
        URL_SAFE_NO_PAD.encode(value.to_bytes_be())
    }

    #[test]
    fn parse_discovery_document_accepts_encrypted_worker_fields() {
        let (private_jwk, public_key) = generated_private_jwk("disc-1");
        let name = encrypt_discovery_value(&public_key, "disc-1", &json!("Encrypted Site"));
        let admin_emails =
            encrypt_discovery_value(&public_key, "disc-1", &json!(["OWNER@Example.COM"]));
        let document = json!({
            "atrium": "v1",
            "name": format!("{}{}", ENCRYPTED_FIELD_PREFIX, name),
            "admin_emails": format!("{}{}", ENCRYPTED_FIELD_PREFIX, admin_emails),
            "contact_email": "Support@Example.COM"
        });
        let ctx = test_context(&private_jwk, Some("disc-1"));

        let metadata = parse_discovery_document(
            &ctx,
            &document.to_string(),
            "https://blog.example.com",
            "well-known",
        )
        .expect("parse discovery document");

        assert_eq!(metadata.origin, "https://blog.example.com");
        assert_eq!(metadata.website_key, "blog.example.com");
        assert_eq!(metadata.name, "Encrypted Site");
        assert_eq!(metadata.admin_emails, vec!["owner@example.com"]);
        assert_eq!(
            metadata.contact_email.as_deref(),
            Some("support@example.com")
        );
        assert_eq!(metadata.source, "well-known");
    }

    #[test]
    fn parse_discovery_document_rejects_wrong_jwe_kid() {
        let (private_jwk, public_key) = generated_private_jwk("disc-1");
        let name = encrypt_discovery_value(&public_key, "disc-1", &json!("Encrypted Site"));
        let document = json!({
            "atrium": "v1",
            "name": format!("{}{}", ENCRYPTED_FIELD_PREFIX, name),
            "admin_emails": ["owner@example.com"]
        });
        let ctx = test_context(&private_jwk, Some("disc-2"));

        let error = parse_discovery_document(
            &ctx,
            &document.to_string(),
            "https://blog.example.com",
            "dns",
        )
        .expect_err("kid mismatch should fail");

        assert_eq!(error.status, 400);
        assert_eq!(error.body.message, "name could not be decrypted");
    }

    #[tokio::test]
    async fn discovery_public_key_strips_private_jwk_fields() {
        let (private_jwk, _) = generated_private_jwk("disc-1");
        let ctx = test_context(&private_jwk, Some("disc-override"));

        let response = discovery_public_key(&ctx)
            .await
            .expect("public key response");

        assert_eq!(response["kid"], "disc-override");
        assert_eq!(response["alg"], DISCOVERY_JWE_ALG);
        assert_eq!(response["enc"], DISCOVERY_JWE_ENC);
        assert_eq!(response["jwk"]["kid"], "disc-override");
        assert_eq!(response["jwk"]["alg"], DISCOVERY_JWE_ALG);
        assert_eq!(response["jwk"]["key_ops"], json!(["encrypt"]));
        assert!(response["jwk"]["kty"].is_string());
        assert!(response["jwk"]["d"].is_null());
        assert!(response["jwk"]["p"].is_null());
        assert!(response["jwk"]["q"].is_null());
    }

    #[test]
    fn parse_dns_txt_data_decodes_worker_compatible_segments() {
        let parts = parse_dns_txt_data(
            "\"atrium-site={\\034atrium\\034:\\034v1\\034,\" \"\\034admin_emails\\034:[]}\"",
        );

        assert_eq!(
            parts.join(""),
            "atrium-site={\"atrium\":\"v1\",\"admin_emails\":[]}"
        );
    }
}
