use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct ApiFieldError {
    pub resource: String,
    pub field: String,
    pub code: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ApiErrorBody {
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<ApiFieldError>,
    pub documentation_url: String,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub body: ApiErrorBody,
}

pub type Result<T> = std::result::Result<T, ApiError>;

impl ApiError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                message: message.into(),
                errors: Vec::new(),
                documentation_url: "https://docs.github.com/rest".to_string(),
            },
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(401, "Requires authentication")
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, message)
    }

    pub fn not_found(resource: &str) -> Self {
        Self::new(404, format!("{} not found", resource))
    }

    pub fn validation(resource: &str, field: &str, code: &str) -> Self {
        let mut err = Self::new(422, "Validation Failed");
        err.body.errors.push(ApiFieldError {
            resource: resource.to_string(),
            field: field.to_string(),
            code: code.to_string(),
        });
        err
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }

    pub fn to_native_response(&self) -> serde_json::Value {
        let error = match self.status {
            400 => "bad_request",
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            422 => "validation_failed",
            _ => "internal_error",
        };
        serde_json::json!({
            "error": error,
            "message": self.body.message,
        })
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.body.message)
    }
}

impl std::error::Error for ApiError {}

impl From<serde_json::Error> for ApiError {
    fn from(value: serde_json::Error) -> Self {
        Self::internal(format!("serialization error: {}", value))
    }
}

#[cfg(feature = "server")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(self.body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;

    #[test]
    fn to_native_response_maps_statuses() {
        let not_found = ApiError::not_found("Issue").to_native_response();
        assert_eq!(not_found["error"], "not_found");

        let internal = ApiError::new(599, "x").to_native_response();
        assert_eq!(internal["error"], "internal_error");
    }

    #[test]
    fn display_and_from_serde_error_work() {
        let err = ApiError::forbidden("denied");
        assert_eq!(format!("{}", err), "denied");

        let serde_err = serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
        let converted = ApiError::from(serde_err);
        assert_eq!(converted.status, 500);
        assert!(converted.body.message.contains("serialization error"));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn into_response_uses_status_and_body() {
        use axum::response::IntoResponse;

        let resp = ApiError::validation("Issue", "title", "missing_field").into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json["message"], "Validation Failed");
    }
}
