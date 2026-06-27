use super::*;

pub async fn discovery_public_key(ctx: &AppContext<'_>) -> Result<Value> {
    let raw = ctx
        .discovery_public_jwk
        .or(ctx.discovery_private_jwk)
        .ok_or_else(|| ApiError::internal("ATRIUM_DISCOVERY_PUBLIC_JWK is not configured"))?;
    let mut jwk = serde_json::from_str::<Map<String, Value>>(raw)
        .map_err(|_| ApiError::internal("ATRIUM_DISCOVERY_PUBLIC_JWK is invalid"))?;
    for key in ["d", "p", "q", "dp", "dq", "qi", "oth", "k", "priv"] {
        jwk.remove(key);
    }
    if !jwk.contains_key("kty") {
        return Err(ApiError::internal(
            "ATRIUM_DISCOVERY_PUBLIC_JWK must be a JWK",
        ));
    }
    let kid = ctx
        .discovery_key_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| jwk.get("kid").and_then(Value::as_str).map(str::to_string));
    if let Some(kid) = kid.clone() {
        jwk.insert("kid".to_string(), Value::String(kid));
    }
    jwk.insert(
        "alg".to_string(),
        Value::String(DISCOVERY_JWE_ALG.to_string()),
    );
    jwk.insert(
        "key_ops".to_string(),
        Value::Array(vec![Value::String("encrypt".to_string())]),
    );
    Ok(json!({
        "kid": kid,
        "alg": DISCOVERY_JWE_ALG,
        "enc": DISCOVERY_JWE_ENC,
        "jwk": Value::Object(jwk),
    }))
}

