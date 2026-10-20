# Implementation plan

| ID | Task | Risk | Depends on | Tests |
|---|---|---|---|---|
| T1 | Monorepo skeleton with test commands | low |  | `cargo test`; `pnpm test`; `pnpm build` |
| T2 | Database schema migrations and db module | high | T1 | `cargo test -p nodemind-api --test schema` |
| T3 | Shared domain types with ts-rs and schemars generation | low | T1 | `cargo test -p nodemind-api types`; `pnpm typecheck` |
| T4 | HTTP foundation: router, errors, CSRF, headers, logging | high | T2, T3 | `cargo test -p nodemind-api --test http` |
| T5 | Auth core: invites, sessions, login, logout | high | T4 | `cargo test -p nodemind-api --test auth` |
| T6 | Auth abuse protection and rate limiter | high | T5 | `cargo test -p nodemind-api --test auth_limits` |
| T7 | Canvas CRUD, meta endpoint and ownership tests | high | T5 | `cargo test -p nodemind-api --test canvas` |
| T8 | Account deletion endpoint | high | T6, T7 | `cargo test -p nodemind-api --test account` |
| T9 | Context: tokenizers, ratios and checkpoint fields | medium | T3, T5 | `cargo test -p nodemind-api context::tokens`; `cargo test -p nodemind-api context::checkpoint` |
| T10 | Context: anchors, prompts and quick-action templates | medium | T3 | `cargo test -p nodemind-api context::anchor`; `cargo test -p nodemind-api context::templates` |
| T11 | Context: plan/render assembly with property tests | medium | T9, T10 | `cargo test -p nodemind-api --test context_props` |
| T12 | LLM gateway and fake provider | medium | T3 | `cargo test -p nodemind-api --test llm` |
| T13 | Metering ledger and Reservation guard | medium | T2 | `cargo test -p nodemind-api --test metering` |
| T14 | Stream registry and SSE attach | medium | T4 | `cargo test -p nodemind-api --test stream_attach` |
| T15 | Instance liveness, sweeper and graceful drain | high | T13, T14 | `cargo test -p nodemind-api --test liveness` |
| T16 | Send-message endpoint and stream task | high | T6, T7, T11, T12, T15 | `cargo test -p nodemind-api --test send` |
| T17 | Retry and stream reattach endpoints | medium | T15, T16 | `cargo test -p nodemind-api --test retry` |
| T18 | Compaction summary chain and precompute scheduler | medium | T16 | `cargo test -p nodemind-api --test compaction` |
| T19 | Merge back, collapse and events endpoints | medium | T16 | `cargo test -p nodemind-api --test merge` |
| T20 | Canvas load with token counters and positions save | medium | T18, T19 | `cargo test -p nodemind-api --test canvas_load` |
| T21 | Fault-injection integration suite | medium | T17, T18 | `cargo test -p nodemind-api --test faults -- --ignored` |
| T22 | Web foundation: tokens, API client, sign-in and invite pages | medium | T3 | `pnpm --filter web test`; `pnpm --filter web typecheck` |
| T23 | Canvas list and account pages | medium | T22 | `pnpm --filter web test` |
| T24 | Auto-layout pure function | low | T3 | `pnpm --filter web test` |
| T25 | Canvas screen: React Flow, store, nodes and edges | medium | T22, T24 | `pnpm --filter web test`; `pnpm --filter web exec playwright test e2e/canvas.spec.ts` |
| T26 | Composer, SSE client, streaming and retry UI | medium | T25 | `pnpm --filter web test`; `pnpm --filter web exec playwright test e2e/stream.spec.ts` |
| T27 | Selection toolbar, anchors and quick actions | medium | T26 | `pnpm --filter web test`; `pnpm --filter web exec playwright test e2e/branch.spec.ts` |
| T28 | Collapse, merge dialog and back to main | medium | T20, T24, T27 | `pnpm --filter web exec playwright test e2e/merge.spec.ts` |
| T29 | Keyboard paths and accessibility checks | low | T28 | `pnpm --filter web exec playwright test e2e/a11y.spec.ts` |
| T30 | Static serving, CSP hashes, container image and CI | high | T16, T25 | `cargo test -p nodemind-api static_files`; `pnpm build` |
| T31 | Performance benchmarks on a seeded canvas | low | T21, T29 | `cargo bench -p nodemind-api`; `pnpm --filter web exec playwright test e2e/perf.spec.ts` |
| T32 | End-to-end phase 1 journey test | low | T29, T30 | `pnpm --filter web exec playwright test e2e/journey.spec.ts` |
