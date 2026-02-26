pub mod http;
pub mod routes;
pub mod sqlite;

use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::Result;

use self::{http::ReqwestHttpClient, sqlite::SqliteDatabase};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<SqliteDatabase>,
    pub http: Arc<ReqwestHttpClient>,
    pub base_url: String,
    pub token_cache_ttl: i64,
}

pub async fn build_app(database_url: &str, base_url: String, token_cache_ttl: i64) -> Result<Router> {
    let db = Arc::new(SqliteDatabase::connect_and_migrate(database_url).await?);
    let http = Arc::new(ReqwestHttpClient::new()?);

    let state = AppState {
        db,
        http,
        base_url,
        token_cache_ttl,
    };

    let app = routes::router(state).layer(
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any),
    );

    Ok(app)
}
