use crate::{
    handlers::{body_json, path_param, respond},
    router::{AppRequest, AppResponse},
    services,
    types::CreateLabelInput,
    AppContext,
};

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;

    let labels = services::label::list_labels(ctx, &owner, &repo).await?;
    Ok(AppResponse::json(200, &labels))
}

pub async fn create(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(create_inner(req, ctx).await)
}

async fn create_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let input: CreateLabelInput = body_json(&req)?;

    let label = services::label::create_label(ctx, &owner, &repo, &input).await?;
    Ok(AppResponse::json(201, &label))
}
