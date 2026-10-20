//! Metering ledger admission, settlement and the Reservation guard (harness: tests/common).

mod common;

use std::time::Duration;

use axum::response::IntoResponse;
use common::{Harness, count, exec, user};
use nodemind_api::error::ApiError;
use nodemind_api::metering::{
    ANSWER_RESERVE, Kind, MERGE_PROPOSAL_RESERVE, Meter, Reservation, Usage, compaction_reserve,
    settle,
};
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use uuid::Uuid;

const CAP: i64 = 200_000;

#[tokio::main]
async fn main() {
    let mut h = Harness::start().await;
    macro_rules! run {
        ($($case:ident),*) => {$( h.run(stringify!($case), true, $case).await; )*};
    }
    run!(
        reservation_sizes_and_startup_check,
        over_cap_is_429_and_writes_nothing,
        one_answer_below_cap_is_admitted,
        fourth_concurrent_call_is_429_but_compaction_is_not,
        day_is_the_utc_day,
        settlement_overwrites_with_zero_not_null,
        dropped_guard_settles_estimated,
        panicked_guard_settles_estimated
    );
    h.finish().await;
}

fn meter(pool: &PgPool) -> Meter {
    Meter::new(pool.clone(), CAP, 1.1).unwrap()
}

/// Admits in its own transaction: commit on success, rollback on refusal.
async fn admit(
    meter: &Meter,
    pool: &PgPool,
    user: Uuid,
    kind: Kind,
) -> Result<Reservation, ApiError> {
    let mut tx = pool.begin().await.unwrap();
    match meter.admit(&mut tx, user, kind, "m", None, 7).await {
        Ok(r) => {
            tx.commit().await.unwrap();
            Ok(r)
        }
        Err(e) => {
            tx.rollback().await.unwrap();
            Err(e)
        }
    }
}

fn refusal(res: Result<Reservation, ApiError>) -> &'static str {
    match res {
        Err(ApiError::TooManyRequests { code, .. }) => code,
        Err(e) => panic!("unexpected error {e:?}"),
        Ok(_) => panic!("admitted"),
    }
}

/// A settled usage row of `tokens` for today.
async fn spent(pool: &PgPool, user: Uuid, tokens: i64) {
    exec(pool, &format!(
        "INSERT INTO llm_usage (user_id, kind, model, owner_key, reserved_tokens, prompt_tokens, completion_tokens, settled_at) \
         VALUES ('{user}', 'answer', 'm', 1, 0, {tokens}, 0, now())"
    ))
    .await
    .unwrap();
}

type Row = (Option<i32>, Option<i32>, Option<i32>, bool, bool);

