use async_trait::async_trait;

use crate::{
    auth::HttpClient,
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
}
