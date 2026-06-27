use super::*;

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

pub(super) async fn website_origins(ctx: &AppContext<'_>, website_id: i64) -> Result<Vec<String>> {
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

pub(super) async fn get_website(ctx: &AppContext<'_>, website_key: &str) -> Result<WebsiteRow> {
    find_website(ctx, website_key)
        .await?
        .ok_or_else(|| ApiError::not_found("Website"))
}

pub(super) async fn find_website(
    ctx: &AppContext<'_>,
    website_key: &str,
) -> Result<Option<WebsiteRow>> {
    db::query_opt::<WebsiteRow>(
        ctx.db,
        "SELECT id, key, name, created_at, updated_at FROM websites WHERE key = ?1",
        &[DbValue::Text(website_key.to_string())],
    )
    .await
}

pub(super) async fn get_page(
    ctx: &AppContext<'_>,
    website_id: i64,
    page_key: &str,
) -> Result<PageRow> {
    find_page(ctx, website_id, page_key)
        .await?
        .ok_or_else(|| ApiError::not_found("Page"))
}

pub(super) async fn find_page(
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

pub(super) async fn insert_page_row(
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

pub(super) async fn require_website_admin_or_super(
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

pub(super) async fn is_website_admin_or_super(
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

pub(super) async fn add_website_admins(
    ctx: &AppContext<'_>,
    website_id: i64,
    user_ids: &[i64],
) -> Result<()> {
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

pub(super) async fn save_pending_website_admins(
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

pub(super) async fn claim_pending_website_admins(
    ctx: &AppContext<'_>,
    user: &GitHubUser,
) -> Result<()> {
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

pub(super) async fn claim_pending_website_admins_for_website(
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
