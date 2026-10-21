//! HTTP shell: Origin check, error mapping, panics, log allowlist (NFR-9), health,
//! sanitize (harness: tests/common).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Router, response::Response};
use common::Harness;
use nodemind_api::config::{Config, normalize_origin};
use nodemind_api::http::error::{ApiError, Json};
use nodemind_api::http::sanitize::sanitize;
use nodemind_api::http::{self, AppState, middleware};
use proptest::prelude::*;
use serde_json::Value;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::ServiceExt;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

const ORIGIN: &str = "https://app.test";
const MARKER: &str = "MARKER-7f3a-secret-question";

static LOGS: Mutex<Vec<u8>> = Mutex::new(Vec::new());

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::registry().with(
        middleware::json_log(|line| {
            LOGS.lock().unwrap().extend_from_slice(line);
            eprint!("{}", String::from_utf8_lossy(line)); // failing asserts show as panic locations
        })
        .with_filter(middleware::log_filter()),
    );
    tracing::subscriber::set_global_default(subscriber).unwrap();
    middleware::install_panic_hook();

    sanitize_replaces_nul();
    println!("test sanitize_replaces_nul ... ok");
    app_origin_is_normalized();
    println!("test app_origin_is_normalized ... ok");

    let mut h = Harness::start().await;
    macro_rules! run {
        ($($case:ident),*) => {$( h.run(stringify!($case), false, $case).await; )*};
    }
    run!(
        mutation_without_matching_origin_is_403,
        get_and_head_need_no_origin,
        malformed_json_is_generic_400,
        body_over_1mb_is_rejected,
        panic_is_500_and_logs_no_body,
        logs_keep_only_allowlisted_fields,
        security_headers_on_every_response,
        unknown_route_and_method_are_json_404,
        dependency_logs_drop_text,
        db_errors_are_classified,
        livez_needs_no_db,
        healthz_ok_with_db,
        healthz_503_without_db
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

/// The real stack plus test routes that echo, log and panic with the request body,
/// and fail in Postgres. Handler logs use the crate's target, as app code would.
fn app(pool: PgPool) -> Router {
    async fn echo(Json(body): Json<Value>) -> Result<StatusCode, ApiError> {
        tracing::info!(target: "nodemind_api::test", content = %body, node_id = "n1", "handled");
        Ok(StatusCode::NO_CONTENT)
    }
    async fn boom(Json(body): Json<Value>) -> StatusCode {
        panic!("boom {body}");
    }
    async fn div0(State(s): State<AppState>) -> Result<(), ApiError> {
        sqlx::query("SELECT 1 / 0").execute(&s.pool).await?;
        Ok(())
    }
    async fn missing(State(s): State<AppState>) -> Result<(), ApiError> {
        sqlx::query("SELECT 1 WHERE false")
            .fetch_one(&s.pool)
            .await?;
        Ok(())
    }
    let routes = Router::new()
        .route("/api/echo", post(echo))
        .route("/api/boom", post(boom))
        .route("/api/div0", get(div0))
        .route("/api/missing", get(missing));
    http::app_with(routes, state(pool))
}

/// A pool whose every acquire fails fast: nothing listens on port 1.
fn dead_pool() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy_with(PgConnectOptions::new().host("127.0.0.1").port(1))
}

fn req(method: &str, uri: &str, origin: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(o) = origin {
        r = r.header("origin", o);
    }
    r.body(body.into()).unwrap()
}

