use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{types::JwtClaims, ApiError, Result};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct JwtHeader {
    alg: String,
    typ: String,
}

pub fn sign_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String> {
    if secret.len() < 16 {
        return Err(ApiError::internal("jwt secret is too short"));
    }

    let header = JwtHeader {
        alg: "HS256".to_string(),
        typ: "JWT".to_string(),
    };

    let header_b64 = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&header)
            .map_err(|e| ApiError::internal(format!("jwt header encode failed: {}", e)))?,
    );
    let payload_b64 = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(claims)
            .map_err(|e| ApiError::internal(format!("jwt claims encode failed: {}", e)))?,
    );
    let signing_input = format!("{}.{}", header_b64, payload_b64);

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|_| ApiError::internal("invalid jwt secret"))?;
    mac.update(signing_input.as_bytes());
    let signature = mac.finalize().into_bytes();
    let signature_b64 = URL_SAFE_NO_PAD.encode(signature);

    Ok(format!("{}.{}", signing_input, signature_b64))
}

pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<JwtClaims> {
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

    let signing_input = format!("{}.{}", header_b64, payload_b64);

    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| ApiError::unauthorized())?;

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|_| ApiError::internal("invalid jwt secret"))?;
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&signature)
        .map_err(|_| ApiError::unauthorized())?;

    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| ApiError::unauthorized())?;
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| ApiError::unauthorized())?;
    if header.alg != "HS256" || header.typ != "JWT" {
        return Err(ApiError::unauthorized());
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| ApiError::unauthorized())?;
    let claims: JwtClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| ApiError::unauthorized())?;

    let now = chrono::Utc::now().timestamp();
    if claims.exp <= now {
        return Err(ApiError::unauthorized());
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::{sign_jwt, verify_jwt};
    use crate::types::JwtClaims;

    fn claims_with_exp(exp: i64) -> JwtClaims {
        JwtClaims {
            sub: "42".to_string(),
            login: "alice".to_string(),
            iss: "xtalk".to_string(),
            iat: exp - 3600,
            exp,
            jti: "jti-1".to_string(),
            token_type: "access".to_string(),
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let now = chrono::Utc::now().timestamp();
        let claims = claims_with_exp(now + 3600);
        let secret = b"0123456789abcdef0123456789abcdef";

        let token = sign_jwt(&claims, secret).expect("sign should succeed");
        let parsed = verify_jwt(&token, secret).expect("verify should succeed");
        assert_eq!(parsed.sub, "42");
        assert_eq!(parsed.login, "alice");
        assert_eq!(parsed.token_type, "access");
    }

    #[test]
    fn reject_short_secret_on_sign() {
        let now = chrono::Utc::now().timestamp();
        let claims = claims_with_exp(now + 3600);
        let err = sign_jwt(&claims, b"short").err().expect("must fail");
        assert_eq!(err.status, 500);
    }

    #[test]
    fn reject_invalid_token_shape() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let err = verify_jwt("just-two-parts.token", secret)
            .err()
            .expect("must fail");
        assert_eq!(err.status, 401);
    }

    #[test]
    fn reject_tampered_signature() {
        let now = chrono::Utc::now().timestamp();
        let claims = claims_with_exp(now + 3600);
        let secret = b"0123456789abcdef0123456789abcdef";

        let token = sign_jwt(&claims, secret).expect("sign should succeed");
        let mut parts: Vec<_> = token.split('.').collect();
        parts[2] = "AAAA";
        let bad = parts.join(".");

        let err = verify_jwt(&bad, secret).err().expect("must fail");
        assert_eq!(err.status, 401);
    }

    #[test]
    fn reject_expired_token() {
        let now = chrono::Utc::now().timestamp();
        let claims = claims_with_exp(now - 1);
        let secret = b"0123456789abcdef0123456789abcdef";
        let token = sign_jwt(&claims, secret).expect("sign should succeed");

        let err = verify_jwt(&token, secret).err().expect("must fail");
        assert_eq!(err.status, 401);
    }
}
