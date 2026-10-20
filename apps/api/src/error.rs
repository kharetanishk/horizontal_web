use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// API error in the documented `{ "error": { "code", "message" } }` shape.
// ponytail: only the variants metering needs; T4 grows this into the full code list.
#[derive(Debug)]
pub enum ApiError {
    /// 429 with `Retry-After` (seconds).
    TooManyRequests {
        code: &'static str,
        retry_after: u64,
    },
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Db(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after) = match self {
            ApiError::TooManyRequests { code, retry_after } => (
                StatusCode::TOO_MANY_REQUESTS,
                code,
                "Too many requests",
                Some(retry_after),
            ),
            ApiError::Db(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Service unavailable",
                None,
            ),
        };
        let body = format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#);
        let mut res = (status, [(header::CONTENT_TYPE, "application/json")], body).into_response();
        if let Some(secs) = retry_after {
            res.headers_mut().insert(header::RETRY_AFTER, secs.into());
        }
        res
    }
}
