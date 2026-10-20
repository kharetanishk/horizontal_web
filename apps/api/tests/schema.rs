//! Schema constraints, cascades and db module. Uses DATABASE_URL (a role that can CREATE DATABASE)
//! if set, otherwise downloads and starts a throwaway Postgres. Each case gets its own fresh
//! database, with ./migrations applied unless the case tests migrating itself.

use std::future::Future;

use nodemind_api::db;
// Test-only SQL built from generated UUIDs and literals, never from untrusted input.
use sqlx::postgres::PgConnectOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

#[tokio::main]
async fn main() {
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
                std::fs::copy(tz.join("Universal"), tz.join("UTC")).expect("restore UTC tzdata");
            }
            pg.start().await.expect("start embedded postgres");
            let url = pg.settings().url("postgres");
            server = Some(pg);
            url
        }
    };
    let base: PgConnectOptions = url.parse().expect("invalid DATABASE_URL");
    let admin = PgPool::connect_with(base.clone()).await.unwrap();

    let mut failed = Vec::new();
    macro_rules! run {
        ($migrate:expr => $($case:ident),*) => {$(
            if !run(&admin, &base, stringify!($case), $migrate, $case).await {
                failed.push(stringify!($case));
            }
        )*};
    }
    run!(true => content_over_100k_chars_rejected, edge_type_constraints, edge_uniqueness,
        edge_cannot_cross_canvases, settled_usage_needs_token_counts, deletes_cascade,
        pgvector_not_enabled);
    run!(false => migrate_is_idempotent, pool_connections_are_utc_with_statement_timeout);

    admin.close().await;
    if let Some(pg) = server {
        pg.stop().await.expect("stop embedded postgres");
    }
    if !failed.is_empty() {
        eprintln!("failures: {failed:?}");
        std::process::exit(1);
    }
}

/// Runs one case in a fresh database; a panic in the case counts as failure.
async fn run<F>(
    admin: &PgPool,
    base: &PgConnectOptions,
    name: &str,
    migrate: bool,
    case: impl FnOnce(PgPool, PgConnectOptions) -> F,
) -> bool
where
    F: Future<Output = ()> + 'static,
{
    let database = format!("t_{}", Uuid::now_v7().simple());
    exec(admin, &format!("CREATE DATABASE {database}"))
        .await
        .unwrap();
    let opts = base.clone().database(&database);
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
    exec(admin, &format!("DROP DATABASE {database} WITH (FORCE)"))
        .await
        .unwrap();
    println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
    ok
}

async fn exec(pool: &PgPool, sql: &str) -> sqlx::Result<()> {
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .map(drop)
}

/// Name of the constraint the statement violated.
async fn violated(pool: &PgPool, sql: &str) -> String {
    let err = exec(pool, sql)
        .await
        .expect_err("statement should be rejected");
    err.as_database_error()
        .and_then(|e| e.constraint())
        .unwrap_or_else(|| panic!("not a constraint violation: {err:?}"))
        .to_owned()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn user(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        pool,
        &format!("INSERT INTO users (id, email) VALUES ('{id}', '{id}@test')"),
    )
    .await
    .unwrap();
    id
}

async fn canvas(pool: &PgPool, user: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        pool,
        &format!("INSERT INTO canvases (id, user_id, title) VALUES ('{id}', '{user}', 't')"),
    )
    .await
    .unwrap();
    id
}

fn node_sql(id: Uuid, canvas: Uuid, content: &str) -> String {
    format!(
        "INSERT INTO nodes (id, canvas_id, type, content, status, position_x, position_y) \
         VALUES ('{id}', '{canvas}', 'user_message', '{content}', 'complete', 0, 0)"
    )
}

async fn node(pool: &PgPool, canvas: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    exec(pool, &node_sql(id, canvas, "hi")).await.unwrap();
    id
}

