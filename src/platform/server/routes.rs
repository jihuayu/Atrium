use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};

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

use super::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/repos/{owner}/{repo}/issues/comments/{id}/reactions/{rid}",
            delete(delete_reaction),
        )
        .route(
            "/repos/{owner}/{repo}/issues/comments/{id}/reactions",
            get(list_reactions).post(create_reaction),
        )
        .route(
            "/repos/{owner}/{repo}/issues/comments/{id}",
            get(get_comment).patch(update_comment).delete(delete_comment),
        )
        .route(
            "/repos/{owner}/{repo}/issues/{number}/comments",
            get(list_comments).post(create_comment),
        )
        .route(
            "/repos/{owner}/{repo}/issues/{number}",
            get(get_issue).patch(update_issue),
        )
        .route(
            "/repos/{owner}/{repo}/issues",
            get(list_issues).post(create_issue),
        )
        .route(
            "/repos/{owner}/{repo}/labels",
            get(list_labels).post(create_label),
        )
        .route("/search/issues", get(search_issues))
        .with_state(state)
}

async fn list_issues(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<ListIssuesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let (items, total, page, per_page) = services::issue::list_issues(&ctx, &owner, &repo, &query).await?;
    let items: Vec<_> = items.into_iter().map(|v| apply_issue_accept(v, accept)).collect();

    let mut response = Json(items).into_response();
    if let Some(link) = build_link_header(
        &state.base_url,
        &format!("/repos/{}/{}/issues", owner, repo),
        page,
        per_page,
        total,
    ) {
        set_link_header(&mut response, &link);
    }

    Ok(response)
}

async fn create_issue(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CreateIssueInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let issue = services::issue::create_issue(&ctx, &owner, &repo, &input).await?;
    Ok((StatusCode::CREATED, Json(apply_issue_accept(issue, accept))).into_response())
}

async fn get_issue(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let issue = services::issue::get_issue(&ctx, &owner, &repo, number).await?;
    Ok(Json(apply_issue_accept(issue, accept)).into_response())
}

async fn update_issue(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(input): Json<UpdateIssueInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let issue = services::issue::update_issue(&ctx, &owner, &repo, number, &input).await?;
    Ok(Json(apply_issue_accept(issue, accept)).into_response())
}

async fn list_comments(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Query(query): Query<ListCommentsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let (items, total, page, per_page) = services::comment::list_comments(&ctx, &owner, &repo, number, &query).await?;
    let items: Vec<_> = items
        .into_iter()
        .map(|v| apply_comment_accept(v, accept))
        .collect();

    let mut response = Json(items).into_response();
    if let Some(link) = build_link_header(
        &state.base_url,
        &format!("/repos/{}/{}/issues/{}/comments", owner, repo, number),
        page,
        per_page,
        total,
    ) {
        set_link_header(&mut response, &link);
    }

    Ok(response)
}

async fn create_comment(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(input): Json<CreateCommentInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let comment = services::comment::create_comment(&ctx, &owner, &repo, number, &input).await?;
    Ok((StatusCode::CREATED, Json(apply_comment_accept(comment, accept))).into_response())
}

async fn get_comment(
    State(state): State<AppState>,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let comment = services::comment::get_comment(&ctx, &owner, &repo, id).await?;
    Ok(Json(apply_comment_accept(comment, accept)).into_response())
}

async fn update_comment(
    State(state): State<AppState>,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(input): Json<UpdateCommentInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let comment = services::comment::update_comment(&ctx, &owner, &repo, id, &input).await?;
    Ok(Json(apply_comment_accept(comment, accept)).into_response())
}

async fn delete_comment(
    State(state): State<AppState>,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());
    services::comment::delete_comment(&ctx, &owner, &repo, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_reactions(
    State(state): State<AppState>,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Query(pagination): Query<PaginationQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());

    let (items, total, page, per_page) = services::reaction::list_reactions(
        &ctx,
        &owner,
        &repo,
        id,
        pagination.page,
        pagination.per_page,
    )
    .await?;

    let mut response = Json(items).into_response();
    if let Some(link) = build_link_header(
        &state.base_url,
        &format!("/repos/{}/{}/issues/comments/{}/reactions", owner, repo, id),
        page,
        per_page,
        total,
    ) {
        set_link_header(&mut response, &link);
    }

    Ok(response)
}

async fn create_reaction(
    State(state): State<AppState>,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(input): Json<CreateReactionInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());

    let (reaction, created) = services::reaction::create_reaction(&ctx, &owner, &repo, id, &input).await?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(reaction)).into_response())
}

async fn delete_reaction(
    State(state): State<AppState>,
    Path((owner, repo, id, rid)): Path<(String, String, i64, i64)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());

    services::reaction::delete_reaction(&ctx, &owner, &repo, id, rid).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn search_issues(
    State(state): State<AppState>,
    Query(query): Query<SearchIssuesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let accept = parse_accept(header_value(&headers, header::ACCEPT));
    let ctx = app_context(&state, user.as_ref());

    let (items, total, page, per_page) = services::search::search_issues(&ctx, &query).await?;
    let items = items
        .into_iter()
        .map(|v| apply_issue_accept(v, accept))
        .collect();

    let mut response = Json(SearchIssuesResponse {
        total_count: total,
        incomplete_results: false,
        items,
    })
    .into_response();

    if let Some(link) = build_link_header(&state.base_url, "/search/issues", page, per_page, total) {
        set_link_header(&mut response, &link);
    }

    Ok(response)
}

async fn list_labels(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());

    let labels = services::label::list_labels(&ctx, &owner, &repo).await?;
    Ok(Json(labels).into_response())
}

async fn create_label(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CreateLabelInput>,
) -> Result<Response, ApiError> {
    let user = resolve_request_user(&state, &headers).await?;
    let ctx = app_context(&state, user.as_ref());

    let label = services::label::create_label(&ctx, &owner, &repo, &input).await?;
    Ok((StatusCode::CREATED, Json(label)).into_response())
}

async fn resolve_request_user(state: &AppState, headers: &HeaderMap) -> Result<Option<crate::types::GitHubUser>, ApiError> {
    let raw_header = header_value(headers, header::AUTHORIZATION);
    let token = bearer_from_header(raw_header)?;

    match token {
        None => Ok(None),
        Some(token) => {
            let user = resolve_user(state.db.as_ref(), state.http.as_ref(), &token, state.token_cache_ttl).await?;
            Ok(Some(user))
        }
    }
}

fn app_context<'a>(state: &'a AppState, user: Option<&'a crate::types::GitHubUser>) -> AppContext<'a> {
    AppContext {
        db: state.db.as_ref(),
        http: state.http.as_ref(),
        base_url: &state.base_url,
        user,
    }
}

fn header_value<'a>(headers: &'a HeaderMap, name: header::HeaderName) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn set_link_header(response: &mut Response, link: &str) {
    if let Ok(value) = HeaderValue::from_str(link) {
        response.headers_mut().insert(header::LINK, value);
    }
}
