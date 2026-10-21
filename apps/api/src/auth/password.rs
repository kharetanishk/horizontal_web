//! argon2id hashing, always off the async threads.

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

pub const MIN_LEN: usize = 12;
pub const MAX_LEN: usize = 128;

// Bounds concurrent argon2 jobs (~19 MiB each) so unauthenticated logins can't exhaust
// memory or the blocking pool. ponytail: no per-IP/email throttle; add when exposed publicly.
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub async fn hash(password: String) -> String {
    let _slot = SLOTS.acquire().await.expect("semaphore open");
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .expect("argon2 hash")
            .to_string()
    })
    .await
    .expect("hash task")
}

/// Checks `password` against `stored`. With no usable hash it verifies against a dummy one
/// so unknown emails and password-less users cost the same time as a wrong password.
pub async fn verify(password: String, stored: Option<String>) -> bool {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let _slot = SLOTS.acquire().await.expect("semaphore open");
    tokio::task::spawn_blocking(move || {
        let real = stored
            .as_deref()
            .and_then(|s| s.parse::<PasswordHash>().ok());
        let usable = real.is_some();
        let parsed = real.unwrap_or_else(|| {
            DUMMY
                .get_or_init(|| {
                    Argon2::default()
                        .hash_password(b"dummy password for timing")
                        .expect("argon2 hash")
                        .to_string()
                })
                .parse()
                .expect("dummy hash")
        });
        let matched = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        matched && usable
    })
    .await
    .expect("verify task")
}
