use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::comment::ReactionCounts,
    services::normalize_pagination,
    types::{CreateReactionInput, ReactionResponse},
    AppContext, Result,
};

const ALLOWED_REACTIONS: [&str; 8] = [
    "+1", "-1", "laugh", "confused", "heart", "hooray", "rocket", "eyes",
];

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
    issue_id: i64,
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
    let comment = ensure_comment(ctx, owner, repo_name, comment_id).await?;

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
        if let Some(cache) = ctx.comment_cache {
            cache.invalidate_issue(comment.issue_id).await?;
            cache.invalidate_comment(comment_id).await?;
        }
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
    let comment = ensure_comment(ctx, owner, repo_name, comment_id).await?;

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
        if let Some(cache) = ctx.comment_cache {
            cache.invalidate_issue(comment.issue_id).await?;
            cache.invalidate_comment(comment_id).await?;
        }
    }

    Ok(())
}

async fn ensure_comment(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    comment_id: i64,
) -> Result<CommentRow> {
    db::query_opt::<CommentRow>(
        ctx.db,
        "SELECT c.id, c.issue_id, c.reactions \
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

async fn update_cached_reactions(
    ctx: &AppContext<'_>,
    comment_id: i64,
    content: &str,
    delta: i64,
) -> Result<()> {
    let row = db::query_opt::<CommentRow>(
        ctx.db,
        "SELECT issue_id, reactions FROM comments WHERE id = ?1",
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

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;

    use super::{create_reaction, delete_reaction, list_reactions, to_iso8601};
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::{Database, DbValue},
        error::ApiError,
        types::{CreateReactionInput, GitHubApiUser, GitHubUser},
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

    async fn make_db() -> (tempfile::TempPath, crate::platform::server::sqlite::SqliteDatabase) {
        let db_file = tempfile::NamedTempFile::new().expect("temp file").into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    async fn seed_graph(db: &dyn Database) -> (i64, i64, i64) {
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES \
             (1, 'admin', 'admin@test.com', 'https://avatars/admin', 'User', 1, datetime('now')), \
             (2, 'alice', 'alice@test.com', 'https://avatars/alice', 'User', 0, datetime('now')), \
             (3, 'bob', 'bob@test.com', 'https://avatars/bob', 'User', 0, datetime('now'))",
            &[],
        )
        .await
        .expect("insert users");

        db.execute(
            "INSERT INTO repos (id, owner, name, admin_user_id, issue_counter, created_at) VALUES (?1, ?2, ?3, ?4, 1, datetime('now'))",
            &[
                DbValue::Integer(10),
                DbValue::Text("o".to_string()),
                DbValue::Text("r".to_string()),
                DbValue::Integer(1),
            ],
        )
        .await
        .expect("insert repo");

        db.execute(
            "INSERT INTO issues (id, repo_id, number, title, body, state, locked, user_id, comment_count, created_at, updated_at) \
             VALUES (?1, ?2, 1, 't', 'b', 'open', 0, 2, 1, datetime('now'), datetime('now'))",
            &[DbValue::Integer(20), DbValue::Integer(10)],
        )
        .await
        .expect("insert issue");

        db.execute(
            "INSERT INTO comments (id, repo_id, issue_id, body, user_id, created_at, updated_at, reactions) \
             VALUES (?1, ?2, ?3, 'c1', 2, datetime('now'), datetime('now'), '{}')",
            &[
                DbValue::Integer(30),
                DbValue::Integer(10),
                DbValue::Integer(20),
            ],
        )
        .await
        .expect("insert comment");

        (10, 20, 30)
    }

    fn ctx<'a>(
        db: &'a dyn Database,
        http: &'a dyn HttpClient,
        user: Option<&'a GitHubUser>,
    ) -> AppContext<'a> {
        AppContext {
            db,
            http,
            comment_cache: None,
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
    fn to_iso8601_formats_sqlite_and_keeps_iso() {
        assert_eq!(to_iso8601("2025-01-01 00:00:00"), "2025-01-01T00:00:00Z");
        assert_eq!(to_iso8601("2025-01-01T00:00:00Z"), "2025-01-01T00:00:00Z");
    }

    #[tokio::test]
    async fn list_create_delete_and_validation_paths() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let (_repo_id, _issue_id, comment_id) = seed_graph(&db).await;

        let alice = GitHubUser {
            id: 2,
            login: "alice".to_string(),
            email: "alice@test.com".to_string(),
            avatar_url: "https://avatars/alice".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };
        let bob = GitHubUser {
            id: 3,
            login: "bob".to_string(),
            email: "bob@test.com".to_string(),
            avatar_url: "https://avatars/bob".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let no_user_ctx = ctx(&db, &http, None);
        let bob_ctx = ctx(&db, &http, Some(&bob));
        let alice_ctx = ctx(&db, &http, Some(&alice));

        let invalid = create_reaction(
            &bob_ctx,
            "o",
            "r",
            comment_id,
            &CreateReactionInput {
                content: "invalid".to_string(),
            },
        )
        .await
        .expect_err("invalid reaction should fail");
        assert_eq!(invalid.status, 422);

        let (created, inserted) = create_reaction(
            &bob_ctx,
            "o",
            "r",
            comment_id,
            &CreateReactionInput {
                content: "heart".to_string(),
            },
        )
        .await
        .expect("create reaction");
        assert!(inserted);
        assert_eq!(created.content, "heart");

        let (_same, inserted_again) = create_reaction(
            &bob_ctx,
            "o",
            "r",
            comment_id,
            &CreateReactionInput {
                content: "heart".to_string(),
            },
        )
        .await
        .expect("idempotent reaction");
        assert!(!inserted_again);

        let (items, total, page, per_page) =
            list_reactions(&no_user_ctx, "o", "r", comment_id, Some(1), Some(1))
                .await
                .expect("list reactions");
        assert_eq!(total, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(page, 1);
        assert_eq!(per_page, 1);

        let missing = list_reactions(&no_user_ctx, "o", "r", 999999, Some(1), Some(10))
            .await
            .expect_err("missing comment");
        assert_eq!(missing.status, 404);

        let forbidden = delete_reaction(&alice_ctx, "o", "r", comment_id, created.id)
            .await
            .expect_err("cannot delete others reaction");
        assert_eq!(forbidden.status, 403);

        delete_reaction(&bob_ctx, "o", "r", comment_id, created.id)
            .await
            .expect("owner delete reaction");
        let (_, total_after, _, _) =
            list_reactions(&no_user_ctx, "o", "r", comment_id, Some(1), Some(10))
                .await
                .expect("list after delete");
        assert_eq!(total_after, 0);
    }
}
