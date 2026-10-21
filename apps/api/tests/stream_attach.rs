//! Stream registry + SSE attach (no DB).
use std::time::Duration;

use axum::body::Body;
use axum::response::IntoResponse;
use futures_util::StreamExt;
use nodemind_api::http::error::ApiError;
use nodemind_api::stream::{Registry, attach};
use nodemind_api::types::{ErrorCode, NodeStatus};
use uuid::Uuid;

fn body_of(r: impl IntoResponse) -> Body {
    r.into_response().into_body()
}

async fn read_all(b: Body) -> String {
    let mut s = b.into_data_stream();
    let mut out = String::new();
    while let Some(c) = s.next().await {
        out.push_str(std::str::from_utf8(&c.unwrap()).unwrap());
    }
    out
}

#[tokio::test]
async fn two_attachers_see_identical_text_and_one_terminal() {
    let reg = Registry::new();
    let (n, u) = (Uuid::now_v7(), Uuid::now_v7());
    let w = reg.start(n, u, 1);
    w.append("hello ");
    let a = body_of(attach(&reg, n, u).unwrap());
    let ta = tokio::spawn(read_all(a));
    tokio::task::yield_now().await;
    w.append("wor");
    let b = body_of(attach(&reg, n, u).unwrap());
    let tb = tokio::spawn(read_all(b));
    w.append("ld");
    w.finish(NodeStatus::Complete);
    let (a, b) = (ta.await.unwrap(), tb.await.unwrap());
    for out in [&a, &b] {
        assert_eq!(out.matches("event: done").count(), 1, "{out}");
        assert!(!out.contains("event: error"));
    }
    let text = |s: &str| -> String {
        s.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .map(|v| v["text"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(text(&a), "hello world");
    assert_eq!(text(&b), "hello world");
}

#[tokio::test]
async fn stalled_reader_coalesces_and_never_misses_terminal() {
    let reg = Registry::new();
    let (n, u) = (Uuid::now_v7(), Uuid::now_v7());
    let w = reg.start(n, u, 1);
    let mut s = body_of(attach(&reg, n, u).unwrap()).into_data_stream();
    let first = s.next().await.unwrap().unwrap(); // snapshot, then stall
    assert!(
        std::str::from_utf8(&first)
            .unwrap()
            .contains("event: snapshot")
    );
    // Writer must not block on the stalled reader.
    tokio::time::timeout(Duration::from_secs(1), async {
        for _ in 0..10_000 {
            w.append("x");
        }
        w.finish(NodeStatus::Error);
    })
    .await
    .unwrap();
    let mut rest = String::new();
    while let Some(c) = s.next().await {
        rest.push_str(std::str::from_utf8(&c.unwrap()).unwrap());
    }
    assert_eq!(
        rest.matches("event: delta").count(),
        1,
        "one coalesced delta"
    );
    assert!(rest.contains(&"x".repeat(10_000)));
    assert_eq!(rest.matches("event: error").count(), 1);
    assert!(!rest.contains("event: done"));
}

#[tokio::test(start_paused = true)]
async fn keepalive_every_15s_and_removal_after_30s() {
    let reg = Registry::new();
    let (n, u) = (Uuid::now_v7(), Uuid::now_v7());
    let w = reg.start(n, u, 1);
    let mut s = body_of(attach(&reg, n, u).unwrap()).into_data_stream();
    s.next().await.unwrap().unwrap(); // snapshot
    let t = tokio::time::Instant::now();
    let c = s.next().await.unwrap().unwrap();
    // axum writes an empty comment as ": \n\n" (still an SSE comment line).
    assert!(
        c.starts_with(b":") && c.ends_with(b"\n\n") && c.len() <= 4,
        "{c:?}"
    );
    assert_eq!(t.elapsed(), Duration::from_secs(15));
    w.finish(NodeStatus::Complete);
    assert!(attach(&reg, n, u).is_ok());
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert!(matches!(
        attach(&reg, n, u),
        Err(ApiError::Code(ErrorCode::NotFound))
    ));
}

#[tokio::test]
async fn wrong_user_is_not_found_and_eleventh_attach_is_429() {
    let reg = Registry::new();
    let (n, u) = (Uuid::now_v7(), Uuid::now_v7());
    let _w = reg.start(n, u, 1);
    assert!(matches!(
        attach(&reg, n, Uuid::now_v7()),
        Err(ApiError::Code(ErrorCode::NotFound))
    ));
    let held: Vec<_> = (0..10).map(|_| attach(&reg, n, u).unwrap()).collect();
    assert!(matches!(
        attach(&reg, n, u),
        Err(ApiError::TooManyRequests { .. })
    ));
    drop(held);
    assert!(attach(&reg, n, u).is_ok());
}
