use serde::Serialize;
use worker::{Request, Response, Result, RouteContext, Router};

use crate::{
    auth::{bearer_from_header, resolve_user},
    fmt::{apply_comment_accept, apply_issue_accept, pagination::build_link_header, parse_accept},
    services,
    types::{
        CreateCommentInput, CreateIssueInput, CreateLabelInput, CreateReactionInput, ListCommentsQuery,
        ListIssuesQuery, PaginationQuery, SearchIssuesQuery, SearchIssuesResponse, UpdateCommentInput,
        UpdateIssueInput,
    },
    AppContext, ApiError,
};

use super::{d1::D1Db, http::WorkerHttpClient};

pub struct WorkerState {
    pub base_url: String,
    pub token_cache_ttl: i64,
}

impl WorkerState {
    pub fn from_env(env: &worker::Env) -> Self {
        let base_url = env
            .var("BASE_URL")
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
        let token_cache_ttl = env
            .var("TOKEN_CACHE_TTL")
            .ok()
            .and_then(|v| v.to_string().parse::<i64>().ok())
            .unwrap_or(3600);

        Self {
            base_url,
            token_cache_ttl,
        }
    }
}

pub fn router(state: WorkerState) -> Router<'static, WorkerState> {
    Router::with_data(state)
        .delete_async(
            "/repos/:owner/:repo/issues/comments/:id/reactions/:rid",
            |req, ctx| async move { to_worker_result(delete_reaction(req, ctx).await) },
        )
        .get_async(
            "/repos/:owner/:repo/issues/comments/:id/reactions",
            |req, ctx| async move { to_worker_result(list_reactions(req, ctx).await) },
        )
        .post_async(
            "/repos/:owner/:repo/issues/comments/:id/reactions",
            |req, ctx| async move { to_worker_result(create_reaction(req, ctx).await) },
        )
        .get_async(
            "/repos/:owner/:repo/issues/comments/:id",
            |req, ctx| async move { to_worker_result(get_comment(req, ctx).await) },
        )
        .patch_async(
            "/repos/:owner/:repo/issues/comments/:id",
            |req, ctx| async move { to_worker_result(update_comment(req, ctx).await) },
        )
        .delete_async(
            "/repos/:owner/:repo/issues/comments/:id",
            |req, ctx| async move { to_worker_result(delete_comment(req, ctx).await) },
        )
        .get_async(
            "/repos/:owner/:repo/issues/:number/comments",
            |req, ctx| async move { to_worker_result(list_comments(req, ctx).await) },
        )
        .post_async(
            "/repos/:owner/:repo/issues/:number/comments",
            |req, ctx| async move { to_worker_result(create_comment(req, ctx).await) },
        )
        .get_async(
            "/repos/:owner/:repo/issues/:number",
            |req, ctx| async move { to_worker_result(get_issue(req, ctx).await) },
        )
        .patch_async(
            "/repos/:owner/:repo/issues/:number",
            |req, ctx| async move { to_worker_result(update_issue(req, ctx).await) },
        )
        .get_async("/repos/:owner/:repo/issues", |req, ctx| async move {
            to_worker_result(list_issues(req, ctx).await)
        })
        .post_async("/repos/:owner/:repo/issues", |req, ctx| async move {
            to_worker_result(create_issue(req, ctx).await)
        })
        .get_async("/repos/:owner/:repo/labels", |req, ctx| async move {
            to_worker_result(list_labels(req, ctx).await)
        })
        .post_async("/repos/:owner/:repo/labels", |req, ctx| async move {
            to_worker_result(create_label(req, ctx).await)
        })
        .get_async("/search/issues", |req, ctx| async move {
            to_worker_result(search_issues(req, ctx).await)
        })
}

