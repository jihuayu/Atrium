use std::collections::HashMap;

use crate::{
    handlers::{header_value, respond},
    router::{AppRequest, AppResponse},
    AppContext,
};

pub async fn proxy_token(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(proxy_token_inner(req, ctx).await)
}

async fn proxy_token_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let mut headers = HashMap::new();

    // Keep the upstream request shape compatible with utterances' token exchange.
    headers.insert(
        "content-type".to_string(),
        header_value(&req, "content-type").unwrap_or_else(|| "application/json".to_string()),
    );

    for name in [
        "referer",
        "origin",
        "user-agent",
        "cookie",
        "sec-ch-ua",
        "sec-ch-ua-mobile",
        "sec-ch-ua-platform",
    ] {
        if let Some(value) = header_value(&req, name) {
            headers.insert(name.to_string(), value);
        }
    }

    let upstream = ctx.http.post_utterances_token(&req.body, &headers).await?;
    Ok(AppResponse {
        status: upstream.status,
        headers: upstream.headers,
        body: upstream.body,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;

    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::{Database, DbValue},
        router::AppRequest,
        types::GitHubApiUser,
        AppContext,
    };

    struct NoopDb;

    #[async_trait]
    impl Database for NoopDb {
        async fn execute(&self, _sql: &str, _params: &[DbValue]) -> crate::Result<u64> {
            Err(crate::ApiError::internal("not used"))
        }

        async fn query_opt_value(
            &self,
            _sql: &str,
            _params: &[DbValue],
        ) -> crate::Result<Option<serde_json::Value>> {
            Err(crate::ApiError::internal("not used"))
        }

        async fn query_all_value(
            &self,
            _sql: &str,
            _params: &[DbValue],
        ) -> crate::Result<Vec<serde_json::Value>> {
            Err(crate::ApiError::internal("not used"))
        }

        async fn batch(&self, _stmts: Vec<(&str, Vec<DbValue>)>) -> crate::Result<()> {
            Err(crate::ApiError::internal("not used"))
        }
    }

    struct MockHttp {
        seen_headers: Mutex<Option<HashMap<String, String>>>,
        seen_body: Mutex<Vec<u8>>,
    }

    impl MockHttp {
        fn new() -> Self {
            Self {
                seen_headers: Mutex::new(None),
                seen_body: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HttpClient for MockHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(crate::ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(crate::ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            body: &[u8],
            headers: &HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            *self
                .seen_headers
                .lock()
                .expect("lock headers") = Some(headers.clone());
            *self.seen_body.lock().expect("lock body") = body.to_vec();

            Ok(UpstreamResponse {
                status: 202,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: Bytes::from_static(br#"{"ok":true}"#),
            })
        }
    }

    #[tokio::test]
    async fn proxy_token_forwards_headers_and_body() {
        let db = NoopDb;
        let http = MockHttp::new();
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

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("origin".to_string(), "https://example.com".to_string());
        headers.insert("user-agent".to_string(), "xtalk-test".to_string());

        let req = AppRequest {
            method: "POST".to_string(),
            path: "/api/utterances/token".to_string(),
            path_params: HashMap::new(),
            query: HashMap::new(),
            headers,
            auth_header: None,
            accept: None,
            body: Bytes::from_static(br#"{"repo":"o/r"}"#),
        };

        let resp = super::proxy_token(req, &ctx).await;
        assert_eq!(resp.status, 202);
        assert_eq!(resp.body, Bytes::from_static(br#"{"ok":true}"#));
        assert!(
            resp.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );

        let seen_headers = http
            .seen_headers
            .lock()
            .expect("lock")
            .clone()
            .expect("seen headers");
        assert_eq!(
            seen_headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            seen_headers.get("origin").map(String::as_str),
            Some("https://example.com")
        );
        assert_eq!(
            seen_headers.get("user-agent").map(String::as_str),
            Some("xtalk-test")
        );

        let seen_body = http.seen_body.lock().expect("lock body").clone();
        assert_eq!(seen_body, br#"{"repo":"o/r"}"#);
    }
}
