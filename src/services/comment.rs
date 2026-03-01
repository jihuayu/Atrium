use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::comment as comment_fmt,
    markdown::render_markdown,
    services::{normalize_pagination, repo},
    types::{
        CommentResponse, CreateCommentInput, GitHubUser, ListCommentsQuery, UpdateCommentInput,
    },
    AppContext, Result,
};

#[derive(Debug, Deserialize, Clone)]
struct CommentRow {
    id: i64,
    issue_id: i64,
    body: String,
    user_id: i64,
    created_at: String,
    updated_at: String,
    reactions: String,
    login: String,
    avatar_url: String,
    user_type: String,
    site_admin: i64,
    issue_number: i64,
    repo_owner: String,
    repo_name: String,
    admin_user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct IssueRow {
    issue_id: i64,
    repo_id: i64,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    total: i64,
}

pub async fn list_comments(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
    query: &ListCommentsQuery,
) -> Result<(Vec<CommentResponse>, i64, i64, i64)> {
    let issue = resolve_issue(ctx, owner, repo_name, number).await?;
    let (page, per_page, offset) = normalize_pagination(query.page, query.per_page);

    if query.since.is_none() {
        if let Some(cache) = ctx.comment_cache {
            if let Some((cached, total)) = cache.get_list(issue.issue_id, page, per_page).await? {
                return Ok((cached, total, page, per_page));
            }
        }
    }

    let mut filters = vec![
        "c.issue_id = ?1".to_string(),
        "c.deleted_at IS NULL".to_string(),
    ];
    let mut params = vec![DbValue::Integer(issue.issue_id)];
    let mut idx = 2;
    if let Some(since) = &query.since {
        filters.push(format!("c.updated_at >= ?{}", idx));
        params.push(DbValue::Text(since.clone()));
        idx += 1;
    }

    let where_sql = filters.join(" AND ");
    let count_sql = format!(
        "SELECT COUNT(*) AS total FROM comments c WHERE {}",
        where_sql
    );
    let total = db::query_opt::<CountRow>(ctx.db, &count_sql, &params)
        .await?
        .map(|v| v.total)
        .unwrap_or(0);

    let mut list_params = params.clone();
    list_params.push(DbValue::Integer(per_page));
    list_params.push(DbValue::Integer(offset));

    let list_sql = format!(
        "SELECT \
            c.id, c.issue_id, c.body, c.user_id, c.created_at, c.updated_at, c.reactions, \
            u.login, u.avatar_url, u.type AS user_type, u.site_admin, \
            i.number AS issue_number, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id \
         FROM comments c \
         JOIN users u ON u.id = c.user_id \
         JOIN issues i ON i.id = c.issue_id \
         JOIN repos r ON r.id = c.repo_id \
         WHERE {} \
         ORDER BY c.created_at ASC \
         LIMIT ?{} OFFSET ?{}",
        where_sql,
        idx,
        idx + 1
    );

    let rows = db::query_all::<CommentRow>(ctx.db, &list_sql, &list_params).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(to_response(ctx, &row));
    }

    if query.since.is_none() {
        if let Some(cache) = ctx.comment_cache {
            cache
                .set_list(issue.issue_id, page, per_page, items.clone(), total)
                .await?;
        }
    }

    Ok((items, total, page, per_page))
}

pub async fn create_comment(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
    input: &CreateCommentInput,
) -> Result<CommentResponse> {
    let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
    if input.body.trim().is_empty() {
        return Err(ApiError::validation(
            "IssueComment",
            "body",
            "missing_field",
        ));
    }

    let issue = resolve_issue(ctx, owner, repo_name, number).await?;

    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    let comment_id = db::query_opt::<IdRow>(
        ctx.db,
        "INSERT INTO comments (repo_id, issue_id, body, user_id, created_at, updated_at, reactions) \
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'), '{}') \
         RETURNING id",
        &[
            DbValue::Integer(issue.repo_id),
            DbValue::Integer(issue.issue_id),
            DbValue::Text(input.body.clone()),
            DbValue::Integer(user.id),
        ],
    )
    .await?
    .ok_or_else(|| ApiError::internal("comment insert failed"))?
    .id;

    ctx.db
        .execute(
            "UPDATE issues SET comment_count = comment_count + 1, updated_at = datetime('now') WHERE id = ?1",
            &[DbValue::Integer(issue.issue_id)],
        )
        .await?;

    if let Some(cache) = ctx.comment_cache {
        cache.invalidate_issue(issue.issue_id).await?;
        cache.invalidate_comment(comment_id).await?;
    }

    get_comment(ctx, owner, repo_name, comment_id).await
}

