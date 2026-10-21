//! Operator commands: `invite <email>` and `reset-password <email>` print a single-use,
//! 7-day link whose token is stored only as a hash.

use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::session;
use crate::config::Config;

type CliResult = Result<String, Box<dyn std::error::Error>>;

/// Creates the user if needed and returns a fresh invite link. Refuses an account that
/// already has a password; that is `reset_password`.
pub async fn invite(pool: &PgPool, config: &Config, email: &str) -> CliResult {
    let email = email.trim();
    if email.len() > 254 || !email.contains('@') {
        return Err("invalid email".into());
    }
    let mut tx = pool.begin().await?;
    let (id, has_password): (Uuid, bool) = sqlx::query_as(
        "INSERT INTO users (id, email) VALUES ($1, $2) ON CONFLICT (email) DO UPDATE SET email = users.email RETURNING id, password_hash IS NOT NULL",
    )
    .bind(Uuid::now_v7())
    .bind(email)
    .fetch_one(&mut *tx)
    .await?;
    if has_password {
        return Err("user already has a password; use reset-password".into());
    }
    sqlx::query("DELETE FROM invites WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let link = new_invite(&mut tx, config, id).await?;
    tx.commit().await?;
    Ok(link)
}

/// Drops invites and sessions, clears the password, records a tombstone and issues a new link.
pub async fn reset_password(pool: &PgPool, config: &Config, email: &str) -> CliResult {
    let mut tx = pool.begin().await?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = $1::citext FOR UPDATE")
        .bind(email.trim())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or("no such user")?;
    sqlx::query("DELETE FROM invites WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO tombstones (kind, id) VALUES ('password_reset', $1)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let link = new_invite(&mut tx, config, id).await?;
    tx.commit().await?;
    tracing::info!(event = "tombstone", user_id = %id, "password_reset");
    Ok(link)
}

async fn new_invite(
    tx: &mut sqlx::PgConnection,
    config: &Config,
    user_id: Uuid,
) -> Result<String, sqlx::Error> {
    let token = session::new_token();
    sqlx::query(
        "INSERT INTO invites (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '7 days')",
    )
    .bind(session::hash(&token))
    .bind(user_id)
    .execute(tx)
    .await?;
    Ok(format!("{}/invite#t={token}", config.app_origin))
}
