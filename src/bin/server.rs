use std::{env, io};

struct ServerConfig {
    base_url: String,
    database_url: String,
    token_cache_ttl: i64,
    cache_max_issues: u64,
    cache_ttl: u64,
    listen: String,
    jwt_secret: Vec<u8>,
    google_client_id: Option<String>,
    apple_app_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(load_config_from_env()?).await
}

async fn run(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let app = atrium::platform::server::build_app(
        &config.database_url,
        config.base_url,
        config.token_cache_ttl,
        config.cache_max_issues,
        config.cache_ttl,
        config.jwt_secret,
        config.google_client_id,
        config.apple_app_id,
    )
    .await
    .map_err(|e| io::Error::other(e.to_string()))?;

    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn load_config_from_env() -> Result<ServerConfig, io::Error> {
    let jwt_secret_raw = env_with_fallback("ATRIUM_JWT_SECRET", "XTALK_JWT_SECRET")
        .ok_or_else(|| io::Error::other("ATRIUM_JWT_SECRET or XTALK_JWT_SECRET is required"))?;
    let jwt_secret = parse_secret_bytes(&jwt_secret_raw);
    if jwt_secret.len() < 16 {
        return Err(io::Error::other("JWT secret must be at least 16 bytes"));
    }

    Ok(ServerConfig {
        base_url: env_with_fallback("ATRIUM_BASE_URL", "XTALK_BASE_URL")
            .unwrap_or_else(|| "http://localhost:3000".to_string()),
        database_url: env_with_fallback("ATRIUM_DATABASE_URL", "XTALK_DATABASE_URL")
            .unwrap_or_else(|| "sqlite:///data/atrium.db".to_string()),
        token_cache_ttl: env_with_fallback("ATRIUM_TOKEN_CACHE_TTL", "XTALK_TOKEN_CACHE_TTL")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(3600),
        cache_max_issues: env_with_fallback("ATRIUM_CACHE_MAX_ISSUES", "XTALK_CACHE_MAX_ISSUES")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(256),
        cache_ttl: env_with_fallback("ATRIUM_CACHE_TTL", "XTALK_CACHE_TTL")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60),
        listen: env_with_fallback("ATRIUM_LISTEN", "XTALK_LISTEN")
            .unwrap_or_else(|| "0.0.0.0:3000".to_string()),
        jwt_secret,
        google_client_id: env_with_fallback("ATRIUM_GOOGLE_CLIENT_ID", "XTALK_GOOGLE_CLIENT_ID")
            .filter(|v| !v.trim().is_empty()),
        apple_app_id: env_with_fallback("ATRIUM_APPLE_APP_ID", "XTALK_APPLE_APP_ID")
            .filter(|v| !v.trim().is_empty()),
    })
}

fn env_with_fallback(primary: &str, legacy: &str) -> Option<String> {
    env::var(primary).ok().or_else(|| env::var(legacy).ok())
}

