use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct JwksDoc {
    pub(super) keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
pub(super) struct JwkKey {
    pub(super) kid: Option<String>,
    pub(super) alg: Option<String>,
    pub(super) kty: String,
    pub(super) n: Option<String>,
    pub(super) e: Option<String>,
    pub(super) x: Option<String>,
    pub(super) y: Option<String>,
}

pub(super) fn verify_signature(
    alg: &str,
    key: &JwkKey,
    signing_input: &str,
    signature: &[u8],
) -> Result<()> {
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
