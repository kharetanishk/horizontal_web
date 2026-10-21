//! The only module that talks to LLM providers (NFR-13). Callers see `Gateway`, never a provider crate.
pub mod fake;
pub mod gateway;

pub use gateway::{Gateway, Models, Timeouts};

use crate::metering::Usage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    /// Hit `max_tokens`; the answer is cut off.
    Length,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chunk {
    Delta(String),
    /// Always last. `usage` is `None` when the provider reported none (caller estimates).
    End {
        usage: Option<Usage>,
        finish: FinishReason,
    },
}

/// Carries no provider text: provider errors can echo request content or credentials (NFR-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmError {
    ConnectTimeout,
    FirstTokenTimeout,
    ChunkTimeout,
    /// `retryable` marks errors worth one pre-first-token retry.
    Provider {
        retryable: bool,
    },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LlmError::ConnectTimeout => "llm connect timeout",
            LlmError::FirstTokenTimeout => "llm first-token timeout",
            LlmError::ChunkTimeout => "llm chunk timeout",
            LlmError::Provider { .. } => "llm provider error",
        })
    }
}

impl std::error::Error for LlmError {}
