use crate::{
    fmt::{apply_comment_accept, pagination::build_link_header, parse_accept},
    handlers::{body_json, path_i64, path_param, query_i64, query_value, respond},
    router::{AppRequest, AppResponse},
    services,
    types::{CreateCommentInput, ListCommentsQuery, UpdateCommentInput},
    AppContext,
};

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;
    let query = ListCommentsQuery {
        per_page: query_i64(&req, "per_page"),
        page: query_i64(&req, "page"),
        since: query_value(&req, "since"),
    };

    let accept = parse_accept(req.accept.as_deref());
    let (items, total, page, per_page) =
        services::comment::list_comments(ctx, &owner, &repo, number, &query).await?;
    let items: Vec<_> = items
        .into_iter()
        .map(|v| apply_comment_accept(v, accept))
        .collect();

    let mut response = AppResponse::json(200, &items);
    if let Some(link) = build_link_header(
        ctx.base_url,
        &format!("/repos/{}/{}/issues/{}/comments", owner, repo, number),
        page,
        per_page,
        total,
    ) {
        response = response.with_header("Link", &link);
    }

    Ok(response)
}

pub async fn create(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(create_inner(req, ctx).await)
}

async fn create_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;
    let input: CreateCommentInput = body_json(&req)?;
    let accept = parse_accept(req.accept.as_deref());

    let comment = services::comment::create_comment(ctx, &owner, &repo, number, &input).await?;
    Ok(AppResponse::json(201, &apply_comment_accept(comment, accept)))
}

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;
    let accept = parse_accept(req.accept.as_deref());

    let comment = services::comment::get_comment(ctx, &owner, &repo, id).await?;
    Ok(AppResponse::json(200, &apply_comment_accept(comment, accept)))
}

pub async fn update(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(update_inner(req, ctx).await)
}

async fn update_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;
    let input: UpdateCommentInput = body_json(&req)?;
    let accept = parse_accept(req.accept.as_deref());

    let comment = services::comment::update_comment(ctx, &owner, &repo, id, &input).await?;
    Ok(AppResponse::json(200, &apply_comment_accept(comment, accept)))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;

    services::comment::delete_comment(ctx, &owner, &repo, id).await?;
    Ok(AppResponse::no_content())
}