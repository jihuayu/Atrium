use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use p256::ecdsa::{
    signature::Verifier as EcVerifier, Signature as EcSignature, VerifyingKey as EcVerifyingKey,
};
use rsa::{
    pkcs1v15::{Signature as RsaSignature, VerifyingKey as RsaVerifyingKey},
    BigUint, RsaPublicKey,
};
use serde::Deserialize;

use crate::{
    auth::HttpClient,
    db::{self, Database, DbValue},
    types::ProviderUser,
    ApiError, Result,
};

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwtPayload {
    sub: String,
    email: Option<String>,
    picture: Option<String>,
    iss: String,
    exp: i64,
    aud: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JwksDoc {
    keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
struct JwkKey {
    kid: Option<String>,
    alg: Option<String>,
    kty: String,
    n: Option<String>,
    e: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksCacheRow {
    jwks_json: String,
    expires_at: String,
}

#[cfg(feature = "server")]
fn server_cache() -> &'static moka::future::Cache<String, (String, i64)> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<moka::future::Cache<String, (String, i64)>> = OnceLock::new();
    CACHE.get_or_init(|| moka::future::Cache::builder().max_capacity(16).build())
}

pub async fn verify_google_id_token(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    audience: Option<&str>,
) -> Result<ProviderUser> {
    verify_provider_id_token(
        db,
        http,
        token,
        "google",
        GOOGLE_JWKS_URL,
        "https://accounts.google.com",
        audience,
    )
    .await
}

pub async fn verify_apple_id_token(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    audience: Option<&str>,
) -> Result<ProviderUser> {
    verify_provider_id_token(
        db,
        http,
        token,
        "apple",
        APPLE_JWKS_URL,
        "https://appleid.apple.com",
        audience,
    )
    .await
}

async fn verify_provider_id_token(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    provider: &str,
    jwks_url: &str,
    expected_iss: &str,
    expected_aud: Option<&str>,
) -> Result<ProviderUser> {
    let (header, payload, signing_input, signature) = parse_jwt_parts(token)?;
    let kid = header.kid.clone().ok_or_else(|| ApiError::unauthorized())?;

    let jwks_json = get_jwks_cached(db, http, provider, jwks_url).await?;
    let jwks: JwksDoc = serde_json::from_str(&jwks_json).map_err(|_| ApiError::unauthorized())?;
    let key = jwks
        .keys
        .iter()
        .find(|k| k.kid.as_deref() == Some(kid.as_str()))
        .ok_or_else(ApiError::unauthorized)?;

    verify_signature(&header.alg, key, &signing_input, &signature)?;

    let now = chrono::Utc::now().timestamp();
    if payload.exp <= now {
        return Err(ApiError::unauthorized());
    }
    if payload.iss != expected_iss {
        return Err(ApiError::unauthorized());
    }
    if let Some(expected_aud) = expected_aud {
        if !aud_matches(&payload.aud, expected_aud) {
            return Err(ApiError::unauthorized());
        }
    }

    Ok(ProviderUser {
        provider: provider.to_string(),
        provider_user_id: payload.sub.clone(),
        login: payload
            .email
            .clone()
            .unwrap_or_else(|| format!("{}-{}", provider, payload.sub)),
        email: payload.email.unwrap_or_default(),
        avatar_url: payload.picture.unwrap_or_default(),
        r#type: "User".to_string(),
        site_admin: false,
    })
}

fn parse_jwt_parts(token: &str) -> Result<(JwtHeader, JwtPayload, String, Vec<u8>)> {
    let mut parts = token.split('.');
    let Some(header_b64) = parts.next() else {
        return Err(ApiError::unauthorized());
    };
    let Some(payload_b64) = parts.next() else {
        return Err(ApiError::unauthorized());
    };
    let Some(signature_b64) = parts.next() else {
        return Err(ApiError::unauthorized());
    };
    if parts.next().is_some() {
        return Err(ApiError::unauthorized());
    }

    let header: JwtHeader = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(header_b64)
            .map_err(|_| ApiError::unauthorized())?,
    )
    .map_err(|_| ApiError::unauthorized())?;
    let payload: JwtPayload = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|_| ApiError::unauthorized())?,
    )
    .map_err(|_| ApiError::unauthorized())?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| ApiError::unauthorized())?;

    Ok((
        header,
        payload,
        format!("{}.{}", header_b64, payload_b64),
        signature,
    ))
}

fn verify_signature(alg: &str, key: &JwkKey, signing_input: &str, signature: &[u8]) -> Result<()> {
    match alg {
        "RS256" => verify_rs256(key, signing_input.as_bytes(), signature),
        "ES256" => verify_es256(key, signing_input.as_bytes(), signature),
        _ => Err(ApiError::unauthorized()),
    }
}

fn verify_rs256(key: &JwkKey, msg: &[u8], signature: &[u8]) -> Result<()> {
    if key.kty != "RSA" {
        return Err(ApiError::unauthorized());
    }
    if key.alg.as_deref().map(|v| v != "RS256").unwrap_or(false) {
        return Err(ApiError::unauthorized());
    }

    let n = key
        .n
        .as_deref()
        .ok_or_else(ApiError::unauthorized)
        .and_then(decode_base64url)?;
    let e = key
        .e
        .as_deref()
        .ok_or_else(ApiError::unauthorized)
        .and_then(decode_base64url)?;

    let public_key = RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e))
        .map_err(|_| ApiError::unauthorized())?;
    let verifying_key = RsaVerifyingKey::<sha2::Sha256>::new(public_key);
    let signature = RsaSignature::try_from(signature).map_err(|_| ApiError::unauthorized())?;
    verifying_key
        .verify(msg, &signature)
        .map_err(|_| ApiError::unauthorized())
}

fn verify_es256(key: &JwkKey, msg: &[u8], signature: &[u8]) -> Result<()> {
    if key.kty != "EC" {
        return Err(ApiError::unauthorized());
    }
    if key.alg.as_deref().map(|v| v != "ES256").unwrap_or(false) {
        return Err(ApiError::unauthorized());
    }

    let x = key
        .x
        .as_deref()
        .ok_or_else(ApiError::unauthorized)
        .and_then(decode_base64url)?;
    let y = key
        .y
        .as_deref()
        .ok_or_else(ApiError::unauthorized)
        .and_then(decode_base64url)?;

    let mut sec1 = Vec::with_capacity(1 + x.len() + y.len());
    sec1.push(0x04);
    sec1.extend(x);
    sec1.extend(y);

    let verifying_key =
        EcVerifyingKey::from_sec1_bytes(&sec1).map_err(|_| ApiError::unauthorized())?;
    let sig = EcSignature::from_slice(signature).map_err(|_| ApiError::unauthorized())?;
    verifying_key
        .verify(msg, &sig)
        .map_err(|_| ApiError::unauthorized())
}

fn decode_base64url(value: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::unauthorized())
}

fn aud_matches(aud: &serde_json::Value, expected: &str) -> bool {
    match aud {
        serde_json::Value::String(value) => value == expected,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|v| v.as_str().map(|s| s == expected).unwrap_or(false)),
        _ => false,
    }
}

async fn get_jwks_cached(
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

fn parse_max_age(headers: &[(String, String)]) -> Option<u64> {
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
