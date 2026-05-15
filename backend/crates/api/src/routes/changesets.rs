use axum::{http::StatusCode, response::IntoResponse};

pub async fn list_changesets() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn create_changeset() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn get_changeset() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

/// Transition: draft → validated (pre-flight validation).
pub async fn validate_changeset() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

/// Transition: validated → applying (kick off the reconcile worker).
pub async fn apply_changeset() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

/// Transition: applied / frozen → rolling_back (manual rollback).
pub async fn rollback_changeset() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
