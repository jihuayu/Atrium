use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

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
    github_client_id: Option<String>,
    github_client_secret: Option<String>,
    account_base_url: Option<String>,
    account_audience: Option<String>,
    account_internal_secret: Option<String>,
    super_admin_account_ids: Option<String>,
    discovery_private_jwk: Option<String>,
    discovery_public_jwk: Option<String>,
    discovery_key_id: Option<String>,
    cors_origin: Option<String>,
    prune_legacy_sqlite_on_start: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(load_config_from_env()?).await
}

async fn run(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    if config.prune_legacy_sqlite_on_start {
        prune_legacy_sqlite_files(&config.database_url)?;
    }

    let app = atrium::platform::server::build_app(
        &config.database_url,
        config.base_url,
        config.token_cache_ttl,
        config.cache_max_issues,
        config.cache_ttl,
        config.jwt_secret,
        config.google_client_id,
        config.apple_app_id,
        config.github_client_id,
        config.github_client_secret,
        config.account_base_url,
        config.account_audience,
        config.account_internal_secret,
        config.super_admin_account_ids,
        config.discovery_private_jwk,
        config.discovery_public_jwk,
        config.discovery_key_id,
        config.cors_origin,
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
            .or_else(|| {
                env::var("PORT")
                    .ok()
                    .map(|port| format!("0.0.0.0:{}", port))
            })
            .unwrap_or_else(|| "0.0.0.0:3000".to_string()),
        jwt_secret,
        google_client_id: env_with_fallback("ATRIUM_GOOGLE_CLIENT_ID", "XTALK_GOOGLE_CLIENT_ID")
            .filter(|v| !v.trim().is_empty()),
        apple_app_id: env_with_fallback("ATRIUM_APPLE_APP_ID", "XTALK_APPLE_APP_ID")
            .filter(|v| !v.trim().is_empty()),
        github_client_id: env_with_fallback("ATRIUM_GITHUB_CLIENT_ID", "XTALK_GITHUB_CLIENT_ID")
            .filter(|v| !v.trim().is_empty()),
        github_client_secret: env_with_fallback(
            "ATRIUM_GITHUB_CLIENT_SECRET",
            "XTALK_GITHUB_CLIENT_SECRET",
        )
        .filter(|v| !v.trim().is_empty()),
        account_base_url: env_with_fallback("ACCOUNT_BASE_URL", "ACCOUNT_ISSUER")
            .filter(|v| !v.trim().is_empty()),
        account_audience: env::var("ACCOUNT_AUDIENCE")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        account_internal_secret: env::var("ACCOUNT_INTERNAL_SECRET")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        super_admin_account_ids: env::var("ATRIUM_SUPER_ADMIN_ACCOUNT_IDS")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        discovery_private_jwk: env::var("ATRIUM_DISCOVERY_PRIVATE_JWK")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        discovery_public_jwk: env::var("ATRIUM_DISCOVERY_PUBLIC_JWK")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        discovery_key_id: env::var("ATRIUM_DISCOVERY_KEY_ID")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        cors_origin: env_with_fallback("ATRIUM_CORS_ORIGIN", "XTALK_CORS_ORIGIN")
            .filter(|v| !v.trim().is_empty()),
        prune_legacy_sqlite_on_start: env::var("ATRIUM_PRUNE_LEGACY_SQLITE_ON_START")
            .ok()
            .map(|v| parse_bool_flag(&v))
            .unwrap_or(false),
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

fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn prune_legacy_sqlite_files(database_url: &str) -> io::Result<()> {
    let Some(active_path) = sqlite_path_from_url(database_url) else {
        return Ok(());
    };
    let Some(parent) = active_path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || is_active_sqlite_path(&path, &active_path) {
            continue;
        }
        if !looks_like_sqlite_file(&path) {
            continue;
        }

        fs::remove_file(&path)?;
        eprintln!("pruned legacy sqlite file: {}", path.display());
    }

    Ok(())
}

fn sqlite_path_from_url(database_url: &str) -> Option<PathBuf> {
    let raw_path = database_url
        .strip_prefix("sqlite://")?
        .split('?')
        .next()
        .unwrap_or_default();
    if raw_path.is_empty() || raw_path == ":memory:" {
        return None;
    }
    Some(PathBuf::from(raw_path))
}

fn is_active_sqlite_path(path: &Path, active_path: &Path) -> bool {
    path == active_path
        || path == sqlite_sidecar_path(active_path, "-wal")
        || path == sqlite_sidecar_path(active_path, "-shm")
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", path.display(), suffix))
}

fn looks_like_sqlite_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };

    file_name.ends_with(".db")
        || file_name.ends_with(".sqlite")
        || file_name.ends_with(".sqlite3")
        || file_name.ends_with(".db-wal")
        || file_name.ends_with(".db-shm")
        || file_name.ends_with(".sqlite-wal")
        || file_name.ends_with(".sqlite-shm")
        || file_name.ends_with(".sqlite3-wal")
        || file_name.ends_with(".sqlite3-shm")
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Mutex, OnceLock},
        time::Duration,
    };

    use super::{
        ServerConfig, load_config_from_env, parse_bool_flag, parse_secret_bytes,
        prune_legacy_sqlite_files, run,
    };

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
            "ATRIUM_CORS_ORIGIN",
            "ATRIUM_GITHUB_CLIENT_ID",
            "ATRIUM_GITHUB_CLIENT_SECRET",
            "ACCOUNT_BASE_URL",
            "ACCOUNT_ISSUER",
            "ACCOUNT_AUDIENCE",
            "ACCOUNT_INTERNAL_SECRET",
            "ATRIUM_SUPER_ADMIN_ACCOUNT_IDS",
            "ATRIUM_DISCOVERY_PRIVATE_JWK",
            "ATRIUM_DISCOVERY_PUBLIC_JWK",
            "ATRIUM_DISCOVERY_KEY_ID",
            "ATRIUM_PRUNE_LEGACY_SQLITE_ON_START",
            "PORT",
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
            "XTALK_CORS_ORIGIN",
            "XTALK_GITHUB_CLIENT_ID",
            "XTALK_GITHUB_CLIENT_SECRET",
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
    fn parse_bool_flag_accepts_enabled_values_only() {
        assert!(parse_bool_flag("1"));
        assert!(parse_bool_flag("TRUE"));
        assert!(parse_bool_flag(" yes "));
        assert!(parse_bool_flag("on"));
        assert!(!parse_bool_flag("0"));
        assert!(!parse_bool_flag("false"));
        assert!(!parse_bool_flag(""));
    }

    #[test]
    fn prune_legacy_sqlite_files_keeps_active_db_and_other_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let active = dir.path().join("atrium-fresh.db");
        let active_wal = dir.path().join("atrium-fresh.db-wal");
        let legacy = dir.path().join("atrium.db");
        let legacy_shm = dir.path().join("atrium.db-shm");
        let other = dir.path().join("keep.txt");

        std::fs::write(&active, "active").expect("write active");
        std::fs::write(&active_wal, "active wal").expect("write active wal");
        std::fs::write(&legacy, "legacy").expect("write legacy");
        std::fs::write(&legacy_shm, "legacy shm").expect("write legacy shm");
        std::fs::write(&other, "other").expect("write other");

        let db_url = format!("sqlite://{}", active.to_string_lossy().replace('\\', "/"));
        prune_legacy_sqlite_files(&db_url).expect("prune");

        assert!(active.exists());
        assert!(active_wal.exists());
        assert!(other.exists());
        assert!(!legacy.exists());
        assert!(!legacy_shm.exists());
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

        let err = load_config_from_env()
            .err()
            .expect("missing secret must fail");
        assert!(
            err.to_string()
                .contains("ATRIUM_JWT_SECRET or XTALK_JWT_SECRET is required")
        );
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
        assert_eq!(cfg.github_client_id, None);
        assert_eq!(cfg.github_client_secret, None);
        assert_eq!(cfg.account_base_url, None);
        assert_eq!(cfg.account_audience, None);
        assert_eq!(cfg.account_internal_secret, None);
        assert_eq!(cfg.super_admin_account_ids, None);
        assert_eq!(cfg.cors_origin, None);
        assert!(!cfg.prune_legacy_sqlite_on_start);
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
            github_client_id: None,
            github_client_secret: None,
            account_base_url: None,
            account_audience: None,
            account_internal_secret: None,
            super_admin_account_ids: None,
            discovery_private_jwk: None,
            discovery_public_jwk: None,
            discovery_key_id: None,
            cors_origin: None,
            prune_legacy_sqlite_on_start: false,
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
            github_client_id: None,
            github_client_secret: None,
            account_base_url: None,
            account_audience: None,
            account_internal_secret: None,
            super_admin_account_ids: None,
            discovery_private_jwk: None,
            discovery_public_jwk: None,
            discovery_key_id: None,
            cors_origin: None,
            prune_legacy_sqlite_on_start: false,
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
