use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::comment::ReactionCounts,
    handlers::{path_param, query_value},
    router::{AppRequest, AppResponse},
    services, AppContext,
};

use super::respond_native;

#[derive(Debug, Deserialize)]
struct LabelRow {
    id: i64,
    name: String,
    color: String,
}

#[derive(Debug, Deserialize)]
struct ThreadRow {
    id: i64,
    number: i64,
    title: String,
    body: Option<String>,
    state: String,
    author_id: i64,
    author_login: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct ThreadLabelRow {
    issue_id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CommentRow {
    id: i64,
    issue_id: i64,
    body: String,
    author_id: i64,
    author_login: String,
    created_at: String,
    updated_at: String,
    reactions: String,
}

#[derive(Debug, Serialize)]
struct NativeExportResponse {
    repo: NativeExportRepo,
    exported_at: String,
    labels: Vec<NativeExportLabel>,
    threads: Vec<NativeExportThread>,
}

#[derive(Debug, Serialize)]
struct NativeExportRepo {
    owner: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct NativeExportLabel {
    id: i64,
    name: String,
    color: String,
}

#[derive(Debug, Serialize)]
struct NativeExportUserLite {
    id: i64,
    login: String,
}

#[derive(Debug, Serialize)]
struct NativeExportThread {
    number: i64,
    title: String,
    body: String,
    state: String,
    author: NativeExportUserLite,
    labels: Vec<String>,
    created_at: String,
    updated_at: String,
    comments: Vec<NativeExportComment>,
}

#[derive(Debug, Serialize)]
struct NativeExportComment {
    id: i64,
    body: String,
    author: NativeExportUserLite,
    reactions: NativeExportReactions,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct NativeExportReactions {
    #[serde(rename = "+1")]
    plus_one: i64,
    #[serde(rename = "-1")]
    minus_one: i64,
    laugh: i64,
    confused: i64,
    heart: i64,
    hooray: i64,
    rocket: i64,
    eyes: i64,
    total: i64,
}

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let format = query_value(&req, "format").unwrap_or_else(|| "json".to_string());
    let since = normalize_since(query_value(&req, "since"))?;

    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let repo_row = services::repo::get_repo(ctx, &owner, &repo).await?;
    if repo_row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden("Admin required"));
    }

    let labels = db::query_all::<LabelRow>(
        ctx.db,
        "SELECT id, name, color FROM labels WHERE repo_id = ?1 ORDER BY id ASC",
        &[DbValue::Integer(repo_row.id)],
    )
    .await?;

    let mut thread_sql = "SELECT i.id, i.number, i.title, i.body, i.state, u.id AS author_id, u.login AS author_login, i.created_at, i.updated_at \
         FROM issues i \
         JOIN users u ON u.id = i.user_id \
         WHERE i.repo_id = ?1 AND i.deleted_at IS NULL"
        .to_string();
    let mut thread_params = vec![DbValue::Integer(repo_row.id)];
    if let Some(since) = &since {
        thread_sql.push_str(" AND i.updated_at >= ?2");
        thread_params.push(DbValue::Text(since.clone()));
    }
    thread_sql.push_str(" ORDER BY i.number ASC");
    let threads = db::query_all::<ThreadRow>(ctx.db, &thread_sql, &thread_params).await?;

    let thread_label_rows = db::query_all::<ThreadLabelRow>(
        ctx.db,
        "SELECT il.issue_id, l.name \
         FROM issue_labels il \
         JOIN labels l ON l.id = il.label_id \
         WHERE l.repo_id = ?1 \
         ORDER BY il.issue_id ASC, l.name ASC",
        &[DbValue::Integer(repo_row.id)],
    )
    .await?;
    let mut labels_by_issue: HashMap<i64, Vec<String>> = HashMap::new();
    for row in thread_label_rows {
        labels_by_issue
            .entry(row.issue_id)
            .or_default()
            .push(row.name);
    }

    let mut comment_sql = "SELECT c.id, c.issue_id, c.body, u.id AS author_id, u.login AS author_login, c.created_at, c.updated_at, c.reactions \
         FROM comments c \
         JOIN users u ON u.id = c.user_id \
         WHERE c.repo_id = ?1 AND c.deleted_at IS NULL"
        .to_string();
    let mut comment_params = vec![DbValue::Integer(repo_row.id)];
    if let Some(since) = &since {
        comment_sql.push_str(" AND c.updated_at >= ?2");
        comment_params.push(DbValue::Text(since.clone()));
    }
    comment_sql.push_str(" ORDER BY c.issue_id ASC, c.id ASC");
    let comment_rows = db::query_all::<CommentRow>(ctx.db, &comment_sql, &comment_params).await?;

    let mut comments_by_issue: HashMap<i64, Vec<NativeExportComment>> = HashMap::new();
    for row in comment_rows {
        let counts: ReactionCounts = serde_json::from_str(&row.reactions).unwrap_or_default();
        comments_by_issue
            .entry(row.issue_id)
            .or_default()
            .push(NativeExportComment {
                id: row.id,
                body: row.body,
                author: NativeExportUserLite {
                    id: row.author_id,
                    login: row.author_login,
                },
                reactions: NativeExportReactions {
                    plus_one: counts.plus_one,
                    minus_one: counts.minus_one,
                    laugh: counts.laugh,
                    confused: counts.confused,
                    heart: counts.heart,
                    hooray: counts.hooray,
                    rocket: counts.rocket,
                    eyes: counts.eyes,
                    total: counts.total,
                },
                created_at: to_iso8601(&row.created_at),
                updated_at: to_iso8601(&row.updated_at),
            });
    }

    let export_threads: Vec<NativeExportThread> = threads
        .into_iter()
        .map(|thread| NativeExportThread {
            number: thread.number,
            title: thread.title,
            body: thread.body.unwrap_or_default(),
            state: thread.state,
            author: NativeExportUserLite {
                id: thread.author_id,
                login: thread.author_login,
            },
            labels: labels_by_issue.remove(&thread.id).unwrap_or_default(),
            created_at: to_iso8601(&thread.created_at),
            updated_at: to_iso8601(&thread.updated_at),
            comments: comments_by_issue.remove(&thread.id).unwrap_or_default(),
        })
        .collect();

    let exported_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    if format.eq_ignore_ascii_case("csv") {
        let csv = build_csv(&owner, &repo, &export_threads);
        return Ok(AppResponse {
            status: 200,
            headers: vec![
                (
                    "Content-Type".to_string(),
                    "text/csv; charset=utf-8".to_string(),
                ),
                (
                    "Content-Disposition".to_string(),
                    format!(
                        "attachment; filename=\"{}-{}-export.csv\"",
                        owner.replace('"', ""),
                        repo.replace('"', "")
                    ),
                ),
            ],
            body: bytes::Bytes::from(csv),
        });
    }

    if !format.eq_ignore_ascii_case("json") {
        return Err(ApiError::bad_request(
            "invalid format, expected json or csv",
        ));
    }

    let payload = NativeExportResponse {
        repo: NativeExportRepo { owner, name: repo },
        exported_at,
        labels: labels
            .into_iter()
            .map(|v| NativeExportLabel {
                id: v.id,
                name: v.name,
                color: v.color,
            })
            .collect(),
        threads: export_threads,
    };

    Ok(AppResponse::json(200, &payload))
}

fn build_csv(owner: &str, repo: &str, threads: &[NativeExportThread]) -> String {
    let mut out = String::new();
    out.push_str("repo_owner,repo_name,thread_number,thread_title,thread_state,thread_author_id,thread_author_login,thread_created_at,thread_updated_at,comment_id,comment_author_id,comment_author_login,comment_body,comment_created_at,comment_updated_at,comment_reactions,labels\n");

    for thread in threads {
        let labels = thread.labels.join("|");
        if thread.comments.is_empty() {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},,,,,,,{},{}\n",
                csv_escape(owner),
                csv_escape(repo),
                thread.number,
                csv_escape(&thread.title),
                csv_escape(&thread.state),
                thread.author.id,
                csv_escape(&thread.author.login),
                csv_escape(&thread.created_at),
                csv_escape(&thread.updated_at),
                csv_escape("{}"),
                csv_escape(&labels),
            ));
            continue;
        }

        for comment in &thread.comments {
            let reactions_json =
                serde_json::to_string(&comment.reactions).unwrap_or_else(|_| "{}".to_string());
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_escape(owner),
                csv_escape(repo),
                thread.number,
                csv_escape(&thread.title),
                csv_escape(&thread.state),
                thread.author.id,
                csv_escape(&thread.author.login),
                csv_escape(&thread.created_at),
                csv_escape(&thread.updated_at),
                comment.id,
                comment.author.id,
                csv_escape(&comment.author.login),
                csv_escape(&comment.body),
                csv_escape(&comment.created_at),
                csv_escape(&comment.updated_at),
                csv_escape(&reactions_json),
                csv_escape(&labels),
            ));
        }
    }

    out
}

