use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{BoxStream, StreamExt};
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent};

use super::fake::{self, Fake, Script, Step};
use super::{Chunk, FinishReason, LlmError, Message, Role};
use crate::context::Mode;
use crate::metering::Usage;

const DEFAULT_COMPLETE_MAX_TOKENS: u32 = 2048;

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    pub first_token: Duration,
    pub chunk_gap: Duration,
    /// Upper bound of the random wait before the single pre-first-token retry.
    pub retry_jitter: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(10),
            first_token: Duration::from_secs(30),
            chunk_gap: Duration::from_secs(30),
            retry_jitter: Duration::from_millis(500),
        }
    }
}

/// One model name per mode; changing env changes the model with no code change (NFR-13).
#[derive(Clone, Debug)]
pub struct Models {
    pub main: String,
    pub doubt: String,
    pub summary: String,
}

impl Models {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} must be set"));
        Models {
            main: get("MODEL_MAIN"),
            doubt: get("MODEL_DOUBT"),
            summary: get("MODEL_SUMMARY"),
        }
    }

    pub fn for_mode(&self, mode: Mode) -> &str {
        match mode {
            Mode::Main => &self.main,
            Mode::Doubt => &self.doubt,
            Mode::Summary => &self.summary,
        }
    }
}

pub struct Completion {
    pub text: String,
    pub usage: Option<Usage>,
    pub finish: FinishReason,
}

enum Backend {
    Genai(genai::Client),
    Fake(Arc<Fake>),
}

pub struct Gateway {
    backend: Backend,
    models: Models,
    timeouts: Timeouts,
}

type Opened = BoxStream<'static, Result<Chunk, LlmError>>;

impl Gateway {
    /// `LLM_PROVIDER=fake` selects the scripted provider; otherwise genai, which reads provider
    /// API keys from the environment itself. Keys are never stored, logged or returned (NFR-9).
    pub fn from_env() -> Self {
        let models = Models::from_env();
        match std::env::var("LLM_PROVIDER").as_deref() {
            Ok("fake") => Gateway::fake(models).0,
            _ => Gateway {
                backend: Backend::Genai(genai::Client::new().expect("genai client")),
                models,
                timeouts: Timeouts::default(),
            },
        }
    }

    /// Fake-backed gateway plus the handle for scripting and inspecting calls.
    pub fn fake(models: Models) -> (Self, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        (
            Gateway {
                backend: Backend::Fake(fake.clone()),
                models,
                timeouts: Timeouts::default(),
            },
            fake,
        )
    }

    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Deltas, then exactly one `Chunk::End`; or an `Err` as the last item.
    pub fn stream(
        &self,
        mode: Mode,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> BoxStream<'static, Result<Chunk, LlmError>> {
        let model = self.models.for_mode(mode).to_owned();
        let t = self.timeouts;
        let backend = match &self.backend {
            Backend::Genai(c) => Backend::Genai(c.clone()),
            Backend::Fake(f) => Backend::Fake(f.clone()),
        };
        Box::pin(async_stream::stream! {
            for attempt in 0..2 {
                let retry_ok = attempt == 0;
                let opened = tokio::time::timeout(t.connect, open(&backend, &model, &messages, max_tokens)).await;
                let mut inner = match opened {
                    Err(_) => { yield Err(LlmError::ConnectTimeout); return; }
                    Ok(Err(e)) if retry_ok && is_retryable(e) => { jitter(t.retry_jitter).await; continue; }
                    Ok(Err(e)) => { yield Err(e); return; }
                    Ok(Ok(s)) => s,
                };
                let mut got_first = false;
                loop {
                    let (limit, on_timeout) = if got_first {
                        (t.chunk_gap, LlmError::ChunkTimeout)
                    } else {
                        (t.first_token, LlmError::FirstTokenTimeout)
                    };
                    match tokio::time::timeout(limit, inner.next()).await {
                        Err(_) => { yield Err(on_timeout); return; }
                        Ok(None) => { yield Err(LlmError::Provider { retryable: false }); return; }
                        Ok(Some(Err(e))) if !got_first && retry_ok && is_retryable(e) => {
                            jitter(t.retry_jitter).await;
                            break;
                        }
                        Ok(Some(Err(e))) => { yield Err(e); return; }
                        Ok(Some(Ok(c))) => {
                            let end = matches!(c, Chunk::End { .. });
                            got_first = true;
                            yield Ok(c);
                            if end { return; }
                        }
                    }
                }
            }
        })
    }

