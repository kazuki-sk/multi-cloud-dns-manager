use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,

    #[error("conflict")]
    Conflict,

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    UnprocessableEntity(String),

    #[error("internal server error")]
    Internal(String),
}

impl From<dns_manager_db::Error> for ApiError {
    fn from(e: dns_manager_db::Error) -> Self {
        tracing::error!(error = %e, "database error");
        ApiError::Internal(e.to_string())
    }
}

impl From<dns_manager_core::CryptoError> for ApiError {
    fn from(e: dns_manager_core::CryptoError) -> Self {
        tracing::error!(error = %e, "crypto error");
        ApiError::Internal(e.to_string())
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Conflict => (
                StatusCode::CONFLICT,
                "conflict: resource already exists".to_string(),
            ),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::UnprocessableEntity(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
            // Never expose internal details; the real error is logged in From<db::Error>.
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
