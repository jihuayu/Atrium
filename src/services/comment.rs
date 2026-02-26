use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::comment as comment_fmt,
    markdown::render_markdown,
    services::{normalize_pagination, repo},
    types::{CommentResponse, CreateCommentInput, GitHubUser, ListCommentsQuery, UpdateCommentInput},
    AppContext, Result,
};

#[derive(Debug, Deserialize, Clone)]
struct CommentRow {
    id: i64,
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

    let mut filters = vec!["c.issue_id = ?1".to_string(), "c.deleted_at IS NULL".to_string()];
    let mut params = vec![DbValue::Integer(issue.issue_id)];
    let mut idx = 2;
    if let Some(since) = &query.since {
        filters.push(format!("c.updated_at >= ?{}", idx));
        params.push(DbValue::Text(since.clone()));
        idx += 1;
    }

    let where_sql = filters.join(" AND ");
    let count_sql = format!("SELECT COUNT(*) AS total FROM comments c WHERE {}", where_sql);
    let total = db::query_opt::<CountRow>(ctx.db, &count_sql, &params)
        .await?
        .map(|v| v.total)
        .unwrap_or(0);

    let mut list_params = params.clone();
    list_params.push(DbValue::Integer(per_page));
    list_params.push(DbValue::Integer(offset));

    let list_sql = format!(
        "SELECT \
            c.id, c.body, c.user_id, c.created_at, c.updated_at, c.reactions, \
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
        return Err(ApiError::validation("IssueComment", "body", "missing_field"));
    }

    let issue = resolve_issue(ctx, owner, repo_name, number).await?;

    ctx.db
        .execute(
            "INSERT INTO comments (repo_id, issue_id, body, user_id, created_at, updated_at, reactions) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'), '{}')",
            &[
                DbValue::Integer(issue.repo_id),
                DbValue::Integer(issue.issue_id),
                DbValue::Text(input.body.clone()),
                DbValue::Integer(user.id),
            ],
        )
        .await?;

    ctx.db
        .execute(
            "UPDATE issues SET comment_count = comment_count + 1, updated_at = datetime('now') WHERE id = ?1",
            &[DbValue::Integer(issue.issue_id)],
        )
        .await?;

    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    let comment_id = db::query_opt::<IdRow>(
        ctx.db,
            "SELECT id FROM comments WHERE issue_id = ?1 AND user_id = ?2 ORDER BY id DESC LIMIT 1",
            &[DbValue::Integer(issue.issue_id), DbValue::Integer(user.id)],
        )
    .await?
        .ok_or_else(|| ApiError::internal("comment insert verification failed"))?
        .id;

    get_comment(ctx, owner, repo_name, comment_id).await
}

pub async fn get_comment(ctx: &AppContext<'_>, owner: &str, repo_name: &str, comment_id: i64) -> Result<CommentResponse> {
    let _repo = repo::ensure_repo(ctx, owner, repo_name, ctx.user).await?;

    let row = fetch_comment_row(ctx, owner, repo_name, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    Ok(to_response(ctx, &row))
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
        return Err(ApiError::validation("IssueComment", "body", "missing_field"));
    }

    let row = fetch_comment_row(ctx, owner, repo_name, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    if actor.id != row.user_id && row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden("You are not allowed to update this comment"));
    }

    ctx.db
        .execute(
            "UPDATE comments SET body = ?1, updated_at = datetime('now') WHERE id = ?2 AND deleted_at IS NULL",
            &[DbValue::Text(input.body.clone()), DbValue::Integer(comment_id)],
        )
        .await?;

    get_comment(ctx, owner, repo_name, comment_id).await
}

pub async fn delete_comment(ctx: &AppContext<'_>, owner: &str, repo_name: &str, comment_id: i64) -> Result<()> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let row = fetch_comment_row(ctx, owner, repo_name, comment_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    if actor.id != row.user_id && row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden("You are not allowed to delete this comment"));
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

    Ok(())
}

async fn resolve_issue(ctx: &AppContext<'_>, owner: &str, repo_name: &str, number: i64) -> Result<IssueRow> {
    let _repo = repo::ensure_repo(ctx, owner, repo_name, ctx.user).await?;

    db::query_opt::<IssueRow>(
        ctx.db,
            "SELECT i.id AS issue_id, i.number AS issue_number, i.repo_id AS repo_id \
             FROM issues i \
             JOIN repos r ON r.id = i.repo_id \
             WHERE r.owner = ?1 AND r.name = ?2 AND i.number = ?3 AND i.deleted_at IS NULL",
            &[
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
                DbValue::Integer(number),
            ],
        )
    .await?
        .ok_or_else(|| ApiError::not_found("Issue"))
}

async fn fetch_comment_row(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
) -> Result<Option<CommentRow>> {
    db::query_opt::<CommentRow>(
        ctx.db,
            "SELECT \
                c.id, c.body, c.user_id, c.created_at, c.updated_at, c.reactions, \
                u.login, u.avatar_url, u.type AS user_type, u.site_admin, \
                i.number AS issue_number, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id \
             FROM comments c \
             JOIN users u ON u.id = c.user_id \
             JOIN issues i ON i.id = c.issue_id \
             JOIN repos r ON r.id = c.repo_id \
             WHERE c.id = ?1 AND r.owner = ?2 AND r.name = ?3 AND c.deleted_at IS NULL",
            &[
                DbValue::Integer(comment_id),
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
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
        reactions: comment_fmt::to_reactions(ctx.base_url, &row.repo_owner, &row.repo_name, row.id, &row.reactions),
    }
}

fn to_iso8601(value: &str) -> String {
    if value.contains('T') && value.ends_with('Z') {
        return value.to_string();
    }
    value.replace(' ', "T") + "Z"
}
