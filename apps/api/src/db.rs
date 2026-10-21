use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions};
use sqlx::{Connection, Executor};

/// App pool: every connection is pinned to UTC (metering's `date_trunc('day', now())`)
/// and a 10 s statement timeout; acquiring waits at most 5 s.
pub async fn connect(opts: PgConnectOptions) -> sqlx::Result<PgPool> {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("SET TimeZone = 'UTC'; SET statement_timeout = '10s'")
                    .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

/// Runs embedded migrations over a direct (non-pooled) connection, since SQLx's
/// advisory lock is session-scoped. Idempotent; tolerates migrations newer than this binary.
pub async fn migrate(direct: &PgConnectOptions) -> Result<(), sqlx::migrate::MigrateError> {
    let mut conn = PgConnection::connect_with(direct).await?;
    let mut migrator = sqlx::migrate!();
    migrator.set_ignore_missing(true);
    migrator.run(&mut conn).await?;
    conn.close().await?;
    Ok(())
}
