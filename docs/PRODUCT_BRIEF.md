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
- F-architecture-L3-1 (high): Final write retries forever on permanent DB errors (e.g. NUL byte), pinning the node and an in-flight slot
- F-architecture-L3-2 (medium): A merged branch's own summary is injected into prompts on that same branch
- F-architecture-L3-3 (medium): Canvas deletions are resurrected by a backup restore (no tombstones)
- F-architecture-L3-4 (medium): Unbounded argon2 queue lets anyone degrade login for every user
- F-architecture-L3-5 (medium): Compaction reservation at the summary model's window maximum can exceed the daily cap, disabling compaction
- F-architecture-L3-6 (medium): SSE has no keepalive; proxies will cut long silent phases
- F-architecture-L3-7 (medium): Canvas load recomputes the mode-tokenizer count of every node from scratch after a restart

**Next:**
- `factory run` to continue the architecture phase
<!-- factory:status:end -->
