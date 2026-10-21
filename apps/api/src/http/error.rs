use axum::extract::FromRequest;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::types::ErrorCode;

/// API error in the documented `{ "error": { "code", "message" } }` shape.
/// Messages are fixed per code, so no input is ever echoed (NFR-9).
#[derive(Debug)]
pub enum ApiError {
    Code(ErrorCode),
    /// 429 with `Retry-After` (seconds).
    TooManyRequests {
        code: ErrorCode,
        retry_after: u64,
    },
    /// Connection-level failures → 503 `unavailable`, `RowNotFound` → 404, anything else
    /// (constraint, SQL bugs) → 500 `internal`. Only the variant and SQLSTATE are logged;
    /// the driver message can quote row values.
    Db(sqlx::Error),
}

/// Error code stashed on the response for the request log line.
#[derive(Clone)]
pub struct LoggedErrorCode(pub String);

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Db(e)
    }
}

/// Any malformed, oversized or mistyped JSON body becomes a bare `bad_request`.
impl From<JsonRejection> for ApiError {
    fn from(_: JsonRejection) -> Self {
        ApiError::Code(ErrorCode::BadRequest)
    }
}

/// `axum::Json` whose rejection is the generic [`ApiError`] instead of serde's input-quoting text.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Json<T>(pub T);

fn status_and_message(code: ErrorCode) -> (StatusCode, &'static str) {
    use ErrorCode::*;
    match code {
        BadRequest => (StatusCode::BAD_REQUEST, "Invalid request"),
        Unauthorized => (StatusCode::UNAUTHORIZED, "Sign in required"),
        ForbiddenOrigin => (StatusCode::FORBIDDEN, "Forbidden origin"),
        NotFound => (StatusCode::NOT_FOUND, "Not found"),
        Conflict => (StatusCode::CONFLICT, "Conflict"),
        MessageTooLong => (StatusCode::UNPROCESSABLE_ENTITY, "Message too long"),
        InvalidAnchor => (StatusCode::UNPROCESSABLE_ENTITY, "Invalid anchor"),
        ParentNotComplete => (StatusCode::UNPROCESSABLE_ENTITY, "Parent is not complete"),
        BranchStreaming => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Branch is still streaming",
        ),
        MergeBudgetExhausted => (StatusCode::UNPROCESSABLE_ENTITY, "Merge budget exhausted"),
        RateLimited | DailyCapReached => (StatusCode::TOO_MANY_REQUESTS, "Too many requests"),
        LlmUnavailable => (StatusCode::BAD_GATEWAY, "Model provider unavailable"),
        Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable"),
        Internal => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code_of = |c: ErrorCode| serde_json::json!(c).as_str().unwrap_or_default().to_owned();
        let (status, code, message, retry_after) = match self {
            ApiError::Code(c) => {
                let (status, message) = status_and_message(c);
                (status, code_of(c), message, None)
            }
            ApiError::TooManyRequests { code, retry_after } => (
                StatusCode::TOO_MANY_REQUESTS,
                code_of(code),
                "Too many requests",
                Some(retry_after),
            ),
            ApiError::Db(e) => {
                let c = db_error_code(&e);
                let (status, message) = status_and_message(c);
                (status, code_of(c), message, None)
            }
        };
        let body = serde_json::json!({ "error": { "code": code, "message": message } });
        let mut res = (status, axum::Json(body)).into_response();
        if let Some(secs) = retry_after {
            res.headers_mut().insert(header::RETRY_AFTER, secs.into());
        }
        res.extensions_mut().insert(LoggedErrorCode(code));
        res
    }
}

/// Classifies and logs a driver error without its text.
fn db_error_code(e: &sqlx::Error) -> ErrorCode {
    use sqlx::Error::*;
    let (db_error, code) = match e {
        PoolTimedOut => ("pool_timed_out", ErrorCode::Unavailable),
        PoolClosed => ("pool_closed", ErrorCode::Unavailable),
        Io(_) => ("io", ErrorCode::Unavailable),
        Tls(_) => ("tls", ErrorCode::Unavailable),
        RowNotFound => ("row_not_found", ErrorCode::NotFound),
        Database(_) => ("database", ErrorCode::Internal),
        _ => ("other", ErrorCode::Internal),
    };
    let sqlstate = e
        .as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned());
    tracing::error!(db_error, sqlstate = sqlstate.as_deref(), "db error");
    code
}