/// `cols`/`vals` are extra columns after (id, canvas_id, source_id, target_id).
fn edge_sql(canvas: Uuid, src: Uuid, tgt: Uuid, cols: &str, vals: &str) -> String {
    format!(
        "INSERT INTO edges (id, canvas_id, source_id, target_id, {cols}) \
         VALUES ('{}', '{canvas}', '{src}', '{tgt}', {vals})",
        Uuid::now_v7()
    )
}

const BRANCH: (&str, &str) = (
    "type, anchor, doubt_type",
    r#"'branch', '{"kind":"none"}', 'why'"#,
);

async fn content_over_100k_chars_rejected(pool: PgPool, _: PgConnectOptions) {
    let c = canvas(&pool, user(&pool).await).await;
    // char_length, not bytes: 100,000 two-byte chars fit
    exec(&pool, &node_sql(Uuid::now_v7(), c, &"é".repeat(100_000)))
        .await
        .unwrap();
    let sql = node_sql(Uuid::now_v7(), c, &"a".repeat(100_001));
    assert_eq!(violated(&pool, &sql).await, "nodes_content_length");
}

async fn edge_type_constraints(pool: PgPool, _: PgConnectOptions) {
    let c = canvas(&pool, user(&pool).await).await;
    let (a, b) = (node(&pool, c).await, node(&pool, c).await);

    for vals in [
        "'branch', NULL, NULL",
        r#"'branch', '{"kind":"none"}', NULL"#,
        "'branch', NULL, 'why'",
        r#"'reply', '{"kind":"none"}', 'why'"#,
    ] {
        let sql = edge_sql(c, a, b, BRANCH.0, vals);
        assert_eq!(violated(&pool, &sql).await, "edges_branch_fields", "{vals}");
    }
    for (cols, vals) in [
        ("type, merge_summary", "'reply', 's'"),
        ("type, collapsed", "'reply', true"),
    ] {
        let sql = edge_sql(c, a, b, cols, vals);
        assert_eq!(
            violated(&pool, &sql).await,
            "edges_branch_only_state",
            "{cols}"
        );
    }
    // a branch edge may carry both
    let (cols, vals) = (
        format!("{}, merge_summary, collapsed", BRANCH.0),
        format!("{}, 's', true", BRANCH.1),
    );
    exec(&pool, &edge_sql(c, a, b, &cols, &vals)).await.unwrap();
}

async fn edge_uniqueness(pool: PgPool, _: PgConnectOptions) {
    let c = canvas(&pool, user(&pool).await).await;
    let (a, b, d, e) = (
        node(&pool, c).await,
        node(&pool, c).await,
        node(&pool, c).await,
        node(&pool, c).await,
    );

    exec(&pool, &edge_sql(c, a, b, "type", "'reply'"))
        .await
        .unwrap();
    // a node has at most one parent
    let sql = edge_sql(c, d, b, BRANCH.0, BRANCH.1);
    assert_eq!(violated(&pool, &sql).await, "edges_target_id_key");
    // at most one reply per source...
    let sql = edge_sql(c, a, d, "type", "'reply'");
    assert_eq!(violated(&pool, &sql).await, "edges_one_reply_per_source");
    // ...but any number of branches
    exec(&pool, &edge_sql(c, a, d, BRANCH.0, BRANCH.1))
        .await
        .unwrap();
    exec(&pool, &edge_sql(c, a, e, BRANCH.0, BRANCH.1))
        .await
        .unwrap();
}

async fn edge_cannot_cross_canvases(pool: PgPool, _: PgConnectOptions) {
    let u = user(&pool).await;
    let (c1, c2) = (canvas(&pool, u).await, canvas(&pool, u).await);
    let (a, b) = (node(&pool, c1).await, node(&pool, c2).await);
    let sql = edge_sql(c1, a, b, "type", "'reply'");
    assert_eq!(violated(&pool, &sql).await, "edges_target_fkey");
    let sql = edge_sql(c2, a, b, "type", "'reply'");
    assert_eq!(violated(&pool, &sql).await, "edges_source_fkey");
}

