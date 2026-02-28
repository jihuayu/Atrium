pub mod admin;
pub mod auth;
pub mod comments;
pub mod export;
pub mod labels;
pub mod reactions;
pub mod threads;

use crate::{router::AppResponse, ApiError};

pub fn respond_native(result: crate::Result<AppResponse>) -> AppResponse {
    match result {
        Ok(response) => response,
        Err(error) => native_error_response(error),
    }
}

pub fn native_error_response(error: ApiError) -> AppResponse {
    AppResponse::json(error.status, &error.to_native_response())
}
