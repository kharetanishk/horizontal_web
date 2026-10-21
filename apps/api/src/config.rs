use sqlx::postgres::PgConnectOptions;

/// Runtime settings shared with handlers through `AppState`. No `Debug`: later fields hold secrets.
pub struct Config {
    /// Scheme + host (+ port), no trailing slash; mutating requests must send exactly this `Origin`.
    pub app_origin: String,
}

impl Config {
    pub fn from_env() -> Self {
        let raw = std::env::var("APP_ORIGIN").expect("APP_ORIGIN must be set");
        let app_origin = normalize_origin(&raw).unwrap_or_else(|| {
            panic!("APP_ORIGIN must be http(s)://host[:port] with no path, query or userinfo")
        });
        Config { app_origin }
    }
}

/// The form browsers send in `Origin`: lowercase scheme and host, default port dropped.
/// `None` for anything else (path, query, fragment, userinfo, other schemes).
pub fn normalize_origin(raw: &str) -> Option<String> {
    let raw = raw.strip_suffix('/').unwrap_or(raw);
    if raw.contains(['#', '@']) {
        return None;
    }
    let uri: axum::http::Uri = raw.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    if uri.path_and_query().is_some_and(|p| p.as_str() != "/") {
        return None;
    }
    let host = uri.host().filter(|h| !h.is_empty())?.to_ascii_lowercase();
    Some(match uri.port_u16() {
        Some(p) if p != default_port => format!("{scheme}://{host}:{p}"),
        _ => format!("{scheme}://{host}"),
    })
}

/// `(pooled, direct)` Postgres options.
pub fn database_from_env() -> (PgConnectOptions, PgConnectOptions) {
    let pooled = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // ponytail: falls back to DATABASE_URL for local dev; production must set DATABASE_URL_DIRECT
    let direct = std::env::var("DATABASE_URL_DIRECT").unwrap_or_else(|_| pooled.clone());
    (
        pooled.parse().expect("invalid DATABASE_URL"),
        direct.parse().expect("invalid DATABASE_URL_DIRECT"),
    )
}
