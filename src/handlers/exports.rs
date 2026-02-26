use crate::{
    handlers::respond,
    router::{AppRequest, AppResponse},
    services, AppContext,
};

pub async fn export_user_repos(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(export_user_repos_inner(ctx).await)
}

async fn export_user_repos_inner(ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let payload = services::exports::export_user_repos(ctx).await?;
    Ok(AppResponse::json(200, &payload))
}
