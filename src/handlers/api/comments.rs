use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    fmt::{api as api_fmt, apply_comment_accept, AcceptMode},
    handlers::{body_json, path_i64, path_param, query_i64, query_value},
    router::{AppRequest, AppResponse},
    services,
    types::{
        CreateCommentInput, CursorPage, CursorPagination, NativeCommentListQuery,
        UpdateCommentInput,
    },
    AppContext,
};

use super::respond_native;

#[derive(Debug, Deserialize)]
struct CommentPointer {
    id: i64,
}

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;

    let query = NativeCommentListQuery {
        limit: query_i64(&req, "limit"),
        cursor: query_value(&req, "cursor"),
        order: query_value(&req, "order"),
    };

    let issue = services::issue::get_issue(ctx, &owner, &repo, number).await?;
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let order = query.order.unwrap_or_else(|| "asc".to_string());
    let cursor_id = match query.cursor {
        Some(cursor) => Some(services::cursor::decode_cursor(&cursor)?),
        None => None,
    };

    let mut filters = vec![
        "c.issue_id = ?1".to_string(),
        "c.deleted_at IS NULL".to_string(),
    ];
    let mut params = vec![DbValue::Integer(issue.id)];
    let mut idx = 2;

    if let Some(cursor_id) = cursor_id {
        if order.eq_ignore_ascii_case("desc") {
            filters.push(format!("c.id < ?{}", idx));
        } else {
            filters.push(format!("c.id > ?{}", idx));
        }
        params.push(DbValue::Integer(cursor_id));
        idx += 1;
    }

    params.push(DbValue::Integer(limit + 1));

    let where_sql = filters.join(" AND ");
    let order_sql = if order.eq_ignore_ascii_case("desc") {
        "DESC"
    } else {
        "ASC"
    };
    let sql = format!(
        "SELECT c.id FROM comments c WHERE {} ORDER BY c.id {} LIMIT ?{}",
        where_sql, order_sql, idx
    );

    let mut pointers = db::query_all::<CommentPointer>(ctx.db, &sql, &params).await?;
    let has_more = pointers.len() as i64 > limit;
    if has_more {
        pointers.pop();
    }

    let mut data = Vec::with_capacity(pointers.len());
    for pointer in &pointers {
        let comment = services::comment::get_comment(ctx, &owner, &repo, pointer.id).await?;
        data.push(api_fmt::to_native_comment(&apply_comment_accept(
            comment,
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
    let number = path_i64(&req, "number")?;
    let input: CreateCommentInput = body_json(&req)?;

    let comment = services::comment::create_comment(ctx, &owner, &repo, number, &input).await?;
    Ok(AppResponse::json(
        201,
        &api_fmt::to_native_comment(&apply_comment_accept(comment, AcceptMode::Full)),
    ))
}

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;

    let comment = services::comment::get_comment(ctx, &owner, &repo, id).await?;
    Ok(AppResponse::json(
        200,
        &api_fmt::to_native_comment(&apply_comment_accept(comment, AcceptMode::Full)),
    ))
}

pub async fn update(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(update_inner(req, ctx).await)
}

async fn update_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;
    let input: UpdateCommentInput = body_json(&req)?;

    let comment = services::comment::update_comment(ctx, &owner, &repo, id, &input).await?;
    Ok(AppResponse::json(
        200,
        &api_fmt::to_native_comment(&apply_comment_accept(comment, AcceptMode::Full)),
    ))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;

    services::comment::delete_comment(ctx, &owner, &repo, id).await?;
    Ok(AppResponse::no_content())
}
