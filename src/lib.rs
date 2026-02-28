#[cfg(all(feature = "worker", feature = "server"))]
compile_error!("Features `worker` and `server` are mutually exclusive. Enable only one.");

pub mod auth;
pub mod cache;
pub mod db;
pub mod error;
pub mod fmt;
pub mod handlers;
pub mod jwks;
pub mod jwt;
pub mod markdown;
pub mod platform;
pub mod router;
pub mod services;
pub mod types;

#[cfg(all(test, feature = "server", feature = "test-utils"))]
extern crate self as atrium;

#[cfg(all(test, feature = "server", feature = "test-utils"))]
#[path = "../tests/common/mod.rs"]
pub mod common;

#[cfg(all(test, feature = "server", feature = "test-utils"))]
#[path = "../tests/e2e/mod.rs"]
mod lib_e2e;

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
    pub jwt_secret: &'a [u8],
    pub google_client_id: Option<&'a str>,
    pub apple_app_id: Option<&'a str>,
    pub stateful_sessions: bool,
    pub test_bypass_secret: Option<&'a str>,
}
