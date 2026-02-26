pub mod auth;
pub mod cache;
pub mod db;
pub mod error;
pub mod fmt;
pub mod handlers;
pub mod markdown;
pub mod platform;
pub mod router;
pub mod services;
pub mod types;

pub use error::{ApiError, Result};

use auth::HttpClient;
use cache::CommentCacheStore;
use db::Database;
use types::GitHubUser;

pub struct AppContext<'a> {
    pub db: &'a dyn Database,
    pub http: &'a dyn HttpClient,
    pub comment_cache: Option<&'a dyn CommentCacheStore>,
    pub base_url: &'a str,
    pub user: Option<&'a GitHubUser>,
}
