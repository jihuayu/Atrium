use std::collections::HashMap;

use async_trait::async_trait;

use crate::{
    Result,
    auth::{HttpClient, UpstreamResponse},
    error::ApiError,
    types::{GitHubApiUser, GitHubUser},
};

#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
    github_user_url: String,
    utterances_token_url: String,
}

impl ReqwestHttpClient {
    pub fn new() -> Result<Self> {
        Self::with_urls(
            "https://api.github.com/user".to_string(),
            "https://api.utteranc.es/token".to_string(),
        )
    }

    fn with_urls(github_user_url: String, utterances_token_url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("atrium/0.1")
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ApiError::internal(format!("create reqwest client failed: {}", e)))?;
        Ok(Self {
            client,
            github_user_url,
            utterances_token_url,
        })
    }

    pub async fn health_check_user(&self, token: &str) -> Result<GitHubUser> {
        let user = self.get_github_user(token).await?;
        Ok(user.into())
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser> {
        let response = self
            .client
            .get(&self.github_user_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("github request failed: {}", e)))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::unauthorized());
        }

        if !response.status().is_success() {
            return Err(ApiError::new(
                response.status().as_u16(),
                format!("GitHub API error: {}", response.status()),
            ));
        }

        response
            .json::<GitHubApiUser>()
            .await
            .map_err(|e| ApiError::internal(format!("decode github user failed: {}", e)))
    }

    async fn post_utterances_token(
        &self,
        body: &[u8],
        headers: &HashMap<String, String>,
    ) -> Result<UpstreamResponse> {
        let mut request = self
            .client
            .post(&self.utterances_token_url)
            .body(body.to_vec())
            .header(
                "Content-Type",
                headers
                    .get("content-type")
                    .map(String::as_str)
                    .unwrap_or("application/json"),
            );

        for (key, name) in [
            ("referer", "Referer"),
            ("origin", "Origin"),
            ("user-agent", "User-Agent"),
            ("cookie", "Cookie"),
            ("sec-ch-ua", "Sec-CH-UA"),
            ("sec-ch-ua-mobile", "Sec-CH-UA-Mobile"),
            ("sec-ch-ua-platform", "Sec-CH-UA-Platform"),
        ] {
            if let Some(value) = headers.get(key) {
                request = request.header(name, value);
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("utterances token request failed: {}", e)))?;

        let status = response.status().as_u16();
        let mut response_headers = Vec::new();
        for (key, name) in [
            ("Content-Type", "Content-Type"),
            ("Cache-Control", "Cache-Control"),
            ("X-Frame-Options", "X-Frame-Options"),
            ("Content-Security-Policy", "Content-Security-Policy"),
        ] {
            if let Some(value) = response.headers().get(key).and_then(|v| v.to_str().ok()) {
                response_headers.push((name.to_string(), value.to_string()));
            }
        }

        let body = response.bytes().await.map_err(|e| {
            ApiError::internal(format!("read utterances token response failed: {}", e))
        })?;

        Ok(UpstreamResponse {
            status,
            headers: response_headers,
            body,
        })
    }

    async fn exchange_github_oauth_code(
        &self,
        code: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct TokenResponse {
            access_token: Option<String>,
            error: Option<String>,
            error_description: Option<String>,
        }

        let response = self
            .client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("github oauth exchange failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ApiError::new(
                response.status().as_u16(),
                format!("GitHub OAuth token exchange failed: {}", response.status()),
            ));
        }

        let token_response: TokenResponse = response.json().await.map_err(|e| {
            ApiError::internal(format!("decode github oauth response failed: {}", e))
        })?;

        if let Some(error) = token_response.error {
            let message = token_response.error_description.unwrap_or(error.clone());
            return Err(ApiError::bad_request(&format!(
                "GitHub OAuth error: {}",
                message
            )));
        }

        token_response
            .access_token
            .ok_or_else(|| ApiError::internal("GitHub OAuth response missing access_token"))
    }

    async fn get_jwks(&self, url: &str) -> Result<UpstreamResponse> {
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("jwks request failed: {}", e)))?;

        let status = response.status().as_u16();
        let mut response_headers = Vec::new();
        if let Some(value) = response
            .headers()
            .get("Cache-Control")
            .and_then(|v| v.to_str().ok())
        {
            response_headers.push(("Cache-Control".to_string(), value.to_string()));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("read jwks response failed: {}", e)))?;

        Ok(UpstreamResponse {
            status,
            headers: response_headers,
            body,
        })
    }

    async fn post_account_introspect(
        &self,
        base_url: &str,
        cookie_header: &str,
        internal_secret: Option<&str>,
        audience: &str,
    ) -> Result<UpstreamResponse> {
        let url = format!(
            "{}/internal/session/introspect",
            base_url.trim_end_matches('/')
        );
        let mut request = self
            .client
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Cookie", cookie_header)
            .json(&serde_json::json!({ "audience": audience }));
        if let Some(secret) = internal_secret.filter(|v| !v.trim().is_empty()) {
            request = request.header("x-internal-secret", secret);
        }
        let response = request
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("account introspection failed: {}", e)))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("read account response failed: {}", e)))?;
        Ok(UpstreamResponse {
            status,
            headers: Vec::new(),
            body,
        })
    }

    async fn get_url(&self, url: &str, accept: &str) -> Result<UpstreamResponse> {
        let response = self
            .client
            .get(url)
            .header("Accept", accept)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("http get failed: {}", e)))?;
        let status = response.status().as_u16();
        let mut response_headers = Vec::new();
        for key in ["Content-Type", "Location", "Cache-Control"] {
            if let Some(value) = response.headers().get(key).and_then(|v| v.to_str().ok()) {
                response_headers.push((key.to_string(), value.to_string()));
            }
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("read http response failed: {}", e)))?;
        Ok(UpstreamResponse {
            status,
            headers: response_headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::{
        Json, Router,
        body::Bytes,
        extract::Request,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };

    use crate::auth::HttpClient;

    use super::ReqwestHttpClient;

    async fn spawn_server() -> (String, tokio::task::AbortHandle) {
        async fn user_ok() -> impl IntoResponse {
            Json(serde_json::json!({
                "id": 7,
                "login": "alice",
                "email": "alice@test.com",
                "avatar_url": "https://avatars/a",
                "type": "User",
                "site_admin": false
            }))
        }

        async fn user_unauth() -> impl IntoResponse {
            StatusCode::UNAUTHORIZED
        }

        async fn user_fail() -> impl IntoResponse {
            StatusCode::INTERNAL_SERVER_ERROR
        }

        async fn user_bad_json() -> impl IntoResponse {
            (StatusCode::OK, "not-json")
        }

        async fn token_handler(req: Request) -> impl IntoResponse {
            let headers = req.headers();
            if headers.get("referer").is_none() || headers.get("origin").is_none() {
                return (StatusCode::BAD_REQUEST, "missing forwarded headers").into_response();
            }

            (
                StatusCode::CREATED,
                [
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "max-age=60"),
                    ("X-Frame-Options", "DENY"),
                    ("Content-Security-Policy", "frame-ancestors 'none'"),
                ],
                Bytes::from_static(br#"{"token":"ok"}"#),
            )
                .into_response()
        }

        async fn jwks_handler() -> impl IntoResponse {
            (
                StatusCode::OK,
                [("Cache-Control", "max-age=120")],
                Bytes::from_static(br#"{"keys":[]}"#),
            )
                .into_response()
        }

        let app = Router::new()
            .route("/user-ok", get(user_ok))
            .route("/user-unauth", get(user_unauth))
            .route("/user-fail", get(user_fail))
            .route("/user-bad-json", get(user_bad_json))
            .route("/token", post(token_handler))
            .route("/jwks", get(jwks_handler));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        (format!("http://{}", addr), handle.abort_handle())
    }

    #[tokio::test]
    async fn github_user_paths() {
        let (base, _handle) = spawn_server().await;

        let ok_client =
            ReqwestHttpClient::with_urls(format!("{}/user-ok", base), format!("{}/token", base))
                .expect("create client");
        let user = ok_client
            .get_github_user("token")
            .await
            .expect("github user ok");
        assert_eq!(user.login, "alice");

        let unauth_client = ReqwestHttpClient::with_urls(
            format!("{}/user-unauth", base),
            format!("{}/token", base),
        )
        .expect("create client");
        let err = unauth_client
            .get_github_user("token")
            .await
            .err()
            .expect("must unauthorized");
        assert_eq!(err.status, 401);

        let fail_client =
            ReqwestHttpClient::with_urls(format!("{}/user-fail", base), format!("{}/token", base))
                .expect("create client");
        let err = fail_client
            .get_github_user("token")
            .await
            .err()
            .expect("must fail");
        assert_eq!(err.status, 500);

        let bad_json_client = ReqwestHttpClient::with_urls(
            format!("{}/user-bad-json", base),
            format!("{}/token", base),
        )
        .expect("create client");
        let err = bad_json_client
            .get_github_user("token")
            .await
            .err()
            .expect("must fail decode");
        assert_eq!(err.status, 500);
    }

    #[tokio::test]
    async fn utterances_and_jwks_paths() {
        let (base, _handle) = spawn_server().await;
        let client =
            ReqwestHttpClient::with_urls(format!("{}/user-ok", base), format!("{}/token", base))
                .expect("create client");

        let mut forwarded = HashMap::new();
        forwarded.insert("content-type".to_string(), "application/json".to_string());
        forwarded.insert("referer".to_string(), "https://x.test".to_string());
        forwarded.insert("origin".to_string(), "https://x.test".to_string());
        forwarded.insert("user-agent".to_string(), "atrium-test".to_string());
        forwarded.insert("cookie".to_string(), "a=1".to_string());
        forwarded.insert("sec-ch-ua".to_string(), "ua".to_string());
        forwarded.insert("sec-ch-ua-mobile".to_string(), "?0".to_string());
        forwarded.insert("sec-ch-ua-platform".to_string(), "\"Windows\"".to_string());

        let upstream = client
            .post_utterances_token(br#"{"repo":"o/r"}"#, &forwarded)
            .await
            .expect("post token");
        assert_eq!(upstream.status, 201);
        assert_eq!(upstream.body, Bytes::from_static(br#"{"token":"ok"}"#));
        assert!(
            upstream
                .headers
                .iter()
                .any(|(k, v)| k == "Cache-Control" && v == "max-age=60")
        );

        let jwks = client
            .get_jwks(&format!("{}/jwks", base))
            .await
            .expect("get jwks");
        assert_eq!(jwks.status, 200);
        assert!(
            jwks.headers
                .iter()
                .any(|(k, v)| k == "Cache-Control" && v == "max-age=120")
        );

        let missing = client
            .get_jwks(&format!("{}/missing-jwks", base))
            .await
            .expect("upstream response still returned");
        assert_eq!(missing.status, 404);
    }

    #[tokio::test]
    async fn health_check_and_send_error_paths() {
        let (base, _handle) = spawn_server().await;

        let ok_client =
            ReqwestHttpClient::with_urls(format!("{}/user-ok", base), format!("{}/token", base))
                .expect("create client");
        let user = ok_client
            .health_check_user("token")
            .await
            .expect("health check ok");
        assert_eq!(user.login, "alice");

        let fail_user_client =
            ReqwestHttpClient::with_urls(format!("{}/user-fail", base), format!("{}/token", base))
                .expect("create client");
        let user_err = fail_user_client
            .get_github_user("token")
            .await
            .err()
            .expect("github response must fail");
        assert_eq!(user_err.status, 500);

        let missing_token_client = ReqwestHttpClient::with_urls(
            format!("{}/user-ok", base),
            format!("{}/missing-token", base),
        )
        .expect("create client");
        let token_missing = missing_token_client
            .post_utterances_token(br#"{}"#, &HashMap::new())
            .await
            .expect("upstream response still returned");
        assert_eq!(token_missing.status, 404);

        let bad_headers_upstream = ok_client
            .post_utterances_token(br#"{}"#, &HashMap::new())
            .await
            .expect("upstream response still returned");
        assert_eq!(bad_headers_upstream.status, 400);
    }
}
