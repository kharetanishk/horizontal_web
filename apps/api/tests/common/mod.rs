//! Custom test harness shared by the DB suites. Uses DATABASE_URL (a role that can CREATE
//! DATABASE) if set, otherwise downloads and starts a throwaway Postgres. Each case gets its
//! own fresh database, with ./migrations applied unless the case tests migrating itself.
#![allow(dead_code)] // each suite uses a different subset

use std::future::Future;

use nodemind_api::db;
// Test-only SQL built from generated UUIDs and literals, never from untrusted input.
use sqlx::postgres::PgConnectOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

pub struct Harness {
    server: Option<postgresql_embedded::PostgreSQL>,
    base: PgConnectOptions,
    admin: PgPool,
    failed: Vec<&'static str>,
}

impl Harness {
    pub async fn start() -> Self {
        let mut server = None;
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                let mut pg = postgresql_embedded::PostgreSQL::new(postgresql_embedded::Settings {
                    version: postgresql_embedded::VersionReq::parse("=16").unwrap(), // as in compose/CI
                    installation_dir: std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
                        .join("postgresql"),
                    timeout: Some(std::time::Duration::from_secs(60)), // first initdb on macOS is slow
                    ..Default::default()
                });
                pg.setup().await.expect("install embedded postgres");
                // the downloaded archive drops tzdata hard links, including UTC; Universal is identical
                let tz = pg.settings().installation_dir.join("share/timezone");
                if !tz.join("UTC").exists() {
                    std::fs::copy(tz.join("Universal"), tz.join("UTC"))
                        .expect("restore UTC tzdata");
                }
                pg.start().await.expect("start embedded postgres");
                let url = pg.settings().url("postgres");
                server = Some(pg);
                url
            }
        };
        let base: PgConnectOptions = url.parse().expect("invalid DATABASE_URL");
        let admin = PgPool::connect_with(base.clone()).await.unwrap();
        Harness {
            server,
            base,
            admin,
            failed: Vec::new(),
        }
    }

    /// Runs one case in a fresh database; a panic in the case counts as failure.
    pub async fn run<F>(
        &mut self,
        name: &'static str,
        migrate: bool,
        case: impl FnOnce(PgPool, PgConnectOptions) -> F,
    ) where
        F: Future<Output = ()> + 'static,
    {
        let database = format!("t_{}", Uuid::now_v7().simple());
        exec(&self.admin, &format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let opts = self.base.clone().database(&database);
        if migrate {
            db::migrate(&opts).await.unwrap();
        }
        let pool = PgPool::connect_with(opts.clone()).await.unwrap();
        // local task: catches panics without requiring the case future to be Send
        let fut = case(pool.clone(), opts);
        let ok = tokio::task::LocalSet::new()
            .run_until(async { tokio::task::spawn_local(fut).await })
            .await
            .is_ok();
        pool.close().await;
        exec(
            &self.admin,
            &format!("DROP DATABASE {database} WITH (FORCE)"),
        )
        .await
        .unwrap();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            self.failed.push(name);
        }
    }

    /// Stops the server and exits non-zero if any case failed.
    pub async fn finish(self) {
        self.admin.close().await;
        if let Some(pg) = self.server {
            pg.stop().await.expect("stop embedded postgres");
        }
        if !self.failed.is_empty() {
            eprintln!("failures: {:?}", self.failed);
            std::process::exit(1);
        }
    }
}

pub async fn exec(pool: &PgPool, sql: &str) -> sqlx::Result<()> {
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .map(drop)
}

pub async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn user(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        pool,
        &format!("INSERT INTO users (id, email) VALUES ('{id}', '{id}@test')"),
    )
    .await
    .unwrap();
    id
}
