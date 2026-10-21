//! LLM gateway against the scripted fake provider (no network, no DB).
use std::time::Duration;

use futures_util::StreamExt;
use nodemind_api::context::Mode;
use nodemind_api::llm::fake::{Script, Step};
use nodemind_api::llm::{Chunk, FinishReason, Gateway, LlmError, Message, Models, Role, Timeouts};

const MS: fn(u64) -> Duration = Duration::from_millis;

fn models() -> Models {
    Models {
        main: "m-main".into(),
        doubt: "m-doubt".into(),
        summary: "m-sum".into(),
    }
}

fn msgs() -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content: "hi".into(),
    }]
}

fn fast() -> Timeouts {
    Timeouts {
        connect: MS(100),
        first_token: MS(100),
        chunk_gap: MS(100),
        retry_jitter: MS(5),
    }
}

async fn run(g: &Gateway, mode: Mode) -> Vec<Result<Chunk, LlmError>> {
    g.stream(mode, msgs(), 100).collect().await
}

fn deltas(items: &[Result<Chunk, LlmError>]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|c| match c {
            Ok(Chunk::Delta(d)) => Some(d.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn chunked_deltas_then_usage_with_cached_tokens() {
    let (g, fake) = Gateway::fake(models());
    fake.push(Script::text(&["a", "b", "c"]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(deltas(&out), ["a", "b", "c"]);
    let Some(Ok(Chunk::End {
        usage: Some(u),
        finish,
    })) = out.last()
    else {
        panic!("{out:?}")
    };
    assert_eq!(*finish, FinishReason::Stop);
    assert_eq!(u.cached_tokens, Some(4));
    assert!(!u.estimated);
}

#[tokio::test]
async fn finish_length_and_usage_absent() {
    let (g, fake) = Gateway::fake(models());
    fake.push(Script::new(vec![
        Step::Delta("x".into()),
        Step::End {
            usage: None,
            finish: FinishReason::Length,
        },
    ]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(
        out.last(),
        Some(&Ok(Chunk::End {
            usage: None,
            finish: FinishReason::Length
        }))
    );
}

#[tokio::test]
async fn error_after_partial_text_is_not_retried() {
    let (g, fake) = Gateway::fake(models());
    fake.push(Script::new(vec![
        Step::Delta("par".into()),
        Step::Fail { retryable: true },
    ]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(deltas(&out), ["par"]);
    assert_eq!(
        out.last(),
        Some(&Err(LlmError::Provider { retryable: true }))
    );
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn error_before_first_token_retries_once() {
    let (g, fake) = Gateway::fake(models());
    fake.push(Script::new(vec![Step::Fail { retryable: true }]));
    fake.push(Script::text(&["ok"]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(deltas(&out), ["ok"]);
    assert_eq!(fake.calls().len(), 2);

    // Second failure surfaces; no third attempt.
    fake.push(Script::new(vec![Step::Fail { retryable: true }]));
    fake.push(Script::new(vec![Step::Fail { retryable: true }]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(out, [Err(LlmError::Provider { retryable: true })]);
    assert_eq!(fake.calls().len(), 4);

    // Non-retryable is not retried.
    fake.push(Script::new(vec![Step::Fail { retryable: false }]));
    run(&g, Mode::Main).await;
    assert_eq!(fake.calls().len(), 5);
}

#[tokio::test]
async fn connect_timeout_fires() {
    let (g, fake) = Gateway::fake(models());
    let g = g.with_timeouts(fast());
    fake.push(Script {
        connect_delay: MS(1000),
        steps: vec![],
    });
    assert_eq!(run(&g, Mode::Main).await, [Err(LlmError::ConnectTimeout)]);
}

#[tokio::test]
async fn first_token_timeout_fires() {
    let (g, fake) = Gateway::fake(models());
    let g = g.with_timeouts(fast());
    fake.push(Script::new(vec![
        Step::Sleep(MS(1000)),
        Step::Delta("late".into()),
    ]));
    assert_eq!(
        run(&g, Mode::Main).await,
        [Err(LlmError::FirstTokenTimeout)]
    );
}

#[tokio::test]
async fn chunk_gap_timeout_fires_after_first_token() {
    let (g, fake) = Gateway::fake(models());
    let g = g.with_timeouts(fast());
    fake.push(Script::new(vec![
        Step::Delta("a".into()),
        Step::Sleep(MS(1000)),
        Step::Delta("b".into()),
    ]));
    let out = run(&g, Mode::Main).await;
    assert_eq!(
        out,
        [Ok(Chunk::Delta("a".into())), Err(LlmError::ChunkTimeout)]
    );
}

#[tokio::test]
async fn model_follows_mode_config() {
    let (g, fake) = Gateway::fake(models());
    run(&g, Mode::Doubt).await;
    run(&g, Mode::Main).await;
    run(&g, Mode::Summary).await;
    let used: Vec<_> = fake.calls().into_iter().map(|c| c.model).collect();
    assert_eq!(used, ["m-doubt", "m-main", "m-sum"]);

    // Same code, different config: only MODEL_DOUBT's value changed.
    let (g2, fake2) = Gateway::fake(Models {
        doubt: "other-doubt".into(),
        ..models()
    });
    run(&g2, Mode::Doubt).await;
    assert_eq!(fake2.calls()[0].model, "other-doubt");
}

#[tokio::test]
async fn complete_collects_text_and_passes_max_tokens() {
    let (g, fake) = Gateway::fake(models());
    fake.push(Script::text(&["he", "llo"]));
    let c = g.complete(Mode::Summary, msgs()).await.unwrap();
    assert_eq!(c.text, "hello");
    assert_eq!(c.finish, FinishReason::Stop);
    assert!(fake.calls()[0].max_tokens > 0);
}

#[test]
fn errors_carry_no_provider_text() {
    // NFR-9: Display of every variant is a fixed string.
    for e in [
        LlmError::ConnectTimeout,
        LlmError::Provider { retryable: true },
    ] {
        assert!(e.to_string().starts_with("llm "));
    }
}