pub(super) async fn discover_website_for_origin(
    ctx: &AppContext<'_>,
    origin: &str,
) -> Result<Option<WebsiteRow>> {
    if has_fresh_discovery_failure(ctx, origin).await? {
        return Ok(None);
    }
    let (metadata, failure) = discover_origin_metadata(ctx, origin).await?;
    let Some(metadata) = metadata else {
        let failure = failure.unwrap_or_else(|| DiscoveryFailure {
            status: "not_found",
            source: None,
            error: "discovery metadata not found".to_string(),
        });
        record_discovery_failure(
            ctx,
            origin,
            failure.status,
            failure.source.as_deref(),
            &failure.error,
        )
        .await?;
        return Ok(None);
    };
    if metadata.admin_emails.is_empty() {
        record_discovery_failure(
            ctx,
            origin,
            "invalid",
            Some(&metadata.source),
            "admin_emails is required",
        )
        .await?;
        return Ok(None);
    }
    if let Some(existing) = find_website(ctx, &metadata.website_key).await? {
        let origins = website_origins(ctx, existing.id).await?;
        if origins.iter().any(|item| item == &metadata.origin) {
            return Ok(Some(existing));
        }
        record_discovery_failure(
            ctx,
            origin,
            "conflict",
            Some(&metadata.source),
            "derived website key already exists for another origin",
        )
        .await?;
        return Ok(None);
    }
    let website = db::query_opt::<WebsiteRow>(
        ctx.db,
        "INSERT INTO websites (key, name, created_at, updated_at) VALUES (?1, ?2, datetime('now'), datetime('now')) RETURNING id, key, name, created_at, updated_at",
        &[DbValue::Text(metadata.website_key), DbValue::Text(metadata.name)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to create discovered website"))?;
    ctx.db
        .execute(
            "INSERT INTO website_origins (website_id, origin, created_at) VALUES (?1, ?2, datetime('now'))",
            &[DbValue::Integer(website.id), DbValue::Text(metadata.origin.clone())],
        )
        .await?;
    save_pending_website_admins(ctx, website.id, &metadata.admin_emails, &metadata.source).await?;
    claim_pending_website_admins_for_website(ctx, website.id).await?;
    record_discovery_success(ctx, origin, website.id, &metadata.source).await?;
    Ok(Some(website))
}

async fn discover_origin_metadata(
    ctx: &AppContext<'_>,
    origin: &str,
) -> Result<(Option<DiscoveryMetadata>, Option<DiscoveryFailure>)> {
    let origin_url = match Url::parse(origin) {
        Ok(url) => url,
        Err(_) => {
            return Ok((
                None,
                Some(DiscoveryFailure {
                    status: "invalid",
                    source: None,
                    error: "invalid origin".to_string(),
                }),
            ));
        }
    };
    if origin_url.scheme() != "https" {
        return Ok((
            None,
            Some(DiscoveryFailure {
                status: "not_found",
                source: None,
                error: "discovery requires https origin".to_string(),
            }),
        ));
    }
    let mut failures = Vec::<DiscoveryFailure>::new();
    if let Some(text) = mocked_well_known_text(ctx, origin) {
        if let Some(metadata) =
            parse_discovery_candidate(ctx, &text, origin, "well-known", &mut failures)
        {
            return Ok((Some(metadata), None));
        }
    } else {
        let url = format!("{}{}", origin.trim_end_matches('/'), DISCOVERY_PATH);
        match ctx.http.get_url(&url, "application/json").await {
            Ok(response) if response.status == 200 => {
                if let Ok(text) = String::from_utf8(response.body.to_vec()) {
                    if let Some(metadata) =
                        parse_discovery_candidate(ctx, &text, origin, "well-known", &mut failures)
                    {
                        return Ok((Some(metadata), None));
                    }
                }
            }
            Ok(response) if response.status == 404 || response.status == 410 => {}
            Ok(response) => failures.push(DiscoveryFailure {
                status: "error",
                source: Some("well-known".to_string()),
                error: format!("well-known returned {}", response.status),
            }),
            Err(error) => failures.push(DiscoveryFailure {
                status: "error",
                source: Some("well-known".to_string()),
                error: error.to_string(),
            }),
        }
    }

    let hostname = origin_url.host_str().unwrap_or_default();
    let payloads = if let Some(payloads) = mocked_dns_txt_payloads(ctx, hostname) {
        payloads
    } else {
        let url = format!(
            "https://cloudflare-dns.com/dns-query?name={}&type=TXT",
            urlencoding::encode(&format!("_atrium.{}", hostname))
        );
        match ctx.http.get_url(&url, "application/dns-json").await {
            Ok(response) if response.status == 200 => {
                let payload: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
                parse_atrium_txt_payloads_from_doh(&payload)
            }
            Ok(response) => {
                failures.push(DiscoveryFailure {
                    status: "error",
                    source: Some("dns-txt".to_string()),
                    error: format!("dns query returned {}", response.status),
                });
                Vec::new()
            }
            Err(error) => {
                failures.push(DiscoveryFailure {
                    status: "error",
                    source: Some("dns-txt".to_string()),
                    error: error.to_string(),
                });
                Vec::new()
            }
        }
    };
    for payload in payloads {
        if let Some(metadata) =
            parse_discovery_candidate(ctx, &payload, origin, "dns-txt", &mut failures)
        {
            return Ok((Some(metadata), None));
        }
    }
    let failure = failures
        .iter()
        .find(|item| item.status == "invalid")
        .cloned()
        .or_else(|| failures.iter().find(|item| item.status == "error").cloned())
        .unwrap_or_else(|| DiscoveryFailure {
            status: "not_found",
            source: None,
            error: "discovery metadata not found".to_string(),
        });
    Ok((None, Some(failure)))
}

fn parse_discovery_candidate(
    ctx: &AppContext<'_>,
    text: &str,
    expected_origin: &str,
    source: &str,
    failures: &mut Vec<DiscoveryFailure>,
) -> Option<DiscoveryMetadata> {
    match parse_discovery_document(ctx, text, expected_origin, source) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            failures.push(DiscoveryFailure {
                status: if error.status >= 500 {
                    "error"
                } else {
                    "invalid"
                },
                source: Some(source.to_string()),
                error: error.body.message,
            });
            None
        }
    }
}

