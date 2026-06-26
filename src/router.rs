use std::collections::HashMap;

use bytes::Bytes;
use serde::Serialize;

use crate::{ApiError, AppContext, handlers};

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

    pub fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            headers: vec![("Location".to_string(), location.to_string())],
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
    DocsDiscovery,
    ApiAuthAccount,
    ApiAuthAccountAuthorize,
    ApiAuthAccountCallback,
    ApiDiscoveryPublicKey,
    ApiCreateWebsite,
    ApiListWebsites,
    ApiGetWebsite,
    ApiUpdateWebsite,
    ApiListWebsiteAdmins,
    ApiAddWebsiteAdmin,
    ApiRemoveWebsiteAdmin,
    ApiUpsertPage,
    ApiListPages,
    ApiGetPage,
    ApiListPageComments,
    ApiCreatePageComment,
    ApiUpdateNativeComment,
    ApiDeleteNativeComment,
    ApiSetNativeReaction,
    ApiDeleteNativeReaction,
    ApiCurrentComments,
    ApiCreateCurrentComment,
    ApiCurrentReplies,
    ApiSetCurrentReaction,
    ApiDeleteCurrentReaction,
    ApiModerationComments,
    ApiBanUser,
    ApiListBans,
    ApiUnbanUser,
    ApiAuthRefresh,
    ApiAuthSessionDelete,
    ApiAuthMe,
    Root,
}

