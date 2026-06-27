use std::collections::{HashMap, HashSet};

mod auth;
mod comments;
mod discovery;
mod site;

pub use auth::{
    is_super_admin, issue_atrium_tokens, refresh_atrium_tokens, resolve_account_cookie_user,
    resolve_atrium_jwt_user, resolve_native_request_user, upsert_auth_user,
};
pub use comments::{
    ban_website_user, create_current_comment, create_page_comment, delete_comment,
    delete_comment_reaction, delete_current_reaction, get_current_comments, list_current_replies,
    list_moderation_comments, list_page_comments, list_website_bans, set_comment_reaction,
    set_current_reaction, unban_website_user, update_comment,
};
pub use discovery::discovery_public_key;
pub use site::{
    PageRow, WebsiteRow, add_website_admin_by_input, create_website, get_page_response,
    get_website_response, list_pages, list_website_admins, list_websites, remove_website_admin,
    update_website, upsert_page,
};

use discovery::discover_website_for_origin;
use site::{
    claim_pending_website_admins, claim_pending_website_admins_for_website, find_page,
    find_website, get_page, get_website, insert_page_row, is_website_admin_or_super,
    require_website_admin_or_super, save_pending_website_admins, website_origins,
};

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
