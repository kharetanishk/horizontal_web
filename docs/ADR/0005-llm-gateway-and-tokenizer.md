# ADR 0005: LLM gateway on the genai crate; token counting behind the gateway

- Status: accepted (pending spike)
- Date: 2026-10-18

## Context

ADR 0002 left the Rust LLM client open (genai or async-openai). NFR-13 needs a model per mode set by config, and NFR-6 needs the token counter within ±5% of provider usage. `tiktoken-rs` only covers OpenAI encodings (assumption, per research.md). Providers are not chosen yet.

## Decision

The `llm` module is the only code that calls providers. It exposes `stream`, `complete` and `count_tokens`, and is built on `genai` for multi-provider streaming (assumption: it reports usage for the chosen providers; a one-day spike verifies this). Fallback is `async-openai` against OpenAI-compatible APIs. Models per mode (`MODEL_MAIN`, `MODEL_DOUBT`, `MODEL_SUMMARY`) come from env. `count_tokens` is always local: `tiktoken-rs` for OpenAI models, otherwise a local approximation marked `estimated` and calibrated against provider-reported usage. Canvas load never calls a provider to count. Models with a local tokenizer are preferred for main and doubt modes.

## Consequences

Swapping a model is a config change; swapping the client crate touches one module. The provider choice still decides whether counting stays local and fast. AC-12.2 (20 real requests) is the check.
