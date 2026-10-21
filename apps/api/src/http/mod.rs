//! Axum app shell. Each feature module exposes `fn routes() -> Router<AppState>`,
//! merged in [`routes`]; handlers return `Result<_, ApiError>`.

pub mod error;
pub mod middleware;
pub mod sanitize;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::Config;
use crate::stream::{Drain, Liveness};
use crate::types::ErrorCode;
use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub liveness: Arc<Liveness>,
    pub drain: Arc<Drain>,
}

pub fn app(state: AppState) -> Router {
    app_with(Router::new(), state)
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/livez", get(|| async {}))
        .route("/api/healthz", get(healthz))
        .merge(crate::auth::routes::routes())
}

/// [`routes`] plus `extra` (tests add their own) in the shared stack. Outermost
/// first: security headers, request log span, panic → 500, Origin check, 1 MB body limit.
pub fn app_with(extra: Router<AppState>, state: AppState) -> Router {
    let header =
        |name, value| SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value));
    let not_found = || async { ApiError::Code(ErrorCode::NotFound) };
    // ponytail: DefaultBodyLimit binds only Json/Bytes/String extractors, and CatchPanicLayer
    // misses panics while a streaming body is polled; when SSE lands, add RequestBodyLimitLayer
    // for raw-body handlers and run stream producers in a guarded task.
    routes()
        .merge(extra)
        .fallback(not_found)
        .method_not_allowed_fallback(not_found)
        .layer(DefaultBodyLimit::max(1 << 20))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::check_origin,
        ))
        .layer(CatchPanicLayer::custom(|_| {
            ApiError::Code(ErrorCode::Internal).into_response()
        }))
        .layer(axum::middleware::from_fn(middleware::log_request))
        .layer(header(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=63072000; includeSubDomains",
        ))
        .layer(header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header(header::REFERRER_POLICY, "no-referrer"))
        .with_state(state)
}

/// Readiness: 503 `unavailable` when Postgres can't be reached.
async fn healthz(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    if state.drain.is_draining() || !state.liveness.is_held() {
        return Err(ApiError::Code(ErrorCode::Unavailable));
    }
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(json!({ "db": "ok" })))
}