fn parse_secret_bytes(value: &str) -> Vec<u8> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .or_else(|_| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, value)
        })
        .unwrap_or_else(|_| value.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Mutex, OnceLock},
        time::Duration,
    };

    use super::{load_config_from_env, parse_secret_bytes, run, ServerConfig};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        env_lock().lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clear_server_envs() {
        for key in [
            "ATRIUM_BASE_URL",
            "ATRIUM_DATABASE_URL",
            "ATRIUM_TOKEN_CACHE_TTL",
            "ATRIUM_CACHE_MAX_ISSUES",
            "ATRIUM_CACHE_TTL",
            "ATRIUM_LISTEN",
            "ATRIUM_JWT_SECRET",
            "ATRIUM_GOOGLE_CLIENT_ID",
            "ATRIUM_APPLE_APP_ID",
            "ATRIUM_TEST_BYPASS_SECRET",
            "XTALK_BASE_URL",
            "XTALK_DATABASE_URL",
            "XTALK_TOKEN_CACHE_TTL",
            "XTALK_CACHE_MAX_ISSUES",
            "XTALK_CACHE_TTL",
            "XTALK_LISTEN",
            "XTALK_JWT_SECRET",
            "XTALK_GOOGLE_CLIENT_ID",
            "XTALK_APPLE_APP_ID",
            "XTALK_TEST_BYPASS_SECRET",
        ] {
            remove_env_var(key);
        }
    }

    fn set_env_var(key: &str, value: &str) {
        // SAFETY: tests serialize env mutation with `env_lock`.
        unsafe { std::env::set_var(key, value) };
    }

    fn remove_env_var(key: &str) {
        // SAFETY: tests serialize env mutation with `env_lock`.
        unsafe { std::env::remove_var(key) };
    }

    fn temp_db_url() -> (tempfile::TempPath, String) {
        let db_file = tempfile::NamedTempFile::new()
            .expect("temp file")
            .into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        (db_file, db_url)
    }

    #[test]
    fn parse_secret_bytes_supports_standard_and_urlsafe() {
        assert_eq!(parse_secret_bytes("YXRyaXVt"), b"atrium".to_vec());
        assert_eq!(parse_secret_bytes("YXRyaXVt"), b"atrium".to_vec());
        assert_eq!(parse_secret_bytes("not-base64"), b"not-base64".to_vec());
    }

    #[test]
    fn load_config_requires_jwt_secret() {
        let _guard = lock_env();
        clear_server_envs();

        let err = load_config_from_env().err().expect("missing secret must fail");
        assert!(err
            .to_string()
            .contains("ATRIUM_JWT_SECRET or XTALK_JWT_SECRET is required"));
    }

    #[test]
    fn load_config_uses_defaults_when_secret_present() {
        let _guard = lock_env();
        clear_server_envs();
        set_env_var("ATRIUM_JWT_SECRET", "YXRyaXVtLWRlZmF1bHQtand0LXNlY3JldA");

        let cfg = load_config_from_env().expect("load config");
        assert_eq!(cfg.base_url, "http://localhost:3000");
        assert_eq!(cfg.database_url, "sqlite:///data/atrium.db");
        assert_eq!(cfg.token_cache_ttl, 3600);
        assert_eq!(cfg.cache_max_issues, 256);
        assert_eq!(cfg.cache_ttl, 60);
        assert_eq!(cfg.listen, "0.0.0.0:3000");
        assert_eq!(cfg.jwt_secret, b"atrium-default-jwt-secret".to_vec());
        assert_eq!(cfg.google_client_id, None);
        assert_eq!(cfg.apple_app_id, None);
    }

    #[test]
    fn load_config_prefers_atrium_and_falls_back_to_xtalk() {
        let _guard = lock_env();
        clear_server_envs();

        set_env_var("XTALK_BASE_URL", "https://legacy.example");
        set_env_var("XTALK_DATABASE_URL", "sqlite://legacy.db");
        set_env_var("XTALK_TOKEN_CACHE_TTL", "10");
        set_env_var("XTALK_CACHE_MAX_ISSUES", "11");
        set_env_var("XTALK_CACHE_TTL", "12");
        set_env_var("XTALK_LISTEN", "127.0.0.1:9999");
        set_env_var("XTALK_JWT_SECRET", "bGVnYWN5LXRlc3Qtc2VjcmV0");
        set_env_var("XTALK_GOOGLE_CLIENT_ID", "legacy-google");
        set_env_var("XTALK_APPLE_APP_ID", "legacy-apple");

        let fallback_cfg = load_config_from_env().expect("load legacy");
        assert_eq!(fallback_cfg.base_url, "https://legacy.example");
        assert_eq!(fallback_cfg.database_url, "sqlite://legacy.db");
        assert_eq!(fallback_cfg.token_cache_ttl, 10);
        assert_eq!(fallback_cfg.cache_max_issues, 11);
        assert_eq!(fallback_cfg.cache_ttl, 12);
        assert_eq!(fallback_cfg.listen, "127.0.0.1:9999");
        assert_eq!(fallback_cfg.jwt_secret, b"legacy-test-secret".to_vec());
        assert_eq!(
            fallback_cfg.google_client_id.as_deref(),
            Some("legacy-google")
        );
        assert_eq!(fallback_cfg.apple_app_id.as_deref(), Some("legacy-apple"));

        set_env_var("ATRIUM_BASE_URL", "https://atrium.example");
        set_env_var("ATRIUM_TOKEN_CACHE_TTL", "20");
        set_env_var("ATRIUM_JWT_SECRET", "YXRyaXVtLXRlc3Qtc2VjcmV0");
        set_env_var("ATRIUM_GOOGLE_CLIENT_ID", "atrium-google");

        let preferred_cfg = load_config_from_env().expect("load preferred");
        assert_eq!(preferred_cfg.base_url, "https://atrium.example");
        assert_eq!(preferred_cfg.token_cache_ttl, 20);
        assert_eq!(preferred_cfg.jwt_secret, b"atrium-test-secret".to_vec());
        assert_eq!(
            preferred_cfg.google_client_id.as_deref(),
            Some("atrium-google")
        );
    }

    #[tokio::test]
    async fn run_returns_error_on_invalid_listen_address() {
        let (_db_file, db_url) = temp_db_url();
        let cfg = ServerConfig {
            base_url: "http://localhost:3000".to_string(),
            database_url: db_url,
            token_cache_ttl: 3600,
            cache_max_issues: 16,
            cache_ttl: 60,
            listen: "invalid-listen".to_string(),
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!".to_vec(),
            google_client_id: None,
            apple_app_id: None,
        };

        let err = run(cfg).await.err().expect("invalid listen must fail");
        assert!(err.to_string().contains("invalid"));
    }

    #[tokio::test]
    async fn run_enters_serve_loop_for_valid_config() {
        let (_db_file, db_url) = temp_db_url();
        let cfg = ServerConfig {
            base_url: "http://localhost:3000".to_string(),
            database_url: db_url,
            token_cache_ttl: 3600,
            cache_max_issues: 16,
            cache_ttl: 60,
            listen: "127.0.0.1:0".to_string(),
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!".to_vec(),
            google_client_id: None,
            apple_app_id: None,
        };

        let timed = tokio::time::timeout(Duration::from_millis(120), run(cfg)).await;
        assert!(timed.is_err(), "server should keep serving until timeout");
    }

    #[test]
    fn main_reads_env_and_propagates_bind_error() {
        let _guard = lock_env();
        clear_server_envs();

        let (_db_file, db_url) = temp_db_url();
        set_env_var("ATRIUM_DATABASE_URL", &db_url);
        set_env_var("ATRIUM_LISTEN", "invalid-listen");
        set_env_var("ATRIUM_JWT_SECRET", "YXRyaXVtLXRlc3Qtc2VjcmV0");

        let err = super::main().err().expect("main must fail");
        assert!(err.to_string().contains("invalid"));
    }
}
