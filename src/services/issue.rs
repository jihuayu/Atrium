use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::{comment::ReactionCounts, issue as issue_fmt},
    markdown::render_markdown,
    services::{label, normalize_pagination, repo},
    types::{
        CreateIssueInput, GitHubUser, IssueResponse, Label, ListIssuesQuery, RepoRow,
        UpdateIssueInput,
    },
    AppContext, Result,
};

#[derive(Debug, Deserialize, Clone)]
struct IssueRow {
    id: i64,
    number: i64,
    title: String,
    body: Option<String>,
    state: String,
    locked: i64,
    user_id: i64,
    comment_count: i64,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    login: String,
    avatar_url: String,
    user_type: String,
    site_admin: i64,
    repo_id: i64,
    repo_owner: String,
    repo_name: String,
    admin_user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    total: i64,
}

pub async fn create_issue(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    input: &CreateIssueInput,
) -> Result<IssueResponse> {
    let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
    if input.title.trim().is_empty() {
        return Err(ApiError::validation("Issue", "title", "missing_field"));
    }

    let repo = repo::ensure_repo(ctx, owner, repo_name, Some(user)).await?;

    #[derive(Debug, Deserialize)]
    struct CounterRow {
        issue_counter: i64,
    }

    let counter = db::query_opt::<CounterRow>(
        ctx.db,
        "UPDATE repos SET issue_counter = issue_counter + 1 WHERE id = ?1 RETURNING issue_counter",
        &[DbValue::Integer(repo.id)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to allocate issue number"))?;

    let issue_number = counter.issue_counter;

    ctx.db
        .execute(
            "INSERT INTO issues (repo_id, number, title, body, state, locked, user_id, comment_count, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'open', 0, ?5, 0, datetime('now'), datetime('now'))",
            &[
                DbValue::Integer(repo.id),
                DbValue::Integer(issue_number),
                DbValue::Text(input.title.trim().to_string()),
                input
                    .body
                    .as_ref()
                    .map(|v| DbValue::Text(v.clone()))
                    .unwrap_or(DbValue::Null),
                DbValue::Integer(user.id),
            ],
        )
        .await?;

    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    let issue_id = db::query_opt::<IdRow>(
        ctx.db,
        "SELECT id FROM issues WHERE repo_id = ?1 AND number = ?2",
        &[DbValue::Integer(repo.id), DbValue::Integer(issue_number)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("issue insert verification failed"))?
    .id;

    if let Some(names) = &input.labels {
        set_issue_labels(ctx, repo.id, issue_id, names).await?;
    }

    get_issue(ctx, owner, repo_name, issue_number).await
}

pub async fn get_issue(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<IssueResponse> {
    let _repo = repo::ensure_repo(ctx, owner, repo_name, ctx.user).await?;
    let row = fetch_issue_row(ctx, owner, repo_name, number)
        .await?
        .ok_or_else(|| ApiError::not_found("Issue"))?;
    build_issue_response(ctx, &row).await
}

pub async fn list_issues(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    query: &ListIssuesQuery,
) -> Result<(Vec<IssueResponse>, i64, i64, i64)> {
    let repo = repo::ensure_repo(ctx, owner, repo_name, ctx.user).await?;
    let (page, per_page, offset) = normalize_pagination(query.page, query.per_page);

    let mut filters = vec![
        "i.repo_id = ?1".to_string(),
        "i.deleted_at IS NULL".to_string(),
    ];
    let mut params = vec![DbValue::Integer(repo.id)];
    let mut idx = 2;

    let state = query.state.clone().unwrap_or_else(|| "open".to_string());
    if state != "all" {
        filters.push(format!("i.state = ?{}", idx));
        params.push(DbValue::Text(state));
        idx += 1;
    }

    if let Some(creator) = &query.creator {
        filters.push(format!("u.login = ?{}", idx));
        params.push(DbValue::Text(creator.clone()));
        idx += 1;
    }

    if let Some(since) = &query.since {
        filters.push(format!("i.updated_at >= ?{}", idx));
        params.push(DbValue::Text(since.clone()));
        idx += 1;
    }

    if let Some(labels) = &query.labels {
        for label_name in labels
            .split(',')
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            filters.push(format!(
                "EXISTS (SELECT 1 FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id = i.id AND l.name = ?{})",
                idx
            ));
            params.push(DbValue::Text(label_name.to_string()));
            idx += 1;
        }
    }

    let where_sql = filters.join(" AND ");

    let count_sql = format!(
        "SELECT COUNT(*) AS total FROM issues i JOIN users u ON u.id = i.user_id WHERE {}",
        where_sql
    );
    let total = db::query_opt::<CountRow>(ctx.db, &count_sql, &params)
        .await?
        .map(|v| v.total)
        .unwrap_or(0);

    let sort_col = match query.sort.as_deref().unwrap_or("created") {
        "updated" => "i.updated_at",
        "comments" => "i.comment_count",
        _ => "i.created_at",
    };
    let direction = match query.direction.as_deref().unwrap_or("desc") {
        "asc" => "ASC",
        _ => "DESC",
    };

    let mut list_params = params.clone();
    list_params.push(DbValue::Integer(per_page));
    list_params.push(DbValue::Integer(offset));

    let list_sql = format!(
        "SELECT \
            i.id, i.number, i.title, i.body, i.state, i.locked, i.user_id, i.comment_count, i.created_at, i.updated_at, i.closed_at, \
            u.login, u.avatar_url, u.type AS user_type, u.site_admin, \
            r.id AS repo_id, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id \
         FROM issues i \
         JOIN users u ON u.id = i.user_id \
         JOIN repos r ON r.id = i.repo_id \
         WHERE {} \
         ORDER BY {} {} \
         LIMIT ?{} OFFSET ?{}",
        where_sql,
        sort_col,
        direction,
        idx,
        idx + 1
    );

    let rows = db::query_all::<IssueRow>(ctx.db, &list_sql, &list_params).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(build_issue_response(ctx, &row).await?);
    }

    Ok((out, total, page, per_page))
}

pub async fn update_issue(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
    input: &UpdateIssueInput,
) -> Result<IssueResponse> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let row = fetch_issue_row(ctx, owner, repo_name, number)
        .await?
        .ok_or_else(|| ApiError::not_found("Issue"))?;

    if actor.id != row.user_id && row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden(
            "You are not allowed to update this issue",
        ));
    }

    let mut sets = Vec::new();
    let mut params = Vec::new();
    let mut idx = 1;

    if let Some(title) = &input.title {
        if title.trim().is_empty() {
            return Err(ApiError::validation("Issue", "title", "missing_field"));
        }
        sets.push(format!("title = ?{}", idx));
        params.push(DbValue::Text(title.trim().to_string()));
        idx += 1;
    }

    if let Some(body) = &input.body {
        sets.push(format!("body = ?{}", idx));
        params.push(DbValue::Text(body.clone()));
        idx += 1;
    }

    if let Some(state) = &input.state {
        if state != "open" && state != "closed" {
            return Err(ApiError::validation("Issue", "state", "invalid"));
        }
        sets.push(format!("state = ?{}", idx));
        params.push(DbValue::Text(state.clone()));
        idx += 1;

        if state == "closed" {
            sets.push("closed_at = datetime('now')".to_string());
        } else {
            sets.push("closed_at = NULL".to_string());
        }
    }

    if let Some(state_reason) = &input.state_reason {
        sets.push(format!("state_reason = ?{}", idx));
        params.push(DbValue::Text(state_reason.clone()));
        idx += 1;
    }

    if !sets.is_empty() {
        sets.push("updated_at = datetime('now')".to_string());
        let sql = format!("UPDATE issues SET {} WHERE id = ?{}", sets.join(", "), idx);
        params.push(DbValue::Integer(row.id));
        ctx.db.execute(&sql, &params).await?;
    }

    if let Some(names) = &input.labels {
        set_issue_labels(ctx, row.repo_id, row.id, names).await?;
    }

    get_issue(ctx, owner, repo_name, number).await
}

