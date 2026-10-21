# nodemind: Product Brief

> Keep this reviewable in two minutes. Link out to detail; do not paste history here.

## What it does
A learning-focused AI chat on a canvas. Each prompt and answer is a node; the main discussion flows vertically, and a doubt about any part of an answer (a phrase, a sentence, code lines, or the whole node) branches sideways beside it. Each node is answered using only its root-to-node path, so detours don't pollute the main thread, and a finished branch merges back as a one-line summary. MVP is this branching canvas (brief phase 1); composing notes and sketchnotes is roadmap.

## Who it is for / problem
Self-learners and students who study topics by chatting with AI. In linear chat, asking about part of an answer buries that answer, and the side questions stay in context and cause drift, so learners lose their place or skip their doubts. Demand is an assumption, not yet validated with users. Detail: [discovery.md](product/discovery.md).

## Architecture in one breath
See [ARCHITECTURE.md](ARCHITECTURE.md). TODO(factory): main request/data flow in 3 lines.

## Main technologies
TODO(factory)

## What I learned
See [learning/](learning/) and [DAILY_LOG.md](DAILY_LOG.md).

## Current status

<!-- factory:status:start -->
_Auto-generated from recorded factory state. Do not edit inside this block._

**Stage:** implementation (Implementation) · profile `standard`

**Verified & accepted work:**
- T1 Monorepo skeleton with test commands (`5aad63f8`)
- T2 Database schema migrations and db module (`dcb97fa8`)
- T3 Shared domain types with ts-rs and schemars generation (`f0eb7b0e`)
- T10 Context: anchors, prompts and quick-action templates (`c47fbe32`)
- T13 Metering ledger and Reservation guard (`75a11748`)

**Open risks:**
- task T12 failed: Claude Code run agent_error
- F-architecture-L3-1 (high): Advisory-lock liveness depends on a direct connection, but DATABASE_URL_DIRECT is optional
- F-architecture-L3-2 (medium): The 15-minute reservation safety net assumes tasks end by TASK_DEADLINE_SECS, but the final write retries without limit
- F-architecture-L3-3 (medium): Device-cookie signing key is missing from config
- F-architecture-L3-4 (medium): NFR-11 'a completed node is never lost' can't hold with daily backups only
- F-architecture-L3-5 (medium): Canvas load recomputes the full prompt assembly for every node
- F-t2-L3-1 (medium): Migrations silently fall back to the pooled DATABASE_URL, though production requires DATABASE_URL_DIRECT
- F-t2-L3-2 (medium): 10 s pool statement_timeout is not in the architecture
- F-t4-L3-1 (medium): Connection-level Postgres failures reported as SQLSTATE get 500 `internal` instead of 503 `unavailable`
- F-t4-L3-2 (medium): Only `Json` has a safe rejection; `Path`/`Query`/`Form` rejections echo input as plain text outside the error shape

**Next:**
- T4 HTTP foundation: router, errors, CSRF, headers, logging (awaiting_approval)
- T5 Auth core: invites, sessions, login, logout (pending)
- T6 Auth abuse protection and rate limiter (pending)
- T7 Canvas CRUD, meta endpoint and ownership tests (pending)
- T8 Account deletion endpoint (pending)
<!-- factory:status:end -->
