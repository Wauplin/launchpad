use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Error returned by API handlers: a status code and a message shown in the UI.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError { status, message: message.into() }
    }
    pub fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, message)
    }
    pub fn conflict(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, message)
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> ApiError {
        let e = e.into();
        tracing::warn!("api error: {e:#}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

pub type ApiResult<T> = Result<axum::Json<T>, ApiError>;