pub async fn set_issue_labels(
    ctx: &AppContext<'_>,
    repo_id: i64,
    issue_id: i64,
    names: &[String],
) -> Result<()> {
    ctx.db
        .execute(
            "DELETE FROM issue_labels WHERE issue_id = ?1",
            &[DbValue::Integer(issue_id)],
        )
        .await?;

    let label_ids = label::ensure_label_ids(ctx, repo_id, names).await?;
    for label_id in label_ids {
        ctx.db
            .execute(
                "INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)",
                &[DbValue::Integer(issue_id), DbValue::Integer(label_id)],
            )
            .await?;
    }

    Ok(())
}

async fn fetch_issue_row(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<Option<IssueRow>> {
    db::query_opt::<IssueRow>(
        ctx.db,
            "SELECT \
                i.id, i.number, i.title, i.body, i.state, i.locked, i.user_id, i.comment_count, i.created_at, i.updated_at, i.closed_at, \
                u.login, u.avatar_url, u.type AS user_type, u.site_admin, \
                r.id AS repo_id, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id \
             FROM issues i \
             JOIN users u ON u.id = i.user_id \
             JOIN repos r ON r.id = i.repo_id \
             WHERE r.owner = ?1 AND r.name = ?2 AND i.number = ?3 AND i.deleted_at IS NULL",
            &[
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
                DbValue::Integer(number),
            ],
        )
    .await
}

async fn issue_labels(ctx: &AppContext<'_>, issue_id: i64) -> Result<Vec<Label>> {
    #[derive(Debug, Deserialize)]
    struct Row {
        id: i64,
        name: String,
        color: String,
        description: String,
    }

    let rows = db::query_all::<Row>(
        ctx.db,
        "SELECT l.id, l.name, l.color, l.description \
             FROM labels l \
             JOIN issue_labels il ON il.label_id = l.id \
             WHERE il.issue_id = ?1 \
             ORDER BY l.name ASC",
        &[DbValue::Integer(issue_id)],
    )
    .await?;

    Ok(rows
        .into_iter()
        .map(|v| Label {
            id: v.id,
            name: v.name,
            color: v.color,
            description: v.description,
        })
        .collect())
}

async fn build_issue_response(ctx: &AppContext<'_>, row: &IssueRow) -> Result<IssueResponse> {
    let labels = issue_labels(ctx, row.id).await?;
    let user = GitHubUser {
        id: row.user_id,
        login: row.login.clone(),
        email: String::new(),
        avatar_url: row.avatar_url.clone(),
        r#type: row.user_type.clone(),
        site_admin: row.site_admin != 0,
    };
    let repo = RepoRow {
        id: row.repo_id,
        owner: row.repo_owner.clone(),
        name: row.repo_name.clone(),
        admin_user_id: row.admin_user_id,
        issue_counter: 0,
    };

    let body = row.body.clone().unwrap_or_default();
    let _counts: ReactionCounts = ReactionCounts::default();

    Ok(IssueResponse {
        id: row.id,
        node_id: issue_fmt::issue_node_id(row.id),
        number: row.number,
        title: row.title.clone(),
        body: Some(body.clone()),
        body_html: Some(render_markdown(&body)),
        state: row.state.clone(),
        locked: row.locked != 0,
        user: crate::fmt::user::to_api_user(&user),
        labels,
        comments: row.comment_count,
        created_at: to_iso8601(&row.created_at),
        updated_at: to_iso8601(&row.updated_at),
        closed_at: row.closed_at.clone().map(|v| to_iso8601(&v)),
        author_association: issue_fmt::author_association(&repo, row.user_id),
        reactions: issue_fmt::issue_reactions(
            ctx.base_url,
            &row.repo_owner,
            &row.repo_name,
            row.number,
        ),
        url: format!(
            "{}/repos/{}/{}/issues/{}",
            ctx.base_url, row.repo_owner, row.repo_name, row.number
        ),
        html_url: format!(
            "{}/repos/{}/{}/issues/{}",
            ctx.base_url, row.repo_owner, row.repo_name, row.number
        ),
        comments_url: format!(
            "{}/repos/{}/{}/issues/{}/comments",
            ctx.base_url, row.repo_owner, row.repo_name, row.number
        ),
    })
}

fn to_iso8601(value: &str) -> String {
    if value.contains('T') && value.ends_with('Z') {
        return value.to_string();
    }
    value.replace(' ', "T") + "Z"
}
