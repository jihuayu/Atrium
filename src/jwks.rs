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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::atomic::{AtomicUsize, Ordering}};

    use async_trait::async_trait;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use bytes::Bytes;
    use p256::ecdsa::{Signature as EcRawSignature, SigningKey as EcSigningKey};
    use rand::rngs::OsRng;
    use rsa::{pkcs1v15::SigningKey as RsaSigningKey, traits::PublicKeyParts, RsaPrivateKey};
    use sha2::Sha256;

    use crate::{
        auth::{HttpClient, UpstreamResponse},
        error::ApiError,
        types::GitHubApiUser,
    };

    use super::{
        aud_matches, parse_jwt_parts, parse_max_age, verify_apple_id_token,
        verify_google_id_token, verify_provider_id_token, verify_signature, JwkKey,
    };

    #[cfg(feature = "server")]
    async fn make_db() -> (tempfile::TempPath, crate::platform::server::sqlite::SqliteDatabase) {
        let db_file = tempfile::NamedTempFile::new().expect("temp file").into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    struct MockHttp {
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
        calls: AtomicUsize,
    }

    impl MockHttp {
        fn jwks_ok(jwks_json: String) -> Self {
            Self {
                status: 200,
                headers: vec![("Cache-Control".to_string(), "max-age=120".to_string())],
                body: Bytes::from(jwks_json),
                calls: AtomicUsize::new(0),
            }
        }

        fn jwks_fail() -> Self {
            Self {
                status: 500,
                headers: Vec::new(),
                body: Bytes::new(),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl HttpClient for MockHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(UpstreamResponse {
                status: self.status,
                headers: self.headers.clone(),
                body: self.body.clone(),
            })
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }
    }

    fn rsa_jwk_and_token(
        kid: &str,
        sub: &str,
        aud: serde_json::Value,
        iss: &str,
        exp: i64,
    ) -> (String, String) {
        use rsa::signature::{SignatureEncoding, Signer};

        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let public = private.to_public_key();

        let n = URL_SAFE_NO_PAD.encode(public.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(public.e().to_bytes_be());
        let jwks = serde_json::json!({
            "keys": [{
                "kid": kid,
                "alg": "RS256",
                "kty": "RSA",
                "n": n,
                "e": e
            }]
        })
        .to_string();

        let header_b64 = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"alg": "RS256", "kid": kid}))
                .expect("header"),
        );
        let payload_b64 = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "sub": sub,
                "email": "u@test.com",
                "picture": "https://avatars/u",
                "iss": iss,
                "exp": exp,
                "aud": aud
            }))
            .expect("payload"),
        );
        let signing_input = format!("{}.{}", header_b64, payload_b64);
        let signing_key = RsaSigningKey::<Sha256>::new(private);
        let signature = signing_key.sign(signing_input.as_bytes()).to_vec();
        let token = format!("{}.{}", signing_input, URL_SAFE_NO_PAD.encode(signature));

        (jwks, token)
    }

    fn ec_jwk_and_signature(msg: &[u8]) -> (JwkKey, Vec<u8>) {
        use p256::ecdsa::signature::{Signer, SignatureEncoding};

        let signing = EcSigningKey::from_slice(&[7u8; 32]).expect("ec key");
        let verify = signing.verifying_key();
        let point = verify.to_encoded_point(false);
        let x = URL_SAFE_NO_PAD.encode(point.x().expect("x"));
        let y = URL_SAFE_NO_PAD.encode(point.y().expect("y"));
        let sig: EcRawSignature = signing.sign(msg);
        let sig = sig.to_vec();

        (
            JwkKey {
                kid: Some("ec-1".to_string()),
                alg: Some("ES256".to_string()),
                kty: "EC".to_string(),
                n: None,
                e: None,
                x: Some(x),
                y: Some(y),
            },
            sig,
        )
    }

    fn make_jwt(header: serde_json::Value, payload: serde_json::Value) -> String {
        let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("header json"));
        let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload json"));
        format!("{}.{}.{}", h, p, URL_SAFE_NO_PAD.encode("sig"))
    }

    #[test]
    fn parse_max_age_from_cache_control() {
        let headers = vec![(
            "Cache-Control".to_string(),
            "public, max-age=3600, must-revalidate".to_string(),
        )];
        assert_eq!(parse_max_age(&headers), Some(3600));
    }

    #[test]
    fn parse_max_age_absent_returns_none() {
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        assert_eq!(parse_max_age(&headers), None);
    }

    #[test]
    fn aud_matches_string_and_array() {
        assert!(aud_matches(&serde_json::json!("client-1"), "client-1"));
        assert!(aud_matches(
            &serde_json::json!(["other", "client-1"]),
            "client-1"
        ));
        assert!(!aud_matches(&serde_json::json!(["other"]), "client-1"));
        assert!(!aud_matches(&serde_json::json!({"aud": "bad"}), "client-1"));
    }

    #[test]
    fn parse_jwt_parts_success() {
        let token = make_jwt(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "sub": "u1",
                "email": "a@b.com",
                "picture": "x",
                "iss": "https://accounts.google.com",
                "exp": chrono::Utc::now().timestamp() + 3600,
                "aud": "client-1"
            }),
        );

        let (header, payload, signing_input, sig) = parse_jwt_parts(&token).expect("must parse");
        assert_eq!(header.alg, "RS256");
        assert_eq!(header.kid.as_deref(), Some("k1"));
        assert_eq!(payload.sub, "u1");
        assert!(signing_input.contains('.'));
        assert!(!sig.is_empty());
    }

    #[test]
    fn parse_jwt_parts_rejects_bad_shape() {
        let err = parse_jwt_parts("a.b").err().expect("must fail");
        assert_eq!(err.status, 401);
    }

    #[test]
    fn parse_jwt_parts_rejects_extra_and_missing_parts() {
        let missing_payload = parse_jwt_parts("a")
            .err()
            .expect("missing payload must fail");
        assert_eq!(missing_payload.status, 401);

        let extra_part = parse_jwt_parts("a.b.c.d")
            .err()
            .expect("extra part must fail");
        assert_eq!(extra_part.status, 401);
    }

    #[test]
    fn verify_signature_supports_rs256_and_es256() {
        use rsa::signature::{SignatureEncoding, Signer};

        let message = b"header.payload";

        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let public = private.to_public_key();
        let key = JwkKey {
            kid: Some("k1".to_string()),
            alg: Some("RS256".to_string()),
            kty: "RSA".to_string(),
            n: Some(URL_SAFE_NO_PAD.encode(public.n().to_bytes_be())),
            e: Some(URL_SAFE_NO_PAD.encode(public.e().to_bytes_be())),
            x: None,
            y: None,
        };
        let rsa_sig = RsaSigningKey::<Sha256>::new(private).sign(message).to_vec();
        verify_signature("RS256", &key, "header.payload", &rsa_sig).expect("rsa verify ok");

        let (ec_key, ec_sig) = ec_jwk_and_signature(message);
        verify_signature("ES256", &ec_key, "header.payload", &ec_sig).expect("ec verify ok");

        let err = verify_signature("HS256", &ec_key, "header.payload", &ec_sig)
            .err()
            .expect("unsupported alg");
        assert_eq!(err.status, 401);
    }

    #[test]
    fn verify_signature_rejects_mismatched_kty_and_alg() {
        let message = b"header.payload";
        let (ec_key, ec_sig) = ec_jwk_and_signature(message);

        let wrong_kty_for_es = JwkKey {
            kid: ec_key.kid.clone(),
            alg: ec_key.alg.clone(),
            kty: "RSA".to_string(),
            n: ec_key.n.clone(),
            e: ec_key.e.clone(),
            x: ec_key.x.clone(),
            y: ec_key.y.clone(),
        };
        let err = verify_signature("ES256", &wrong_kty_for_es, "header.payload", &ec_sig)
            .err()
            .expect("wrong kty for es256");
        assert_eq!(err.status, 401);

        let wrong_alg_for_es = JwkKey {
            kid: ec_key.kid.clone(),
            alg: Some("ES384".to_string()),
            kty: ec_key.kty.clone(),
            n: ec_key.n.clone(),
            e: ec_key.e.clone(),
            x: ec_key.x.clone(),
            y: ec_key.y.clone(),
        };
        let err = verify_signature("ES256", &wrong_alg_for_es, "header.payload", &ec_sig)
            .err()
            .expect("wrong alg for es256");
        assert_eq!(err.status, 401);

        let rsa_like = JwkKey {
            kid: Some("k".to_string()),
            alg: Some("RS384".to_string()),
            kty: "RSA".to_string(),
            n: Some("AQAB".to_string()),
            e: Some("AQAB".to_string()),
            x: None,
            y: None,
        };
        let err = verify_signature("RS256", &rsa_like, "header.payload", &[1, 2, 3])
            .err()
            .expect("wrong alg for rs256");
        assert_eq!(err.status, 401);

        let wrong_kty_for_rs = JwkKey {
            kid: Some("k".to_string()),
            alg: Some("RS256".to_string()),
            kty: "EC".to_string(),
            n: Some("AQAB".to_string()),
            e: Some("AQAB".to_string()),
            x: None,
            y: None,
        };
        let err = verify_signature("RS256", &wrong_kty_for_rs, "header.payload", &[1, 2, 3])
            .err()
            .expect("wrong kty for rs256");
        assert_eq!(err.status, 401);
    }

    #[test]
    fn parse_max_age_invalid_returns_none() {
        let headers = vec![("Cache-Control".to_string(), "max-age=abc".to_string())];
        assert_eq!(parse_max_age(&headers), None);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn verify_provider_id_token_success_and_cache() {
        let (_db_file, db) = make_db().await;
        let now = chrono::Utc::now().timestamp();
        let (jwks, token) = rsa_jwk_and_token(
            "k-success",
            "user-1",
            serde_json::json!("client-1"),
            "https://issuer.test",
            now + 3600,
        );
        let http = MockHttp::jwks_ok(jwks);

        let user = verify_provider_id_token(
            &db,
            &http,
            &token,
            "provider_success",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .expect("verify ok");
        assert_eq!(user.provider_user_id, "user-1");
        assert_eq!(user.email, "u@test.com");

        let _ = verify_provider_id_token(
            &db,
            &http,
            &token,
            "provider_success",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .expect("verify cached ok");
        assert!(http.calls() >= 1);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn verify_google_and_apple_wrappers_and_cache_paths() {
        use crate::db::{Database, DbValue};

        let (_db_file, db) = make_db().await;
        let now = chrono::Utc::now().timestamp();

        let (google_jwks, google_token) = rsa_jwk_and_token(
            "k-google",
            "google-user",
            serde_json::json!("google-client"),
            "https://accounts.google.com",
            now + 3600,
        );
        let google_http = MockHttp::jwks_ok(google_jwks);
        let google_user = verify_google_id_token(
            &db,
            &google_http,
            &google_token,
            Some("google-client"),
        )
        .await
        .expect("google verify");
        assert_eq!(google_user.provider, "google");

        let (apple_jwks, apple_token) = rsa_jwk_and_token(
            "k-apple",
            "apple-user",
            serde_json::json!("apple-client"),
            "https://appleid.apple.com",
            now + 3600,
        );
        let apple_http = MockHttp::jwks_ok(apple_jwks);
        let apple_user =
            verify_apple_id_token(&db, &apple_http, &apple_token, Some("apple-client"))
                .await
                .expect("apple verify");
        assert_eq!(apple_user.provider, "apple");

        let (no_aud_jwks, no_aud_token) = rsa_jwk_and_token(
            "k-no-aud",
            "u-no-aud",
            serde_json::json!("anything"),
            "https://issuer.noaud",
            now + 3600,
        );
        let no_aud_http = MockHttp::jwks_ok(no_aud_jwks);
        let no_aud_user = verify_provider_id_token(
            &db,
            &no_aud_http,
            &no_aud_token,
            "provider_no_aud",
            "https://jwks.noaud",
            "https://issuer.noaud",
            None,
        )
        .await
        .expect("verify without aud check");
        assert_eq!(no_aud_user.provider_user_id, "u-no-aud");

        let (cached_jwks, cached_token) = rsa_jwk_and_token(
            "k-db-cache",
            "u-db-cache",
            serde_json::json!("db-client"),
            "https://issuer.db",
            now + 3600,
        );
        db.execute(
            "INSERT INTO jwks_cache (provider, jwks_json, expires_at) VALUES (?1, ?2, ?3)",
            &[
                DbValue::Text("provider_db_cache".to_string()),
                DbValue::Text(cached_jwks),
                DbValue::Text((now + 3600).to_string()),
            ],
        )
        .await
        .expect("insert jwks cache row");
        let fail_http = MockHttp::jwks_fail();
        let from_db = verify_provider_id_token(
            &db,
            &fail_http,
            &cached_token,
            "provider_db_cache",
            "https://jwks.db",
            "https://issuer.db",
            Some("db-client"),
        )
        .await
        .expect("must use db cache");
        assert_eq!(from_db.provider_user_id, "u-db-cache");

        super::server_cache()
            .insert(
                "provider_stale".to_string(),
                ("{\"keys\":[]}".to_string(), 0),
            )
            .await;

        let (stale_jwks, stale_token) = rsa_jwk_and_token(
            "k-stale",
            "u-stale",
            serde_json::json!("stale-client"),
            "https://issuer.stale",
            now + 3600,
        );
        let stale_http = MockHttp::jwks_ok(stale_jwks);
        let stale_user = verify_provider_id_token(
            &db,
            &stale_http,
            &stale_token,
            "provider_stale",
            "https://jwks.stale",
            "https://issuer.stale",
            Some("stale-client"),
        )
        .await
        .expect("stale server cache fallback");
        assert_eq!(stale_user.provider_user_id, "u-stale");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn mock_http_unused_methods_are_exercised() {
        let http = MockHttp::jwks_fail();
        let github_err = http
            .get_github_user("token")
            .await
            .err()
            .expect("not used");
        assert_eq!(github_err.status, 500);
        let utterances_err = http
            .post_utterances_token(&[], &HashMap::new())
            .await
            .err()
            .expect("not used");
        assert_eq!(utterances_err.status, 500);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn verify_provider_id_token_rejects_aud_iss_exp_and_http_failure() {
        let (_db_file, db) = make_db().await;
        let now = chrono::Utc::now().timestamp();

        let (jwks_bad_aud, token_bad_aud) = rsa_jwk_and_token(
            "k-aud",
            "user-2",
            serde_json::json!("other-client"),
            "https://issuer.test",
            now + 3600,
        );
        let http_bad_aud = MockHttp::jwks_ok(jwks_bad_aud);
        let err = verify_provider_id_token(
            &db,
            &http_bad_aud,
            &token_bad_aud,
            "provider_bad_aud",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .err()
        .expect("aud mismatch");
        assert_eq!(err.status, 401);

        let (jwks_bad_iss, token_bad_iss) = rsa_jwk_and_token(
            "k-iss",
            "user-3",
            serde_json::json!("client-1"),
            "https://wrong-issuer.test",
            now + 3600,
        );
        let http_bad_iss = MockHttp::jwks_ok(jwks_bad_iss);
        let err = verify_provider_id_token(
            &db,
            &http_bad_iss,
            &token_bad_iss,
            "provider_bad_iss",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .err()
        .expect("iss mismatch");
        assert_eq!(err.status, 401);

        let (jwks_expired, token_expired) = rsa_jwk_and_token(
            "k-exp",
            "user-4",
            serde_json::json!("client-1"),
            "https://issuer.test",
            now - 1,
        );
        let http_expired = MockHttp::jwks_ok(jwks_expired);
        let err = verify_provider_id_token(
            &db,
            &http_expired,
            &token_expired,
            "provider_expired",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .err()
        .expect("expired");
        assert_eq!(err.status, 401);

        let (_jwks_unused, token_any) = rsa_jwk_and_token(
            "k-http",
            "user-5",
            serde_json::json!("client-1"),
            "https://issuer.test",
            now + 3600,
        );
        let http_fail = MockHttp::jwks_fail();
        let err = verify_provider_id_token(
            &db,
            &http_fail,
            &token_any,
            "provider_http_fail",
            "https://jwks.test",
            "https://issuer.test",
            Some("client-1"),
        )
        .await
        .err()
        .expect("http fail");
        assert_eq!(err.status, 401);
    }
}
