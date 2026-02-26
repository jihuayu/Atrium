use crate::{
    fmt::pagination::build_link_header,
    handlers::{body_json, path_i64, path_param, query_i64, respond},
    router::{AppRequest, AppResponse},
    services,
    types::{CreateReactionInput, PaginationQuery},
    AppContext,
};

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;
    let pagination = PaginationQuery {
        per_page: query_i64(&req, "per_page"),
        page: query_i64(&req, "page"),
    };

    let (items, total, page, per_page) = services::reaction::list_reactions(
        ctx,
        &owner,
        &repo,
        id,
        pagination.page,
        pagination.per_page,
    )
    .await?;

    let mut response = AppResponse::json(200, &items);
    if let Some(link) = build_link_header(
        ctx.base_url,
        &format!("/repos/{}/{}/issues/comments/{}/reactions", owner, repo, id),
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
    let id = path_i64(&req, "id")?;
    let input: CreateReactionInput = body_json(&req)?;

    let (reaction, created) = services::reaction::create_reaction(ctx, &owner, &repo, id, &input).await?;
    let status = if created { 201 } else { 200 };
    Ok(AppResponse::json(status, &reaction))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let id = path_i64(&req, "id")?;
    let rid = path_i64(&req, "rid")?;

    services::reaction::delete_reaction(ctx, &owner, &repo, id, rid).await?;
    Ok(AppResponse::no_content())
}