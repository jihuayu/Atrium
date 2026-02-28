use std::collections::HashMap;

use async_trait::async_trait;
#[cfg(target_arch = "wasm32")]
use worker::{wasm_bindgen::JsValue, Fetch, Headers, Method, Request, RequestInit};

use crate::{
    auth::{HttpClient, UpstreamResponse},
    error::ApiError,
    types::GitHubApiUser,
    Result,
};

#[derive(Default)]
pub struct WorkerHttpClient;

#[cfg(target_arch = "wasm32")]
#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
impl HttpClient for WorkerHttpClient {
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser> {
        let headers = Headers::new();
        headers
            .set("Authorization", &format!("Bearer {}", token))
            .map_err(|e| ApiError::internal(format!("set auth header failed: {}", e)))?;
        headers
            .set("Accept", "application/vnd.github+json")
            .map_err(|e| ApiError::internal(format!("set accept header failed: {}", e)))?;
        headers
            .set("User-Agent", "atrium/0.1")
            .map_err(|e| ApiError::internal(format!("set ua header failed: {}", e)))?;

        let mut init = RequestInit::new();
        init.with_method(Method::Get).with_headers(headers);

        let request = Request::new_with_init("https://api.github.com/user", &init)
            .map_err(|e| ApiError::internal(format!("build request failed: {}", e)))?;
        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("github fetch failed: {}", e)))?;

        if response.status_code() == 401 {
            return Err(ApiError::unauthorized());
        }
        if !(200..=299).contains(&response.status_code()) {
            return Err(ApiError::new(
                response.status_code(),
                format!("GitHub API error: {}", response.status_code()),
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
        let payload =
            std::str::from_utf8(body).map_err(|_| ApiError::bad_request("Invalid UTF-8 body"))?;

        let request_headers = Headers::new();
        request_headers
            .set(
                "Content-Type",
                headers
                    .get("content-type")
                    .map(String::as_str)
                    .unwrap_or("application/json"),
            )
            .map_err(|e| ApiError::internal(format!("set content-type header failed: {}", e)))?;

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
                request_headers.set(name, value).map_err(|e| {
                    ApiError::internal(format!("set {} header failed: {}", name, e))
                })?;
            }
        }

        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(request_headers)
            .with_body(Some(JsValue::from_str(payload)));

        let request = Request::new_with_init("https://api.utteranc.es/token", &init)
            .map_err(|e| ApiError::internal(format!("build request failed: {}", e)))?;
        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("utterances token fetch failed: {}", e)))?;

        let status = response.status_code();
        let mut response_headers = Vec::new();
        for name in [
            "Content-Type",
            "Cache-Control",
            "X-Frame-Options",
            "Content-Security-Policy",
        ] {
            if let Some(value) = response.headers().get(name).ok().flatten() {
                response_headers.push((name.to_string(), value));
            }
        }

        let body = response.bytes().await.map_err(|e| {
            ApiError::internal(format!("read utterances token response failed: {}", e))
        })?;

        Ok(UpstreamResponse {
            status,
            headers: response_headers,
            body: bytes::Bytes::from(body),
        })
    }

    async fn get_jwks(&self, url: &str) -> Result<UpstreamResponse> {
        let request_headers = Headers::new();
        request_headers
            .set("Accept", "application/json")
            .map_err(|e| ApiError::internal(format!("set accept header failed: {}", e)))?;

        let mut init = RequestInit::new();
        init.with_method(Method::Get).with_headers(request_headers);

        let request = Request::new_with_init(url, &init)
            .map_err(|e| ApiError::internal(format!("build jwks request failed: {}", e)))?;
        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("jwks fetch failed: {}", e)))?;

        let status = response.status_code();
        let mut response_headers = Vec::new();
        if let Some(value) = response.headers().get("Cache-Control").ok().flatten() {
            response_headers.push(("Cache-Control".to_string(), value));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("read jwks response failed: {}", e)))?;

        Ok(UpstreamResponse {
            status,
            headers: response_headers,
            body: bytes::Bytes::from(body),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
impl HttpClient for WorkerHttpClient {
    async fn get_github_user(&self, _token: &str) -> Result<GitHubApiUser> {
        Err(ApiError::internal(
            "worker http client only supports wasm32 target",
        ))
    }

    async fn post_utterances_token(
        &self,
        body: &[u8],
        _headers: &HashMap<String, String>,
    ) -> Result<UpstreamResponse> {
        if std::str::from_utf8(body).is_err() {
            return Err(ApiError::bad_request("Invalid UTF-8 body"));
        }
        Err(ApiError::internal(
            "worker http client only supports wasm32 target",
        ))
    }

    async fn get_jwks(&self, _url: &str) -> Result<UpstreamResponse> {
        Err(ApiError::internal(
            "worker http client only supports wasm32 target",
        ))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::HashMap;

    use crate::auth::HttpClient;

    use super::WorkerHttpClient;

    #[tokio::test]
    async fn non_wasm_stub_returns_expected_errors() {
        let http = WorkerHttpClient;

        let err = http
            .get_github_user("token")
            .await
            .err()
            .expect("stub must fail");
        assert_eq!(err.status, 500);

        let bad_utf8 = http
            .post_utterances_token(&[0xff], &HashMap::new())
            .await
            .err()
            .expect("bad utf8");
        assert_eq!(bad_utf8.status, 400);

        let post_err = http
            .post_utterances_token(br#"{}"#, &HashMap::new())
            .await
            .err()
            .expect("stub must fail");
        assert_eq!(post_err.status, 500);

        let jwks_err = http
            .get_jwks("https://example.com/jwks")
            .await
            .err()
            .expect("stub must fail");
        assert_eq!(jwks_err.status, 500);
    }
}
