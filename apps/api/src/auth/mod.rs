//! ADR 0004: invite-only email + password accounts with Postgres sessions.

pub mod password;
pub mod routes;
pub mod session;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::types::ErrorCode;

/// The signed-in user. Add it to a handler's arguments to protect the route (401 otherwise).
#[derive(Debug, Clone, Copy)]
pub struct CurrentUser {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthorized = || ApiError::Code(ErrorCode::Unauthorized);
        let token = session::token_from(&parts.headers).ok_or_else(unauthorized)?;
        let id = sqlx::query_scalar(
            "SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now()",
        )
        .bind(session::hash(token))
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(unauthorized)?;
        Ok(CurrentUser { id })
    }
}
