use crate::{
    fmt::{apply_issue_accept, pagination::build_link_header, parse_accept},
    handlers::{body_json, path_i64, path_param, query_i64, query_value, respond},
    router::{AppRequest, AppResponse},
    services,
    types::{CreateIssueInput, ListIssuesQuery, UpdateIssueInput},
    AppContext,
};

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let query = ListIssuesQuery {
        state: query_value(&req, "state"),
        labels: query_value(&req, "labels"),
        sort: query_value(&req, "sort"),
        direction: query_value(&req, "direction"),
        since: query_value(&req, "since"),
        per_page: query_i64(&req, "per_page"),
        page: query_i64(&req, "page"),
        creator: query_value(&req, "creator"),
    };

    let accept = parse_accept(req.accept.as_deref());
    let (items, total, page, per_page) =
        services::issue::list_issues(ctx, &owner, &repo, &query).await?;
    let items: Vec<_> = items
        .into_iter()
        .map(|v| apply_issue_accept(v, accept))
        .collect();

    let mut response = AppResponse::json(200, &items);
    if let Some(link) = build_link_header(
        ctx.base_url,
        &format!("/repos/{}/{}/issues", owner, repo),
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
    let input: CreateIssueInput = body_json(&req)?;
    let accept = parse_accept(req.accept.as_deref());

    let issue = services::issue::create_issue(ctx, &owner, &repo, &input).await?;
    Ok(AppResponse::json(201, &apply_issue_accept(issue, accept)))
}

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;
    let accept = parse_accept(req.accept.as_deref());

    let issue = services::issue::get_issue(ctx, &owner, &repo, number).await?;
    Ok(AppResponse::json(200, &apply_issue_accept(issue, accept)))
}

pub async fn update(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(update_inner(req, ctx).await)
}

async fn update_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let number = path_i64(&req, "number")?;
    let input: UpdateIssueInput = body_json(&req)?;
    let accept = parse_accept(req.accept.as_deref());

    let issue = services::issue::update_issue(ctx, &owner, &repo, number, &input).await?;
    Ok(AppResponse::json(200, &apply_issue_accept(issue, accept)))
}
