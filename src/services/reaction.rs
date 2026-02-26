use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::comment::ReactionCounts,
    services::normalize_pagination,
    types::{CreateReactionInput, ReactionResponse},
    AppContext, Result,
};

const ALLOWED_REACTIONS: [&str; 8] = ["+1", "-1", "laugh", "confused", "heart", "hooray", "rocket", "eyes"];

#[derive(Debug, Deserialize)]
struct ReactionRow {
    id: i64,
    content: String,
    user_id: i64,
    created_at: String,
    login: String,
    avatar_url: String,
    user_type: String,
}

#[derive(Debug, Deserialize)]
struct CommentRow {
    reactions: String,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    total: i64,
}

pub async fn list_reactions(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<(Vec<ReactionResponse>, i64, i64, i64)> {
    ensure_comment(ctx, owner, repo_name, comment_id).await?;
    let (page, per_page, offset) = normalize_pagination(page, per_page);

    let total = db::query_opt::<CountRow>(
        ctx.db,
            "SELECT COUNT(*) AS total \
             FROM reactions r \
             JOIN comments c ON c.id = r.comment_id \
             JOIN repos rp ON rp.id = c.repo_id \
             WHERE c.id = ?1 AND rp.owner = ?2 AND rp.name = ?3",
            &[
                DbValue::Integer(comment_id),
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
            ],
        )
    .await?
        .map(|v| v.total)
        .unwrap_or(0);

    let rows = db::query_all::<ReactionRow>(
        ctx.db,
            "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type \
             FROM reactions r \
             JOIN users u ON u.id = r.user_id \
             JOIN comments c ON c.id = r.comment_id \
             JOIN repos rp ON rp.id = c.repo_id \
             WHERE c.id = ?1 AND rp.owner = ?2 AND rp.name = ?3 \
             ORDER BY r.id ASC \
             LIMIT ?4 OFFSET ?5",
            &[
                DbValue::Integer(comment_id),
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
                DbValue::Integer(per_page),
                DbValue::Integer(offset),
            ],
        )
    .await?;

    let items = rows
        .into_iter()
        .map(|row| ReactionResponse {
            id: row.id,
            content: row.content,
            user: crate::types::ApiUser {
                login: row.login.clone(),
                id: row.user_id,
                avatar_url: row.avatar_url.clone(),
                html_url: format!("https://github.com/{}", row.login),
                r#type: row.user_type,
            },
            created_at: to_iso8601(&row.created_at),
        })
        .collect();

    Ok((items, total, page, per_page))
}

pub async fn create_reaction(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
    input: &CreateReactionInput,
) -> Result<(ReactionResponse, bool)> {
    let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
    ensure_content(&input.content)?;
    ensure_comment(ctx, owner, repo_name, comment_id).await?;

    let affected = ctx
        .db
        .execute(
            "INSERT INTO reactions (comment_id, user_id, content, created_at) \
             VALUES (?1, ?2, ?3, datetime('now')) \
             ON CONFLICT(comment_id, user_id, content) DO NOTHING",
            &[
                DbValue::Integer(comment_id),
                DbValue::Integer(user.id),
                DbValue::Text(input.content.clone()),
            ],
        )
        .await?;

    let row = db::query_opt::<ReactionRow>(
        ctx.db,
            "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type \
             FROM reactions r \
             JOIN users u ON u.id = r.user_id \
             WHERE r.comment_id = ?1 AND r.user_id = ?2 AND r.content = ?3",
            &[
                DbValue::Integer(comment_id),
                DbValue::Integer(user.id),
                DbValue::Text(input.content.clone()),
            ],
        )
    .await?
        .ok_or_else(|| ApiError::internal("reaction create failed"))?;

    if affected > 0 {
        update_cached_reactions(ctx, comment_id, &input.content, 1).await?;
    }

    let response = ReactionResponse {
        id: row.id,
        content: row.content,
        user: crate::types::ApiUser {
            login: row.login.clone(),
            id: row.user_id,
            avatar_url: row.avatar_url,
            html_url: format!("https://github.com/{}", row.login),
            r#type: row.user_type,
        },
        created_at: to_iso8601(&row.created_at),
    };

    Ok((response, affected > 0))
}

pub async fn delete_reaction(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
    reaction_id: i64,
) -> Result<()> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;

    let row = db::query_opt::<ReactionRow>(
        ctx.db,
            "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type \
             FROM reactions r \
             JOIN users u ON u.id = r.user_id \
             JOIN comments c ON c.id = r.comment_id \
             JOIN repos rp ON rp.id = c.repo_id \
             WHERE r.id = ?1 AND r.comment_id = ?2 AND rp.owner = ?3 AND rp.name = ?4",
            &[
                DbValue::Integer(reaction_id),
                DbValue::Integer(comment_id),
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
            ],
        )
    .await?
        .ok_or_else(|| ApiError::not_found("Reaction"))?;

    if row.user_id != actor.id {
        return Err(ApiError::forbidden("You can only delete your own reaction"));
    }

    let affected = ctx
        .db
        .execute(
            "DELETE FROM reactions WHERE id = ?1 AND comment_id = ?2",
            &[DbValue::Integer(reaction_id), DbValue::Integer(comment_id)],
        )
        .await?;

    if affected > 0 {
        update_cached_reactions(ctx, comment_id, &row.content, -1).await?;
    }

    Ok(())
}

async fn ensure_comment(ctx: &AppContext<'_>, owner: &str, repo_name: &str, comment_id: i64) -> Result<CommentRow> {
    db::query_opt::<CommentRow>(
        ctx.db,
            "SELECT c.id, c.reactions \
             FROM comments c \
             JOIN repos r ON r.id = c.repo_id \
             WHERE c.id = ?1 AND r.owner = ?2 AND r.name = ?3 AND c.deleted_at IS NULL",
            &[
                DbValue::Integer(comment_id),
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo_name.to_string()),
            ],
        )
    .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))
}

async fn update_cached_reactions(ctx: &AppContext<'_>, comment_id: i64, content: &str, delta: i64) -> Result<()> {
    let row = db::query_opt::<CommentRow>(
        ctx.db,
            "SELECT id, reactions FROM comments WHERE id = ?1",
            &[DbValue::Integer(comment_id)],
        )
    .await?
        .ok_or_else(|| ApiError::not_found("IssueComment"))?;

    let mut counts: ReactionCounts = serde_json::from_str(&row.reactions).unwrap_or_default();
    counts.apply_delta(content, delta);

    ctx.db
        .execute(
            "UPDATE comments SET reactions = ?1, updated_at = datetime('now') WHERE id = ?2",
            &[
                DbValue::Text(serde_json::to_string(&counts)?),
                DbValue::Integer(comment_id),
            ],
        )
        .await?;

    Ok(())
}

fn ensure_content(content: &str) -> Result<()> {
    if ALLOWED_REACTIONS.contains(&content) {
        Ok(())
    } else {
        Err(ApiError::validation("Reaction", "content", "invalid"))
    }
}

fn to_iso8601(value: &str) -> String {
    if value.contains('T') && value.ends_with('Z') {
        return value.to_string();
    }
    value.replace(' ', "T") + "Z"
}
