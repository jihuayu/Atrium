use crate::{
    fmt::{apply_issue_accept, pagination::build_link_header, parse_accept},
    handlers::{query_i64, query_value, respond},
    router::{AppRequest, AppResponse},
    services,
    types::{SearchIssuesQuery, SearchIssuesResponse},
    ApiError, AppContext,
};

pub async fn search(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(search_inner(req, ctx).await)
}

async fn search_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let query = SearchIssuesQuery {
        q: query_value(&req, "q").ok_or_else(|| ApiError::bad_request("Missing or invalid search query"))?,
        sort: query_value(&req, "sort"),
        order: query_value(&req, "order"),
        per_page: query_i64(&req, "per_page"),
        page: query_i64(&req, "page"),
    };

    let accept = parse_accept(req.accept.as_deref());
    let (items, total, page, per_page) = services::search::search_issues(ctx, &query).await?;
    let items = items
        .into_iter()
        .map(|v| apply_issue_accept(v, accept))
        .collect();

    let mut response = AppResponse::json(
        200,
        &SearchIssuesResponse {
            total_count: total,
            incomplete_results: false,
            items,
        },
    );

    if let Some(link) = build_link_header(ctx.base_url, "/search/issues", page, per_page, total) {
        response = response.with_header("Link", &link);
    }

    Ok(response)
}