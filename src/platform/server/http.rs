use std::collections::HashMap;

use async_trait::async_trait;

use crate::{
    auth::{HttpClient, UpstreamResponse},
    error::ApiError,
    types::{GitHubApiUser, GitHubUser},
    Result,
};

#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("xtalk/0.1")
            .build()
            .map_err(|e| ApiError::internal(format!("create reqwest client failed: {}", e)))?;
        Ok(Self { client })
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
            .get("https://api.github.com/user")
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
            .post("https://api.utteranc.es/token")
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
}
