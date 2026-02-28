use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use crate::{ApiError, Result};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CursorPayload {
    id: i64,
}

pub fn encode_cursor(id: i64) -> Result<String> {
    let payload = CursorPayload { id };
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| ApiError::internal(format!("encode cursor failed: {}", e)))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn decode_cursor(cursor: &str) -> Result<i64> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::bad_request("invalid cursor"))?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("invalid cursor"))?;
    Ok(payload.id)
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor};

    #[test]
    fn encode_decode_roundtrip() {
        let cursor = encode_cursor(12345).expect("encode should succeed");
        let value = decode_cursor(&cursor).expect("decode should succeed");
        assert_eq!(value, 12345);
    }

    #[test]
    fn reject_invalid_cursor() {
        let err = decode_cursor("not-a-valid-cursor")
            .err()
            .expect("must fail");
        assert_eq!(err.status, 400);
    }
}