fn parse_discovery_document(
    ctx: &AppContext<'_>,
    text: &str,
    expected_origin: &str,
    source: &str,
) -> Result<DiscoveryMetadata> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|_| ApiError::bad_request("invalid discovery document"))?;
    let obj = raw
        .as_object()
        .ok_or_else(|| ApiError::bad_request("document must be a JSON object"))?;
    let document = decrypt_flat_discovery_fields(ctx, obj)?;
    if document.get("atrium").and_then(Value::as_str) != Some("v1") {
        return Err(ApiError::bad_request("atrium must be v1"));
    }
    if document.contains_key("website_key") {
        return Err(ApiError::bad_request("website_key is not allowed"));
    }
    let origin = document
        .get("origin")
        .and_then(Value::as_str)
        .map(normalize_discovery_origin)
        .transpose()?
        .unwrap_or_else(|| expected_origin.to_string());
    if origin != expected_origin {
        return Err(ApiError::bad_request(
            "origin does not match referer origin",
        ));
    }
    let origin_url = Url::parse(expected_origin)
        .map_err(|_| ApiError::validation("Website", "origins", "invalid"))?;
    let website_key = normalize_key(
        Some(&Value::String(
            origin_url.host_str().unwrap_or_default().to_string(),
        )),
        "Website",
        "key",
    )?;
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&website_key)
        .trim()
        .to_string();
    if name.is_empty() || name.len() > 160 {
        return Err(ApiError::bad_request("name is invalid"));
    }
    let mut emails = Vec::new();
    if let Some(value) = document.get("admin_emails") {
        let values = value
            .as_array()
            .ok_or_else(|| ApiError::bad_request("admin_emails must be an array"))?;
        if values.len() > 20 {
            return Err(ApiError::bad_request("admin_emails has too many entries"));
        }
        let mut seen = HashSet::new();
        for value in values {
            let email = value
                .as_str()
                .ok_or_else(|| ApiError::bad_request("admin_emails must be a string"))?;
            let email = normalize_discovery_email(email, "admin_emails")?;
            if seen.insert(email.clone()) {
                emails.push(email);
            }
        }
    }
    let contact_email = document
        .get("contact_email")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| ApiError::bad_request("contact_email must be a string"))
                .and_then(|email| normalize_discovery_email(email, "contact_email"))
        })
        .transpose()?;
    Ok(DiscoveryMetadata {
        origin,
        website_key,
        name,
        admin_emails: emails,
        contact_email,
        source: source.to_string(),
    })
}

