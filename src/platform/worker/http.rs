use async_trait::async_trait;

use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::{auth::HttpClient, error::ApiError, types::GitHubApiUser, Result};

#[derive(Default)]
pub struct WorkerHttpClient;

#[async_trait(?Send)]
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
            .set("User-Agent", "xtalk/0.1")
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
}
