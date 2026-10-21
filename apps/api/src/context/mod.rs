//! Context builder pieces (docs/ARCHITECTURE.md, Context builder).
pub mod anchor;
pub mod templates;

/// Picks the system prompt, model and tokenizer for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Main,
    Doubt,
    Summary,
}