fn decrypt_flat_discovery_fields(
    ctx: &AppContext<'_>,
    obj: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut out = Map::with_capacity(obj.len());
    for (key, value) in obj {
        if let Some(raw) = value.as_str().and_then(|text| {
            text.strip_prefix(ENCRYPTED_FIELD_PREFIX)
                .map(str::to_string)
        }) {
            out.insert(key.clone(), decrypt_discovery_field(ctx, key, &raw)?);
        } else {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(out)
}

fn decrypt_discovery_field(ctx: &AppContext<'_>, field: &str, compact: &str) -> Result<Value> {
    let private_jwk = ctx
        .discovery_private_jwk
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("ATRIUM_DISCOVERY_PRIVATE_JWK is not configured"))?;

    decrypt_compact_discovery_jwe(private_jwk, ctx.discovery_key_id, compact)
        .map_err(|_| ApiError::bad_request(format!("{} could not be decrypted", field)))
}

fn decrypt_compact_discovery_jwe(
    private_jwk: &str,
    configured_kid: Option<&str>,
    compact: &str,
) -> std::result::Result<Value, String> {
    let parts = compact.split('.').collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err("compact JWE must have five parts".to_string());
    }

    let protected = b64url_decode(parts[0])?;
    let protected_header = serde_json::from_slice::<Value>(&protected)
        .map_err(|_| "protected header is not JSON".to_string())?;
    let protected_header = protected_header
        .as_object()
        .ok_or_else(|| "protected header must be an object".to_string())?;
    if protected_header.get("alg").and_then(Value::as_str) != Some(DISCOVERY_JWE_ALG) {
        return Err("unsupported JWE alg".to_string());
    }
    if protected_header.get("enc").and_then(Value::as_str) != Some(DISCOVERY_JWE_ENC) {
        return Err("unsupported JWE enc".to_string());
    }

    let private_jwk = serde_json::from_str::<Value>(private_jwk)
        .map_err(|_| "private JWK is not JSON".to_string())?;
    let private_jwk = private_jwk
        .as_object()
        .ok_or_else(|| "private JWK must be an object".to_string())?;
    let expected_kid = configured_kid
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| private_jwk.get("kid").and_then(Value::as_str));
    if let Some(expected_kid) = expected_kid {
        if protected_header.get("kid").and_then(Value::as_str) != Some(expected_kid) {
            return Err("JWE kid mismatch".to_string());
        }
    }

    let private_key = rsa_private_key_from_jwk(private_jwk)?;
    let encrypted_key = b64url_decode(parts[1])?;
    let cek = private_key
        .decrypt(Oaep::new::<Sha256>(), &encrypted_key)
        .map_err(|_| "failed to unwrap content encryption key".to_string())?;
    if cek.len() != 32 {
        return Err("A256GCM requires a 256-bit key".to_string());
    }

    let iv = b64url_decode(parts[2])?;
    if iv.len() != 12 {
        return Err("A256GCM requires a 96-bit IV".to_string());
    }
    let mut ciphertext_and_tag = b64url_decode(parts[3])?;
    let tag = b64url_decode(parts[4])?;
    if tag.len() != 16 {
        return Err("A256GCM requires a 128-bit tag".to_string());
    }
    ciphertext_and_tag.extend_from_slice(&tag);

    let cipher = Aes256Gcm::new_from_slice(&cek).map_err(|_| "invalid A256GCM key".to_string())?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: &ciphertext_and_tag,
                aad: parts[0].as_bytes(),
            },
        )
        .map_err(|_| "failed to decrypt ciphertext".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|_| "plaintext is not JSON".to_string())
}

fn rsa_private_key_from_jwk(
    jwk: &Map<String, Value>,
) -> std::result::Result<RsaPrivateKey, String> {
    if jwk.get("kty").and_then(Value::as_str) != Some("RSA") {
        return Err("private JWK must be RSA".to_string());
    }
    let n = jwk_biguint(jwk, "n")?;
    let e = jwk_biguint(jwk, "e")?;
    let d = jwk_biguint(jwk, "d")?;
    let primes = vec![jwk_biguint(jwk, "p")?, jwk_biguint(jwk, "q")?];
    RsaPrivateKey::from_components(n, e, d, primes)
        .map_err(|_| "invalid RSA private JWK".to_string())
}

fn jwk_biguint(jwk: &Map<String, Value>, name: &str) -> std::result::Result<BigUint, String> {
    let raw = jwk
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("private JWK missing {}", name))?;
    let bytes = b64url_decode(raw)?;
    if bytes.is_empty() {
        return Err(format!("private JWK {} is empty", name));
    }
    Ok(BigUint::from_bytes_be(&bytes))
}

fn b64url_decode(raw: &str) -> std::result::Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "invalid base64url".to_string())
}

fn normalize_discovery_email(raw: &str, field: &str) -> Result<String> {
    normalize_email(raw).ok_or_else(|| ApiError::bad_request(format!("{} is invalid", field)))
}

fn parse_atrium_txt_payloads_from_doh(payload: &Value) -> Vec<String> {
    payload
        .get("Answer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|answer| {
            if answer.get("type").and_then(Value::as_i64) != Some(16) {
                return None;
            }
            let data = answer.get("data")?.as_str()?;
            let joined = parse_dns_txt_data(data).join("");
            joined.strip_prefix(TXT_RECORD_PREFIX).map(str::to_string)
        })
        .collect()
}

