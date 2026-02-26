use std::collections::HashMap;

use bytes::Bytes;
use serde::Serialize;

use crate::{handlers, ApiError, AppContext};

pub struct AppRequest {
    pub method: String,
    pub path: String,
    pub path_params: HashMap<String, String>,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub auth_header: Option<String>,
    pub accept: Option<String>,
    pub body: Bytes,
}

pub struct AppResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl AppResponse {
    pub fn json<T: Serialize>(status: u16, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self {
                status,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: Bytes::from(body),
            },
            Err(_) => Self::from_error(ApiError::internal("serialization failed")),
        }
    }

    pub fn no_content() -> Self {
        Self {
            status: 204,
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    pub fn from_error(error: ApiError) -> Self {
        Self::json(error.status, &error.body)
    }
}

#[derive(Clone)]
enum Route {
    ListIssues,
    CreateIssue,
    GetIssue,
    UpdateIssue,
    ListComments,
    CreateComment,
    GetComment,
    UpdateComment,
    DeleteComment,
    ListReactions,
    CreateReaction,
    DeleteReaction,
    ListLabels,
    CreateLabel,
    SearchIssues,
    RenderMarkdown,
    ProxyUtterancesToken,
    GetCurrentUser,
}

pub struct AppRouter {
    get: matchit::Router<Route>,
    post: matchit::Router<Route>,
    patch: matchit::Router<Route>,
    delete: matchit::Router<Route>,
}

impl Default for AppRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl AppRouter {
    pub fn new() -> Self {
        let mut router = Self {
            get: matchit::Router::new(),
            post: matchit::Router::new(),
            patch: matchit::Router::new(),
            delete: matchit::Router::new(),
        };

        router
            .get
            .insert(
                "/repos/:owner/:repo/issues/comments/:id/reactions",
                Route::ListReactions,
            )
            .unwrap();
        router
            .get
            .insert("/repos/:owner/:repo/issues/comments/:id", Route::GetComment)
            .unwrap();
        router
            .get
            .insert(
                "/repos/:owner/:repo/issues/:number/comments",
                Route::ListComments,
            )
            .unwrap();
        router
            .get
            .insert("/repos/:owner/:repo/issues/:number", Route::GetIssue)
            .unwrap();
        router
            .get
            .insert("/repos/:owner/:repo/issues", Route::ListIssues)
            .unwrap();
        router
            .get
            .insert("/repos/:owner/:repo/labels", Route::ListLabels)
            .unwrap();
        router
            .get
            .insert("/search/issues", Route::SearchIssues)
            .unwrap();
        router.get.insert("/user", Route::GetCurrentUser).unwrap();

        router
            .post
            .insert(
                "/repos/:owner/:repo/issues/comments/:id/reactions",
                Route::CreateReaction,
            )
            .unwrap();
        router
            .post
            .insert(
                "/repos/:owner/:repo/issues/:number/comments",
                Route::CreateComment,
            )
            .unwrap();
        router
            .post
            .insert("/repos/:owner/:repo/issues", Route::CreateIssue)
            .unwrap();
        router
            .post
            .insert("/repos/:owner/:repo/labels", Route::CreateLabel)
            .unwrap();
        router
            .post
            .insert("/markdown", Route::RenderMarkdown)
            .unwrap();
        router
            .post
            .insert("/api/utterances/token", Route::ProxyUtterancesToken)
            .unwrap();
        router
            .post
            .insert("/token", Route::ProxyUtterancesToken)
            .unwrap();

        router
            .patch
            .insert(
                "/repos/:owner/:repo/issues/comments/:id",
                Route::UpdateComment,
            )
            .unwrap();
        router
            .patch
            .insert("/repos/:owner/:repo/issues/:number", Route::UpdateIssue)
            .unwrap();

        router
            .delete
            .insert(
                "/repos/:owner/:repo/issues/comments/:id/reactions/:rid",
                Route::DeleteReaction,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/repos/:owner/:repo/issues/comments/:id",
                Route::DeleteComment,
            )
            .unwrap();

        router
    }

    pub async fn handle(&self, mut req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
        let table = match req.method.as_str() {
            "GET" => &self.get,
            "POST" => &self.post,
            "PATCH" => &self.patch,
            "DELETE" => &self.delete,
            "OPTIONS" => {
                return AppResponse::no_content()
                    .with_header("Allow", "GET,POST,PATCH,DELETE,OPTIONS")
            }
            _ => {
                return AppResponse::json(
                    405,
                    &serde_json::json!({"message": "Method Not Allowed"}),
                )
            }
        };

        let matched = match table.at(&req.path) {
            Ok(matched) => matched,
            Err(_) => return AppResponse::json(404, &serde_json::json!({"message": "Not Found"})),
        };

        req.path_params = matched
            .params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        match matched.value {
            Route::ListIssues => handlers::issues::list(req, ctx).await,
            Route::CreateIssue => handlers::issues::create(req, ctx).await,
            Route::GetIssue => handlers::issues::get(req, ctx).await,
            Route::UpdateIssue => handlers::issues::update(req, ctx).await,
            Route::ListComments => handlers::comments::list(req, ctx).await,
            Route::CreateComment => handlers::comments::create(req, ctx).await,
            Route::GetComment => handlers::comments::get(req, ctx).await,
            Route::UpdateComment => handlers::comments::update(req, ctx).await,
            Route::DeleteComment => handlers::comments::delete(req, ctx).await,
            Route::ListReactions => handlers::reactions::list(req, ctx).await,
            Route::CreateReaction => handlers::reactions::create(req, ctx).await,
            Route::DeleteReaction => handlers::reactions::delete(req, ctx).await,
            Route::ListLabels => handlers::labels::list(req, ctx).await,
            Route::CreateLabel => handlers::labels::create(req, ctx).await,
            Route::SearchIssues => handlers::search::search(req, ctx).await,
            Route::RenderMarkdown => handlers::render_markdown(req, ctx).await,
            Route::ProxyUtterancesToken => handlers::utterances::proxy_token(req, ctx).await,
            Route::GetCurrentUser => handlers::current_user(req, ctx).await,
        }
    }
}

pub fn parse_query_string(raw_query: Option<&str>) -> HashMap<String, String> {
    let mut query = HashMap::new();
    let Some(raw_query) = raw_query else {
        return query;
    };

    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };

        let key = urlencoding::decode(raw_key)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| raw_key.to_string());
        let value = urlencoding::decode(raw_value)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| raw_value.to_string());

        query.insert(key, value);
    }

    query
}
