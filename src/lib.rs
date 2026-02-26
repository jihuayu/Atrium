pub mod auth;
pub mod db;
pub mod error;
pub mod fmt;
pub mod markdown;
pub mod platform;
pub mod services;
pub mod types;

pub use error::{ApiError, Result};

use auth::HttpClient;
use db::Database;
use types::GitHubUser;

pub struct AppContext<'a> {
    pub db: &'a dyn Database,
    pub http: &'a dyn HttpClient,
    pub base_url: &'a str,
    pub user: Option<&'a GitHubUser>,
}