async fn list_issues(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let query = req.query::<ListIssuesQuery>().unwrap_or(ListIssuesQuery {
        state: None,
        labels: None,
        sort: None,
        direction: None,
        since: None,
        per_page: None,
        page: None,
        creator: None,
    });

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let (items, total, page, per_page) = services::issue::list_issues(&ctx, &owner, &repo, &query).await?;
    let items: Vec<_> = items.into_iter().map(|v| apply_issue_accept(v, accept)).collect();
    let link = build_link_header(
        &route.data.base_url,
        &format!("/repos/{}/{}/issues", owner, repo),
        page,
        per_page,
        total,
    );

    json_response(200, &items, link)
}

async fn create_issue(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let input = req
        .json::<CreateIssueInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let issue = services::issue::create_issue(&ctx, &owner, &repo, &input).await?;
    json_response(201, &apply_issue_accept(issue, accept), None)
}

async fn get_issue(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let number = param_i64(&route, "number")?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let issue = services::issue::get_issue(&ctx, &owner, &repo, number).await?;
    json_response(200, &apply_issue_accept(issue, accept), None)
}

async fn update_issue(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let number = param_i64(&route, "number")?;
    let input = req
        .json::<UpdateIssueInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let issue = services::issue::update_issue(&ctx, &owner, &repo, number, &input).await?;
    json_response(200, &apply_issue_accept(issue, accept), None)
}

async fn list_comments(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let number = param_i64(&route, "number")?;
    let query = req.query::<ListCommentsQuery>().unwrap_or(ListCommentsQuery {
        per_page: None,
        page: None,
        since: None,
    });

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let (items, total, page, per_page) =
        services::comment::list_comments(&ctx, &owner, &repo, number, &query).await?;
    let items: Vec<_> = items.into_iter().map(|v| apply_comment_accept(v, accept)).collect();
    let link = build_link_header(
        &route.data.base_url,
        &format!("/repos/{}/{}/issues/{}/comments", owner, repo, number),
        page,
        per_page,
        total,
    );

    json_response(200, &items, link)
}

async fn create_comment(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let number = param_i64(&route, "number")?;
    let input = req
        .json::<CreateCommentInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let comment = services::comment::create_comment(&ctx, &owner, &repo, number, &input).await?;
    json_response(201, &apply_comment_accept(comment, accept), None)
}

async fn get_comment(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let comment = services::comment::get_comment(&ctx, &owner, &repo, id).await?;
    json_response(200, &apply_comment_accept(comment, accept), None)
}

async fn update_comment(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;
    let input = req
        .json::<UpdateCommentInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let comment = services::comment::update_comment(&ctx, &owner, &repo, id, &input).await?;
    json_response(200, &apply_comment_accept(comment, accept), None)
}

async fn delete_comment(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    services::comment::delete_comment(&ctx, &owner, &repo, id).await?;
    empty_response(204)
}

async fn list_reactions(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;
    let pagination = req.query::<PaginationQuery>().unwrap_or(PaginationQuery {
        per_page: None,
        page: None,
    });

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let (items, total, page, per_page) =
        services::reaction::list_reactions(&ctx, &owner, &repo, id, pagination.page, pagination.per_page).await?;
    let link = build_link_header(
        &route.data.base_url,
        &format!("/repos/{}/{}/issues/comments/{}/reactions", owner, repo, id),
        page,
        per_page,
        total,
    );

    json_response(200, &items, link)
}

async fn create_reaction(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;
    let input = req
        .json::<CreateReactionInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let (reaction, created) = services::reaction::create_reaction(&ctx, &owner, &repo, id, &input).await?;
    let status = if created { 201 } else { 200 };
    json_response(status, &reaction, None)
}

async fn delete_reaction(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let id = param_i64(&route, "id")?;
    let rid = param_i64(&route, "rid")?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    services::reaction::delete_reaction(&ctx, &owner, &repo, id, rid).await?;
    empty_response(204)
}

