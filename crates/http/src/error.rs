use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Error returned by API handlers: a status code and a message shown in the UI.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    /// Fields added to the JSON body next to `error` (such as an oracle pre-check's `problems`).
    pub detail: Option<serde_json::Map<String, serde_json::Value>>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError { status, message: message.into(), detail: None }
    }
    /// The same error with these fields in its JSON body.
    pub fn with_detail(mut self, detail: serde_json::Map<String, serde_json::Value>) -> ApiError {
        self.detail = Some(detail);
        self
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
        // A write refused by Home's maintenance (a planned update): retryable, never a server error.
        if let Some(m) = launchpad_core::maintenance::of_error(&e) {
            return maintenance(&m);
        }
        tracing::warn!("api error: {e:#}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = self.detail.unwrap_or_default();
        body.insert("error".into(), serde_json::Value::String(self.message));
        (self.status, axum::Json(serde_json::Value::Object(body))).into_response()
    }
}

/// The 503 of a write refused during maintenance: the reason, `retryable` and the `maintenance` object (windows show
/// it, the CLI prints it, agents retry after `retry_after` seconds).
pub fn maintenance(m: &launchpad_core::maintenance::Maintenance) -> ApiError {
    let mut detail = serde_json::Map::new();
    detail.insert("retryable".into(), serde_json::Value::Bool(true));
    detail.insert("maintenance".into(), serde_json::to_value(m).unwrap_or_default());
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, launchpad_core::maintenance::message(m)).with_detail(detail)
}

pub type ApiResult<T> = Result<axum::Json<T>, ApiError>;
