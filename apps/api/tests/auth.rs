//! ADR 0004 auth: invites, sessions, login, logout, reset-password, CurrentUser (tests/common harness).

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::routing::get;
use axum::{Router, response::Response};
use common::{Harness, count, exec};
use nodemind_api::auth::CurrentUser;
use nodemind_api::cli;
use nodemind_api::config::Config;
use nodemind_api::http::error::ApiError;
use nodemind_api::http::{self, AppState, middleware};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tower::ServiceExt;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

const ORIGIN: &str = "https://app.test";
const PASSWORD: &str = "correct horse battery";

static LOGS: Mutex<Vec<u8>> = Mutex::new(Vec::new());

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::registry().with(
        middleware::json_log(|line| LOGS.lock().unwrap().extend_from_slice(line))
            .with_filter(middleware::log_filter()),
    );
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let mut h = Harness::start().await;
    macro_rules! run {
        ($($case:ident),*) => {$( h.run(stringify!($case), true, $case).await; )*};
    }
    run!(
        invite_sets_password_once_and_signs_in,
        expired_invite_is_401,
        no_signup_endpoint,
        unauthenticated_requests_are_401,
        password_length_is_checked_before_claiming,
        concurrent_accept_claims_once,
        login_failures_look_identical,
        logout_deletes_the_session,
        session_cookie_flags_and_expiry,
        reset_password_revokes_everything
    );
    h.finish().await;
}

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Arc::new(Config {
            app_origin: ORIGIN.into(),
        }),
    }
}

fn config() -> Config {
    Config {
        app_origin: ORIGIN.into(),
    }
}

/// The real stack plus a stand-in canvas route guarded by `CurrentUser`.
fn app(pool: PgPool) -> Router {
    async fn canvases(user: CurrentUser, State(_): State<AppState>) -> Result<String, ApiError> {
        Ok(user.id.to_string())
    }
    http::app_with(
        Router::new().route("/api/canvases", get(canvases)),
        state(pool),
    )
}

