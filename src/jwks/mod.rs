use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{
    Signature as EcSignature, VerifyingKey as EcVerifyingKey, signature::Verifier as EcVerifier,
};
use rsa::{
    BigUint, RsaPublicKey,
    pkcs1v15::{Signature as RsaSignature, VerifyingKey as RsaVerifyingKey},
};
use serde::Deserialize;

use crate::{
    ApiError, Result,
    auth::HttpClient,
    db::{self, Database, DbValue},
    types::ProviderUser,
};

mod cache;
mod parts;
mod signature;

#[cfg(test)]
mod tests;

use cache::get_jwks_cached;
#[cfg(test)]
use cache::parse_max_age;
#[cfg(all(test, feature = "server"))]
use cache::server_cache;
use parts::{aud_matches, parse_jwt_parts};
#[cfg(test)]
use signature::JwkKey;
use signature::{JwksDoc, verify_signature};

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";

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
