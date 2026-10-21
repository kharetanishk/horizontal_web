//! Scripted provider for tests and Playwright. Each call to the gateway pops the next `Script`;
//! when none is queued it plays `Script::default()` (two deltas, usage present).
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::{FinishReason, LlmError, Message};
use crate::metering::Usage;

#[derive(Clone, Debug)]
pub enum Step {
    Delta(String),
    /// Silence for this long before the next step.
    Sleep(Duration),
    /// Provider error at this point (before the first delta, or after partial text).
    Fail {
        retryable: bool,
    },
    End {
        usage: Option<Usage>,
        finish: FinishReason,
    },
}

#[derive(Clone, Debug)]
pub struct Script {
    /// Delay before the stream opens (connect phase).
    pub connect_delay: Duration,
    pub steps: Vec<Step>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Self {
        Script {
            connect_delay: Duration::ZERO,
            steps,
        }
    }

    /// `parts` as deltas, then a stop with usage.
    pub fn text(parts: &[&str]) -> Self {
        let mut steps: Vec<Step> = parts.iter().map(|p| Step::Delta((*p).into())).collect();
        steps.push(Step::End {
            usage: Some(Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(parts.len() as i32),
                cached_tokens: Some(4),
                estimated: false,
            }),
            finish: FinishReason::Stop,
        });
        Script::new(steps)
    }
}

impl Default for Script {
    fn default() -> Self {
        Script::text(&["Hello", " world"])
    }
}

/// What the gateway asked of the provider, for assertions.
#[derive(Clone, Debug)]
pub struct Call {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
}

#[derive(Default)]
pub struct Fake {
    scripts: Mutex<VecDeque<Script>>,
    calls: Mutex<Vec<Call>>,
}

impl Fake {
    pub fn push(&self, script: Script) {
        self.scripts.lock().unwrap().push_back(script);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn next(&self, model: &str, messages: &[Message], max_tokens: u32) -> Script {
        self.calls.lock().unwrap().push(Call {
            model: model.into(),
            messages: messages.to_vec(),
            max_tokens,
        });
        self.scripts.lock().unwrap().pop_front().unwrap_or_default()
    }
}

pub(super) fn fail(retryable: bool) -> LlmError {
    LlmError::Provider { retryable }
}
