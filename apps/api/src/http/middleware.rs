use std::cell::Cell;
use std::fmt;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Instrument, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, LoggedErrorCode};
use crate::types::ErrorCode;

/// CSRF: every non-GET/HEAD request, bodyless or not, must carry `Origin` equal to the app origin.
pub async fn check_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let safe = req.method() == Method::GET || req.method() == Method::HEAD;
    let origin = req.headers().get(header::ORIGIN).map(|o| o.as_bytes());
    if !safe && origin != Some(state.config.app_origin.as_bytes()) {
        return ApiError::Code(ErrorCode::ForbiddenOrigin).into_response();
    }
    next.run(req).await
}

/// Runs the request inside a span carrying `request_id` and the route template
/// (never the raw URI), then logs status, latency and error code.
pub async fn log_request(req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", |p| p.as_str())
        .to_owned();
    let span = tracing::info_span!("request", request_id = %Uuid::now_v7(), route);
    let start = Instant::now();
    let res = next.run(req).instrument(span.clone()).await;
    let error_code = res
        .extensions()
        .get::<LoggedErrorCode>()
        .map(|c| c.0.as_str());
    tracing::info!(
        parent: &span,
        status = res.status().as_u16(),
        latency_ms = start.elapsed().as_millis() as u64,
        error_code,
        "request"
    );
    res
}

/// Logs a panic as its location and thread name only, process-wide: the payload may
/// quote user content (NFR-9), so background-task panics lose their message too.
/// The current request span supplies `request_id`. Install after startup, so
/// startup failures still print their message.
pub fn install_panic_hook() {
    thread_local!(static IN_HOOK: Cell<bool> = const { Cell::new(false) });
    std::panic::set_hook(Box::new(|info| {
        // A panic inside the log sink would re-enter here; don't log it again.
        if IN_HOOK.replace(true) {
            return;
        }
        let location = info.location().map_or_else(
            || "unknown".to_owned(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        tracing::error!(location, thread, "panic");
        IN_HOOK.set(false);
    }));
}

/// INFO for this crate, WARN for dependencies. Dependency events also lose `message`
/// in [`JsonLog`], since their text may interpolate data.
pub fn log_filter() -> Targets {
    Targets::new()
        .with_target(OWN_TARGET, Level::INFO)
        .with_default(Level::WARN)
}

const OWN_TARGET: &str = "nodemind_api";

/// The only fields that reach the logs (docs/ARCHITECTURE.md, Observability). Content,
/// anchors, summaries, emails and SQL never match. `message` is the event text, so
/// log calls must use fixed messages and put data in allowlisted fields.
const ALLOWED_FIELDS: &[&str] = &[
    "message",
    "request_id",
    "route",
    "status",
    "latency_ms",
    "user_id",
    "canvas_id",
    "node_id",
    "model",
    "prompt_tokens",
    "completion_tokens",
    "cached_tokens",
    "ttft_ms",
    "error_code",
    "owner_key",
    "location",
    "thread",
    "event",
    "db_error",
    "sqlstate",
];

/// One JSON object per line: span fields (outermost first), then event fields,
/// all filtered through [`ALLOWED_FIELDS`].
type Sink = dyn Fn(&[u8]) + Send + Sync;

pub struct JsonLog {
    sink: Box<Sink>,
}

pub fn json_log(sink: impl Fn(&[u8]) + Send + Sync + 'static) -> JsonLog {
    JsonLog {
        sink: Box::new(sink),
    }
}

struct SpanFields(Map<String, Value>);

struct Allowed<'a>(&'a mut Map<String, Value>);

impl Allowed<'_> {
    fn put(&mut self, field: &Field, value: Value) {
        if ALLOWED_FIELDS.contains(&field.name()) {
            self.0.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for Allowed<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.into());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.into());
    }
}

impl<S> Layer<S> for JsonLog
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Map::new();
        attrs.record(&mut Allowed(&mut fields));
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(fields));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(fields) = span.extensions_mut().get_mut::<SpanFields>()
        {
            values.record(&mut Allowed(&mut fields.0));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut line = Map::new();
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        line.insert("ts_ms".into(), ms.into());
        line.insert("level".into(), event.metadata().level().as_str().into());
        line.insert("target".into(), event.metadata().target().into());
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    line.extend(fields.0.clone());
                }
            }
        }
        event.record(&mut Allowed(&mut line));
        if !event.metadata().target().starts_with(OWN_TARGET) {
            line.remove("message");
        }
        let mut bytes = serde_json::to_vec(&line).unwrap_or_default();
        bytes.push(b'\n');
        (self.sink)(&bytes);
    }
}
