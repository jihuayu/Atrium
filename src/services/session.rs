use serde::Deserialize;

use crate::{
    auth::hash_token,
    db::{self, DbValue},
    error::ApiError,
    AppContext, Result,
};

#[derive(Debug, Deserialize)]
struct SessionRow {
    user_id: i64,
    expires_at: String,
    revoked_at: Option<String>,
}

pub async fn create_session(
    ctx: &AppContext<'_>,
    refresh_token: &str,
    user_id: i64,
    ttl_secs: i64,
) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    ctx.db
        .execute(
            "INSERT INTO sessions (refresh_token_hash, user_id, created_at, expires_at, revoked_at) \
             VALUES (?1, ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds'), NULL) \
             ON CONFLICT(refresh_token_hash) DO UPDATE SET \
             user_id = excluded.user_id, created_at = datetime('now'), expires_at = excluded.expires_at, revoked_at = NULL",
            &[
                DbValue::Text(token_hash),
                DbValue::Integer(user_id),
                DbValue::Integer(ttl_secs),
            ],
        )
        .await?;
    Ok(())
}

pub async fn validate_session(
    ctx: &AppContext<'_>,
    refresh_token: &str,
    user_id: i64,
) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    let row = db::query_opt::<SessionRow>(
        ctx.db,
        "SELECT user_id, expires_at, revoked_at FROM sessions WHERE refresh_token_hash = ?1",
        &[DbValue::Text(token_hash)],
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;

    if row.user_id != user_id || row.revoked_at.is_some() {
        return Err(ApiError::unauthorized());
    }
    if !row.expires_at.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let expires = parse_sqlite_datetime(&row.expires_at).unwrap_or(0);
        if expires <= now {
            return Err(ApiError::unauthorized());
        }
    }

    Ok(())
}

pub async fn revoke_session(ctx: &AppContext<'_>, refresh_token: &str) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    ctx.db
        .execute(
            "UPDATE sessions SET revoked_at = datetime('now') WHERE refresh_token_hash = ?1",
            &[DbValue::Text(token_hash)],
        )
        .await?;
    Ok(())
}

pub async fn revoke_user_sessions(ctx: &AppContext<'_>, user_id: i64) -> Result<()> {
    ctx.db
        .execute(
            "UPDATE sessions SET revoked_at = datetime('now') WHERE user_id = ?1 AND revoked_at IS NULL",
            &[DbValue::Integer(user_id)],
        )
        .await?;
    Ok(())
}

fn parse_sqlite_datetime(value: &str) -> Option<i64> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(parsed.timestamp());
    }
    let fixed = if value.contains('T') {
        value.to_string()
    } else {
        value.replace(' ', "T") + "Z"
    };
    chrono::DateTime::parse_from_rfc3339(&fixed)
        .ok()
        .map(|v| v.timestamp())
}
