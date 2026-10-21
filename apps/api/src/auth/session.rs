//! Random 32-byte tokens (sessions and invites); only their SHA-256 is stored.

use axum::http::{HeaderMap, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use sqlx::PgExecutor;
use uuid::Uuid;

const COOKIE: &str = "__Host-nm_session";
const DAYS: i32 = 30;

pub fn new_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Inserts a 30-day session and returns the `Set-Cookie` value carrying its token.
pub async fn create(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<String> {
    let token = new_token();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(hash(&token))
    .bind(user_id)
    .bind(DAYS)
    .execute(db)
    .await?;
    Ok(format!(
        "{COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        DAYS * 86400
    ))
}

pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
}

pub fn token_from(headers: &HeaderMap) -> Option<&str> {
    let mut found = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='));
    // More than one session cookie is ambiguous (planted sibling cookie): treat as none.
    let first = found.next()?;
    found
        .next()
        .is_none()
        .then_some(first)
        .filter(|t| !t.is_empty())
}