async fn send(
    pool: &PgPool,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Value,
) -> (Response, String) {
    let mut r = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("origin", ORIGIN);
    if let Some(c) = cookie {
        r = r.header("cookie", c);
    }
    let res = app(pool.clone())
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 16).await.unwrap();
    (
        Response::from_parts(parts, Body::empty()),
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn token_of(link: &str) -> String {
    let (base, token) = link.split_once("/invite#t=").expect("fragment link");
    assert_eq!(base, ORIGIN);
    token.to_owned()
}

/// `name=value` of the session cookie, ready for a `Cookie` header.
fn cookie_of(res: &Response) -> String {
    let set = res.headers()["set-cookie"].to_str().unwrap();
    set.split(';').next().unwrap().to_owned()
}

async fn invited(pool: &PgPool, email: &str) -> String {
    token_of(&cli::invite(pool, &config(), email).await.unwrap())
}

/// Invites `email`, accepts with PASSWORD and returns the session cookie.
async fn signed_up(pool: &PgPool, email: &str) -> String {
    let token = invited(pool, email).await;
    let (res, _) = send(
        pool,
        "POST",
        "/api/auth/accept-invite",
        None,
        json!({ "token": token, "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status(), 200);
    cookie_of(&res)
}

fn error_code(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap();
    v["error"]["code"].as_str().unwrap().to_owned()
}

async fn invite_sets_password_once_and_signs_in(pool: PgPool, _: PgConnectOptions) {
    let token = invited(&pool, "Ann@Test.com").await;
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM invites")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.len(), 32);
    assert_ne!(stored, token.as_bytes());

    let accept = json!({ "token": token, "password": PASSWORD });
    let (res, body) = send(
        &pool,
        "POST",
        "/api/auth/accept-invite",
        None,
        accept.clone(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["email"],
        "Ann@Test.com"
    );
    let cookie = cookie_of(&res);

    let (res, body) = send(&pool, "GET", "/api/me", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["email"],
        "Ann@Test.com"
    );
    let (res, _) = send(&pool, "GET", "/api/canvases", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 200);

    // the argon2id hash is stored, never the password; the cookie token is stored only hashed
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    let raw = cookie.strip_prefix("__Host-nm_session=").unwrap();
    let plain: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE token_hash = $1")
        .bind(raw.as_bytes())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(plain, 0);

    // second use → 401, even with a different valid password
    for password in [PASSWORD, "another valid password"] {
        let (res, body) = send(
            &pool,
            "POST",
            "/api/auth/accept-invite",
            None,
            json!({ "token": token, "password": password }),
        )
        .await;
        assert_eq!(res.status(), 401);
        assert_eq!(error_code(&body), "unauthorized");
    }
    let after: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, hash, "second use must not change the password");

    // a bogus token → 401 too
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/accept-invite",
        None,
        json!({ "token": "nope", "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status(), 401);
}

async fn expired_invite_is_401(pool: PgPool, _: PgConnectOptions) {
    let token = invited(&pool, "a@test.com").await;
    exec(
        &pool,
        "UPDATE invites SET expires_at = now() - interval '1 second'",
    )
    .await
    .unwrap();
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/accept-invite",
        None,
        json!({ "token": token, "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status(), 401);
    assert_eq!(count(&pool, "sessions").await, 0);
}

async fn no_signup_endpoint(pool: PgPool, _: PgConnectOptions) {
    for uri in [
        "/api/auth/signup",
        "/api/auth/register",
        "/api/signup",
        "/api/users",
    ] {
        let (res, _) = send(
            &pool,
            "POST",
            uri,
            None,
            json!({ "email": "x@test.com", "password": PASSWORD }),
        )
        .await;
        assert_eq!(res.status(), 404, "{uri}");
    }
    assert_eq!(count(&pool, "users").await, 0);
}

async fn unauthenticated_requests_are_401(pool: PgPool, _: PgConnectOptions) {
    for (uri, cookie) in [
        ("/api/me", None),
        ("/api/canvases", None),
        ("/api/canvases", Some("__Host-nm_session=garbage")),
        ("/api/canvases", Some("other=1")),
    ] {
        let (res, body) = send(&pool, "GET", uri, cookie, json!({})).await;
        assert_eq!(res.status(), 401, "{uri} {cookie:?}");
        assert_eq!(error_code(&body), "unauthorized");
    }
}

async fn password_length_is_checked_before_claiming(pool: PgPool, _: PgConnectOptions) {
    let token = invited(&pool, "a@test.com").await;
    for password in ["x".repeat(11), "x".repeat(129)] {
        let (res, body) = send(
            &pool,
            "POST",
            "/api/auth/accept-invite",
            None,
            json!({ "token": token, "password": password }),
        )
        .await;
        assert_eq!(res.status(), 400);
        assert_eq!(error_code(&body), "bad_request");
    }
    // the rejected attempts left the invite unclaimed and the account without a password
    let used: i64 = sqlx::query_scalar("SELECT count(*) FROM invites WHERE used_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(used, 1);
    // bounds are inclusive: 12 and 128 chars work
    for (n, email) in [(12, "b@test.com"), (128, "c@test.com")] {
        let token = invited(&pool, email).await;
        let (res, _) = send(
            &pool,
            "POST",
            "/api/auth/accept-invite",
            None,
            json!({ "token": token, "password": "é".repeat(n) }),
        )
        .await;
        assert_eq!(res.status(), 200, "{n}");
    }
    // login also refuses oversized passwords before hashing
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/login",
        None,
        json!({ "email": "b@test.com", "password": "x".repeat(129) }),
    )
    .await;
    assert_eq!(res.status(), 400);
}

async fn concurrent_accept_claims_once(pool: PgPool, _: PgConnectOptions) {
    let token = invited(&pool, "a@test.com").await;
    let attempt = |password: &'static str| {
        let (pool, token) = (pool.clone(), token.clone());
        async move {
            send(
                &pool,
                "POST",
                "/api/auth/accept-invite",
                None,
                json!({ "token": token, "password": password }),
            )
            .await
            .0
            .status()
            .as_u16()
        }
    };
    let (a, b) = tokio::join!(
        attempt("first valid password"),
        attempt("second valid password")
    );
    let mut got = [a, b];
    got.sort();
    assert_eq!(got, [200, 401]);
    assert_eq!(count(&pool, "sessions").await, 1);
}

async fn login_failures_look_identical(pool: PgPool, _: PgConnectOptions) {
    signed_up(&pool, "a@test.com").await;
    cli::invite(&pool, &config(), "pending@test.com")
        .await
        .unwrap(); // no password yet
    let mut seen = Vec::new();
    for (email, password) in [
        ("nobody@test.com", PASSWORD),
        ("a@test.com", "wrong password here"),
        ("pending@test.com", PASSWORD),
    ] {
        let (res, body) = send(
            &pool,
            "POST",
            "/api/auth/login",
            None,
            json!({ "email": email, "password": password }),
        )
        .await;
        assert_eq!(res.status(), 401, "{email}");
        assert!(res.headers().get("set-cookie").is_none());
        seen.push(body);
    }
    assert!(seen.iter().all(|b| *b == seen[0]), "{seen:?}");

    // success is case-insensitive on email and issues a working session
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/login",
        None,
        json!({ "email": "A@TEST.com", "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status(), 200);
    let (res, _) = send(&pool, "GET", "/api/me", Some(&cookie_of(&res)), json!({})).await;
    assert_eq!(res.status(), 200);
}

async fn logout_deletes_the_session(pool: PgPool, _: PgConnectOptions) {
    let cookie = signed_up(&pool, "a@test.com").await;
    assert_eq!(count(&pool, "sessions").await, 1);
    let (res, _) = send(&pool, "POST", "/api/auth/logout", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 204);
    assert!(
        res.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(count(&pool, "sessions").await, 0);
    let (res, _) = send(&pool, "GET", "/api/me", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 401);
}

async fn session_cookie_flags_and_expiry(pool: PgPool, _: PgConnectOptions) {
    let token = invited(&pool, "a@test.com").await;
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/accept-invite",
        None,
        json!({ "token": token, "password": PASSWORD }),
    )
    .await;
    let set = res.headers()["set-cookie"].to_str().unwrap().to_owned();
    for flag in [
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=2592000",
    ] {
        assert!(set.contains(flag), "{flag} missing in {set}");
    }
    let days: f64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM expires_at - now()) / 86400)::float8 FROM sessions",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((29.9..=30.0).contains(&days), "{days}");

    let cookie = cookie_of(&res);
    exec(
        &pool,
        "UPDATE sessions SET expires_at = now() - interval '1 second'",
    )
    .await
    .unwrap();
    let (res, _) = send(&pool, "GET", "/api/me", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 401);
}

async fn reset_password_revokes_everything(pool: PgPool, _: PgConnectOptions) {
    let cookie = signed_up(&pool, "a@test.com").await;
    let (_, other_login) = (
        (),
        send(
            &pool,
            "POST",
            "/api/auth/login",
            None,
            json!({ "email": "a@test.com", "password": PASSWORD }),
        )
        .await,
    );
    assert_eq!(other_login.0.status(), 200);
    assert_eq!(count(&pool, "sessions").await, 2);
    let stale = invited_again(&pool).await; // reset must also kill outstanding invites

    LOGS.lock().unwrap().clear();
    let link = cli::reset_password(&pool, &config(), "A@test.com")
        .await
        .unwrap();
    let fresh = token_of(&link);

    assert_eq!(count(&pool, "sessions").await, 0);
    assert_eq!(count(&pool, "invites").await, 1);
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(hash.is_none());
    let tombstones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tombstones t JOIN users u ON u.id = t.id WHERE t.kind = 'password_reset'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tombstones, 1);
    let logs = String::from_utf8(LOGS.lock().unwrap().clone()).unwrap();
    let line = logs
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|l| l["event"] == "tombstone")
        .expect("tombstone log line");
    assert!(line["user_id"].as_str().is_some());
    assert!(
        !logs.contains(&fresh) && !logs.contains("a@test.com"),
        "{logs}"
    );

    let (res, _) = send(&pool, "GET", "/api/me", Some(&cookie), json!({})).await;
    assert_eq!(res.status(), 401);
    let (res, _) = send(
        &pool,
        "POST",
        "/api/auth/login",
        None,
        json!({ "email": "a@test.com", "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status(), 401);
    for (token, want) in [(stale, 401), (fresh, 200)] {
        let (res, _) = send(
            &pool,
            "POST",
            "/api/auth/accept-invite",
            None,
            json!({ "token": token, "password": "a brand new password" }),
        )
        .await;
        assert_eq!(res.status(), want);
    }
    assert!(
        cli::reset_password(&pool, &config(), "ghost@test.com")
            .await
            .is_err()
    );
}

/// An unused invite for an existing user, as left behind by an earlier operator action.
async fn invited_again(pool: &PgPool) -> String {
    let token = nodemind_api::auth::session::new_token();
    sqlx::query(
        "INSERT INTO invites (token_hash, user_id, expires_at) SELECT $1, id, now() + interval '1 day' FROM users",
    )
    .bind(nodemind_api::auth::session::hash(&token))
    .execute(pool)
    .await
    .unwrap();
    token
}