pub async fn get_comment(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
) -> Result<CommentResponse> {
    let repo_row = repo::get_repo(ctx, owner, repo_name).await?;

    if let Some(cache) = ctx.comment_cache {
        if let Some(cached) = cache.get_single(comment_id).await? {
            if is_comment_in_repo(&cached, owner, repo_name) {
                return Ok(cached);
            }
        }
    }

    let row = fetch_comment_row(ctx, repo_row.id, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    let response = to_response(ctx, &row);
    if let Some(cache) = ctx.comment_cache {
        cache.set_single(response.clone()).await?;
    }

    Ok(response)
}

pub async fn update_comment(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
    input: &UpdateCommentInput,
) -> Result<CommentResponse> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    if input.body.trim().is_empty() {
        return Err(ApiError::validation(
            "IssueComment",
            "body",
            "missing_field",
        ));
    }

    let repo_row = repo::get_repo(ctx, owner, repo_name).await?;
    let row = fetch_comment_row(ctx, repo_row.id, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    if actor.id != row.user_id && row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden(
            "You are not allowed to update this comment",
        ));
    }

    ctx.db
        .execute(
            "UPDATE comments SET body = ?1, updated_at = datetime('now') WHERE id = ?2 AND deleted_at IS NULL",
            &[DbValue::Text(input.body.clone()), DbValue::Integer(comment_id)],
        )
        .await?;

    if let Some(cache) = ctx.comment_cache {
        cache.invalidate_comment(comment_id).await?;
        cache.invalidate_issue(row.issue_id).await?;
    }

    get_comment(ctx, owner, repo_name, comment_id).await
}

pub async fn delete_comment(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
) -> Result<()> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let repo_row = repo::get_repo(ctx, owner, repo_name).await?;
    let row = fetch_comment_row(ctx, repo_row.id, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    if actor.id != row.user_id && row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden(
            "You are not allowed to delete this comment",
        ));
    }

    ctx.db
        .execute(
            "UPDATE comments SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL",
            &[DbValue::Integer(comment_id)],
        )
        .await?;

    ctx.db
        .execute(
            "UPDATE issues SET comment_count = CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END, updated_at = datetime('now') \
             WHERE id = (SELECT issue_id FROM comments WHERE id = ?1)",
            &[DbValue::Integer(comment_id)],
        )
        .await?;

    if let Some(cache) = ctx.comment_cache {
        cache.invalidate_comment(comment_id).await?;
        cache.invalidate_issue(row.issue_id).await?;
    }

    Ok(())
}