pub struct AppRouter {
    get: matchit::Router<Route>,
    post: matchit::Router<Route>,
    put: matchit::Router<Route>,
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
            put: matchit::Router::new(),
            patch: matchit::Router::new(),
            delete: matchit::Router::new(),
        };

        router.get.insert("/", Route::Root).unwrap();
        router
            .get
            .insert("/docs/discovery", Route::DocsDiscovery)
            .unwrap();
        router
            .get
            .insert("/api/v1/auth/me", Route::ApiAuthMe)
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/auth/account/authorize",
                Route::ApiAuthAccountAuthorize,
            )
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/auth/account/callback",
                Route::ApiAuthAccountCallback,
            )
            .unwrap();
        router
            .get
            .insert("/api/v1/discovery/public-key", Route::ApiDiscoveryPublicKey)
            .unwrap();
        router
            .get
            .insert("/api/v1/websites", Route::ApiListWebsites)
            .unwrap();
        router
            .get
            .insert("/api/v1/websites/{websiteKey}", Route::ApiGetWebsite)
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/websites/{websiteKey}/admins",
                Route::ApiListWebsiteAdmins,
            )
            .unwrap();
        router
            .get
            .insert("/api/v1/websites/{websiteKey}/pages", Route::ApiListPages)
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/websites/{websiteKey}/pages/{pageKey}",
                Route::ApiGetPage,
            )
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/websites/{websiteKey}/pages/{pageKey}/comments",
                Route::ApiListPageComments,
            )
            .unwrap();
        router
            .get
            .insert("/api/v1/comments/current", Route::ApiCurrentComments)
            .unwrap();
        router
            .get
            .insert("/api/v1/comments/current/replies", Route::ApiCurrentReplies)
            .unwrap();
        router
            .get
            .insert(
                "/api/v1/websites/{websiteKey}/admin/comments",
                Route::ApiModerationComments,
            )
            .unwrap();
        router
            .get
            .insert("/api/v1/websites/{websiteKey}/bans", Route::ApiListBans)
            .unwrap();

        router
            .post
            .insert("/api/v1/auth/account", Route::ApiAuthAccount)
            .unwrap();
        router
            .post
            .insert("/api/v1/auth/refresh", Route::ApiAuthRefresh)
            .unwrap();
        router
            .post
            .insert("/api/v1/websites", Route::ApiCreateWebsite)
            .unwrap();
        router
            .post
            .insert(
                "/api/v1/websites/{websiteKey}/admins",
                Route::ApiAddWebsiteAdmin,
            )
            .unwrap();
        router
            .post
            .insert(
                "/api/v1/websites/{websiteKey}/pages/{pageKey}/comments",
                Route::ApiCreatePageComment,
            )
            .unwrap();
        router
            .post
            .insert("/api/v1/comments/current", Route::ApiCreateCurrentComment)
            .unwrap();
        router
            .post
            .insert("/api/v1/websites/{websiteKey}/bans", Route::ApiBanUser)
            .unwrap();

        router
            .put
            .insert(
                "/api/v1/websites/{websiteKey}/pages/{pageKey}",
                Route::ApiUpsertPage,
            )
            .unwrap();
        router
            .put
            .insert(
                "/api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}",
                Route::ApiSetNativeReaction,
            )
            .unwrap();
        router
            .put
            .insert(
                "/api/v1/comments/current/{commentId}/reactions/{content}",
                Route::ApiSetCurrentReaction,
            )
            .unwrap();

        router
            .patch
            .insert("/api/v1/websites/{websiteKey}", Route::ApiUpdateWebsite)
            .unwrap();
        router
            .patch
            .insert(
                "/api/v1/websites/{websiteKey}/comments/{commentId}",
                Route::ApiUpdateNativeComment,
            )
            .unwrap();

        router
            .delete
            .insert("/api/v1/auth/session", Route::ApiAuthSessionDelete)
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/websites/{websiteKey}/admins/{userId}",
                Route::ApiRemoveWebsiteAdmin,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/websites/{websiteKey}/comments/{commentId}",
                Route::ApiDeleteNativeComment,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}",
                Route::ApiDeleteNativeReaction,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/comments/current/{commentId}/reactions/{content}",
                Route::ApiDeleteCurrentReaction,
            )
            .unwrap();
        router
            .delete
            .insert(
                "/api/v1/websites/{websiteKey}/bans/{userId}",
                Route::ApiUnbanUser,
            )
            .unwrap();

        router
    }

    pub async fn handle(&self, mut req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
        let table = match req.method.as_str() {
            "GET" => &self.get,
            "POST" => &self.post,
            "PUT" => &self.put,
            "PATCH" => &self.patch,
            "DELETE" => &self.delete,
            "OPTIONS" => {
                return AppResponse::no_content()
                    .with_header("Allow", "GET,POST,PUT,PATCH,DELETE,OPTIONS");
            }
            _ => {
                return AppResponse::json(
                    405,
                    &serde_json::json!({"message": "Method Not Allowed"}),
                );
            }
        };

        let matched = match table.at(&req.path) {
            Ok(matched) => matched,
            Err(_) => {
                return AppResponse::json(
                    404,
                    &serde_json::json!({"error": "not_found", "message": "Not found"}),
                );
            }
        };

        req.path_params = matched
            .params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        match matched.value {
            Route::DocsDiscovery => handlers::api::native::docs_discovery(req, ctx).await,
            Route::ApiAuthAccount => handlers::api::native::auth_account(req, ctx).await,
            Route::ApiAuthAccountAuthorize => {
                handlers::api::native::auth_account_authorize(req, ctx).await
            }
            Route::ApiAuthAccountCallback => {
                handlers::api::native::auth_account_callback(req, ctx).await
            }
            Route::ApiDiscoveryPublicKey => {
                handlers::api::native::discovery_public_key(req, ctx).await
            }
            Route::ApiCreateWebsite => handlers::api::native::create_website(req, ctx).await,
            Route::ApiListWebsites => handlers::api::native::list_websites(req, ctx).await,
            Route::ApiGetWebsite => handlers::api::native::get_website(req, ctx).await,
            Route::ApiUpdateWebsite => handlers::api::native::update_website(req, ctx).await,
            Route::ApiListWebsiteAdmins => {
                handlers::api::native::list_website_admins(req, ctx).await
            }
            Route::ApiAddWebsiteAdmin => handlers::api::native::add_website_admin(req, ctx).await,
            Route::ApiRemoveWebsiteAdmin => {
                handlers::api::native::remove_website_admin(req, ctx).await
            }
            Route::ApiUpsertPage => handlers::api::native::upsert_page(req, ctx).await,
            Route::ApiListPages => handlers::api::native::list_pages(req, ctx).await,
            Route::ApiGetPage => handlers::api::native::get_page(req, ctx).await,
            Route::ApiListPageComments => handlers::api::native::list_page_comments(req, ctx).await,
            Route::ApiCreatePageComment => {
                handlers::api::native::create_page_comment(req, ctx).await
            }
            Route::ApiUpdateNativeComment => handlers::api::native::update_comment(req, ctx).await,
            Route::ApiDeleteNativeComment => handlers::api::native::delete_comment(req, ctx).await,
            Route::ApiSetNativeReaction => handlers::api::native::set_reaction(req, ctx).await,
            Route::ApiDeleteNativeReaction => {
                handlers::api::native::delete_reaction(req, ctx).await
            }
            Route::ApiCurrentComments => handlers::api::native::current_comments(req, ctx).await,
            Route::ApiCreateCurrentComment => {
                handlers::api::native::create_current_comment(req, ctx).await
            }
            Route::ApiCurrentReplies => handlers::api::native::current_replies(req, ctx).await,
            Route::ApiSetCurrentReaction => {
                handlers::api::native::set_current_reaction(req, ctx).await
            }
            Route::ApiDeleteCurrentReaction => {
                handlers::api::native::delete_current_reaction(req, ctx).await
            }
            Route::ApiModerationComments => {
                handlers::api::native::moderation_comments(req, ctx).await
            }
            Route::ApiBanUser => handlers::api::native::ban_user(req, ctx).await,
            Route::ApiListBans => handlers::api::native::list_bans(req, ctx).await,
            Route::ApiUnbanUser => handlers::api::native::unban_user(req, ctx).await,
            Route::ApiAuthRefresh => handlers::api::native::auth_refresh(req, ctx).await,
            Route::ApiAuthSessionDelete => {
                handlers::api::native::auth_session_delete(req, ctx).await
            }
            Route::ApiAuthMe => handlers::api::native::auth_me(req, ctx).await,
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

    use super::{AppRequest, AppRouter, parse_query_string};
    use crate::{
        AppContext,
        auth::{HttpClient, UpstreamResponse},
        db::{Database, DbValue},
        error::ApiError,
        types::GitHubApiUser,
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
        assert!(router.get.at("/repos/jihuayu/utterances/issues/1").is_err());
        assert!(router.post.at("/repos/jihuayu/utterances/issues").is_err());
        assert!(
            router
                .get
                .at("/api/v1/repos/jihuayu/utterances/threads")
                .is_err()
        );
        assert!(router.post.at("/api/v1/auth/github").is_err());
        assert!(router.post.at("/api/v1/auth/account").is_ok());
        assert!(router.get.at("/api/v1/auth/account/authorize").is_ok());
        assert!(
            router
                .get
                .at("/api/v1/websites/example/pages/post/comments")
                .is_ok()
        );
        assert!(router.put.at("/api/v1/websites/example/pages/post").is_ok());
        assert!(router.get.at("/api/v1/comments/current").is_ok());
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
            github_client_id: None,
            github_client_secret: None,
            account_base_url: None,
            account_audience: None,
            account_internal_secret: None,
            super_admin_account_ids: None,
            discovery_private_jwk: None,
            discovery_public_jwk: None,
            discovery_key_id: None,
            test_discovery_well_known: None,
            test_discovery_dns_txt: None,
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
        assert!(
            options_resp
                .headers
                .iter()
                .any(|(k, v)| k == "Allow" && v == "GET,POST,PUT,PATCH,DELETE,OPTIONS")
        );

        let put_not_found = router
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
        assert_eq!(put_not_found.status, 404);

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
        let body = serde_json::from_slice::<serde_json::Value>(&not_found.body).unwrap();
        assert_eq!(body["error"], "not_found");
        assert_eq!(body["message"], "Not found");

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
        assert_eq!(api_auth.status, 404);

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
        assert_eq!(utterances.status, 404);

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