fn parse_dns_txt_data(data: &str) -> Vec<String> {
    let trimmed = data.trim();
    if !trimmed.starts_with('"') {
        return vec![trimmed.to_string()];
    }
    let chars = trimmed.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if in_quote {
                parts.push(std::mem::take(&mut current));
            }
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote {
            if ch == '\\' {
                if i + 3 < chars.len()
                    && chars[i + 1].is_ascii_digit()
                    && chars[i + 2].is_ascii_digit()
                    && chars[i + 3].is_ascii_digit()
                {
                    let code = [chars[i + 1], chars[i + 2], chars[i + 3]]
                        .iter()
                        .collect::<String>()
                        .parse::<u32>()
                        .unwrap_or(0);
                    if let Some(decoded) = char::from_u32(code) {
                        current.push(decoded);
                    }
                    i += 4;
                    continue;
                }
                if i + 1 < chars.len() {
                    current.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
            }
            current.push(ch);
        }
        i += 1;
    }
    if in_quote && !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        vec![trimmed.to_string()]
    } else {
        parts
    }
}

async fn has_fresh_discovery_failure(ctx: &AppContext<'_>, origin: &str) -> Result<bool> {
    #[derive(Deserialize)]
    struct ExistsRow {}
    Ok(db::query_opt::<ExistsRow>(
        ctx.db,
        "SELECT 1 AS hit FROM website_discovery_cache WHERE origin = ?1 AND status != 'discovered' AND retry_after IS NOT NULL AND retry_after > datetime('now') LIMIT 1",
        &[DbValue::Text(origin.to_string())],
    )
    .await?
    .is_some())
}

async fn record_discovery_failure(
    ctx: &AppContext<'_>,
    origin: &str,
    status: &str,
    source: Option<&str>,
    error: &str,
) -> Result<()> {
    let retry_after = match status {
        "not_found" => "+6 hours",
        "error" => "+10 minutes",
        _ => "+1 hour",
    };
    ctx.db
        .execute(
            "INSERT INTO website_discovery_cache (origin, status, website_id, error, source, checked_at, retry_after) \
             VALUES (?1, ?2, NULL, ?3, ?4, datetime('now'), datetime('now', ?5)) \
             ON CONFLICT(origin) DO UPDATE SET status = excluded.status, website_id = NULL, error = excluded.error, source = excluded.source, checked_at = datetime('now'), retry_after = excluded.retry_after",
            &[
                DbValue::Text(origin.to_string()),
                DbValue::Text(status.to_string()),
                DbValue::Text(error.chars().take(240).collect()),
                opt_text(source.map(str::to_string)),
                DbValue::Text(retry_after.to_string()),
            ],
        )
        .await?;
    Ok(())
}

async fn record_discovery_success(
    ctx: &AppContext<'_>,
    origin: &str,
    website_id: i64,
    source: &str,
) -> Result<()> {
    ctx.db
        .execute(
            "INSERT INTO website_discovery_cache (origin, status, website_id, error, source, checked_at, retry_after) \
             VALUES (?1, 'discovered', ?2, NULL, ?3, datetime('now'), NULL) \
             ON CONFLICT(origin) DO UPDATE SET status = 'discovered', website_id = excluded.website_id, error = NULL, source = excluded.source, checked_at = datetime('now'), retry_after = NULL",
            &[DbValue::Text(origin.to_string()), DbValue::Integer(website_id), DbValue::Text(source.to_string())],
        )
        .await?;
    Ok(())
}

fn normalize_discovery_origin(raw: &str) -> Result<String> {
    let origin = normalize_origin(raw)?;
    if !origin.starts_with("https://") {
        return Err(ApiError::bad_request("origin must be https"));
    }
    Ok(origin)
}

fn mocked_well_known_text(ctx: &AppContext<'_>, origin: &str) -> Option<String> {
    let raw = ctx.test_discovery_well_known?;
    let map = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
    map.get(origin).map(|value| {
        value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())
    })
}

