//! Schema constraints, cascades and db module (harness: tests/common).

mod common;

use common::{Harness, count, exec, user};
use nodemind_api::db;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let mut h = Harness::start().await;
    macro_rules! run {
        ($migrate:expr => $($case:ident),*) => {$(
            h.run(stringify!($case), $migrate, $case).await;
        )*};
    }
    run!(true => content_over_100k_chars_rejected, edge_type_constraints, edge_uniqueness,
        edge_cannot_cross_canvases, settled_usage_needs_token_counts, deletes_cascade,
        pgvector_not_enabled);
    run!(false => migrate_is_idempotent, pool_connections_are_utc_with_statement_timeout);
    h.finish().await;
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
