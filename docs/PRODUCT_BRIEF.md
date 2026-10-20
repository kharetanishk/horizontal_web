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

**Stage:** architecture (Architecture) · profile `standard`

**Verified & accepted work:** none yet

**Open risks:**
- F-architecture-L3-1 (high): Advisory-lock liveness depends on a direct connection, but DATABASE_URL_DIRECT is optional
- F-architecture-L3-2 (medium): The 15-minute reservation safety net assumes tasks end by TASK_DEADLINE_SECS, but the final write retries without limit
- F-architecture-L3-3 (medium): Device-cookie signing key is missing from config
- F-architecture-L3-4 (medium): NFR-11 'a completed node is never lost' can't hold with daily backups only
- F-architecture-L3-5 (medium): Canvas load recomputes the full prompt assembly for every node

**Next:**
- `factory run` to continue the architecture phase
<!-- factory:status:end -->
