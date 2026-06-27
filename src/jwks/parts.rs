use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct JwtHeader {
    pub(super) alg: String,
    pub(super) kid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct JwtPayload {
    pub(super) sub: String,
    pub(super) email: Option<String>,
    pub(super) picture: Option<String>,
    pub(super) iss: String,
    pub(super) exp: i64,
    pub(super) aud: serde_json::Value,
}

pub(super) fn parse_jwt_parts(token: &str) -> Result<(JwtHeader, JwtPayload, String, Vec<u8>)> {
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

pub(super) fn aud_matches(aud: &serde_json::Value, expected: &str) -> bool {
    match aud {
        serde_json::Value::String(value) => value == expected,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|v| v.as_str().map(|s| s == expected).unwrap_or(false)),
        _ => false,
    }
}
