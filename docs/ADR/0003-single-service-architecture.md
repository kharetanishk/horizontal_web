# ADR 0003: One Rust service + Postgres for phase 1

- Status: accepted
- Date: 2026-10-18

## Context

Phase 1 needs auth, canvas CRUD, streamed answers (SSE), path-scoped context and merge back for a handful of test users. The brief's stack also lists Redis/ARQ workers, MinIO, Langfuse and a Next.js server. None of these is needed by a phase 1 requirement: there are no background jobs, uploads or tracing, and the canvas is a client-only app behind login. NFR-10 targets 99% monthly availability on a single instance.

## Decision

One Axum binary serves the JSON API, the SSE streams and the static Next.js export (`output: "export"`, served with tower-http). Postgres is the only dependency. LLM streaming runs in a Tokio task inside the API, with partial text flushed to Postgres. Rejected: a Redis queue + LLM worker (extra hop and ops for no requirement), and a Next.js server proxying to the API (a second runtime, and SSE buffering risk).

## Consequences

One image, one process, same-origin cookies, no CORS. Rate limits are in-process, so the app runs as one instance until they move to Postgres (a brief overlap during deploys is accepted). Interrupted streams are detected through Postgres advisory locks, which is safe with several instances ([ADR 0006](0006-context-and-stream-engine.md)). Phase 2/3 (composer, embeddings) add a worker and a queue; the module boundaries (`llm`, `context`) are kept so a worker binary can reuse them.
