use std::io::Write;
use std::sync::Arc;

use nodemind_api::config::{self, Config};
use nodemind_api::db;
use nodemind_api::http::{self, AppState, middleware};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

#[tokio::main]
async fn main() {
    let logs = tracing_subscriber::registry().with(
        middleware::json_log(|line| {
            let _ = std::io::stdout().lock().write_all(line);
        })
        .with_filter(middleware::log_filter()),
    );
    tracing::subscriber::set_global_default(logs).expect("install log subscriber");

    let config = Config::from_env();
    let (pooled, direct) = config::database_from_env();
    db::migrate(&direct).await.expect("migrations failed");
    let pool = db::connect(pooled).await.expect("database connect failed");
    middleware::install_panic_hook();

    let app = http::app(AppState {
        pool,
        config: Arc::new(config),
    });
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind 0.0.0.0:8080");
    axum::serve(listener, app).await.expect("server error");
}
