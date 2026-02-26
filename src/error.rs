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