async fn row(pool: &PgPool, id: i64) -> Row {
    sqlx::query_as(
        "SELECT prompt_tokens, completion_tokens, cached_tokens, estimated, settled_at IS NOT NULL \
         FROM llm_usage WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Waits for the guard's spawned settle.
async fn settled_row(pool: &PgPool, id: i64) -> Row {
    for _ in 0..250 {
        let r = row(pool, id).await;
        if r.4 {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("reservation {id} never settled");
}

async fn reservation_sizes_and_startup_check(pool: PgPool, _: PgConnectOptions) {
    assert_eq!(ANSWER_RESERVE, 16_800);
    assert_eq!(MERGE_PROPOSAL_RESERVE, 4_305);
    assert_eq!(compaction_reserve(1.0), 6_204); // (700 + 2,000 + 2,500 + 8 + 700) × 1.05
    assert_eq!(compaction_reserve(1.1), 6_751); // ≈ 6,800 at a measured r_up
    // every reservation must be ≤ cap / 10; the answer is the largest
    assert!(Meter::new(pool.clone(), 168_000, 1.1).is_ok());
    assert!(Meter::new(pool.clone(), 167_999, 1.1).is_err());
    // a large summary-model ratio makes compaction the binding size
    assert!(Meter::new(pool.clone(), 168_000, 3.5).is_err());
}

async fn over_cap_is_429_and_writes_nothing(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    // no room for even the smallest call that counts here, a compaction
    spent(&pool, u, CAP - i64::from(compaction_reserve(1.1)) + 1).await;

    let err = admit(&m, &pool, u, Kind::Answer).await.err().unwrap();
    let ApiError::TooManyRequests { code, retry_after } = err else {
        panic!("{err:?}")
    };
    assert_eq!(code, "daily_cap_reached");
    assert!((1..=86_400).contains(&retry_after), "{retry_after}");
    assert_eq!(count(&pool, "llm_usage").await, 1); // just the seed row

    let res = ApiError::TooManyRequests { code, retry_after }.into_response();
    assert_eq!(res.status(), 429);
    assert_eq!(
        res.headers()["retry-after"],
        retry_after.to_string().as_str()
    );
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains(r#""code":"daily_cap_reached""#)
    );

    // compaction counts toward the cap too
    assert_eq!(
        refusal(admit(&m, &pool, u, Kind::Compaction).await),
        "daily_cap_reached"
    );
    // another user is unaffected
    let other = user(&pool).await;
    admit(&m, &pool, other, Kind::Answer)
        .await
        .unwrap()
        .settle(Usage::default())
        .await
        .unwrap();
}

async fn one_answer_below_cap_is_admitted(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    spent(&pool, u, CAP - i64::from(ANSWER_RESERVE)).await;

    let r = admit(&m, &pool, u, Kind::Answer).await.unwrap();
    // its reservation now fills the cap
    assert_eq!(
        refusal(admit(&m, &pool, u, Kind::MergeProposal).await),
        "daily_cap_reached"
    );
    // settling below the reservation frees room again
    r.settle(Usage {
        prompt_tokens: Some(5_000),
        completion_tokens: Some(1_000),
        ..Usage::default()
    })
    .await
    .unwrap();
    admit(&m, &pool, u, Kind::MergeProposal)
        .await
        .unwrap()
        .settle(Usage::default())
        .await
        .unwrap();
}

async fn fourth_concurrent_call_is_429_but_compaction_is_not(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    // four racing admissions: the user-row lock serializes them, so exactly one loses
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let (m, pool) = (m.clone(), pool.clone());
            tokio::spawn(async move { admit(&m, &pool, u, Kind::Answer).await })
        })
        .collect();
    let mut held = Vec::new();
    let mut refused = Vec::new();
    for t in tasks {
        match t.await.unwrap() {
            Ok(r) => held.push(r),
            Err(e) => refused.push(refusal(Err(e))),
        }
    }
    assert_eq!(held.len(), 3);
    assert_eq!(refused, ["rate_limited"]);
    assert_eq!(count(&pool, "llm_usage").await, 3);

    assert_eq!(
        refusal(admit(&m, &pool, u, Kind::MergeProposal).await),
        "rate_limited"
    );
    // system work is not limited by, and does not count toward, in_flight
    let c = admit(&m, &pool, u, Kind::Compaction).await.unwrap();
    held.pop().unwrap().settle(Usage::default()).await.unwrap();
    let r = admit(&m, &pool, u, Kind::Answer).await.unwrap();

    for r in held.into_iter().chain([c, r]) {
        r.settle(Usage::default()).await.unwrap();
    }
}

async fn day_is_the_utc_day(_: PgPool, opts: PgConnectOptions) {
    // a session far from UTC (UTC+14) must still meter by the UTC day
    let pool = PgPool::connect_with(opts.options([("TimeZone", "Etc/GMT-14")]))
        .await
        .unwrap();
    let (m, u) = (meter(&pool), user(&pool).await);
    let at = |offset: &str, tokens: i64| {
        format!(
            "INSERT INTO llm_usage (user_id, kind, model, owner_key, reserved_tokens, prompt_tokens, completion_tokens, created_at, settled_at) \
             VALUES ('{u}', 'answer', 'm', 1, 0, {tokens}, 0, date_trunc('day', now(), 'UTC') + interval '{offset}', now())"
        )
    };
    // yesterday (UTC) does not count
    exec(&pool, &at("-1 second", CAP)).await.unwrap();
    admit(&m, &pool, u, Kind::Answer)
        .await
        .unwrap()
        .settle(Usage::default())
        .await
        .unwrap();
    // UTC midnight today does
    exec(&pool, &at("0 seconds", CAP)).await.unwrap();
    assert_eq!(
        refusal(admit(&m, &pool, u, Kind::Answer).await),
        "daily_cap_reached"
    );
    pool.close().await;
}

async fn settlement_overwrites_with_zero_not_null(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    let r = admit(&m, &pool, u, Kind::Answer).await.unwrap();
    let id = r.id();
    assert_eq!(row(&pool, id).await, (None, None, None, false, false));

    // a sweeper settles first at the reserved amount...
    let estimate = Usage {
        prompt_tokens: Some(ANSWER_RESERVE),
        completion_tokens: None,
        cached_tokens: None,
        estimated: true,
    };
    settle(&pool, id, estimate).await.unwrap();
    assert_eq!(
        row(&pool, id).await,
        (Some(16_800), Some(0), Some(0), true, true)
    );
    // ...and the late owner's real usage replaces it, absent counts as 0
    let actual = Usage {
        prompt_tokens: Some(900),
        completion_tokens: Some(120),
        cached_tokens: None,
        estimated: false,
    };
    r.settle(actual).await.unwrap();
    assert_eq!(
        row(&pool, id).await,
        (Some(900), Some(120), Some(0), false, true)
    );
    // repeating is a harmless overwrite
    settle(&pool, id, actual).await.unwrap();
    assert_eq!(
        row(&pool, id).await,
        (Some(900), Some(120), Some(0), false, true)
    );
    // an explicit settle is not followed by a drop settle
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        row(&pool, id).await,
        (Some(900), Some(120), Some(0), false, true)
    );
}

async fn dropped_guard_settles_estimated(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    let mut r = admit(&m, &pool, u, Kind::Answer).await.unwrap();
    let id = r.id();
    r.count(1_234, 56);
    drop(r);
    assert_eq!(
        settled_row(&pool, id).await,
        (Some(1_234), Some(56), Some(0), true, true)
    );

    // a reservation whose admission rolled back leaves nothing behind
    let mut tx = pool.begin().await.unwrap();
    let r = m
        .admit(&mut tx, u, Kind::Answer, "m", None, 7)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    drop(r);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count(&pool, "llm_usage").await, 1);
}

async fn panicked_guard_settles_estimated(pool: PgPool, _: PgConnectOptions) {
    let (m, u) = (meter(&pool), user(&pool).await);
    let r = admit(&m, &pool, u, Kind::Compaction).await.unwrap();
    let id = r.id();
    let task = tokio::spawn(async move {
        let _guard = r;
        panic!("stream task panicked");
    });
    assert!(task.await.unwrap_err().is_panic());
    assert_eq!(
        settled_row(&pool, id).await,
        (Some(0), Some(0), Some(0), true, true)
    );
}
