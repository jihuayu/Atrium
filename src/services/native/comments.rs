use super::*;

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
