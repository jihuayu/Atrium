use super::*;

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
