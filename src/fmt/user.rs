use crate::types::{ApiUser, GitHubUser};

pub fn to_api_user(user: &GitHubUser) -> ApiUser {
    ApiUser {
        login: user.login.clone(),
        id: user.id,
        avatar_url: user.avatar_url.clone(),
        html_url: format!("https://github.com/{}", user.login),
        r#type: user.r#type.clone(),
    }
}
