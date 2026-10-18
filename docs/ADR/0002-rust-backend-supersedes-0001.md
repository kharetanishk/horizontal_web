# ADR 0002: Rust backend (supersedes 0001)

- Status: accepted
- Date: 2026-10-18

## Context

Owner changed the backend language from Python to Rust on 2026-10-18, before any code existed. Frontend stays Next.js + TypeScript + React Flow.

## Decision

Backend is a Cargo workspace in Rust: Axum (HTTP + SSE), Tokio, SQLx with Postgres + pgvector and SQLx migrations. All model calls go through one LLM gateway module built on a Rust client crate (genai or async-openai, picked in architecture); model per mode is config. Token counting with tiktoken-rs. Shared types: Rust structs are the source of truth; ts-rs generates TypeScript into packages/schemas and schemars emits JSON Schema for LLM structured outputs. API tests use cargo test with a fake LLM provider.

## Consequences

No Python in phase 1: LiteLLM, FastAPI, SQLAlchemy, Alembic, Pydantic and the uv workspace are dropped. Brief items that were Python-only (LiteLLM, Langfuse SDK, MCP Python SDK, promptfoo/pytest evals) are re-chosen when their phase starts. Rust gives low latency and strong typing but fewer ready-made AI libraries, so the gateway module owns provider differences.