async fn settled_usage_needs_token_counts(pool: PgPool, _: PgConnectOptions) {
    let insert = |counts: &str| {
        format!(
            "INSERT INTO llm_usage (user_id, kind, model, owner_key, reserved_tokens, \
             prompt_tokens, completion_tokens, settled_at) \
             VALUES ('{}', 'answer', 'm', 1, 100, {counts}, now())",
            Uuid::now_v7()
        )
    };
    for counts in ["NULL, 5", "5, NULL"] {
        assert_eq!(
            violated(&pool, &insert(counts)).await,
            "llm_usage_settled_counts"
        );
    }
    exec(&pool, &insert("0, 0")).await.unwrap();
}

async fn deletes_cascade(pool: PgPool, _: PgConnectOptions) {
    let u = user(&pool).await;
    let (c1, c2) = (canvas(&pool, u).await, canvas(&pool, u).await);
    for c in [c1, c2] {
        let (a, b) = (node(&pool, c).await, node(&pool, c).await);
        exec(&pool, &edge_sql(c, a, b, "type", "'reply'"))
            .await
            .unwrap();
        exec(&pool, &format!(
            "INSERT INTO events (user_id, canvas_id, kind, payload) VALUES ('{u}', '{c}', 'collapse', '{{}}')"
        ))
        .await
        .unwrap();
        exec(
            &pool,
            &format!(
                "INSERT INTO llm_usage (user_id, node_id, kind, model, owner_key, reserved_tokens) \
             VALUES ('{u}', '{b}', 'answer', 'm', 1, 100)"
            ),
        )
        .await
        .unwrap();
    }
    exec(&pool, &format!(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('\\x01', '{u}', now() + interval '1 day')"
    ))
    .await
    .unwrap();
    exec(&pool, &format!(
        "INSERT INTO invites (token_hash, user_id, expires_at) VALUES ('\\x02', '{u}', now() + interval '7 days')"
    ))
    .await
    .unwrap();

    // AC-2.3: deleting a canvas removes its nodes and edges, nothing else
    exec(&pool, &format!("DELETE FROM canvases WHERE id = '{c1}'"))
        .await
        .unwrap();
    assert_eq!(count(&pool, "nodes").await, 2);
    assert_eq!(count(&pool, "edges").await, 1);
    assert_eq!(
        count(&pool, &format!("nodes WHERE canvas_id = '{c1}'")).await,
        0
    );
    assert_eq!(count(&pool, "llm_usage").await, 2);

    // AC-20.1: deleting a user removes everything they own except the usage ledger
    exec(&pool, &format!("DELETE FROM users WHERE id = '{u}'"))
        .await
        .unwrap();
    for table in [
        "users", "sessions", "invites", "canvases", "nodes", "edges", "events",
    ] {
        assert_eq!(count(&pool, table).await, 0, "{table}");
    }
    assert_eq!(count(&pool, "llm_usage").await, 2);
}

async fn pgvector_not_enabled(pool: PgPool, _: PgConnectOptions) {
    assert_eq!(
        count(&pool, "pg_extension WHERE extname = 'vector'").await,
        0
    );
}

async fn migrate_is_idempotent(_: PgPool, opts: PgConnectOptions) {
    db::migrate(&opts).await.unwrap();
    db::migrate(&opts).await.unwrap();
    let pool = db::connect(opts).await.unwrap();
    assert_eq!(count(&pool, "_sqlx_migrations WHERE success").await, 1);
    assert_eq!(count(&pool, "users").await, 0);
}

async fn pool_connections_are_utc_with_statement_timeout(_: PgPool, opts: PgConnectOptions) {
    // the session would otherwise start in a non-UTC zone
    let pool = db::connect(opts.options([("TimeZone", "Etc/GMT+5")]))
        .await
        .unwrap();
    let (tz, timeout): (String, String) =
        sqlx::query_as("SELECT current_setting('TimeZone'), current_setting('statement_timeout')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tz, "UTC");
    assert_eq!(timeout, "10s");
}