fn mocked_dns_txt_payloads(ctx: &AppContext<'_>, hostname: &str) -> Option<Vec<String>> {
    let raw = ctx.test_discovery_dns_txt?;
    let map = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
    let value = map
        .get(hostname)
        .or_else(|| map.get(&format!("_atrium.{}", hostname)))?;
    let record = if let Some(array) = value.as_array() {
        array
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("")
    } else {
        value.as_str().unwrap_or_default().to_string()
    };
    Some(vec![
        record
            .strip_prefix(TXT_RECORD_PREFIX)
            .unwrap_or(&record)
            .to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::Database,
        types::GitHubApiUser,
    };
    use rand::rngs::OsRng;
    use rsa::{
        RsaPublicKey,
        traits::{PrivateKeyParts, PublicKeyParts},
    };
    use std::collections::HashMap;

    static NOOP_DB: NoopDb = NoopDb;
    static NOOP_HTTP: NoopHttp = NoopHttp;

    struct NoopDb;

    #[cfg_attr(feature = "server", async_trait::async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait::async_trait(?Send))]
    impl Database for NoopDb {
        async fn execute(&self, _sql: &str, _params: &[DbValue]) -> Result<u64> {
            Err(ApiError::internal("not used"))
        }

        async fn query_opt_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Option<Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn query_all_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Vec<Value>> {
            Err(ApiError::internal("not used"))
        }

        async fn batch(&self, _stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()> {
            Err(ApiError::internal("not used"))
        }
    }

    struct NoopHttp;

    #[cfg_attr(feature = "server", async_trait::async_trait)]
    #[cfg_attr(not(feature = "server"), async_trait::async_trait(?Send))]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }
    }

    fn test_context<'a>(private_jwk: &'a str, key_id: Option<&'a str>) -> AppContext<'a> {
        AppContext {
            db: &NOOP_DB,
            http: &NOOP_HTTP,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: None,
            apple_app_id: None,
            github_client_id: None,
            github_client_secret: None,
            account_base_url: None,
            account_audience: None,
            account_internal_secret: None,
            super_admin_account_ids: None,
            discovery_private_jwk: Some(private_jwk),
            discovery_public_jwk: None,
            discovery_key_id: key_id,
            test_discovery_well_known: None,
            test_discovery_dns_txt: None,
            stateful_sessions: false,
            test_bypass_secret: None,
        }
    }

    fn generated_private_jwk(kid: &str) -> (String, RsaPublicKey) {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public_key = RsaPublicKey::from(&private_key);
        let private_jwk = json!({
            "kty": "RSA",
            "kid": kid,
            "n": b64_biguint(public_key.n()),
            "e": b64_biguint(public_key.e()),
            "d": b64_biguint(private_key.d()),
            "p": b64_biguint(&private_key.primes()[0]),
            "q": b64_biguint(&private_key.primes()[1]),
        })
        .to_string();
        (private_jwk, public_key)
    }

    fn encrypt_discovery_value(public_key: &RsaPublicKey, kid: &str, value: &Value) -> String {
        let protected = json!({
            "alg": DISCOVERY_JWE_ALG,
            "enc": DISCOVERY_JWE_ENC,
            "kid": kid,
        });
        let protected =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&protected).expect("protected header"));
        let cek = [17_u8; 32];
        let iv = [23_u8; 12];
        let mut rng = OsRng;
        let encrypted_key = public_key
            .encrypt(&mut rng, Oaep::new::<Sha256>(), &cek)
            .expect("encrypt CEK");
        let cipher = Aes256Gcm::new_from_slice(&cek).expect("cipher");
        let plaintext = serde_json::to_vec(value).expect("plaintext JSON");
        let mut encrypted = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &plaintext,
                    aad: protected.as_bytes(),
                },
            )
            .expect("encrypt field");
        let tag = encrypted.split_off(encrypted.len() - 16);
        format!(
            "{}.{}.{}.{}.{}",
            protected,
            URL_SAFE_NO_PAD.encode(encrypted_key),
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(encrypted),
            URL_SAFE_NO_PAD.encode(tag)
        )
    }

    fn b64_biguint(value: &BigUint) -> String {
        URL_SAFE_NO_PAD.encode(value.to_bytes_be())
    }

    #[test]
    fn parse_discovery_document_accepts_encrypted_worker_fields() {
        let (private_jwk, public_key) = generated_private_jwk("disc-1");
        let name = encrypt_discovery_value(&public_key, "disc-1", &json!("Encrypted Site"));
        let admin_emails =
            encrypt_discovery_value(&public_key, "disc-1", &json!(["OWNER@Example.COM"]));
        let document = json!({
            "atrium": "v1",
            "name": format!("{}{}", ENCRYPTED_FIELD_PREFIX, name),
            "admin_emails": format!("{}{}", ENCRYPTED_FIELD_PREFIX, admin_emails),
            "contact_email": "Support@Example.COM"
        });
        let ctx = test_context(&private_jwk, Some("disc-1"));

        let metadata = parse_discovery_document(
            &ctx,
            &document.to_string(),
            "https://blog.example.com",
            "well-known",
        )
        .expect("parse discovery document");

        assert_eq!(metadata.origin, "https://blog.example.com");
        assert_eq!(metadata.website_key, "blog.example.com");
        assert_eq!(metadata.name, "Encrypted Site");
        assert_eq!(metadata.admin_emails, vec!["owner@example.com"]);
        assert_eq!(
            metadata.contact_email.as_deref(),
            Some("support@example.com")
        );
        assert_eq!(metadata.source, "well-known");
    }

    #[test]
    fn parse_discovery_document_rejects_wrong_jwe_kid() {
        let (private_jwk, public_key) = generated_private_jwk("disc-1");
        let name = encrypt_discovery_value(&public_key, "disc-1", &json!("Encrypted Site"));
        let document = json!({
            "atrium": "v1",
            "name": format!("{}{}", ENCRYPTED_FIELD_PREFIX, name),
            "admin_emails": ["owner@example.com"]
        });
        let ctx = test_context(&private_jwk, Some("disc-2"));

        let error = parse_discovery_document(
            &ctx,
            &document.to_string(),
            "https://blog.example.com",
            "dns",
        )
        .expect_err("kid mismatch should fail");

        assert_eq!(error.status, 400);
        assert_eq!(error.body.message, "name could not be decrypted");
    }

    #[tokio::test]
    async fn discovery_public_key_strips_private_jwk_fields() {
        let (private_jwk, _) = generated_private_jwk("disc-1");
        let ctx = test_context(&private_jwk, Some("disc-override"));

        let response = discovery_public_key(&ctx)
            .await
            .expect("public key response");

        assert_eq!(response["kid"], "disc-override");
        assert_eq!(response["alg"], DISCOVERY_JWE_ALG);
        assert_eq!(response["enc"], DISCOVERY_JWE_ENC);
        assert_eq!(response["jwk"]["kid"], "disc-override");
        assert_eq!(response["jwk"]["alg"], DISCOVERY_JWE_ALG);
        assert_eq!(response["jwk"]["key_ops"], json!(["encrypt"]));
        assert!(response["jwk"]["kty"].is_string());
        assert!(response["jwk"]["d"].is_null());
        assert!(response["jwk"]["p"].is_null());
        assert!(response["jwk"]["q"].is_null());
    }

    #[test]
    fn parse_dns_txt_data_decodes_worker_compatible_segments() {
        let parts = parse_dns_txt_data(
            "\"atrium-site={\\034atrium\\034:\\034v1\\034,\" \"\\034admin_emails\\034:[]}\"",
        );

        assert_eq!(
            parts.join(""),
            "atrium-site={\"atrium\":\"v1\",\"admin_emails\":[]}"
        );
    }
}