async fn search_issues(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let query = req
        .query::<SearchIssuesQuery>()
        .map_err(|_| ApiError::bad_request("Missing or invalid search query"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let accept = parse_accept(header_value(&req, "Accept").as_deref());
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let (items, total, page, per_page) = services::search::search_issues(&ctx, &query).await?;
    let items = items.into_iter().map(|v| apply_issue_accept(v, accept)).collect();
    let response = SearchIssuesResponse {
        total_count: total,
        incomplete_results: false,
        items,
    };
    let link = build_link_header(&route.data.base_url, "/search/issues", page, per_page, total);

    json_response(200, &response, link)
}

async fn list_labels(req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let labels = services::label::list_labels(&ctx, &owner, &repo).await?;
    json_response(200, &labels, None)
}

async fn create_label(mut req: Request, route: RouteContext<WorkerState>) -> crate::Result<Response> {
    let owner = param_required(&route, "owner")?;
    let repo = param_required(&route, "repo")?;
    let input = req
        .json::<CreateLabelInput>()
        .await
        .map_err(|_| ApiError::bad_request("Invalid request body"))?;

    let db = d1_db(&route)?;
    let http = WorkerHttpClient;
    let user = resolve_request_user(&req, &db, &http, route.data.token_cache_ttl).await?;
    let ctx = app_context(&route, user.as_ref(), &db, &http);

    let label = services::label::create_label(&ctx, &owner, &repo, &input).await?;
    json_response(201, &label, None)
}

fn d1_db(route: &RouteContext<WorkerState>) -> crate::Result<D1Db> {
    let db = route
        .env
        .d1("DB")
        .map_err(|e| ApiError::internal(format!("missing D1 binding DB: {}", e)))?;
    Ok(D1Db { db })
}

async fn resolve_request_user(
    req: &Request,
    db: &D1Db,
    http: &WorkerHttpClient,
    ttl: i64,
) -> crate::Result<Option<crate::types::GitHubUser>> {
    let auth = header_value(req, "Authorization");
    let token = bearer_from_header(auth.as_deref())?;
    match token {
        None => Ok(None),
        Some(token) => Ok(Some(resolve_user(db, http, &token, ttl).await?)),
    }
}

fn app_context<'a>(
    route: &'a RouteContext<WorkerState>,
    user: Option<&'a crate::types::GitHubUser>,
    db: &'a D1Db,
    http: &'a WorkerHttpClient,
) -> AppContext<'a> {
    AppContext {
        db,
        http,
        base_url: &route.data.base_url,
        user,
    }
}

fn param_required(route: &RouteContext<WorkerState>, name: &str) -> crate::Result<String> {
    route
        .param(name)
        .cloned()
        .ok_or_else(|| ApiError::bad_request(format!("missing route param: {}", name)))
}

fn param_i64(route: &RouteContext<WorkerState>, name: &str) -> crate::Result<i64> {
    let raw = param_required(route, name)?;
    raw.parse::<i64>()
        .map_err(|_| ApiError::bad_request(format!("invalid integer param: {}", name)))
}

fn header_value(req: &Request, name: &str) -> Option<String> {
    req.headers().get(name).ok().flatten()
}

fn json_response<T: Serialize>(status: u16, payload: &T, link: Option<String>) -> crate::Result<Response> {
    let mut response = Response::from_json(payload)
        .map_err(|e| ApiError::internal(format!("response serialize failed: {}", e)))?
        .with_status(status);

    if let Some(link) = link {
        response
            .headers_mut()
            .set("Link", &link)
            .map_err(|e| ApiError::internal(format!("set Link header failed: {}", e)))?;
    }

    Ok(response)
}

fn empty_response(status: u16) -> crate::Result<Response> {
    let response = Response::empty()
        .map_err(|e| ApiError::internal(format!("empty response failed: {}", e)))?
        .with_status(status);
    Ok(response)
}

fn to_worker_result(result: crate::Result<Response>) -> Result<Response> {
    match result {
        Ok(response) => Ok(response),
        Err(error) => api_error_response(error),
    }
}

fn api_error_response(error: ApiError) -> Result<Response> {
    let response = Response::from_json(&error.body)?.with_status(error.status);
    Ok(response)
}
