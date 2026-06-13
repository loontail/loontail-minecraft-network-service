//! Mojang/Yggdrasil error shaping. Unlike the rest of the API (which emits
//! `{"error":{"code","message"}}`), the Yggdrasil endpoints must serialize errors
//! as `{"error","errorMessage","cause?"}` with the Mojang status codes (400/401/
//! 403/404/500). State-change successes return 204. This local error type keeps
//! that shape inside the crate so the core `AppError` is not touched.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use loontail_core::AppError;

#[derive(Debug, thiserror::Error)]
pub enum YggError {
    #[error("invalid request")]
    BadRequest(&'static str),
    /// Bad credentials / invalid or expired token. Mojang uses 403 ForbiddenOperation.
    #[error("invalid credentials or token")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("internal error")]
    Internal,
}

/// The Mojang error envelope.
#[derive(Debug, Serialize)]
struct YggErrorBody {
    error: &'static str,
    #[serde(rename = "errorMessage")]
    error_message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<&'static str>,
}

impl YggError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            YggError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "IllegalArgumentException", msg),
            YggError::Forbidden => (
                StatusCode::FORBIDDEN,
                "ForbiddenOperationException",
                "Invalid credentials. Invalid username or password.",
            ),
            YggError::NotFound => (
                StatusCode::NOT_FOUND,
                "NotFoundException",
                "The requested resource was not found.",
            ),
            YggError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Exception",
                "An internal error occurred.",
            ),
        }
    }
}

impl IntoResponse for YggError {
    fn into_response(self) -> Response {
        let (status, error, error_message) = self.parts();
        let body = Json(YggErrorBody {
            error,
            error_message,
            cause: None,
        });
        (status, body).into_response()
    }
}

/// Map a core `AppError` onto the Mojang error shape. Database/internal failures
/// collapse to a 500; auth failures to 403; the rest to their nearest Mojang code.
impl From<AppError> for YggError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::BadRequest(_) => YggError::BadRequest("Invalid request."),
            AppError::Unauthorized | AppError::Forbidden => YggError::Forbidden,
            AppError::NotFound(_) => YggError::NotFound,
            AppError::Conflict(_) => YggError::BadRequest("Conflict."),
            AppError::Database(err) => {
                tracing::error!(error = %err, "yggdrasil database error");
                YggError::Internal
            }
            AppError::Internal(err) => {
                tracing::error!(error = %err, "yggdrasil internal error");
                YggError::Internal
            }
        }
    }
}

impl From<crate::crypto::CryptoError> for YggError {
    fn from(err: crate::crypto::CryptoError) -> Self {
        tracing::error!(error = %err, "yggdrasil crypto error");
        YggError::Internal
    }
}

pub type YggResult<T> = Result<T, YggError>;
