use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    db::{self, Database, DbValue},
    error::ApiError,
    types::GitHubApiUser,
    types::GitHubUser,
    Result,
};

#[cfg_attr(feature = "worker", async_trait(?Send))]
#[cfg_attr(not(feature = "worker"), async_trait)]
pub trait HttpClient: Send + Sync {
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser>;
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

pub async fn resolve_user(
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
             WHERE tc.token_hash = ?1 AND tc.expires_at > datetime('now')",
            &[DbValue::Text(token_hash.clone())],
        )
    .await?
    {
        return Ok(cached.into());
    }

    let gh_user = http.get_github_user(token).await?;

    db.batch(vec![
        (
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET \
             login = excluded.login, email = excluded.email, avatar_url = excluded.avatar_url, \
             type = excluded.type, site_admin = excluded.site_admin, cached_at = datetime('now')",
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
             user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
            vec![
                DbValue::Text(token_hash),
                DbValue::Integer(gh_user.id),
                DbValue::Integer(cache_ttl_secs),
            ],
        ),
    ])
    .await?;

    Ok(gh_user.into())
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
