use axum::{http::StatusCode, response::IntoResponse};

pub async fn list_providers() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn create_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn get_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn update_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn get_sync_state() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
