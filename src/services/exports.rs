use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    types::{
        ExportCommentRow, ExportIssueLabelRow, ExportIssueRow, ExportLabelRow, ExportReactionRow,
        ExportRepoRow, ExportUserRow, RepoExportResponse,
    },
    AppContext, Result,
};

const REPO_SCOPE_SQL: &str = "SELECT id FROM repos WHERE owner = ?1 OR admin_user_id = ?2";

#[derive(Debug, Deserialize)]
struct ExportedAtRow {
    exported_at: String,
}

pub async fn export_user_repos(ctx: &AppContext<'_>) -> Result<RepoExportResponse> {
    let user = ctx.user.cloned().ok_or_else(ApiError::unauthorized)?;
    let params = [DbValue::Text(user.login.clone()), DbValue::Integer(user.id)];

    let exported_at = db::query_opt::<ExportedAtRow>(
        ctx.db,
        "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now') AS exported_at",
        &[],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to generate export timestamp"))?
    .exported_at;

    let repos = db::query_all::<ExportRepoRow>(
        ctx.db,
        "SELECT id, owner, name, admin_user_id, issue_counter, created_at \
         FROM repos \
         WHERE owner = ?1 OR admin_user_id = ?2 \
         ORDER BY id ASC",
        &params,
    )
    .await?;

    let issues_sql = format!(
        "SELECT id, repo_id, number, title, body, state, state_reason, locked, user_id, \
                comment_count, created_at, updated_at, closed_at, deleted_at \
         FROM issues \
         WHERE repo_id IN ({}) \
         ORDER BY id ASC",
        REPO_SCOPE_SQL
    );
    let issues = db::query_all::<ExportIssueRow>(ctx.db, &issues_sql, &params).await?;

    let comments_sql = format!(
        "SELECT id, repo_id, issue_id, body, user_id, created_at, updated_at, deleted_at, reactions \
         FROM comments \
         WHERE repo_id IN ({}) \
         ORDER BY id ASC",
        REPO_SCOPE_SQL
    );
    let comments = db::query_all::<ExportCommentRow>(ctx.db, &comments_sql, &params).await?;

    let labels_sql = format!(
        "SELECT id, repo_id, name, description, color \
         FROM labels \
         WHERE repo_id IN ({}) \
         ORDER BY id ASC",
        REPO_SCOPE_SQL
    );
    let labels = db::query_all::<ExportLabelRow>(ctx.db, &labels_sql, &params).await?;

    let issue_labels_sql = format!(
        "SELECT il.issue_id, il.label_id \
         FROM issue_labels il \
         JOIN issues i ON i.id = il.issue_id \
         WHERE i.repo_id IN ({}) \
         ORDER BY il.issue_id ASC, il.label_id ASC",
        REPO_SCOPE_SQL
    );
    let issue_labels =
        db::query_all::<ExportIssueLabelRow>(ctx.db, &issue_labels_sql, &params).await?;

    let reactions_sql = format!(
        "SELECT r.id, r.comment_id, r.user_id, r.content, r.created_at \
         FROM reactions r \
         JOIN comments c ON c.id = r.comment_id \
         WHERE c.repo_id IN ({}) \
         ORDER BY r.id ASC",
        REPO_SCOPE_SQL
    );
    let reactions = db::query_all::<ExportReactionRow>(ctx.db, &reactions_sql, &params).await?;

    let users_sql = format!(
        "SELECT DISTINCT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin, u.cached_at \
         FROM users u \
         WHERE u.id IN ( \
             SELECT admin_user_id FROM repos WHERE (owner = ?1 OR admin_user_id = ?2) AND admin_user_id IS NOT NULL \
             UNION \
             SELECT i.user_id FROM issues i WHERE i.repo_id IN ({0}) \
             UNION \
             SELECT c.user_id FROM comments c WHERE c.repo_id IN ({0}) \
             UNION \
             SELECT r.user_id \
             FROM reactions r \
             JOIN comments c ON c.id = r.comment_id \
             WHERE c.repo_id IN ({0}) \
         ) \
         ORDER BY u.id ASC",
        REPO_SCOPE_SQL
    );
    let users = db::query_all::<ExportUserRow>(ctx.db, &users_sql, &params).await?;

    Ok(RepoExportResponse {
        schema_version: 1,
        exported_at,
        user,
        repos,
        issues,
        comments,
        labels,
        issue_labels,
        reactions,
        users,
    })
}