fn csv_escape(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

fn normalize_since(value: Option<String>) -> crate::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };

    let parsed = chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| ApiError::bad_request("invalid since, expected ISO8601"))?;
    Ok(Some(
        parsed
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    ))
}

fn to_iso8601(value: &str) -> String {
    if value.contains('T') && value.ends_with('Z') {
        return value.to_string();
    }
    value.replace(' ', "T") + "Z"
}

#[cfg(test)]
mod tests {
    use super::{csv_escape, normalize_since, to_iso8601};

    #[test]
    fn csv_escape_handles_quotes() {
        let value = "a\"b";
        let escaped = csv_escape(value);
        assert_eq!(escaped, "\"a\"\"b\"");
    }

    #[test]
    fn normalize_since_accepts_iso8601() {
        let out = normalize_since(Some("2025-01-15T08:00:00Z".to_string()))
            .expect("parse should succeed");
        assert_eq!(out.as_deref(), Some("2025-01-15 08:00:00"));
    }

    #[test]
    fn normalize_since_rejects_invalid() {
        let err = normalize_since(Some("invalid-time".to_string()))
            .err()
            .expect("must fail");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn to_iso8601_formats_sqlite_datetime() {
        assert_eq!(to_iso8601("2025-01-15 08:00:00"), "2025-01-15T08:00:00Z");
        assert_eq!(to_iso8601("2025-01-15T08:00:00Z"), "2025-01-15T08:00:00Z");
    }
}