async fn resolve_issue(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<IssueRow> {
    let repo_row = repo::get_repo(ctx, owner, repo_name).await?;

    db::query_opt::<IssueRow>(
        ctx.db,
        "SELECT i.id AS issue_id, i.repo_id AS repo_id \
         FROM issues i \
         WHERE i.repo_id = ?1 AND i.number = ?2 AND i.deleted_at IS NULL",
        &[
            DbValue::Integer(repo_row.id),
            DbValue::Integer(number),
        ],
    )
    .await?
    .ok_or_else(|| ApiError::not_found("Issue"))
}

async fn fetch_comment_row(
    ctx: &AppContext<'_>,
    repo_id: i64,
    comment_id: i64,
) -> Result<Option<CommentRow>> {
    db::query_opt::<CommentRow>(
        ctx.db,
            "SELECT \
                c.id, c.issue_id, c.body, c.user_id, c.created_at, c.updated_at, c.reactions, \
                u.login, u.avatar_url, u.type AS user_type, u.site_admin, \
                i.number AS issue_number, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id \
             FROM comments c \
             JOIN users u ON u.id = c.user_id \
             JOIN issues i ON i.id = c.issue_id \
             JOIN repos r ON r.id = c.repo_id \
             WHERE c.id = ?1 AND c.repo_id = ?2 AND c.deleted_at IS NULL",
            &[
                DbValue::Integer(comment_id),
                DbValue::Integer(repo_id),
            ],
        )
    .await
}

fn to_response(ctx: &AppContext<'_>, row: &CommentRow) -> CommentResponse {
    let user = GitHubUser {
        id: row.user_id,
        login: row.login.clone(),
        email: String::new(),
        avatar_url: row.avatar_url.clone(),
        r#type: row.user_type.clone(),
        site_admin: row.site_admin != 0,
    };
    let issue_url = format!(
        "{}/repos/{}/{}/issues/{}",
        ctx.base_url, row.repo_owner, row.repo_name, row.issue_number
    );
    let body_html = render_markdown(&row.body);

    CommentResponse {
        id: row.id,
        node_id: comment_fmt::comment_node_id(row.id),
        body: Some(row.body.clone()),
        body_html: Some(body_html),
        user: crate::fmt::user::to_api_user(&user),
        created_at: to_iso8601(&row.created_at),
        updated_at: to_iso8601(&row.updated_at),
        html_url: format!("{}#comment-{}", issue_url, row.id),
        issue_url,
        author_association: if row.admin_user_id == Some(row.user_id) {
            "OWNER".to_string()
        } else {
            "NONE".to_string()
        },
        reactions: comment_fmt::to_reactions(
            ctx.base_url,
            &row.repo_owner,
            &row.repo_name,
            row.id,
            &row.reactions,
        ),
    }
}

fn to_iso8601(value: &str) -> String {
    if value.contains('T') && value.ends_with('Z') {
        return value.to_string();
    }
    value.replace(' ', "T") + "Z"
}

fn is_comment_in_repo(comment: &CommentResponse, owner: &str, repo: &str) -> bool {
    let marker = format!("/repos/{}/{}/issues/", owner, repo);
    comment.issue_url.contains(&marker)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;

    use super::{
        create_comment, delete_comment, get_comment, list_comments, to_iso8601, update_comment,
    };
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::Database,
        error::ApiError,
        platform::server::cache::CommentCache,
        types::{
            CreateCommentInput, GitHubApiUser, GitHubUser, ListCommentsQuery, UpdateCommentInput,
        },
        AppContext,
    };

    struct NoopHttp;

    #[async_trait]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            Ok(UpstreamResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::new(),
            })
        }
    }

    async fn make_db() -> (
        tempfile::TempPath,
        crate::platform::server::sqlite::SqliteDatabase,
    ) {
        let db_file = tempfile::NamedTempFile::new()
            .expect("temp file")
            .into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    async fn seed(db: &dyn Database) {
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES \
             (2, 'alice', 'alice@test.com', 'https://avatars/a', 'User', 0, datetime('now'))",
            &[],
        )
        .await
        .expect("insert user");
        db.execute(
            "INSERT INTO repos (id, owner, name, admin_user_id, issue_counter, created_at) VALUES \
             (10, 'o', 'r', 2, 1, datetime('now'))",
            &[],
        )
        .await
        .expect("insert repo");
        db.execute(
            "INSERT INTO issues (id, repo_id, number, title, body, state, locked, user_id, comment_count, created_at, updated_at) VALUES \
             (20, 10, 1, 't', 'b', 'open', 0, 2, 1, datetime('now'), datetime('now'))",
            &[],
        )
        .await
        .expect("insert issue");
        db.execute(
            "INSERT INTO comments (id, repo_id, issue_id, body, user_id, created_at, updated_at, reactions) VALUES \
             (30, 10, 20, 'seed', 2, datetime('now'), datetime('now'), '{}')",
            &[],
        )
        .await
        .expect("insert comment");
    }

    fn ctx<'a>(
        db: &'a dyn Database,
        http: &'a dyn HttpClient,
        cache: Option<&'a CommentCache>,
        user: Option<&'a GitHubUser>,
    ) -> AppContext<'a> {
        AppContext {
            db,
            http,
            comment_cache: cache.map(|v| v as &dyn crate::cache::CommentCacheStore),
            base_url: "http://localhost",
            user,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: false,
            test_bypass_secret: None,
        }
    }

    #[test]
    fn to_iso8601_keeps_existing_iso() {
        assert_eq!(to_iso8601("2025-01-01T00:00:00Z"), "2025-01-01T00:00:00Z");
    }

    #[tokio::test]
    async fn cache_and_invalidation_paths_are_exercised() {
        let (_db_file, db) = make_db().await;
        seed(&db).await;
        let http = NoopHttp;
        let cache = CommentCache::new(128, 60);
        let alice = GitHubUser {
            id: 2,
            login: "alice".to_string(),
            email: "alice@test.com".to_string(),
            avatar_url: "https://avatars/a".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let anon_ctx = ctx(&db, &http, Some(&cache), None);
        let query = ListCommentsQuery {
            per_page: Some(10),
            page: Some(1),
            since: None,
        };
        let (first_items, _, _, _) = list_comments(&anon_ctx, "o", "r", 1, &query)
            .await
            .expect("first list");
        assert_eq!(first_items.len(), 1);
        let (second_items, _, _, _) = list_comments(&anon_ctx, "o", "r", 1, &query)
            .await
            .expect("second list from cache");
        assert_eq!(second_items.len(), 1);

        let by_db = get_comment(&anon_ctx, "o", "r", 30)
            .await
            .expect("first get from db and set cache");
        assert_eq!(by_db.author_association, "OWNER");
        let by_cache = get_comment(&anon_ctx, "o", "r", 30)
            .await
            .expect("second get from cache");
        assert_eq!(by_cache.id, 30);

        let user_ctx = ctx(&db, &http, Some(&cache), Some(&alice));
        let created = create_comment(
            &user_ctx,
            "o",
            "r",
            1,
            &CreateCommentInput {
                body: "new comment".to_string(),
            },
        )
        .await
        .expect("create comment");
        let updated = update_comment(
            &user_ctx,
            "o",
            "r",
            created.id,
            &UpdateCommentInput {
                body: "edited".to_string(),
            },
        )
        .await
        .expect("update comment");
        assert_eq!(updated.body.as_deref(), Some("edited"));

        delete_comment(&user_ctx, "o", "r", created.id)
            .await
            .expect("delete comment");
    }
}
