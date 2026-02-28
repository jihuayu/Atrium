use serde::Deserialize;

use crate::{
    auth::hash_token,
    db::{self, DbValue},
    error::ApiError,
    jwt,
    services::session,
    types::{AuthTokenResponse, GitHubUser, JwtClaims, NativeUser, ProviderUser},
    AppContext, Result,
};

const ACCESS_TTL_SECS: i64 = 3600;
const REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;

#[derive(Debug, Deserialize)]
struct IdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct UserRow {
    id: i64,
    login: String,
    email: String,
    avatar_url: String,
    r#type: String,
    site_admin: i64,
}

pub async fn resolve_or_create_user(
    ctx: &AppContext<'_>,
    provider_user: &ProviderUser,
) -> Result<GitHubUser> {
    if let Some(row) = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin \
         FROM user_identities ui \
         JOIN users u ON u.id = ui.user_id \
         WHERE ui.provider = ?1 AND ui.provider_user_id = ?2",
        &[
            DbValue::Text(provider_user.provider.clone()),
            DbValue::Text(provider_user.provider_user_id.clone()),
        ],
    )
    .await?
    {
        return Ok(to_user(row));
    }

    let mut user_id = None;
    if !provider_user.email.is_empty() && !provider_user.email.ends_with("privaterelay.appleid.com")
    {
        user_id = db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE email = ?1",
            &[DbValue::Text(provider_user.email.clone())],
        )
        .await?
        .map(|v| v.id);
    }

    let user_id = if let Some(id) = user_id {
        id
    } else {
        let login = allocate_login(ctx, &provider_user.login).await?;
        ctx.db
            .execute(
                "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
                &[
                    DbValue::Text(login.clone()),
                    DbValue::Text(provider_user.email.clone()),
                    DbValue::Text(provider_user.avatar_url.clone()),
                    DbValue::Text(provider_user.r#type.clone()),
                    DbValue::Integer(provider_user.site_admin as i64),
                ],
            )
            .await?;
        db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE login = ?1",
            &[DbValue::Text(login)],
        )
        .await?
        .ok_or_else(|| ApiError::internal("failed to create user"))?
        .id
    };

    ctx.db
        .execute(
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
             ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
             user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
            &[
                DbValue::Integer(user_id),
                DbValue::Text(provider_user.provider.clone()),
                DbValue::Text(provider_user.provider_user_id.clone()),
                DbValue::Text(provider_user.email.clone()),
                DbValue::Text(provider_user.avatar_url.clone()),
            ],
        )
        .await?;

    let row = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to load user"))?;

    Ok(to_user(row))
}

pub async fn issue_xtalk_jwt(ctx: &AppContext<'_>, user: &GitHubUser) -> Result<AuthTokenResponse> {
    let now = chrono::Utc::now().timestamp();
    let access_claims = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "xtalk".to_string(),
        iat: now,
        exp: now + ACCESS_TTL_SECS,
        jti: format!("acc-{}-{}", user.id, now),
        token_type: "access".to_string(),
    };
    let refresh_claims = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "xtalk".to_string(),
        iat: now,
        exp: now + REFRESH_TTL_SECS,
        jti: format!("ref-{}-{}", user.id, now),
        token_type: "refresh".to_string(),
    };

    let access_token = jwt::sign_jwt(&access_claims, ctx.jwt_secret)?;
    let refresh_token = jwt::sign_jwt(&refresh_claims, ctx.jwt_secret)?;

    if ctx.stateful_sessions {
        session::create_session(ctx, &refresh_token, user.id, REFRESH_TTL_SECS).await?;
    }

    Ok(AuthTokenResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TTL_SECS,
        token_type: "Bearer".to_string(),
        user: NativeUser {
            id: user.id,
            login: user.login.clone(),
            avatar_url: user.avatar_url.clone(),
            email: user.email.clone(),
        },
    })
}

pub async fn refresh_xtalk_jwt(
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

    if ctx.stateful_sessions {
        session::validate_session(ctx, refresh_token, user_id).await?;
    }

    let user = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(to_user)
    .ok_or_else(ApiError::unauthorized)?;

    issue_xtalk_jwt(ctx, &user).await
}

pub async fn revoke_current_session(ctx: &AppContext<'_>, user_id: i64) -> Result<()> {
    if ctx.stateful_sessions {
        session::revoke_user_sessions(ctx, user_id).await?;
    }
    Ok(())
}

async fn allocate_login(ctx: &AppContext<'_>, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        "user".to_string()
    } else {
        preferred.trim().to_lowercase()
    };
    for index in 0..1000 {
        let candidate = if index == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, index)
        };
        let exists = db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE login = ?1",
            &[DbValue::Text(candidate.clone())],
        )
        .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
    }
    Err(ApiError::internal("unable to allocate login"))
}

fn to_user(row: UserRow) -> GitHubUser {
    GitHubUser {
        id: row.id,
        login: row.login,
        email: row.email,
        avatar_url: row.avatar_url,
        r#type: row.r#type,
        site_admin: row.site_admin != 0,
    }
}

pub async fn cache_provider_token(
    ctx: &AppContext<'_>,
    provider: &str,
    token: &str,
    user_id: i64,
    ttl_secs: i64,
) -> Result<()> {
    let token_hash = hash_token(token);
    ctx.db
        .execute(
            "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) \
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now', '+' || ?4 || ' seconds')) \
             ON CONFLICT(token_hash, provider) DO UPDATE SET \
             user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
            &[
                DbValue::Text(token_hash),
                DbValue::Text(provider.to_string()),
                DbValue::Integer(user_id),
                DbValue::Integer(ttl_secs),
            ],
        )
        .await?;
    Ok(())
}
