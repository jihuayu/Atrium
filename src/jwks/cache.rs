use super::*;

#[derive(Debug, Deserialize)]
struct JwksCacheRow {
    jwks_json: String,
    expires_at: String,
}

#[cfg(feature = "server")]
pub(super) fn server_cache() -> &'static moka::future::Cache<String, (String, i64)> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<moka::future::Cache<String, (String, i64)>> = OnceLock::new();
    CACHE.get_or_init(|| moka::future::Cache::builder().max_capacity(16).build())
}

pub(super) async fn get_jwks_cached(
    db: &dyn Database,
    http: &dyn HttpClient,
    provider: &str,
    url: &str,
) -> Result<String> {
    let now = chrono::Utc::now().timestamp();

    #[cfg(feature = "server")]
    {
        if let Some((jwks_json, expires_at)) = server_cache().get(&provider.to_string()).await {
            if expires_at > now {
                return Ok(jwks_json);
            }
        }
    }

    if let Some(row) = db::query_opt::<JwksCacheRow>(
        db,
        "SELECT jwks_json, expires_at FROM jwks_cache WHERE provider = ?1",
        &[DbValue::Text(provider.to_string())],
    )
    .await?
    {
        if row.expires_at.parse::<i64>().unwrap_or(0) > now {
            #[cfg(feature = "server")]
            server_cache()
                .insert(
                    provider.to_string(),
                    (row.jwks_json.clone(), row.expires_at.parse().unwrap_or(0)),
                )
                .await;
            return Ok(row.jwks_json);
        }
    }

    let response = http.get_jwks(url).await?;
    if !(200..=299).contains(&response.status) {
        return Err(ApiError::unauthorized());
    }

    let jwks_json =
        String::from_utf8(response.body.to_vec()).map_err(|_| ApiError::unauthorized())?;
    let max_age = parse_max_age(&response.headers).unwrap_or(300);
    let expires_at = now + max_age as i64;

    db.execute(
        "INSERT INTO jwks_cache (provider, jwks_json, expires_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(provider) DO UPDATE SET jwks_json = excluded.jwks_json, expires_at = excluded.expires_at",
        &[
            DbValue::Text(provider.to_string()),
            DbValue::Text(jwks_json.clone()),
            DbValue::Text(expires_at.to_string()),
        ],
    )
    .await?;

    #[cfg(feature = "server")]
    server_cache()
        .insert(provider.to_string(), (jwks_json.clone(), expires_at))
        .await;

    Ok(jwks_json)
}

pub(super) fn parse_max_age(headers: &[(String, String)]) -> Option<u64> {
    let value = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cache-control"))
        .map(|(_, v)| v)?;
    for item in value.split(',') {
        let item = item.trim();
        if let Some(raw) = item.strip_prefix("max-age=") {
            if let Ok(parsed) = raw.parse::<u64>() {
                return Some(parsed);
            }
        }
    }
    None
}
