use std::{env, io};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url =
        env::var("XTALK_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let database_url =
        env::var("XTALK_DATABASE_URL").unwrap_or_else(|_| "sqlite://xtalk.db".to_string());
    let token_cache_ttl = env::var("XTALK_TOKEN_CACHE_TTL")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(3600);
    let cache_max_issues = env::var("XTALK_CACHE_MAX_ISSUES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(256);
    let cache_ttl = env::var("XTALK_CACHE_TTL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    let listen = env::var("XTALK_LISTEN").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let jwt_secret = env::var("XTALK_JWT_SECRET")
        .ok()
        .map(|v| parse_secret_bytes(&v))
        .unwrap_or_else(|| b"xtalk-dev-secret-change-me".to_vec());
    let google_client_id = env::var("XTALK_GOOGLE_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let apple_app_id = env::var("XTALK_APPLE_APP_ID")
        .ok()
        .filter(|v| !v.trim().is_empty());

    let app = xtalk::platform::server::build_app(
        &database_url,
        base_url,
        token_cache_ttl,
        cache_max_issues,
        cache_ttl,
        jwt_secret,
        google_client_id,
        apple_app_id,
    )
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn parse_secret_bytes(value: &str) -> Vec<u8> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .or_else(|_| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, value)
        })
        .unwrap_or_else(|_| value.as_bytes().to_vec())
}
