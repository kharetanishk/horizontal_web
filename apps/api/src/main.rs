use std::io::Write;
use std::sync::Arc;

use nodemind_api::cli;
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

    // Operator commands: `invite <email>`, `reset-password <email>`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(cmd) = args.first() {
        let email = args.get(1).map(String::as_str).unwrap_or_default();
        let link = match cmd.as_str() {
            "invite" => cli::invite(&pool, &config, email).await,
            "reset-password" => cli::reset_password(&pool, &config, email).await,
            _ => Err("usage: api invite|reset-password <email>".into()),
        };
        match link {
            Ok(link) => println!("{link}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let app = http::app(AppState {
        pool,
        config: Arc::new(config),
    });
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind 0.0.0.0:8080");
    axum::serve(listener, app).await.expect("server error");
}
