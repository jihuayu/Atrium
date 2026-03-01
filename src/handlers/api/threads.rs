use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::{api as api_fmt, apply_issue_accept, AcceptMode},
    handlers::{body_json, path_i64, path_param, query_i64, query_value},
    router::{AppRequest, AppResponse},
    services,
    types::{CreateIssueInput, CursorPage, CursorPagination, NativeListQuery, UpdateIssueInput},
    AppContext,
};

use super::respond_native;

#[derive(Debug, Deserialize)]
struct IssuePointer {
    id: i64,
    number: i64,
}

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let query = NativeListQuery {
        state: query_value(&req, "state"),
        limit: query_i64(&req, "limit"),
        cursor: query_value(&req, "cursor"),
        direction: query_value(&req, "direction"),
    };

    let _repo = services::repo::get_repo(ctx, &owner, &repo).await?;

    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let direction = query.direction.unwrap_or_else(|| "desc".to_string());
    let cursor_id = match query.cursor {
        Some(cursor) => Some(services::cursor::decode_cursor(&cursor)?),
        None => None,
    };

    let mut filters = vec![
        "r.owner = ?1".to_string(),
        "r.name = ?2".to_string(),
        "i.deleted_at IS NULL".to_string(),
    ];
    let mut params = vec![DbValue::Text(owner.clone()), DbValue::Text(repo.clone())];
    let mut idx = 3;

    let state = query.state.unwrap_or_else(|| "open".to_string());
    if state != "all" {
        filters.push(format!("i.state = ?{}", idx));
        params.push(DbValue::Text(state));
        idx += 1;
    }

    if let Some(cursor_id) = cursor_id {
        if direction.eq_ignore_ascii_case("asc") {
            filters.push(format!("i.id > ?{}", idx));
        } else {
            filters.push(format!("i.id < ?{}", idx));
        }
        params.push(DbValue::Integer(cursor_id));
        idx += 1;
    }

    params.push(DbValue::Integer(limit + 1));

    let where_sql = filters.join(" AND ");
    let order = if direction.eq_ignore_ascii_case("asc") {
        "ASC"
    } else {
        "DESC"
    };
    let sql = format!(
        "SELECT i.id, i.number FROM issues i \
         JOIN repos r ON r.id = i.repo_id \
         WHERE {} ORDER BY i.id {} LIMIT ?{}",
        where_sql, order, idx
    );

    let mut pointers = db::query_all::<IssuePointer>(ctx.db, &sql, &params).await?;
    let has_more = pointers.len() as i64 > limit;
    if has_more {
        pointers.pop();
    }

    let mut data = Vec::with_capacity(pointers.len());
    for pointer in &pointers {
        let issue = services::issue::get_issue(ctx, &owner, &repo, pointer.number).await?;
        data.push(api_fmt::to_native_thread(&apply_issue_accept(
            issue,
            AcceptMode::Full,
        )));
    }

    let next_cursor = if has_more {
        pointers
            .last()
            .map(|last| services::cursor::encode_cursor(last.id))
            .transpose()?
    } else {
        None
    };

    Ok(AppResponse::json(
        200,
        &CursorPage {
            data,
            pagination: CursorPagination {
                next_cursor,
                has_more,
            },
        },
    ))
}

pub async fn create(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(create_inner(req, ctx).await)
}

async fn create_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let input: CreateIssueInput = body_json(&req)?;

    let issue = services::issue::create_issue(ctx, &owner, &repo, &input).await?;
    Ok(AppResponse::json(
        201,
        &api_fmt::to_native_thread(&apply_issue_accept(issue, AcceptMode::Full)),
    ))
}

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;

    let issue = services::issue::get_issue(ctx, &owner, &repo, number).await?;
    Ok(AppResponse::json(
        200,
        &api_fmt::to_native_thread(&apply_issue_accept(issue, AcceptMode::Full)),
    ))
}

pub async fn update(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(update_inner(req, ctx).await)
}

async fn update_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;
    let input: UpdateIssueInput = body_json(&req)?;

    let issue = services::issue::update_issue(ctx, &owner, &repo, number, &input).await?;
    Ok(AppResponse::json(
        200,
        &api_fmt::to_native_thread(&apply_issue_accept(issue, AcceptMode::Full)),
    ))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;

    let repo_row = services::repo::get_repo(ctx, &owner, &repo).await?;
    if repo_row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden("Admin required"));
    }

    let affected = ctx
        .db
        .execute(
            "UPDATE issues SET deleted_at = datetime('now'), updated_at = datetime('now') \
             WHERE repo_id = ?1 AND number = ?2 AND deleted_at IS NULL",
            &[DbValue::Integer(repo_row.id), DbValue::Integer(number)],
        )
        .await?;
    if affected == 0 {
        return Err(ApiError::not_found("Issue"));
    }

    Ok(AppResponse::no_content())
}
