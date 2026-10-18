# Requirements

Inputs: [discovery.md](discovery.md), [source-brief.md](source-brief.md) (phase 1 only).
`references/catalog.json` is empty, so no requirement links to a reference ID (R#). Every target below is a **target**, not a measurement. Anything not backed by the brief or discovery is marked **assumption**.

## MVP scope

The MVP is brief phase 1, the branching canvas, cut to what tests the riskiest assumption:
**learners will highlight and branch a doubt instead of typing a linear follow-up, and can get back to the main thread without effort** (discovery assumptions 2 and 5; the phase 1 gate).

In the MVP:

- Email sign-in; a user sees only their own canvases.
- Canvases: create, list, open, rename, delete.
- Main thread: question and streamed answer nodes flowing vertically.
- Doubt branches sideways from a whole node, a text highlight, or a code line range; branches can branch again.
- Five quick actions plus a free-text question on highlight.
- Path-scoped context: a node is answered from its root-to-node path only, with compaction of old messages over budget.
- Live token counter per path.
- Merge back: a branch becomes an editable one-line summary on the main path and collapses.
- Canvas navigation: auto-layout, collapse/expand branches, "back to main" zoom-to-node.
- Account deletion and a "may contain errors" notice.

Cut from phase 1 and why:

| Cut | Why |
| --- | --- |
| `region` anchors (box on image or diagram) | Phase 1 has no image or diagram nodes; the brief marks it "later" |
| Learner memory (context step 3), RAG (step 5) | Phase 3 in the brief; not needed to test branching |
| Redis, ARQ workers, MinIO, Langfuse, sandbox | No phase 1 feature needs a queue, object storage or tracing service; streaming runs in the API request. Add when a phase needs them (CLAUDE.md rule) |
| Measuring prompt-cache hit rate, small-model routing evals | Phase 5 quality layer. Phase 1 only keeps path prefixes stable and lets the doubt model be set in config |
| Editing or deleting single nodes | Anchors point at stored text (anchor drift risk); immutable nodes avoid it. Collapse covers hiding unwanted branches |
| Model picker in the UI | Models per mode are server config in the LLM gateway; users don't need the choice to test branching |

## Functional requirements

Each FR is one user-visible capability. Acceptance criteria are under [Acceptance criteria](#acceptance-criteria) with matching IDs.

| ID | Capability | Notes |
| --- | --- | --- |
| FR-1 | Sign in with email; sign out | Method (magic link vs. password) is an architecture decision. Brief `users` table: id, email, created_at |
| FR-2 | Canvas management: create, list own canvases, open, rename, delete | Delete removes all nodes and edges of the canvas |
| FR-3 | Ask on the main thread: a question becomes a `user_message` node, the answer streams token by token into an `ai_message` node below it, joined by `reply` edges | SSE streaming (brief) |
| FR-4 | Continue the main thread: a follow-up appears vertically under the last answer and is answered from that path | |
| FR-5 | Branch a doubt from a whole node (anchor `none`), from any `ai_message` or own `user_message` | Covers "give an example of this whole answer" and "what should I have asked instead?" |
| FR-6 | Branch a doubt from a text highlight (anchor `text`: start, end, text) in a completed answer | Anchor shape from the brief |
| FR-7 | Branch a doubt from a line range in a code block (anchor `code`: block index, start line, end line, text) | Context gets the selected lines plus the rest of the block |
| FR-8 | Quick actions on highlight or node: explain simpler, give an example, why does this work, go deeper, "I think this is wrong"; plus a free-text question | Each maps to a prompt template; the doubt type is stored on the branch for later evals |
| FR-9 | Nested branches: any node inside a branch can be branched or continued, to any depth | |
| FR-10 | Path-scoped context: a message at node X is answered using only system prompt (by mode), merged summaries on the path, path messages root→X, the anchor (for branches) and the new message. Sibling branches never reach the model | Context builder steps 1, 2, 4, 6, 7, 8 of the brief |
| FR-11 | Compaction: when the path exceeds the input token budget, older path messages are replaced by a summary; the most recent messages stay verbatim | Budget is server config. Default 16k input tokens (**assumption**, tune in architecture) |
| FR-12 | Token counter: each node shows the token count of the context its next message would send (path + summaries + system prompt), updated when the path changes | Provider tokenizer (tiktoken-rs) |
| FR-13 | Merge back: on a branch, "merge" generates a one-line summary, shows it for the user to edit, and on confirm attaches it to the branch's origin node on the main path and collapses the branch | Mitigates lossy merge: user sees and can fix the summary |
| FR-14 | Merged summaries are included in context for every node whose path passes through the branch's origin node; the full branch messages are not | |
| FR-15 | Auto-layout: main path flows top to bottom; branches open to the right of their parent, next to the anchor; new nodes never overlap existing ones | Canvas clutter risk |
| FR-16 | Collapse and expand any branch; collapsed branches show a count of hidden nodes and, if merged, the summary | |
| FR-17 | Navigation: from any branch node, "back to main" pans and zooms to the branch's origin node on the main thread; the anchor highlight is visible in the origin node | Phase 1 gate "returning to the main thread feels effortless" |
| FR-18 | Failure handling: if a stream fails or is cut, the node keeps the partial text, shows an error state and a retry action; retry replaces the failed answer | No lost user questions |
| FR-19 | Persistence: canvases, nodes, edges, anchors, positions and summaries are saved; reopening a canvas restores it exactly | |
| FR-20 | Account deletion: a user can delete their account and all their canvases | Privacy (discovery legal risk) |
| FR-21 | Notices: privacy notice naming the LLM providers used, and a "AI answers may contain errors" notice visible on the canvas | Discovery legal risk |

## Nonfunctional requirements

All values are targets for the phase 1 build, verified in tests or by measurement before acceptance. None are measured yet.

| ID | Area | Target |
| --- | --- | --- |
| NFR-1 | Context build latency | Path fetch + prompt assembly p95 < 200 ms for a path of 200 nodes on a 1,000-node canvas (excludes LLM time) |
| NFR-2 | Time to first token | p95 < 3 s from send to first streamed token in the node, with the default models (**assumption**: provider latency dominates; measure, don't promise) |
| NFR-3 | Canvas load | Open a 200-node canvas and render it in < 2 s p95 on a mid-range laptop, Chrome stable |
| NFR-4 | Canvas interaction | Pan and zoom on a 200-node canvas stay at ≥ 30 fps (Chrome performance trace) |
| NFR-5 | Branch friction | Highlight → quick action → branch request sent: ≤ 2 clicks after the highlight; free-text question: 1 click + typing + Enter |
| NFR-6 | Token counter accuracy | Counter within ±5% of the prompt tokens reported by the provider for the same request |
| NFR-7 | Context budget | No request sends more input tokens than the configured budget; reply room reserved (default 2k tokens, **assumption**) |
| NFR-8 | Authorization | Every canvas, node, edge and stream endpoint checks ownership; another user's IDs return 404. Covered by API tests for each endpoint |
| NFR-9 | Data protection | Message content is not written to application logs; secrets only from environment, never sent to the client; HTTPS in production |
| NFR-10 | Availability | 99% monthly for the single-instance deployment (target; no SLA) |
| NFR-11 | Durability | Daily Postgres backup with 7-day retention; a completed node is never lost after the stream ends |
| NFR-12 | Cost | Doubt branches use the model set for doubt mode (can be a smaller model); per-request token usage is recorded on the node (`token_count`, `model`) |
| NFR-13 | Model portability | All model calls go through the backend's LLM gateway module; switching the model for a mode is a config change with no code change |
| NFR-14 | Browser support | Latest Chrome, Firefox, Safari on desktop. Mobile is out of scope |
| NFR-15 | Accessibility | Quick actions, merge, collapse and "back to main" reachable by keyboard; visible focus; text contrast WCAG 2.1 AA |
| NFR-16 | Type sync | Node types, edge types and anchor shape defined once in the Rust backend; Rust types are the source, generating TS (ts-rs) and JSON Schema (schemars); CI fails if generated types are stale |

## Acceptance criteria

Given/When/Then unless a measurable check is clearer. Each is meant to become an automated test (API: cargo test; UI: Playwright; LLM calls stubbed with a fake model unless stated).

**FR-1 Sign in**
- AC-1.1 Given a new email, when the user completes sign-in, then a `users` row exists and the canvas list is shown.
- AC-1.2 Given a signed-out visitor, when they request any canvas API, then the response is 401.

**FR-2 Canvases**
- AC-2.1 Given a signed-in user, when they create a canvas, then it appears in their list with a default title they can rename.
- AC-2.2 Given user A's canvas, when user B lists canvases or requests it by ID, then it is absent / 404.
- AC-2.3 Given a canvas with nodes, when it is deleted, then no nodes or edges with its `canvas_id` remain.

**FR-3, FR-4 Main thread**
- AC-3.1 Given an empty canvas, when the user sends a question, then a `user_message` node and an `ai_message` node exist joined by a `reply` edge, and the answer text arrives in ≥ 2 SSE chunks.
- AC-4.1 Given an answer node, when the user sends a follow-up, then the new `user_message` is a `reply` child of that answer and placed below it.

**FR-5 – FR-8 Branching**
- AC-5.1 Given a completed `ai_message` or `user_message`, when the user picks "branch from whole node" and asks, then a `branch` edge with `anchor.kind = "none"` connects the node to a new `user_message` placed to its right.
- AC-6.1 Given a completed answer, when the user highlights characters 412–540 and asks, then the branch edge stores `{kind: "text", start: 412, end: 540, text}` and `content[start:end] == text`.
- AC-6.2 Given an answer still streaming, when the user tries to highlight-branch, then branching is disabled until the stream completes.
- AC-7.1 Given an answer with a code block, when the user selects lines 5–9 and asks, then the anchor is `kind: "code"` with those lines, and the assembled prompt contains the selected lines marked as the part asked about plus the rest of the block.
- AC-8.1 For each of the five quick actions, when chosen, then the prompt uses that action's template and the branch stores its doubt type; a free-text question stores type `free`.
- AC-8.2 Given "I think this is wrong", then the prompt asks the model to re-check the anchored part.

**FR-9, FR-10 Path-scoped context**
- AC-9.1 Given a branch node, when the user branches from it, then a second-level branch is created to its right and its path includes both branch origins.
- AC-10.1 Given main path A→B→C with sibling branch B→X→Y, when a message is sent at C, then the assembled prompt contains A, B, C and not X or Y.
- AC-10.2 Given a branch at Y, then its prompt contains A, B, X, Y and B's anchor, and not C.
- AC-10.3 Given a branch, then the doubt-mode system prompt is used; on the main thread the main-mode prompt is used.
- AC-10.4 Given the same path, when two messages are built in a row, then the prompt prefix up to the new message is byte-identical (stable for provider caching).

**FR-11 Compaction**
- AC-11.1 Given a path whose messages exceed the budget, when a prompt is built, then input tokens ≤ budget, the latest messages are verbatim, and older ones appear as one summary block.

**FR-12 Token counter**
- AC-12.1 Given any node, then its counter equals the token count of the prompt that would be sent from it (same function as the context builder), and changes when a merge summary is added to its path.
- AC-12.2 Measurable: on 20 real requests, counter vs. provider-reported prompt tokens differs by ≤ 5% (NFR-6; uses a real model, run manually).

**FR-13, FR-14 Merge back**
- AC-13.1 Given a branch, when the user clicks merge, then a one-line summary (≤ 200 characters) is proposed and editable before confirm.
- AC-13.2 When the user confirms, then the summary is stored against the origin node, the branch is collapsed, and the summary is visible on the origin node.
- AC-14.1 Given a merged branch from B, when a message is sent at C (downstream of B), then the prompt contains the summary and none of the branch's messages.
- AC-14.2 Given a merged branch from C, when a message is sent at B (upstream), then the summary is absent.

**FR-15 – FR-17 Canvas navigation**
- AC-15.1 Given 30 nodes created by a scripted session (main path + 3 levels of branches), then no two node bounding boxes overlap and every branch node's x is greater than its parent's.
- AC-16.1 When a branch is collapsed, then its nodes are hidden and the origin shows "N hidden"; expanding restores them at the same positions.
- AC-17.1 Given the viewport on a third-level branch node, when the user activates "back to main", then the main-thread origin node is centered in the viewport and its anchor text is highlighted.

**FR-18 Failure handling**
- AC-18.1 Given the model stream errors after partial text, then the node keeps the partial text, shows an error and a retry; retry streams a new answer into the node and the user's question is unchanged.

**FR-19 Persistence**
- AC-19.1 Given a canvas with branches, collapses and merges, when it is reloaded, then nodes, edges, anchors, positions, collapse state and summaries match the pre-reload state.

**FR-20, FR-21 Privacy and notices**
- AC-20.1 When a user deletes their account, then their `users` row, canvases, nodes and edges are gone and their session is invalid.
- AC-21.1 The canvas shows the "may contain errors" notice; the privacy notice lists the configured LLM providers.

**NFRs**
- NFR-1, NFR-3, NFR-4: benchmark scripts on a seeded 1,000-node canvas; report p95 against the target.
- NFR-5: Playwright test counts clicks from highlight to branch request.
- NFR-7: AC-11.1. NFR-8: one ownership test per endpoint. NFR-9: test asserts a sent message's text is absent from captured logs. NFR-16: CI step regenerates types and fails on diff.
- NFR-2, NFR-10, NFR-11: measured after deploy; not test-verifiable before launch.

## Out of scope

Not in the MVP, with reason:

- Composer, notes, sketchnotes (Sketch and Illustrated), OCR checks: phase 2.
- Embeddings, semantic search, learner memory, RAG over uploads, `source` nodes: phase 3.
- Landing page builder, sandbox (E2B/Firecracker): phase 4.
- Intent router, eval suites (promptfoo), guardrails beyond "I think this is wrong", Langfuse tracing: phase 5.
- MCP server, Neo4j knowledge graph, image/video generation, YouTube and video ingestion: phase 6.
- `region` anchors, `artifact` and `composer` node types, `input` edges.
- Editing or deleting single nodes; model picker; sharing or collaboration; export; mobile layout; teams or billing.
- Redis, workers, MinIO: not needed until a phase uses background jobs or uploads.

## Success metrics

Measured after phase 1 launch. Phase 1 has no tracing service, so instrumentation is Postgres queries over `nodes`/`edges` plus a small `events` table (or structured API logs) for UI actions: `branch_created`, `merge_confirmed`, `back_to_main`, `collapse`. Targets are **assumptions** to check, not results.

| Metric | Definition | Instrumentation | Target (assumption) |
| --- | --- | --- | --- |
| Branch adoption | Share of doubts asked as branches vs. linear follow-ups, per session with ≥ 5 questions | Count `branch` vs. `reply` edges from `user_message` nodes, per canvas | ≥ 40% of non-first questions are branches |
| Return to main | Share of branches after which the user continues the main thread in the same session | `back_to_main` / `merge_confirmed` events followed by a new `reply` on the main path | ≥ 70% |
| Merge use | Share of branches with ≥ 2 nodes that get merged back | `merge_confirmed` events / branches | ≥ 30% |
| Merge edits | Share of merge summaries edited before confirm (proxy for summary quality) | Compare proposed vs. confirmed text in `merge_confirmed` event | ≤ 50% edited |
| Phase 1 gate: effortless | In moderated sessions with 5 self-learners doing a 20-minute study task: median seconds from highlight to branch question sent, and post-task ease rating (1–7) | Event timestamps + a one-question post-task survey | Median < 10 s; median rating ≥ 6 |
| Return | Users who come back to a canvas on a second day within 7 days | `nodes.created_at` on ≥ 2 distinct days per user | ≥ 3 of the 5 test users (assumption) |
| Cost per session | Mean tokens and cost per canvas session | Sum of `nodes.token_count` by `model` | Baseline only; no target until measured |

## Roadmap

From the brief, each phase starts only after the previous gate is met:

1. **Branching canvas** (this MVP). Gate: branching a doubt and returning to the main thread feels effortless (success metrics above).
2. **Composer.** Node selection and ordering, notes, Sketch-style sketchnotes, then Illustrated with OCR checks. Gate: notes pass the faithfulness check. Start only if test users ask to turn sessions into notes (discovery assumption 8).
3. **Search and memory.** Embeddings, semantic search, learner memory, RAG over uploads. Adds workers and object storage.
4. **Landing page builder.** Agentic plan/code/run/fix loop with sandboxed preview.
5. **Quality layer.** Intent routing, eval suites, guardrails, tracing, latency and cost optimization (prompt-cache measurement, small-model evals).
6. **Extras.** MCP server, knowledge graph, image generation, YouTube and video ingestion, Remotion.
