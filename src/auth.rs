use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    db::{self, Database, DbValue},
    error::ApiError,
    jwt,
    types::GitHubApiUser,
    types::GitHubUser,
    Result,
};

#[cfg_attr(feature = "worker", async_trait(?Send))]
#[cfg_attr(not(feature = "worker"), async_trait)]
pub trait HttpClient: Send + Sync {
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser>;
    async fn get_jwks(&self, url: &str) -> Result<UpstreamResponse>;
    async fn post_utterances_token(
        &self,
        body: &[u8],
        headers: &HashMap<String, String>,
    ) -> Result<UpstreamResponse>;
}

pub struct UpstreamResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

pub fn parse_token(header: &str) -> Option<&str> {
    let header = header.trim();
    if let Some(token) = header.strip_prefix("token ") {
        return Some(token.trim());
    }
    if let Some(token) = header.strip_prefix("Bearer ") {
        return Some(token.trim());
    }
    None
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

#[derive(Debug, Deserialize)]
struct CachedUser {
    pub id: i64,
    pub login: String,
    pub email: String,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub site_admin: i64,
}

impl From<CachedUser> for GitHubUser {
    fn from(value: CachedUser) -> Self {
        Self {
            id: value.id,
            login: value.login,
            email: value.email,
            avatar_url: value.avatar_url,
            r#type: value.r#type,
            site_admin: value.site_admin != 0,
        }
    }
}

pub async fn resolve_github_user(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    cache_ttl_secs: i64,
) -> Result<GitHubUser> {
    let token_hash = hash_token(token);

    if let Some(cached) = db::query_opt::<CachedUser>(
        db,
        "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin \
             FROM token_cache tc \
             JOIN users u ON tc.user_id = u.id \
             WHERE tc.token_hash = ?1 AND tc.provider = 'github' AND tc.expires_at > datetime('now')",
        &[DbValue::Text(token_hash.clone())],
    )
    .await?
    {
        return Ok(cached.into());
    }

    let gh_user = http.get_github_user(token).await?;

    let user_id = resolve_or_create_provider_user(
        db,
        "github",
        &gh_user.id.to_string(),
        &gh_user.login,
        gh_user.email.as_deref().unwrap_or(""),
        &gh_user.avatar_url,
        &gh_user.r#type,
        gh_user.site_admin,
    )
    .await?;

    db.batch(vec![
        (
            "UPDATE users SET login = ?1, email = ?2, avatar_url = ?3, type = ?4, site_admin = ?5, cached_at = datetime('now') \
             WHERE id = ?6",
            vec![
                DbValue::Text(gh_user.login.clone()),
                DbValue::Text(gh_user.email.clone().unwrap_or_default()),
                DbValue::Text(gh_user.avatar_url.clone()),
                DbValue::Text(gh_user.r#type.clone()),
                DbValue::Integer(gh_user.site_admin as i64),
                DbValue::Integer(user_id),
            ],
        ),
        (
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
             ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
             user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
            vec![
                DbValue::Integer(user_id),
                DbValue::Text("github".to_string()),
                DbValue::Text(gh_user.id.to_string()),
                DbValue::Text(gh_user.email.clone().unwrap_or_default()),
                DbValue::Text(gh_user.avatar_url.clone()),
            ],
        ),
        (
            "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) \
             VALUES (?1, 'github', ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds')) \
             ON CONFLICT(token_hash, provider) DO UPDATE SET \
             user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
            vec![
                DbValue::Text(token_hash),
                DbValue::Integer(user_id),
                DbValue::Integer(cache_ttl_secs),
            ],
        ),
    ])
    .await?;

    db::query_opt::<CachedUser>(
        db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(Into::into)
    .ok_or_else(|| ApiError::internal("failed to resolve github user"))
}

pub async fn resolve_user(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    cache_ttl_secs: i64,
) -> Result<GitHubUser> {
    resolve_github_user(db, http, token, cache_ttl_secs).await
}

pub async fn resolve_xtalk_jwt_user(
    db: &dyn Database,
    auth_header: Option<&str>,
    jwt_secret: &[u8],
) -> Result<Option<GitHubUser>> {
    let token = bearer_from_header(auth_header)?;
    let Some(token) = token else {
        return Ok(None);
    };

    let claims = jwt::verify_jwt(&token, jwt_secret)?;
    if claims.token_type == "refresh" {
        return Err(ApiError::unauthorized());
    }
    let user_id = claims
        .sub
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;

    let user = db::query_opt::<CachedUser>(
        db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(Into::into)
    .ok_or_else(ApiError::unauthorized)?;

    Ok(Some(user))
}

async fn resolve_or_create_provider_user(
    db: &dyn Database,
    provider: &str,
    provider_user_id: &str,
    login: &str,
    email: &str,
    avatar_url: &str,
    user_type: &str,
    site_admin: bool,
) -> Result<i64> {
    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    if let Some(row) = db::query_opt::<IdRow>(
        db,
        "SELECT user_id AS id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
        &[
            DbValue::Text(provider.to_string()),
            DbValue::Text(provider_user_id.to_string()),
        ],
    )
    .await?
    {
        return Ok(row.id);
    }

    if !email.is_empty() && !email.ends_with("privaterelay.appleid.com") {
        if let Some(row) = db::query_opt::<IdRow>(
            db,
            "SELECT id FROM users WHERE email = ?1",
            &[DbValue::Text(email.to_string())],
        )
        .await?
        {
            db.execute(
                "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
                 ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
                &[
                    DbValue::Integer(row.id),
                    DbValue::Text(provider.to_string()),
                    DbValue::Text(provider_user_id.to_string()),
                    DbValue::Text(email.to_string()),
                    DbValue::Text(avatar_url.to_string()),
                ],
            )
            .await?;
            return Ok(row.id);
        }
    }

    let unique_login = unique_login(db, login).await?;
    db.execute(
        "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        &[
            DbValue::Text(unique_login.clone()),
            DbValue::Text(email.to_string()),
            DbValue::Text(avatar_url.to_string()),
            DbValue::Text(user_type.to_string()),
            DbValue::Integer(site_admin as i64),
        ],
    )
    .await?;

    let row = db::query_opt::<IdRow>(
        db,
        "SELECT id FROM users WHERE login = ?1",
        &[DbValue::Text(unique_login)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to create provider user"))?;

    db.execute(
        "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        &[
            DbValue::Integer(row.id),
            DbValue::Text(provider.to_string()),
            DbValue::Text(provider_user_id.to_string()),
            DbValue::Text(email.to_string()),
            DbValue::Text(avatar_url.to_string()),
        ],
    )
    .await?;

    Ok(row.id)
}

async fn unique_login(db: &dyn Database, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        "user".to_string()
    } else {
        preferred.trim().to_lowercase()
    };

    #[derive(Debug, Deserialize)]
    struct ExistsRow {
        _hit: i64,
    }

    for index in 0..1000 {
        let candidate = if index == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, index)
        };
        let exists = db::query_opt::<ExistsRow>(
            db,
            "SELECT 1 AS hit FROM users WHERE login = ?1",
            &[DbValue::Text(candidate.clone())],
        )
        .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
    }

    Err(ApiError::internal("unable to allocate unique login"))
}

pub fn bearer_from_header(header: Option<&str>) -> Result<Option<String>> {
    match header {
        None => Ok(None),
        Some(h) => parse_token(h)
            .map(|v| Some(v.to_string()))
            .ok_or_else(|| ApiError::unauthorized()),
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_token, parse_token};

    #[test]
    fn parses_token_and_bearer() {
        assert_eq!(parse_token("token abc"), Some("abc"));
        assert_eq!(parse_token("Bearer xyz"), Some("xyz"));
        assert_eq!(parse_token("basic aaa"), None);
    }

    #[test]
    fn token_hash_is_stable() {
        let a = hash_token("ghp_example");
        let b = hash_token("ghp_example");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
