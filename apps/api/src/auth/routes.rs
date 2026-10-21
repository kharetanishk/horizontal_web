use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse};
use axum::routing::{get, post};
use axum::{Json as JsonOut, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{CurrentUser, password, session};
use crate::http::AppState;
use crate::http::error::{ApiError, Json};
use crate::types::ErrorCode;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/accept-invite", post(accept_invite))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
}

#[derive(Serialize)]
struct Me {
    id: Uuid,
    email: String,
}

#[derive(Deserialize)]
struct AcceptInvite {
    token: String,
    password: String,
}

#[derive(Deserialize)]
struct Login {
    email: String,
    password: String,
}

fn bad_request() -> ApiError {
    ApiError::Code(ErrorCode::BadRequest)
}

fn unauthorized() -> ApiError {
    ApiError::Code(ErrorCode::Unauthorized)
}

fn with_cookie(cookie: String, status: StatusCode, body: impl IntoResponse) -> impl IntoResponse {
    let value = HeaderValue::from_str(&cookie).expect("cookie is ascii");
    (status, AppendHeaders([(header::SET_COOKIE, value)]), body)
}

async fn accept_invite(
    State(s): State<AppState>,
    Json(body): Json<AcceptInvite>,
) -> Result<impl IntoResponse, ApiError> {
    let len = body.password.chars().count();
    if !(password::MIN_LEN..=password::MAX_LEN).contains(&len) || body.token.len() > 128 {
        return Err(bad_request());
    }
    let mut tx = s.pool.begin().await?;
    let token_hash = session::hash(&body.token);
    // Lock order is users then invites everywhere (reset, invite), so no deadlocks.
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM invites WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(unauthorized)?;
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    // Atomic claim: a concurrent second use waits on the user lock, then finds no invite.
    let user_id: Uuid = sqlx::query_scalar(
        "UPDATE invites SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING user_id",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(unauthorized)?;
    let hash = password::hash(body.password).await;
    let email: String = sqlx::query_scalar(
        "UPDATE users SET password_hash = $2 WHERE id = $1 RETURNING email::text",
    )
    .bind(user_id)
    .bind(hash)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM invites WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let cookie = session::create(&mut *tx, user_id).await?;
    tx.commit().await?;
    Ok(with_cookie(
        cookie,
        StatusCode::OK,
        JsonOut(Me { id: user_id, email }),
    ))
}

async fn login(
    State(s): State<AppState>,
    Json(body): Json<Login>,
) -> Result<impl IntoResponse, ApiError> {
    if body.password.chars().count() > password::MAX_LEN || body.email.len() > 254 {
        return Err(bad_request());
    }
    // FOR SHARE held through the session insert serializes against reset-password's FOR UPDATE.
    let mut tx = s.pool.begin().await?;
    let row: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email::text, password_hash FROM users WHERE email = $1::citext FOR SHARE",
    )
    .bind(body.email.trim())
    .fetch_optional(&mut *tx)
    .await?;
    let (found, stored) = match row {
        Some((id, email, stored)) => (Some((id, email)), stored),
        None => (None, None),
    };
    // Unknown user, no password yet and wrong password all do one verify and give one 401.
    let ok = password::verify(body.password, stored).await;
    let Some((id, email)) = found.filter(|_| ok) else {
        return Err(unauthorized());
    };
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
        .execute(&mut *tx)
        .await?;
    let cookie = session::create(&mut *tx, id).await?;
    tx.commit().await?;
    Ok(with_cookie(
        cookie,
        StatusCode::OK,
        JsonOut(Me { id, email }),
    ))
}

async fn logout(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(token) = session::token_from(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(session::hash(token))
            .execute(&s.pool)
            .await?;
    }
    Ok(with_cookie(
        session::clear_cookie(),
        StatusCode::NO_CONTENT,
        (),
    ))
}

async fn me(State(s): State<AppState>, user: CurrentUser) -> Result<JsonOut<Me>, ApiError> {
    let email = sqlx::query_scalar("SELECT email::text FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(unauthorized)?;
    Ok(JsonOut(Me { id: user.id, email }))
}
