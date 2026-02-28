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
    ApiAuthGithub,
    ApiAuthGoogle,
    ApiAuthApple,
    ApiAuthRefresh,
    ApiAuthSessionDelete,
    ApiAuthMe,
    ApiListThreads,
    ApiCreateThread,
    ApiGetThread,
    ApiUpdateThread,
    ApiDeleteThread,
    ApiListComments,
    ApiCreateComment,
    ApiGetComment,
    ApiUpdateComment,
    ApiDeleteComment,
    ApiCreateReaction,
    ApiDeleteReaction,
    ApiListLabels,
    ApiCreateLabel,
    ApiDeleteLabel,
    ApiExportRepo,
    ApiGetRepoSettings,
    ApiUpdateRepoSettings,
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
    ExportUserRepos,
    Root,
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

        router.get.insert("/", Route::Root).unwrap();
        router
            .get
            .insert("/api/v1/auth/me", Route::ApiAuthMe)
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads",
                Route::ApiListThreads,
            )
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads/{number}",
                Route::ApiGetThread,
            )
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads/{number}/comments",
                Route::ApiListComments,
            )
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/repos/{owner}/{repo}/comments/{id}",
                Route::ApiGetComment,
            )
            .unwrap();
        router
            .get
            .insert("/api/v1/repos/{owner}/{repo}/labels", Route::ApiListLabels)
            .unwrap();
        router
            .get
            .insert("/api/v1/repos/{owner}/{repo}/export", Route::ApiExportRepo)
            .unwrap();
        router
            .get
            .insert("/api/v1/repos/{owner}/{repo}", Route::ApiGetRepoSettings)
            .unwrap();
        router
            .get
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}/reactions",
                Route::ListReactions,
            )
            .unwrap();
        router
            .get
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}",
                Route::GetComment,
            )
            .unwrap();
        router
            .get
            .insert("/repos/{owner}/{repo}/issues", Route::ListIssues)
            .unwrap();
        router
            .get
            .insert("/repos/{owner}/{repo}/issues/{number}", Route::GetIssue)
            .unwrap();
        router
            .get
            .insert(
                "/repos/{owner}/{repo}/issues/{number}/comments",
                Route::ListComments,
            )
            .unwrap();
        router
            .get
            .insert("/repos/{owner}/{repo}/labels", Route::ListLabels)
            .unwrap();
        router
            .get
            .insert("/search/issues", Route::SearchIssues)
            .unwrap();
        router.get.insert("/user", Route::GetCurrentUser).unwrap();
        router
            .get
            .insert("/user/export", Route::ExportUserRepos)
            .unwrap();

        router
            .post
            .insert("/api/v1/auth/github", Route::ApiAuthGithub)
            .unwrap();
        router
            .post
            .insert("/api/v1/auth/google", Route::ApiAuthGoogle)
            .unwrap();
        router
            .post
            .insert("/api/v1/auth/apple", Route::ApiAuthApple)
            .unwrap();
        router
            .post
            .insert("/api/v1/auth/refresh", Route::ApiAuthRefresh)
            .unwrap();
        router
            .post
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads",
                Route::ApiCreateThread,
            )
            .unwrap();
        router
            .post
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads/{number}/comments",
                Route::ApiCreateComment,
            )
            .unwrap();
        router
            .post
            .insert(
                "/api/v1/repos/{owner}/{repo}/comments/{id}/reactions",
                Route::ApiCreateReaction,
            )
            .unwrap();
        router
            .post
            .insert("/api/v1/repos/{owner}/{repo}/labels", Route::ApiCreateLabel)
            .unwrap();
        router
            .post
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}/reactions",
                Route::CreateReaction,
            )
            .unwrap();
        router
            .post
            .insert("/repos/{owner}/{repo}/issues", Route::CreateIssue)
            .unwrap();
        router
            .post
            .insert(
                "/repos/{owner}/{repo}/issues/{number}/comments",
                Route::CreateComment,
            )
            .unwrap();
        router
            .post
            .insert("/repos/{owner}/{repo}/labels", Route::CreateLabel)
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
                "/api/v1/repos/{owner}/{repo}/threads/{number}",
                Route::ApiUpdateThread,
            )
            .unwrap();
        router
            .patch
            .insert(
                "/api/v1/repos/{owner}/{repo}/comments/{id}",
                Route::ApiUpdateComment,
            )
            .unwrap();
        router
            .patch
            .insert("/api/v1/repos/{owner}/{repo}", Route::ApiUpdateRepoSettings)
            .unwrap();
        router
            .patch
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}",
                Route::UpdateComment,
            )
            .unwrap();
        router
            .patch
            .insert("/repos/{owner}/{repo}/issues/{number}", Route::UpdateIssue)
            .unwrap();

        router
            .delete
            .insert("/api/v1/auth/session", Route::ApiAuthSessionDelete)
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/repos/{owner}/{repo}/threads/{number}",
                Route::ApiDeleteThread,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/repos/{owner}/{repo}/comments/{id}",
                Route::ApiDeleteComment,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/repos/{owner}/{repo}/comments/{id}/reactions/{content}",
                Route::ApiDeleteReaction,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/repos/{owner}/{repo}/labels/{name}",
                Route::ApiDeleteLabel,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}/reactions/{rid}",
                Route::DeleteReaction,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/repos/{owner}/{repo}/issues/comments/{id}",
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
            Route::ApiAuthGithub => handlers::api::auth::github(req, ctx).await,
            Route::ApiAuthGoogle => handlers::api::auth::google(req, ctx).await,
            Route::ApiAuthApple => handlers::api::auth::apple(req, ctx).await,
            Route::ApiAuthRefresh => handlers::api::auth::refresh(req, ctx).await,
            Route::ApiAuthSessionDelete => handlers::api::auth::session_delete(req, ctx).await,
            Route::ApiAuthMe => handlers::api::auth::me(req, ctx).await,
            Route::ApiListThreads => handlers::api::threads::list(req, ctx).await,
            Route::ApiCreateThread => handlers::api::threads::create(req, ctx).await,
            Route::ApiGetThread => handlers::api::threads::get(req, ctx).await,
            Route::ApiUpdateThread => handlers::api::threads::update(req, ctx).await,
            Route::ApiDeleteThread => handlers::api::threads::delete(req, ctx).await,
            Route::ApiListComments => handlers::api::comments::list(req, ctx).await,
            Route::ApiCreateComment => handlers::api::comments::create(req, ctx).await,
            Route::ApiGetComment => handlers::api::comments::get(req, ctx).await,
            Route::ApiUpdateComment => handlers::api::comments::update(req, ctx).await,
            Route::ApiDeleteComment => handlers::api::comments::delete(req, ctx).await,
            Route::ApiCreateReaction => handlers::api::reactions::create(req, ctx).await,
            Route::ApiDeleteReaction => handlers::api::reactions::delete(req, ctx).await,
            Route::ApiListLabels => handlers::api::labels::list(req, ctx).await,
            Route::ApiCreateLabel => handlers::api::labels::create(req, ctx).await,
            Route::ApiDeleteLabel => handlers::api::labels::delete(req, ctx).await,
            Route::ApiExportRepo => handlers::api::export::get(req, ctx).await,
            Route::ApiGetRepoSettings => handlers::api::admin::get(req, ctx).await,
            Route::ApiUpdateRepoSettings => handlers::api::admin::update(req, ctx).await,
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
            Route::ExportUserRepos => handlers::exports::export_user_repos(req, ctx).await,
            Route::Root => handlers::root(req, ctx).await,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;

    use super::{parse_query_string, AppRequest, AppRouter};
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::{Database, DbValue},
        error::ApiError,
        types::GitHubApiUser,
        AppContext,
    };

    struct NoopDb;

    #[cfg_attr(feature = "server", async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait(?Send))]
    impl Database for NoopDb {
        async fn execute(&self, _sql: &str, _params: &[DbValue]) -> crate::Result<u64> {
            Err(ApiError::internal("not used"))
        }

        async fn query_opt_value(
            &self,
            _sql: &str,
            _params: &[DbValue],
        ) -> crate::Result<Option<serde_json::Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn query_all_value(
            &self,
            _sql: &str,
            _params: &[DbValue],
        ) -> crate::Result<Vec<serde_json::Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn batch(&self, _stmts: Vec<(&str, Vec<DbValue>)>) -> crate::Result<()> {
            Err(ApiError::internal("not used"))
        }
    }

    struct NoopHttp;

    #[cfg_attr(feature = "server", async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait(?Send))]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            Ok(UpstreamResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::new(),
            })
        }
    }

    #[test]
    fn matches_repo_routes() {
        let router = AppRouter::new();
        let issue = router.get.at("/repos/jihuayu/utterances/issues/1");
        assert!(issue.is_ok());
        let list = router.get.at("/repos/jihuayu/utterances/issues");
        assert!(list.is_ok());
        let create = router.post.at("/repos/jihuayu/utterances/issues");
        assert!(create.is_ok());
        let comments = router.get.at("/repos/jihuayu/utterances/issues/1/comments");
        assert!(comments.is_ok());
        let export = router.get.at("/user/export");
        assert!(export.is_ok());
        let api_threads = router.get.at("/api/v1/repos/jihuayu/utterances/threads");
        assert!(api_threads.is_ok());
        let api_auth = router.post.at("/api/v1/auth/github");
        assert!(api_auth.is_ok());
    }

    #[tokio::test]
    async fn handle_options_method_not_found_and_query_parse() {
        struct FailSerialize;
        impl serde::Serialize for FailSerialize {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("fail"))
            }
        }

        let serialization_error = super::AppResponse::json(200, &FailSerialize);
        assert_eq!(serialization_error.status, 500);

        let router = AppRouter::default();
        let db = NoopDb;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        let ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: &secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: false,
            test_bypass_secret: None,
        };

        let options_resp = router
            .handle(
                AppRequest {
                    method: "OPTIONS".to_string(),
                    path: "/repos/o/r/issues".to_string(),
                    path_params: HashMap::new(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    auth_header: None,
                    accept: None,
                    body: Bytes::new(),
                },
                &ctx,
            )
            .await;
        assert_eq!(options_resp.status, 204);
        assert!(options_resp.headers.iter().any(|(k, _)| k == "Allow"));

        let method_not_allowed = router
            .handle(
                AppRequest {
                    method: "PUT".to_string(),
                    path: "/repos/o/r/issues".to_string(),
                    path_params: HashMap::new(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    auth_header: None,
                    accept: None,
                    body: Bytes::new(),
                },
                &ctx,
            )
            .await;
        assert_eq!(method_not_allowed.status, 405);

        let not_found = router
            .handle(
                AppRequest {
                    method: "GET".to_string(),
                    path: "/not-found".to_string(),
                    path_params: HashMap::new(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    auth_header: None,
                    accept: None,
                    body: Bytes::new(),
                },
                &ctx,
            )
            .await;
        assert_eq!(not_found.status, 404);

        let api_auth = router
            .handle(
                AppRequest {
                    method: "POST".to_string(),
                    path: "/api/v1/auth/github".to_string(),
                    path_params: HashMap::new(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    auth_header: None,
                    accept: None,
                    body: Bytes::from_static(br#"{"token":"x"}"#),
                },
                &ctx,
            )
            .await;
        assert!(api_auth.status == 200 || api_auth.status == 500 || api_auth.status == 401);

        let utterances = router
            .handle(
                AppRequest {
                    method: "POST".to_string(),
                    path: "/api/utterances/token".to_string(),
                    path_params: HashMap::new(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    auth_header: None,
                    accept: None,
                    body: Bytes::from_static(br#"{}"#),
                },
                &ctx,
            )
            .await;
        assert_eq!(utterances.status, 200);

        let db_err = db.execute("select 1", &[]).await.err().expect("db execute");
        assert_eq!(db_err.status, 500);
        let db_opt_err = db
            .query_opt_value("select 1", &[])
            .await
            .err()
            .expect("db query_opt");
        assert_eq!(db_opt_err.status, 500);
        let db_all_err = db
            .query_all_value("select 1", &[])
            .await
            .err()
            .expect("db query_all");
        assert_eq!(db_all_err.status, 500);
        let db_batch_err = db.batch(Vec::new()).await.err().expect("db batch");
        assert_eq!(db_batch_err.status, 500);

        let gh_err = http
            .get_github_user("token")
            .await
            .err()
            .expect("http github");
        assert_eq!(gh_err.status, 500);
        let jwks_err = http
            .get_jwks("https://example.com/jwks")
            .await
            .err()
            .expect("http jwks");
        assert_eq!(jwks_err.status, 500);
        let utterances_ok = http
            .post_utterances_token(&[], &HashMap::new())
            .await
            .expect("http utterances");
        assert_eq!(utterances_ok.status, 200);

        let parsed = parse_query_string(Some("a=1&b&x=%2Fv&&"));
        assert_eq!(parsed.get("a").map(String::as_str), Some("1"));
        assert_eq!(parsed.get("b").map(String::as_str), Some(""));
        assert_eq!(parsed.get("x").map(String::as_str), Some("/v"));
    }
}