    pub async fn complete(
        &self,
        mode: Mode,
        messages: Vec<Message>,
    ) -> Result<Completion, LlmError> {
        let mut s = self.stream(mode, messages, DEFAULT_COMPLETE_MAX_TOKENS);
        let mut text = String::new();
        while let Some(c) = s.next().await {
            match c? {
                Chunk::Delta(d) => text.push_str(&d),
                Chunk::End { usage, finish } => {
                    return Ok(Completion {
                        text,
                        usage,
                        finish,
                    });
                }
            }
        }
        Err(LlmError::Provider { retryable: false })
    }
}

fn is_retryable(e: LlmError) -> bool {
    matches!(e, LlmError::Provider { retryable: true })
}

async fn jitter(max: Duration) {
    let ms = max.as_millis() as u64;
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(rand::random_range(0..=ms))).await;
    }
}

async fn open(
    backend: &Backend,
    model: &str,
    messages: &[Message],
    max_tokens: u32,
) -> Result<Opened, LlmError> {
    match backend {
        Backend::Fake(f) => Ok(play(f.next(model, messages, max_tokens)).await),
        Backend::Genai(client) => {
            let req = ChatRequest::new(
                messages
                    .iter()
                    .map(|m| match m.role {
                        Role::System => ChatMessage::system(m.content.clone()),
                        Role::User => ChatMessage::user(m.content.clone()),
                        Role::Assistant => ChatMessage::assistant(m.content.clone()),
                    })
                    .collect::<Vec<_>>(),
            );
            let opts = ChatOptions::default()
                .with_max_tokens(max_tokens)
                .with_capture_usage(true)
                .with_capture_content(false);
            let res = client
                .exec_chat_stream(model, req, Some(&opts))
                .await
                .map_err(|_| LlmError::Provider { retryable: true })?;
            Ok(res
                .stream
                .filter_map(|ev| async move {
                    match ev {
                        Err(_) => Some(Err(LlmError::Provider { retryable: true })),
                        Ok(ChatStreamEvent::Chunk(c)) => Some(Ok(Chunk::Delta(c.content))),
                        Ok(ChatStreamEvent::End(end)) => Some(Ok(Chunk::End {
                            usage: end.captured_usage.map(|u| Usage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                cached_tokens: u
                                    .prompt_tokens_details
                                    .and_then(|d| d.cached_tokens),
                                estimated: false,
                            }),
                            finish: match end
                                .captured_stop_reason
                                .as_ref()
                                .map(|r| r.to_string().to_ascii_lowercase())
                            {
                                Some(r) if r == "stop" || r == "end_turn" => FinishReason::Stop,
                                Some(r) if r == "length" || r == "max_tokens" => {
                                    FinishReason::Length
                                }
                                _ => FinishReason::Other,
                            },
                        })),
                        Ok(_) => None,
                    }
                })
                .boxed())
        }
    }
}

async fn play(script: Script) -> Opened {
    tokio::time::sleep(script.connect_delay).await;
    Box::pin(async_stream::stream! {
        for step in script.steps {
            match step {
                Step::Delta(d) => yield Ok(Chunk::Delta(d)),
                Step::Sleep(d) => tokio::time::sleep(d).await,
                Step::Fail { retryable } => { yield Err(fake::fail(retryable)); return; }
                Step::End { usage, finish } => yield Ok(Chunk::End { usage, finish }),
            }
        }
    })
}