async fn send(app: Router, r: Request<Body>) -> (Response, String) {
    let res = app.oneshot(r).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 16).await.unwrap();
    (
        Response::from_parts(parts, Body::empty()),
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn error_code(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap();
    v["error"]["code"].as_str().unwrap().to_owned()
}

fn take_logs() -> String {
    String::from_utf8(std::mem::take(&mut *LOGS.lock().unwrap())).unwrap()
}

fn log_lines(logs: &str) -> Vec<Value> {
    logs.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn marker_body() -> String {
    serde_json::json!({ "content": MARKER }).to_string()
}

async fn mutation_without_matching_origin_is_403(_: PgPool, _: PgConnectOptions) {
    let pool = dead_pool();
    for origin in [
        None,
        Some("https://evil.test"),
        Some("https://app.test.evil.test"),
    ] {
        let (res, body) = send(
            app(pool.clone()),
            req("POST", "/api/echo", origin, marker_body()),
        )
        .await;
        assert_eq!(res.status(), 403, "{origin:?}");
        assert_eq!(error_code(&body), "forbidden_origin");
    }
    // bodyless, no content type, other verbs, unknown routes: same check
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let r = Request::builder()
            .method(method)
            .uri("/api/nodes/x/retry")
            .body(Body::empty())
            .unwrap();
        let (res, body) = send(app(pool.clone()), r).await;
        assert_eq!(res.status(), 403, "{method}");
        assert_eq!(error_code(&body), "forbidden_origin");
    }
    let (res, _) = send(
        app(pool),
        req("POST", "/api/echo", Some(ORIGIN), marker_body()),
    )
    .await;
    assert_eq!(res.status(), 204);
}

async fn get_and_head_need_no_origin(_: PgPool, _: PgConnectOptions) {
    for method in ["GET", "HEAD"] {
        let (res, _) = send(app(dead_pool()), req(method, "/api/livez", None, "")).await;
        assert_eq!(res.status(), 200, "{method}");
    }
}

async fn malformed_json_is_generic_400(_: PgPool, _: PgConnectOptions) {
    take_logs();
    let bad = [
        format!(r#"{{"content": "{MARKER}"#),         // truncated
        format!(r#"{{"content": {MARKER}}}"#),        // bare word
        format!(r#"["{MARKER}"] trailing {MARKER}"#), // trailing garbage
    ];
    for input in bad {
        let (res, body) = send(
            app(dead_pool()),
            req("POST", "/api/echo", Some(ORIGIN), input),
        )
        .await;
        assert_eq!(res.status(), 400);
        assert_eq!(error_code(&body), "bad_request");
        assert!(!body.contains(MARKER), "{body}");
        assert!(!body.contains("line") && !body.contains("column"), "{body}");
    }
    let logs = take_logs();
    assert!(logs.contains(r#""error_code":"bad_request""#), "{logs}");
    assert!(!logs.contains(MARKER), "{logs}");
}

async fn body_over_1mb_is_rejected(_: PgPool, _: PgConnectOptions) {
    let big = serde_json::json!({ "content": "a".repeat(1 << 20) }).to_string();
    let (res, body) = send(
        app(dead_pool()),
        req("POST", "/api/echo", Some(ORIGIN), big),
    )
    .await;
    assert_eq!(res.status(), 400);
    assert_eq!(error_code(&body), "bad_request");
    let just_under = serde_json::json!({ "content": "a".repeat((1 << 20) - 20) }).to_string();
    let (res, _) = send(
        app(dead_pool()),
        req("POST", "/api/echo", Some(ORIGIN), just_under),
    )
    .await;
    assert_eq!(res.status(), 204);
}

async fn panic_is_500_and_logs_no_body(_: PgPool, _: PgConnectOptions) {
    take_logs();
    let (res, body) = send(
        app(dead_pool()),
        req("POST", "/api/boom", Some(ORIGIN), marker_body()),
    )
    .await;
    assert_eq!(res.status(), 500);
    assert_eq!(error_code(&body), "internal");
    assert!(!body.contains(MARKER));

    let logs = take_logs();
    assert!(!logs.contains(MARKER), "{logs}");
    assert!(!logs.contains("boom {"), "panic payload logged: {logs}");
    let lines = log_lines(&logs);
    let panic = lines
        .iter()
        .find(|l| l["message"] == "panic")
        .expect("panic line");
    assert!(
        panic["location"].as_str().unwrap().contains("http.rs:"),
        "{panic}"
    );
    let done = lines
        .iter()
        .find(|l| l["message"] == "request")
        .expect("request line");
    assert_eq!(done["status"], 500);
    assert_eq!(panic["request_id"], done["request_id"]);
}

/// NFR-9: a marker sent as message content never reaches the logs, even when a
/// handler logs it under a non-allowlisted field.
async fn logs_keep_only_allowlisted_fields(_: PgPool, _: PgConnectOptions) {
    take_logs();
    let (res, _) = send(
        app(dead_pool()),
        req("POST", "/api/echo", Some(ORIGIN), marker_body()),
    )
    .await;
    assert_eq!(res.status(), 204);

    let logs = take_logs();
    assert!(!logs.contains(MARKER), "{logs}");
    let lines = log_lines(&logs);
    let handled = lines
        .iter()
        .find(|l| l["message"] == "handled")
        .expect("handler line");
    assert_eq!(handled["node_id"], "n1");
    assert!(handled.get("content").is_none());
    let done = lines
        .iter()
        .find(|l| l["message"] == "request")
        .expect("request line");
    assert_eq!(done["route"], "/api/echo");
    assert_eq!(done["status"], 204);
    assert!(done["latency_ms"].is_u64());
    assert!(done["request_id"].as_str().is_some_and(|id| id.len() == 36));
    assert_eq!(handled["request_id"], done["request_id"]);
}

async fn security_headers_on_every_response(_: PgPool, _: PgConnectOptions) {
    let cases = [
        req("GET", "/api/livez", None, ""),
        req("POST", "/api/echo", None, ""),
        req("GET", "/api/nope", None, ""),
    ];
    for r in cases {
        let (res, _) = send(app(dead_pool()), r).await;
        let h = res.headers();
        assert_eq!(
            h["strict-transport-security"],
            "max-age=63072000; includeSubDomains"
        );
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["referrer-policy"], "no-referrer");
    }
}

async fn livez_needs_no_db(_: PgPool, _: PgConnectOptions) {
    let (res, _) = send(
        http::app(state(dead_pool())),
        req("GET", "/api/livez", None, ""),
    )
    .await;
    assert_eq!(res.status(), 200);
}

async fn healthz_ok_with_db(pool: PgPool, _: PgConnectOptions) {
    let (res, body) = send(http::app(state(pool)), req("GET", "/api/healthz", None, "")).await;
    assert_eq!(res.status(), 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        serde_json::json!({"db": "ok"})
    );
}

async fn healthz_503_without_db(_: PgPool, _: PgConnectOptions) {
    let (res, body) = send(
        http::app(state(dead_pool())),
        req("GET", "/api/healthz", None, ""),
    )
    .await;
    assert_eq!(res.status(), 503);
    assert_eq!(error_code(&body), "unavailable");
}

async fn unknown_route_and_method_are_json_404(_: PgPool, _: PgConnectOptions) {
    for r in [
        req("GET", "/api/nope", None, ""),
        req("GET", "/api/echo", None, ""), // POST-only route
    ] {
        let (res, body) = send(app(dead_pool()), r).await;
        assert_eq!(res.status(), 404);
        assert_eq!(error_code(&body), "not_found");
    }
}

/// NFR-9: dependency INFO is filtered out, and dependency WARN+ keeps no free text.
async fn dependency_logs_drop_text(_: PgPool, _: PgConnectOptions) {
    take_logs();
    tracing::info!(target: "some_dep", "info {MARKER}");
    tracing::warn!(target: "some_dep", "warn {MARKER}");
    let logs = take_logs();
    assert!(!logs.contains(MARKER), "{logs}");
    let lines = log_lines(&logs);
    assert_eq!(lines.len(), 1, "{logs}");
    assert_eq!(lines[0]["target"], "some_dep");
    assert_eq!(lines[0]["level"], "WARN");
}

async fn db_errors_are_classified(pool: PgPool, _: PgConnectOptions) {
    take_logs();
    let (res, body) = send(app(pool.clone()), req("GET", "/api/div0", None, "")).await;
    assert_eq!(res.status(), 500);
    assert_eq!(error_code(&body), "internal");
    let logs = take_logs();
    let db = log_lines(&logs)
        .into_iter()
        .find(|l| l["message"] == "db error")
        .expect("db error line");
    assert_eq!(db["db_error"], "database");
    assert_eq!(db["sqlstate"], "22012");
    assert!(!logs.contains("division"), "driver text logged: {logs}");

    let (res, body) = send(app(pool), req("GET", "/api/missing", None, "")).await;
    assert_eq!(res.status(), 404);
    assert_eq!(error_code(&body), "not_found");

    let (res, _) = send(
        http::app(state(dead_pool())),
        req("GET", "/api/healthz", None, ""),
    )
    .await;
    assert_eq!(res.status(), 503);
    let logs = take_logs();
    assert!(logs.contains(r#""db_error":"pool_timed_out""#), "{logs}");
}

fn app_origin_is_normalized() {
    for (raw, want) in [
        ("https://app.example.com", "https://app.example.com"),
        ("https://app.example.com/", "https://app.example.com"),
        ("HTTPS://App.Example.COM:443", "https://app.example.com"),
        ("http://localhost:80", "http://localhost"),
        ("http://localhost:3000/", "http://localhost:3000"),
    ] {
        assert_eq!(normalize_origin(raw).as_deref(), Some(want), "{raw}");
    }
    for raw in [
        "https://app.example.com/app",
        "https://app.example.com?x=1",
        "https://app.example.com#f",
        "https://u@app.example.com",
        "ftp://app.example.com",
        "app.example.com",
        "",
    ] {
        assert_eq!(normalize_origin(raw), None, "{raw}");
    }
}

fn sanitize_replaces_nul() {
    let chars = prop::collection::vec(prop_oneof![Just('\0'), any::<char>()], 0..64);
    proptest!(|(chars in chars)| {
        let input: String = chars.iter().collect();
        let out = sanitize(input.clone());
        prop_assert!(!out.contains('\0'));
        prop_assert_eq!(out.chars().count(), chars.len());
        for (a, b) in chars.iter().zip(out.chars()) {
            prop_assert_eq!(b, if *a == '\0' { '\u{FFFD}' } else { *a });
        }
    });
    assert_eq!(sanitize("a\0b".into()), "a\u{FFFD}b");
}
