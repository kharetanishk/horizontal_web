# Decisions

| ADR | Title | Status | Date |
|---|---|---|---|
| [0001](ADR/0001-python-backend.md) | Python backend | superseded by 0002 | 2026-10-18 |
| [0002](ADR/0002-rust-backend-supersedes-0001.md) | Rust backend (supersedes 0001) | accepted | 2026-10-18 |
| [0003](ADR/0003-single-service-architecture.md) | One Rust service + Postgres for phase 1 | accepted | 2026-10-18 |
| [0004](ADR/0004-password-auth-postgres-sessions.md) | Email + password auth, Postgres sessions, invite-only accounts | accepted (revised 2026-10-19) | 2026-10-18 |
| [0005](ADR/0005-llm-gateway-and-tokenizer.md) | LLM gateway on genai; token counting behind the gateway | accepted (spike: compile-level only; owner model choice pending) | 2026-10-18 |
| [0006](ADR/0006-context-and-stream-engine.md) | Context and stream engine: checkpoint compaction, attachable streams, lock-based liveness, reservation ledger | accepted | 2026-10-19 |

## T12 spike and model decision (2026-10-21)

- **Spike (genai 0.7.0):** `exec_chat_stream` with `capture_usage` exposes chunk events and an end event with captured usage (`prompt_tokens`, `completion_tokens`, `prompt_tokens_details.cached_tokens`) and stop reason; `apps/api/src/llm/gateway.rs` maps these to `Chunk`/`Usage`/`FinishReason` and compiles. **Not verified against a live provider** (no keys here); AC-12.2 remains the live check. Fallback async-openai not needed so far.
- **Provider and models per mode: OWNER DECISION PENDING.** `MODEL_MAIN`, `MODEL_DOUBT`, `MODEL_SUMMARY` are env-only, no defaults. Per ADR 0005 all three need a local tokenizer (OpenAI via `tiktoken-rs`, or open-weight via `tokenizer.json`). Fill in here before merging T12.
- API keys are read by genai from the provider's env var; the gateway never stores, logs or returns them, and `LlmError` carries no provider text (NFR-9). `LLM_PROVIDER=fake` selects the scripted provider for tests and Playwright.
