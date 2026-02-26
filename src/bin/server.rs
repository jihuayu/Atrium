use std::{env, io};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = env::var("XTALK_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let database_url = env::var("XTALK_DATABASE_URL").unwrap_or_else(|_| "sqlite://xtalk.db".to_string());
    let token_cache_ttl = env::var("XTALK_TOKEN_CACHE_TTL")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(3600);
    let listen = env::var("XTALK_LISTEN").unwrap_or_else(|_| "0.0.0.0:3000".to_string());

    let app = xtalk::platform::server::build_app(&database_url, base_url, token_cache_ttl)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
