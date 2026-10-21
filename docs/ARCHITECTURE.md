# Architecture

Scope: brief phase 1 (branching canvas) only. Inputs: [requirements.md](product/requirements.md) (FR/NFR IDs below refer to it), [source-brief.md](product/source-brief.md), [research.md](product/research.md), [ADR 0002](ADR/0002-rust-backend-supersedes-0001.md), [ADR 0006](ADR/0006-context-and-stream-engine.md).
No code exists yet, so no number here is measured. Library behaviour, provider behaviour and hosting details are **assumptions** until a spike or test checks them.

## Design principles

1. **The conversation is an immutable, append-only tree.** A node's content never changes once it has a child. So everything derived from a path prefix (compaction checkpoints, their summaries, token counts, the provider's prompt cache) is computed once and stays valid for every descendant, on every branch.
2. **One function, one truth.** `assemble()` builds every prompt and every token counter. No second code path estimates what the first one does.
3. **Postgres is the coordination layer.** Transactions do the metering, session advisory locks track which instance is alive, and an `attempt` fence keeps one writer per node. There is no Redis, no heartbeat writes and no clock-based guessing.
4. **A stream is a log, not a channel.** An answer is a growing buffer. Any client can attach at any time, receive everything so far, then follow along. Slow readers coalesce and never drop; terminal states are always delivered.

## Options considered

| | A. One Rust service + Postgres (chosen) | B. API + Redis queue + LLM worker | C. Next.js server (BFF) + Rust API + Postgres |
| --- | --- | --- | --- |
| Shape | Axum binary serves the API, SSE, and the static Next.js export. Postgres is the only dependency | API enqueues each answer; worker calls the LLM and pushes tokens to the API through Redis pub/sub; API relays SSE | Next.js server renders pages and proxies `/api` to Axum |
| Streaming (FR-3, NFR-2) | Direct: LLM stream → buffer → SSE in one process | Extra hop through Redis, more latency and code | Extra proxy hop; SSE buffering through the Next proxy is a known pitfall (assumption) |
| Partial text kept on failure (FR-18) | Stream task in the API writes the partial text | Worker writes it | Same as A |
| Availability 99% (NFR-10) | One instance plus managed Postgres is enough for a 99% target (≈7 h downtime/month) | Three things to keep up | Two app processes to keep up |
| Ops cost | One image, one process | Redis plus worker deploys | Node runtime plus Rust runtime |
| Needed when | Now | Requests outlive an HTTP connection, or slow jobs arrive (phase 2 composer, phase 3 embeddings) | Pages need SSR or SEO (none in phase 1: the canvas is client-only, behind login) |

**Decision: A** ([ADR 0003](ADR/0003-single-service-architecture.md)). No phase 1 requirement or number calls for a queue, cache, second service, Kubernetes, sharding or multi-region. Expected load is a handful of test users (5 moderated learners), so one instance is enough. The engine below is still correct while two instances overlap during a deploy.

**Rejected ambitious options** (each solves a problem phase 1 does not have):

- CRDT / local-first sync: there is no collaboration.
- Event sourcing: the immutable tree already is the log.
- Crypto-shredding per user: it needs a key store outside the database backups; revisit for compliance.
- Shared Rust core compiled to WASM for in-browser counting: server counts are enough; revisit for a live counter while typing.
- Retrieval over embeddings: phase 3.

## Components

```
Browser (Next.js static export, React Flow, Zustand)
   │  same-origin HTTPS: JSON + SSE, session cookie
   ▼
nodemind-api (one Rust binary: Axum + Tokio)
   ├── http      routes, auth extractor, ownership checks, Origin check, error mapping, rate limits
   ├── auth      invite accept/login/logout, argon2id, Postgres sessions
   ├── canvas    canvases, nodes, edges, positions, collapse, merge
   ├── chat      send/retry handlers (admission transaction)
   ├── stream    stream registry, SSE attach, instance liveness lock, sweeper, graceful drain
   ├── context   pure: assemble(), checkpoints, truncation; plus the summary precompute scheduler
   ├── metering  reservation ledger: admission, settlement, daily cap, concurrency
   ├── llm       gateway: model per mode, streaming, usage, timeouts, retries
   ├── static    serves the exported web app (tower-http ServeDir, SPA fallback, CSP hashes)
   └── db        SQLx pool, embedded migrations
   │                         │
   ▼                         ▼
Postgres 16 (managed)     LLM provider(s) over HTTPS
```

| Component | Responsibility | Notes |
| --- | --- | --- |
| `apps/web` | Canvas UI: nodes, selection toolbar, quick actions, merge dialog, collapse, "back to main", token counters, notices | Next.js `output: "export"`; no Node server in production. Canvas route is `/canvas?id=…`, because static export needs build-time params for dynamic segments (assumption, verify). React Flow for pan/zoom/`setCenter`; Zustand store holds canvas nodes/edges |
| Layout (in `apps/web`) | Own tree layout (FR-15): main path down; each branch starts right of its parent at the anchor's y; subtrees stacked so bounding boxes never overlap. Uses measured node heights from React Flow | About 100 lines, pure function with a unit test for AC-15.1. Positions are saved to the server so reload is exact (FR-19) |
| `packages/schemas` | TypeScript types generated from Rust (ts-rs) | NFR-16: CI regenerates and fails on diff |
| `apps/api` | Single crate, modules above | Cargo workspace with one member. `context` has no IO, so it is unit- and property-tested in isolation. Split crates only when a second binary (worker, phase 2+) needs shared code |
| `apps/api/prompts/` | System prompts per mode (main, doubt, summary) and the 5 quick-action templates, loaded with `include_str!` | Versioned with the code |
| `llm` gateway | Only module that talks to providers (NFR-13). Interface: `stream(mode, messages) -> Stream<Delta>` then `Usage`; `complete(mode, messages) -> (String, Usage)` | Built on the `genai` crate (assumption, spike first). Fallback: `async-openai`. [ADR 0005](ADR/0005-llm-gateway-and-tokenizer.md) |
| Postgres | All state: users, sessions, canvases, nodes, edges, llm_usage, events, invites, tombstones | pgvector is **not** enabled in phase 1 |

## Send-message flow (FR-3 – FR-10)

1. The client sends `POST /api/canvases/:id/messages` with client-generated UUIDs for the new user and AI nodes. The input text stays in the client until the `created` event arrives, so a failed request never loses the question (FR-18).
2. **Rate limit first.** The in-process token bucket (20 LLM calls/min per user) is checked before any database work. Over the limit, the handler returns 429 and writes nothing.
3. **Admission transaction**, all or nothing:
   - lock the user's metering row (`SELECT … FROM users WHERE id = $u FOR NO KEY UPDATE`) and check the daily cap and the in-flight count (see [Metering](#metering));
   - lock the canvas row (`UPDATE canvases SET updated_at = now() WHERE id = $c AND user_id = $u RETURNING root_node_id`). This is the ownership check, the list-order bump and a row lock in one statement. Sends on one canvas, merge confirms and the first-message `root_node_id` write therefore serialize, and no lock is ever upgraded.
     Locks are always taken in the order user, then canvas, so admission can't deadlock. 40P01/40001 in admission maps to 409 anyway, and a concurrent-first-send test asserts one 201 and one 409 ([F-L3-6](reports/reviews/architecture-L3.md));
   - check canvas ownership, that the parent is `complete`, and the anchor;
   - insert `user_message` (complete), `ai_message` (`streaming`, empty, `owner_key` = this instance), the two edges, and the `llm_usage` reservation.
   Any failed check rolls back, so a capped user gets a clean 429 and no rows.
4. **After commit, spawn a detached stream task** and register it in the stream registry. The HTTP response only *attaches* to the registry entry (see [Stream engine](#stream-engine)). If the client disconnects, the task keeps running and saves the full answer.
5. The response sends SSE `created` with both nodes and edges, then follows the stream.
6. In the task: `context::assemble(ai_node)`. The compaction summary it needs is normally precomputed already (see [Context builder](#context-builder)). Then call the LLM with `max_tokens = REPLY_RESERVE_TOKENS`, append each delta to the buffer, and flush `content` to Postgres every `STREAM_FLUSH_MS` (1 s).
7. **Single writer.** Every flush and the final write is fenced: `UPDATE … WHERE id = $1 AND attempt = $2 AND (status = 'streaming' OR (status = 'error' AND error_code = 'interrupted'))`, and it sets `status` back to `streaming` or `complete`. A retry increments `attempt`, so a superseded task matches no row and stops. A task whose node was swept as interrupted by another instance, but never retried, reclaims it ([F-L3-2](reports/reviews/architecture-L3.md)).
8. **Final write**, one transaction:
   - write the final `content`, `status = complete`, usage, and the node's checkpoint fields;
   - settle the reservation.
   DB errors are classified ([F-L3-1](reports/reviews/architecture-L3.md)):
   - **Transient** errors are retried: connection loss, pool or statement timeout, and SQLSTATE classes `08`, `57P0x`, `53300`, `40001` and `40P01`.
   - **Any other error is permanent.** The task writes `status = error`, `error_code = 'persist_failed'` and the partial text (best effort), settles usage in a separate statement, and publishes `error`.
   - DB errors are logged only as SQLSTATE plus constraint name, never through `Display` of the sqlx error, because Postgres `DETAIL` can carry row content.
   - The app's DB role sets `log_error_verbosity = terse` and `log_min_error_statement = panic`, so the server log omits it too (host support is an open question).
   - The NFR-9 test includes a forced constraint violation ([F-L3-5](reports/reviews/architecture-L3.md)).
   - U+0000, which Postgres `text` rejects, is replaced with U+FFFD in model deltas and in all user input at the boundary, so the obvious permanent failure can't happen (property-tested).
   Transient errors are retried with capped exponential backoff (max 10 s apart) **for as long as the process lives**. The finished text stays in memory, and attached clients see a non-terminal `finalizing` status. `done` is published only after commit. A terminal `error` is never published while the row is still `streaming`. Only process exit can end the retries. The row is then still `streaming` under a dead `owner_key`, so the next sweep marks it `interrupted` with the last flushed text (`final_write_abandoned` logged at shutdown). On a stream error, the task writes the partial text, `status = error` and `error_code` (also retried), settles usage, and publishes `error`.
9. The buffer is capped at `MAX_NODE_CHARS` (100,000 code points). If the cap is hit, or the provider stops with `finish_reason = length` (the `max_tokens` limit), the task saves the text with `status = complete` and `error_code = 'truncated'`. The UI shows "Answer hit the length limit" and a **Continue** quick action, which sends a reply asking the model to continue ([F-L3-7](reports/reviews/architecture-L3.md)).

## Stream engine

**Registry.** `Mutex<HashMap<NodeId, Arc<Stream>>>`. A `Stream` holds `attempt`, `text: RwLock<String>` and a `tokio::sync::watch` of `{len, status}`. The writer appends, then bumps the watch. Entries are removed 30 s after a terminal state.

**Attach.** Both `POST …/messages` and `GET /api/nodes/:id/stream` work the same way. Attach starts with the ownership query (`nodes JOIN canvases WHERE canvases.user_id = $me`, 404 otherwise) **before** any registry lookup, and registry entries also carry `user_id`, which is compared ([F-L3-2](reports/reviews/architecture-L3.md)). Then:
1. Send a `snapshot` event (the node plus all text so far).
2. Loop: on each watch change, send `delta` with the text since the last offset sent.
3. On a terminal status, send exactly one `done` or `error`, then close.
4. An SSE comment (`:\n\n`) is sent every 15 s on every attach, so edge proxies never cut silent phases: first-token wait, an on-demand compaction, or `finalizing`. An integration test holds the fake provider silent for 90 s behind the proxy ([F-L3-6](reports/reviews/architecture-L3.md)).
A slow reader skips intermediate watch values and receives one larger delta. Nothing is dropped and the writer never waits, so terminal events always arrive while a client is connected ([F-L3-9](reports/reviews/architecture-L3.md)). Reloading the page, opening a second tab, or reconnecting after a network drop all just attach again. If the node is not in this instance's registry (for example during deploy overlap), attach falls back to reading the row every `STREAM_FLUSH_MS` until it settles. If the poller sees `attempt` change (a retry), it sends a fresh `snapshot` that replaces the client's text ([F-L3-6](reports/reviews/architecture-L3.md)). All attachers of one node share **one poller** (an in-process map from node to a `watch` fed by a single poll task). A polling attach lives `TASK_DEADLINE_SECS` + 60 s. It never closes silently: at the end it sends a final `snapshot` and a `reattach` event, and the client attaches again. Concurrent attaches are capped at 10 per user (429 beyond).

**Instance liveness, without heartbeats.**
- At boot, each instance picks a random `owner_key` (bigint). It holds `pg_advisory_lock(owner_key)` on a dedicated connection outside the pool for its whole lifetime. Postgres releases a session lock only when that session ends, which means the process died or its connection did.
- **Dead-peer detection is bounded.** On a host loss or partition no FIN/RST arrives, so the lock session sets server-side `tcp_keepalives_idle = 10`, `tcp_keepalives_interval = 5`, `tcp_keepalives_count = 3` and `tcp_user_timeout = 30000` right after connecting. The client socket uses the same keepalives. Postgres then ends a vanished instance's session, and frees its lock, within about 30 s instead of the kernel default of about 2 h ([F-L3-3](reports/reviews/architecture-L3.md)). Whether the managed host honours these settings is an open question below.
- Streaming nodes and open reservations carry `owner_key`.
- The sweeper runs at boot and then every 30 s:
  1. Collect `SELECT owner_key FROM nodes WHERE status = 'streaming' UNION SELECT owner_key FROM llm_usage WHERE settled_at IS NULL`, minus `$me`. This also catches orphaned compaction and merge-proposal reservations, which have no streaming node.
  2. For each key, try `pg_try_advisory_xact_lock(key)` in a transaction. Success proves that owner is gone. In that transaction, mark its streaming nodes `error`/`interrupted` and settle its open reservations from what exists, with `estimated = true`. An answer is charged its locally counted prompt (`assemble(parent)` is deterministic) plus its last flushed content. A compaction is charged its locally counted input. Only a merge proposal, which has no flushed state, is charged its reservation. A crash loop therefore doesn't burn users' caps ([F-L3-9](reports/reviews/architecture-L3.md)).
- An instance never sweeps its own key, so a Postgres outage of any length can't kill a single instance's live streams.
- The instance tracks lock-held state in an `AtomicBool`. While the lock is not held, sends, retries and merge proposals return 503 `unavailable` and `/api/healthz` fails, so it never admits work under a key it doesn't hold ([F-L3-8](reports/reviews/architecture-L3.md)).
- If the lock connection drops, the instance re-acquires the same key on reconnect with a `pg_try_advisory_lock` retry loop (never the blocking form). A sweep by another instance during that gap is undone by the reclaim rule in send step 7. Accepted edge case, which needs both a lock-connection partition and a second live instance, i.e. a deploy overlap:
  - Clients attached on the sweeping instance see `interrupted` and the Retry button. A reload shows the reclaimed answer.
  - Clicking Retry first supersedes it, by design: the user asked for a new answer.
  - Settlement is keyed by reservation id and always overwrites, so the owner's actual usage replaces the sweeper's estimate.
- The lock connection must bypass transaction-mode poolers, so `DATABASE_URL_DIRECT` is **required** in production. After acquiring the lock, startup verifies it is real: `pg_backend_pid()` on the lock connection is stable across two calls, and a pool connection sees the lock granted to that pid in `pg_locks`. Otherwise the process refuses to start ([F-L3-1](reports/reviews/architecture-L3.md)).
- This also replaces the heartbeat column, the 10 s heartbeat writes and the 60 s staleness guess.

**Graceful shutdown (SIGTERM).**
1. `/api/healthz` returns 503 at once, so the platform stops routing new requests here. New sends and retries that still arrive get 503 `unavailable` ([F-L3-9](reports/reviews/architecture-L3.md)).
2. Running tasks get `DRAIN_SECS` to finish. Every LLM task has one deadline, `TASK_DEADLINE_SECS` (240 s), covering the whole task: on-demand compaction, the pre-first-token retry and the answer. Startup checks `DRAIN_SECS ≥ TASK_DEADLINE_SECS + 30` (default 270), so a drain outlasts every task except a final write still retrying through a Postgres outage (see Failure handling) ([F-L3-5](reports/reviews/architecture-L3.md)). The new instance serves new requests meanwhile, because the engine is overlap-safe. So a deploy on push never cuts a normal answer ([F-L3-6](reports/reviews/architecture-L3.md)).
3. Tasks still running are cancelled: stream tasks, compaction precomputes and merge proposals. Each cancelled stream task writes its own `(id, attempt)` as `error`/`interrupted` with its partial text. Every cancelled task settles its own reservation.
4. The process exits and its lock is released.
No broad `WHERE status = 'streaming'` update exists anywhere.

## Context builder (FR-10 – FR-12, NFR-7)

**Phase 1 tokenizer constraint** ([ADR 0005](ADR/0005-llm-gateway-and-tokenizer.md)). All three models (main, doubt, summary) must have a local tokenizer: OpenAI models through `tiktoken-rs`, or open-weight models through their published `tokenizer.json` (`tokenizers` crate). Counts are then exact up to provider chat framing, so NFR-6 holds as written and every reservation is computable. Models without a local tokenizer (e.g. Anthropic) wait for a later phase.

**Tokenizer ratios.** For each configured model, a fixture test over English, code, CJK and emoji corpora measures both directions against `o200k_base`:
- `r_up` = max(mode / canonical);
- `r_down` = max(canonical / mode).
They live in `tokenizers.toml`, a config file written by `api measure-ratio <model>`. Switching to a measured model is therefore config-only (NFR-13); a new model is measured once first. Startup refuses unmeasured models and checks every inequality below ([F-L3-8](reports/reviews/architecture-L3.md)).

**Path message cap.** Every path message enters prompts middle-truncated to at most `PATH_MSG_MAX_TOKENS` (2,500 canonical). Startup checks `PATH_MSG_MAX_TOKENS ≥ REPLY_RESERVE_TOKENS × r_down`, so a full-length AI reply is never truncated. It bounds 8,000-char user messages in dense scripts.

**Checkpoints (deterministic, prefix-only).** When a node becomes `complete`, three fields are stored:
- `ctok`: the node's tokens, as truncated above, under the canonical tokenizer (`o200k_base` from `tiktoken-rs`, pinned), plus `FRAME_TOKENS` (8) for per-message chat framing and separators. Many tiny messages therefore can't hide unbounded framing inside a block;
- `block_acc = parent.block_acc + ctok`;
- `is_checkpoint = block_acc >= BLOCK_TOKENS` (2,000); a checkpoint resets `block_acc` to 0.
So a block holds at most `BLOCK_TOKENS − 1 + PATH_MSG_MAX_TOKENS + FRAME_TOKENS` (4,507) canonical tokens, framing included.

`BLOCK_TOKENS`, `PATH_MSG_MAX_TOKENS` and the canonical tokenizer are **compile-time constants**, not env config. Changing one needs a migration that recomputes the checkpoint fields and clears cached summaries ([F-L3-9](reports/reviews/architecture-L3.md)).

These values depend only on the node's ancestors. A node only gets children once it is complete, and error nodes never have children. So every descendant, on every branch, sees the same checkpoints.

**Summary chain.** For checkpoint k with previous checkpoint p, `S(k) = summarize(S(p) + messages after p up to and including k)`. The input is bounded (≤ `COMPACTION_SUMMARY_MAX_TOKENS` + `BLOCK_TOKENS` + one middle-truncated message), so it never approaches the summary model's window. `S(k)` is cached on node k with `UPDATE … SET compaction_summary = $s WHERE id = $k AND compaction_summary IS NULL`; a lost race re-reads the stored value. It is valid forever and shared by every branch below k.

**Precompute, off the critical path.** When a node completes as a checkpoint and its path holds ≥ 50% of the path budget in raw tokens, the scheduler computes the missing `S(·)` in the background:
- in order, through a per-user round-robin queue served by 2 workers, so one heavy user can't starve the others;
- metered as a `compaction` reservation, and skipped if the user is over the cap.
Once the process has been up 5 min, the scheduler re-enqueues `is_checkpoint AND compaction_summary IS NULL` checkpoints on canvases active in the last 24 h (one indexed query). A crash loop never reaches that point, so it can't burn caps, so a deploy doesn't push the chain onto the next send's critical path. An on-demand chain inside a send is bounded to 2 sequential summary calls; beyond that it uses drop-oldest for that send and leaves the rest to the queue ([F-L3-9](reports/reviews/architecture-L3.md)). Compaction reservations count toward the daily cap but not the user's in-flight limit, so system work never 429s a user's send ([F-L3-6](reports/reviews/architecture-L3.md)). An on-demand compaction inside a send takes its own `compaction` reservation in its own admission. If the cap refuses it, the send falls back to drop-oldest.
By the time a send needs `S(k)`, it already exists. The scheduler is in-process and not durable. A summary lost to a restart is computed on demand by the next send, with a 30 s timeout and a `compaction_on_critical_path` log. If that fails, the send drops the oldest messages instead.

**Assembly** `assemble(node, mode, new_message?) -> (messages, tokens, estimated)`:

1. **Path**: for a send, one recursive CTE walks up from X through `edges.target_id` and returns the path nodes, root first, with their parent edges and checkpoint fields. Canvas load issues **no** per-node CTE: it fetches all `nodes` + `edges` once and derives every path in memory.
2. **Mode**: `doubt` if any edge on the path is a `branch`, else `main` (AC-10.3). The mode picks the system prompt, the model and the tokenizer.
3. **Budget.** Everything is in the mode tokenizer, and every fixed part includes its own wrapper tags and framing:

   | Part | Allotment |
   | --- | --- |
   | System prompt | 800 |
   | Anchor | 1,500 |
   | New message | 3,000 |
   | Merged summaries (per path) | `MERGE_BUDGET_TOKENS` (2,000) |
   | Compaction summary `S` | 1,000 |

   Slots measured in another tokenizer are sized so they always fit; startup checks both inequalities ([F-L3-3](reports/reviews/architecture-L3.md)):
   - `COMPACTION_SUMMARY_MAX_TOKENS` (the summary model's output limit) is 700, and `700 × r(summary → mode) + wrapper tags ≤ 1,000`.
   - Merged summaries are budgeted in canonical tokens (`merge_ctok` per edge, framing included) against `MERGE_BUDGET_CANON` (1,550), and `1,550 × r_up + tags ≤ 2,000`.
   So S and merged summaries are never truncated, and a message never sits in the wrong-tokenizer slot.
   | **Path tail** | **14,000 − 8,300 = 5,700** |

   `S` lives in the fixed part, never in the tail. An oversized piece is middle-truncated with a `[… N tokens omitted …]` marker.
   **Fit guarantee**: the tail after any cutoff holds at most one block plus the following messages up to X, and that is bounded by the same block bound. The single startup-checked inequality is:
   `(BLOCK_TOKENS − 1 + PATH_MSG_MAX_TOKENS + FRAME_TOKENS) × r_up ≤ tail allotment`, i.e. `4,507 × r ≤ 5,700`, which holds for any `r ≤ 1.26`.
   Here `r` is the mode model's `r_up` (1.0 when the model uses `o200k_base` itself). The server refuses to start if the inequality fails for the configured models.
   So when the parent X has just closed a block, j = m − 1 still fits, and **X is always in the verbatim tail** (AC-11.1).
   **Runtime backstop**: at send time only, the final prompt is re-tokenized once in the mode tokenizer as a guard. If it is over budget anyway (a broken assumption), the oldest tail messages other than X are middle-truncated, then dropped, until it fits, and `budget_enforced` is logged. NFR-7 therefore holds unconditionally ([F-L3-1](reports/reviews/architecture-L3.md)).
4. **Cutoff with built-in hysteresis.** Let ck₁…ckₘ be the checkpoints on the path, with ck₀ meaning "no compaction". Pick the smallest j such that `S(ckⱼ)` plus the messages after ckⱼ fit the tail allotment. The new message is reserved at its full 3,000 allotment whatever its real size. So j doesn't depend on what the user types, and the counter and the send pick the same j. j changes only when the path crosses a checkpoint, at most once per ~2,500 tokens of conversation, never on every send ([F-L3-1](reports/reviews/architecture-L3.md)).
5. **Layout, ordered for prefix caching:**
   - the system prompt;
   - if j > 0, `<conversation_summary>S(ckⱼ)</conversation_summary>`, followed by a `<merged_branches>` block holding the merged summaries whose origin lies in root..ckⱼ (ordered by origin depth, then `merged_at`);
   - the tail messages verbatim; a merged summary whose origin is in the tail is appended to that origin's content in a `<merged_branch>` block.
   A merged summary is included only if its branch edge has `source_id` on the path and `target_id` **not** on the path. Continuing inside a merged branch therefore never sees that branch's own summary ([F-L3-2](reports/reviews/architecture-L3.md)). Summaries of other merged branches from an origin on the path *are* included, branch prompts too: FR-14 says every path through the origin gets them. Sibling *messages* never are;
   - the new message, wrapped by the quick-action template; when the send creates a branch, the template also holds that branch's anchor (AC-7.1, AC-8.x).
   Roles strictly alternate user/assistant. Adjacent same-role items are concatenated into one turn with a fixed `\n\n---\n\n` separator; this covers a summary before a user message and a branch from a `user_message` (U1 →branch→ U2, AC-5.1). The merge is deterministic, so it stays byte-stable ([F-L3-10](reports/reviews/architecture-L3.md)). Two consecutive sends on the same path share every byte up to the new message (AC-10.4). Sibling branches share every byte up to their fork. Explicit Anthropic `cache_control` breakpoints go after the summary block and at the end of the tail, if the gateway exposes them (spike); OpenAI-style providers cache stable prefixes automatically. The provider's `cached_tokens` is recorded.
6. **Anchor** (branches).
   - **History is re-rendered exactly as it was sent.** A branch's first user message was sent as template + anchor + question. It is rendered the same deterministic way wherever it appears in later prompts, at its branch point. Every anchor on a nested path (AC-9.1, AC-10.2) therefore sits where the model originally saw it, and prefixes stay byte-stable across the branch ([F-L3-7](reports/reviews/architecture-L3.md)).
   - That rendered form is the node's content for budgeting: it is counted in `ctok` and capped by `PATH_MSG_MAX_TOKENS` like any path message, so the block bounds hold.
   - The anchor slot (1,500) is used only by a send that creates a branch.
   - **Truncation never cuts the selection.** The selected text is validated at ≤ 1,000 tokens (422 `invalid_anchor` otherwise). Only the surrounding sentence or the rest of the code block is trimmed to fit the slot, keeping the lines nearest the selection ([F-L3-2](reports/reviews/architecture-L3.md)).
   - Anchor rendering: for `text`, for `text`, the highlight plus its surrounding sentence, or for long highlights the text marked "the part being asked about"; for `code`, the selected lines marked plus the rest of the block; nothing extra for `none`.
7. **Merged summaries are never dropped.** The merged-summary tokens on any root-to-leaf path are capped at `MERGE_BUDGET_TOKENS` (2,000).
   - The merge confirm checks it under the canvas row lock with two CTEs: the merge tokens on the origin's ancestors, plus the largest running sum down the origin's subtree.
   - Only paths through the origin can change, so merges elsewhere on the canvas don't count against it ([F-L3-2](reports/reviews/architecture-L3.md)).
   - A merge that would exceed the cap returns 422 `merge_budget_exhausted`, and the UI says so.
   Every accepted summary therefore reaches every downstream prompt (AC-14.1).

**Token counter** (FR-12, AC-12.1): `context_tokens(X) = assemble(X, mode, None).tokens`, the exact same function. It does not use the prefix-sum trick.
- `assemble` reports `tokens` as the sum of its parts' stored per-part counts plus a fixed per-message framing constant.
- The send-time guard's full recount is a check, not a second definition. Property tests assert that sum-of-parts equals the full recount for every generated prompt, and a mismatch in production is logged as `count_drift`.
- Per-message token counts (after path truncation) are **persisted** on the node in `tok jsonb`, keyed by tokenizer id. They are filled when the node completes, for the tokenizers of both modes, Any id still missing (after a model change) is computed in memory on read and backfilled by a background task, never by the `GET`. A model change adds a new key, and no stale count is ever read. Canvas load therefore never re-tokenizes content, even cold after a restart ([F-L3-7](reports/reviews/architecture-L3.md)).
- `assemble` is split into `plan(X) -> parts` (cutoff, slots, truncation decisions, all from stored counts) and `render(parts) -> messages`. A send runs both. The counter runs `plan` only: no string building, no tokenization. Because of step 4, each node's plan touches only its tail (a few dozen stored numbers), so all 1,000 counters are additions over one in-memory graph ([F-L3-5](reports/reviews/architecture-L3.md)). It runs in `spawn_blocking`. The NFR-3 benchmark uses a cold process and maximum-size nodes.
- If `S(ckⱼ)` isn't computed yet, its allotment (1,000) is used and the count is marked `estimated`.
- A merge returns updated counts for the affected nodes, and `done` carries the new node's count. Canvas load never calls a provider.
- Main and doubt models have local tokenizers (constraint above), so counts match the provider's up to chat framing, checked against reported usage (AC-12.2, ±5%, NFR-6).

NFR-1 (path fetch + assembly < 200 ms p95 at 200-deep paths) is expected to hold with the CTE over `edges(target_id)` (**assumption**, seeded benchmark; `ltree` if not).

## Metering

`llm_usage` is a ledger of reservations. Every LLM call (answer, retry, compaction, merge proposal) is admitted by one transaction:

```sql
SELECT 1 FROM users WHERE id = $u FOR NO KEY UPDATE;     -- serializes metering, doesn't block FK inserts
SELECT coalesce(sum(CASE WHEN settled_at IS NULL THEN reserved_tokens
                         ELSE coalesce(prompt_tokens, 0) + coalesce(completion_tokens, 0) END), 0) AS spent_today,
       count(*) FILTER (WHERE settled_at IS NULL AND kind <> 'compaction') AS in_flight
FROM llm_usage WHERE user_id = $u AND (created_at >= date_trunc('day', now()) OR settled_at IS NULL);
-- reject (429, rollback) if spent_today + $reserve > DAILY_TOKEN_CAP or in_flight >= 3 (user-initiated kinds only)
INSERT INTO llm_usage (…, reserved_tokens, owner_key) VALUES (…);
```

- Every pool connection and the lock connection set `TimeZone = 'UTC'` in `after_connect`, so `date_trunc('day', now())` is the UTC day on every connection ([F-L3-11](reports/reviews/architecture-L3.md)).
- The reservation is an upper bound on the call:
  - an answer reserves input ≤ 14,000 (enforced by the runtime backstop) plus output ≤ `max_tokens` 2,000, plus a 5% provider-framing margin: 16,800;
  - a compaction reserves its bounded input, `(COMPACTION_SUMMARY_MAX_TOKENS + BLOCK_TOKENS + PATH_MSG_MAX_TOKENS + FRAME_TOKENS) × r_up` of the summary model, plus its output and the same margin (about 6,800);
  - a merge proposal reserves its 4,000 input plus 100 output, plus the margin.
  Startup checks that every reservation is ≤ `DAILY_TOKEN_CAP` / 10. Near the cap, a compaction can still be refused; the send then uses drop-oldest (`compaction_fallback`). A test puts a user one answer below the cap and asserts the answer is still sent ([F-L3-11](reports/reviews/architecture-L3.md)).
- Each provider attempt has its own reservation, including the automatic pre-first-token retry. The cap holds up to provider framing error beyond the 5% margin (NFR-6 bounds it at ±5%).
- A 4th concurrent call is rejected with 429, not queued ([F-L3-5](reports/reviews/architecture-L3.md)).
- Settlement writes provider usage when the provider reports it. Otherwise it writes the locally counted prompt plus locally counted streamed completion, with `estimated = true`. That covers broken streams, timeouts and failed compaction calls, so every billed call leaves a settled row ([F-L3-4](reports/reviews/architecture-L3.md)).
- An unsettled reservation from a dead instance is settled by the sweeper from existing data (Stream engine). Settlement is `UPDATE … WHERE id = $reservation_id` and always overwrites, so a late owner's real usage replaces that estimate.
- The day is the UTC day.

## Data model

Postgres 16. All ids are UUID v7. Text enums use `CHECK` constraints. The brief's `node_embeddings`, `memories`, `composer_runs` and `artifacts` tables are phase 2/3 and not created.

| Table | Columns | Constraints / indexes |
| --- | --- | --- |
| `users` | `id`, `email citext`, `password_hash`, `created_at` | `UNIQUE(email)`. The row doubles as the per-user metering lock |
| `sessions` | `token_hash bytea` (SHA-256 of the cookie token), `user_id`, `expires_at`, `created_at` | PK `token_hash`; FK `user_id → users ON DELETE CASCADE`; index `user_id` |
| `canvases` | `id`, `user_id`, `title`, `root_node_id` (nullable), `created_at`, `updated_at` | FK `user_id → users ON DELETE CASCADE`; index `(user_id, updated_at DESC)`. The first message sets `root_node_id` with `WHERE root_node_id IS NULL`; no row updated → 409. Merges lock the canvas row to check the per-path merge budget |
| `nodes` | `id`, `canvas_id`, `type` (`user_message`/`ai_message`), `content text`, `status` (`streaming`/`complete`/`error`), `error_code` ("interrupted" in this doc means `status = error, error_code = 'interrupted'`), `attempt int`, `rev int`, `owner_key bigint` (instance streaming it), `model`, `prompt_tokens`, `completion_tokens`, `cached_tokens`, `ctok int`, `tok jsonb` (per-tokenizer counts), `block_acc int`, `is_checkpoint bool`, `compaction_summary`, `compaction_tokens int`, `position_x`, `position_y`, `created_at`, `updated_at` | FK `canvas_id → canvases ON DELETE CASCADE`; `UNIQUE(canvas_id, id)`; index `canvas_id`; partial index `(owner_key) WHERE status = 'streaming'` (sweeper); `CHECK (char_length(content) <= 100000)`; `CHECK (compaction_summary IS NULL OR is_checkpoint)` |
| `edges` | `id`, `canvas_id`, `source_id`, `target_id`, `type` (`reply`/`branch`), `anchor jsonb`, `doubt_type` (`simpler`/`example`/`why`/`deeper`/`wrong`/`free`), `merge_summary`, `merge_ctok int`, `merged_at`, `merged_rev bigint`, `last_proposal`, `collapsed bool`, `created_at` | Composite FKs `(canvas_id, source_id)` and `(canvas_id, target_id) → nodes(canvas_id, id) ON DELETE CASCADE`; `UNIQUE(target_id)` (tree, path-walk index); partial `UNIQUE(source_id) WHERE type = 'reply'`; index `source_id`; index `canvas_id`; `CHECK ((type = 'branch') = (anchor IS NOT NULL AND doubt_type IS NOT NULL))`; `CHECK (type = 'branch' OR (merge_summary IS NULL AND NOT collapsed))` |
| `llm_usage` | `id bigserial`, `user_id` (plain UUID, **no FK**: usage counts hold no content and are kept as pseudonymous cost records after account deletion, as the privacy notice states; with invite-only accounts, deleting an account can't be used to reset the cap), `node_id` (nullable, **no FK**: the ledger outlives canvas deletion, so deleting a canvas never refunds the daily cap or erases cost records; [F-L3-5](reports/reviews/architecture-L3.md)), `kind` (`answer`/`compaction`/`merge_proposal`), `model`, `owner_key`, `reserved_tokens`, `prompt_tokens`, `completion_tokens`, `cached_tokens`, `estimated bool`, `created_at`, `settled_at` | Index `(user_id, created_at)`; partial index `(user_id) WHERE settled_at IS NULL`; `CHECK (settled_at IS NULL OR (prompt_tokens IS NOT NULL AND completion_tokens IS NOT NULL))`. Settlement writes 0, never NULL, for an absent count, and the cap SQL also uses `coalesce` ([F-L3-6](reports/reviews/architecture-L3.md)) |
| `tombstones` | `kind` (`user_deleted`/`canvas_deleted`/`password_reset`), `id`, `at` | No FK. Restore runbook input (see Deployment) |
| `invites` | `token_hash bytea`, `user_id`, `expires_at`, `used_at` | PK `token_hash`; FK `user_id → users ON DELETE CASCADE` |
| `events` | `id bigserial`, `user_id`, `canvas_id`, `kind` (`branch_created`/`merge_confirmed`/`merge_updated`/`back_to_main`/`collapse`), `payload jsonb`, `created_at` | FKs `ON DELETE CASCADE`; index `(user_id, created_at)`. Payload is a typed Rust enum per `kind`; never message text except proposed vs. confirmed merge summaries |

Choices behind the model:

- **Branch state lives on the branch edge** (`merge_summary`, `collapsed`). An origin can have several branches, each merged or collapsed separately. The origin in "stored against the origin node" (AC-13.2) is the edge's `source_id`.
- **Content is immutable once a node is `complete`.** Only `streaming`/`error` AI nodes change content. An `error` node has no children, because sends need a complete parent. So checkpoint fields, summaries and anchors are never invalidated.
- **Merges are snapshots** ([F-L3-7](reports/reviews/architecture-L3.md)). A merged branch can still be continued. Downstream prompts keep the confirmed summary. Every node has `rev int`, incremented by every terminal content write: complete, error, reclaim, retry. The edge stores `merged_rev = count(subtree nodes) + sum(subtree rev)`, computed while the merge holds the canvas row lock. Sends **and retries** take the same lock. Both terms only grow, so `stale = current rev > merged_rev` is exact, and it also catches retried or reclaimed answers inside the branch ([F-L3-2](reports/reviews/architecture-L3.md)). The UI offers "branch changed since merge · update summary". Re-merging replaces the summary and writes `merge_updated`, not a second `merge_confirmed`, so the metrics don't double-count. The same summary again is a no-op. Merge proposal and merge confirm return 422 `branch_streaming` while any node in the branch subtree is streaming.
- **Anchor JSON** (brief shape, `kind` discriminated; the Rust enum is the source for TS via ts-rs): `{"kind":"none"}` · `{"kind":"text","start":412,"end":540,"text":"…"}` · `{"kind":"code","start":900,"end":1340,"start_line":5,"end_line":9,"text":"…"}`.
  - `start`/`end` are UTF-16 code-unit offsets into the raw markdown `content`, matching JS string indexing. For a `code` anchor they cover the whole code block, and `text` is the selected lines.
  - The server never slices Rust strings by those offsets. It converts with `encode_utf16()`, checks `start <= end <= len`, and compares `String::from_utf16(&u[start..end])` with `text` (AC-6.1). A `code` slice must also start with a fence line, and lines `start_line..=end_line` must equal `text`. Any failure → 422 `invalid_anchor`.
  - Mapping a DOM selection back to source offsets uses the renderer's source positions (**assumption**, spike).
- **Ownership**: every query joins through `canvases.user_id = $current_user`.
- **Deletes** cascade from `users` and `canvases` (AC-2.3, AC-20.1). A stream task whose node is deleted matches no row and stops.
- **Every reservation settles exactly once, whatever happens** ([F-L3-1](reports/reviews/architecture-L3.md)):
  - Each LLM task owns a `Reservation` guard. Every exit path settles it from the task's in-memory counts: success, error, a fenced write that matches no row (deleted canvas, superseded retry), cancellation and panic. On a panic or drop, the guard spawns the settle.
  - If the owner is dead and the node or parent is gone, the sweeper settles at the reserved amount.
  - Safety net: a reservation unsettled after 15 min (startup checks this is > `TASK_DEADLINE_SECS`) is settled at its reserved amount only if no live guard holds it. That means another key whose owner lock is acquirable (dead), or this instance's own key with no guard in its registry (orphaned). A final write still retrying through an outage keeps its guard, so it is never settled underneath ([F-L3-2](reports/reviews/architecture-L3.md)). `in_flight` can therefore never leak.

**Migrations**:
- SQLx migrations are embedded with `sqlx::migrate!` and run at startup under SQLx's advisory lock, over `DATABASE_URL_DIRECT`. A session lock needs a non-pooled connection ([F-L3-13](reports/reviews/architecture-L3.md)).
- `set_ignore_missing(true)`, so a rolled-back binary boots on a newer schema.
- Forward-only, using expand → deploy → contract.
- `.sqlx/` offline data is committed.

## API

JSON over HTTPS under `/api`, same origin as the web app. Auth is an `HttpOnly; Secure; SameSite=Strict` session cookie. Every endpoint except accept-invite/login/livez/healthz/meta needs a session (401 otherwise, AC-1.2). Any canvas, node or edge not owned by the caller returns **404**, never 403 (NFR-8).

**Error format** (all non-2xx and the SSE `error` event):

```json
{ "error": { "code": "not_found", "message": "Canvas not found" } }
```

Codes:
- 400 `bad_request`
- 401 `unauthorized`
- 403 `forbidden_origin`
- 404 `not_found`
- 409 `conflict`
- 422 `message_too_long`, `invalid_anchor`, `parent_not_complete`, `branch_streaming`, `merge_budget_exhausted`
- 429 `rate_limited`, `daily_cap_reached` (both with `Retry-After`)
- 500 `internal` (caught panic, non-connection DB error)
- 502 `llm_unavailable`
- 503 `unavailable`

Unknown routes and wrong methods return 404 `not_found` in this shape.

| Method & path | Request | Response |
| --- | --- | --- |
| `POST /api/auth/accept-invite` | `{token, password}` (password 12–128 chars; longer → 400 before any hashing) | 201 `{user}` + cookie. An invalid, used or expired token → 401, with no account and no hashing |
| `POST /api/auth/login` | `{email, password}` | 200 `{user}` + cookie; 401 on bad credentials (same message and same argon2 work for unknown email) |
| `POST /api/auth/logout` | – | 204, session row deleted |
| `GET /api/me` | – | `{id, email}` |
| `DELETE /api/me` | `{password}` (re-auth) | 204; cascades everything, writes tombstones, cookie cleared (FR-20). The re-auth goes through the argon2 semaphore and the global auth bucket, limited to 5/h per user. A 5th wrong password revokes all the user's sessions, so a stolen cookie can't be used to guess passwords ([F-L3-4](reports/reviews/architecture-L3.md)) |
| `GET /api/meta` | – | `{providers: ["…"], context_budget}` for the privacy notice (FR-21) |
| `GET /api/canvases` | – | `[{id, title, updated_at}]` newest first. No pagination in phase 1 (**assumption**: tens of canvases per user) |
| `POST /api/canvases` | `{id?, title?}` | 201 canvas |
| `GET /api/canvases/:id` | – | `{canvas, nodes: [Node & {context_tokens, estimated}], edges: [Edge & {stale}]}`, compressed |
| `PATCH /api/canvases/:id` | `{title}` | 200 canvas |
| `DELETE /api/canvases/:id` | – | 204; cascades; writes a `canvas` tombstone (table + log line) |
| `POST /api/canvases/:id/messages` | `{user_node_id, ai_node_id, parent_id?, edge: "reply" \| "branch", anchor?, doubt_type?, content}` | `text/event-stream`: `created` {nodes, edges} → `snapshot` → `delta`* → `done` {node, context_tokens} or `error` {error, node} |
| `GET /api/nodes/:id/stream` | – | `text/event-stream`: `snapshot` {node} → `delta`* → `done`/`error`. For a settled node: `snapshot` then the terminal event. Used on reload, second tab and reconnect |
| `POST /api/nodes/:id/retry` | – | Rate limit, then one transaction: admission (user lock, then canvas lock, as in a send) + compare-and-set `UPDATE nodes SET status = 'streaming', content = '', attempt = attempt + 1, owner_key = $me WHERE id = $1 AND status = 'error' AND <owned> RETURNING attempt`. No row → 409 and nothing written, including the reservation and the partial text (AC-18.1). Then the same stream as a send |
| `PUT /api/canvases/:id/positions` | `[{id, x, y}]` (≤ 2,000) | 204. One `UPDATE … FROM unnest(…)` scoped to the canvas after the ownership check. Duplicate ids → 400; row count ≠ ids → 404, rolled back |
| `POST /api/edges/:id/merge-proposal` | – | `{summary}` (≤ 200 chars), also stored as `edges.last_proposal` for the metrics. Admitted and metered like a send. The input is the anchor, then the branch's main line (from the branch's first node along `reply` edges to its end), with nested merged summaries inline. Messages are middle-truncated to `PATH_MSG_MAX_TOKENS`, and the oldest are dropped to fit `MERGE_PROPOSAL_INPUT_TOKENS` (4,000). The reservation is 4,000 + 100 output. A property test keeps the input within the cap ([F-L3-8](reports/reviews/architecture-L3.md)). 422 `branch_streaming`; 502 if the LLM fails (the UI lets the user type it) |
| `PUT /api/edges/:id/merge` | `{summary}` (1–200 chars) | 200 edge (`collapsed = true`, AC-13.2) plus updated `context_tokens` for affected nodes. The proposal is taken from `edges.last_proposal`, which the merge-proposal endpoint stores server-side, so the "merge edits" metric never trusts client text ([F-L3-12](reports/reviews/architecture-L3.md)). Same summary → no-op 200. 404 unless a `branch`; 422 `branch_streaming` / `merge_budget_exhausted` |
| `PATCH /api/edges/:id` | `{collapsed}` | 200 edge. 404 unless a `branch` |
| `POST /api/events` | `{canvas_id, kind, payload}`, UI-only kinds (`back_to_main`, `collapse`) | 204. Ownership is checked by joining `canvases.user_id = $me`; otherwise 404, and it is in the NFR-8 per-endpoint tests. Rate limit 60/min per user; payload ≤ 4 KB and typed per kind (422 otherwise). Server-side kinds are never accepted from the client ([F-L3-4](reports/reviews/architecture-L3.md)) |
| `GET /api/livez` | – | 200 while the process runs |
| `GET /api/healthz` | – | 200 `{db: "ok"}` / 503 |

Rules:

- **Length unit**: every length limit counts Unicode code points (`chars().count()`, `char_length`).
- **Message validation**:
  - `content` is 1–8,000 chars **and** ≤ 3,000 tokens in the mode tokenizer minus the largest quick-action template overhead plus framing for that mode (computed at startup), so the wrapped message always fits its slot ([F-L3-1](reports/reviews/architecture-L3.md)). Over that → 422 `message_too_long` with the count, never silent truncation; the composer shows a live count ([F-L3-9](reports/reviews/architecture-L3.md)).
  - `parent_id` may be absent only for the first question, enforced through `canvases.root_node_id`.
  - The parent must be `complete` (AC-6.2).
  - A `reply` parent has at most one reply child (partial unique → 409).
  - `branch` needs an `anchor` and a `doubt_type`; `text`/`code` anchors only on `ai_message` parents.
- **Idempotency**: retried writes carry client-generated ids. A repeat insert hits the PK → 409, and the client reloads. The server rejects ids that aren't UUID v7, so every id carries 74 random bits. A 409 on another tenant's id could only confirm an id the caller already holds; that is an accepted risk ([F-L3-11](reports/reviews/architecture-L3.md)).
- **Rate limits** (in-process token buckets, checked before any DB write):
  - 20 LLM calls/min per user;
  - auth endpoints 10/min per client IP;
  - progressive delays on failed logins per (email, IP), and per email across IPs only after 20 failures/h, capped at 2 s. These delay but never lock out.
  A successful login sets a signed, HttpOnly `device` cookie bound to that email. A login presenting it skips **only** the per-email delay, so attackers can't deny service to a returning user (OWASP device-cookie pattern). It stays inside the global auth bucket and the argon2 semaphore, and has its own bucket of 5/min per cookie id, so a replayed cookie can't reopen the CPU-flood path.
  The client IP comes only from `TRUSTED_IP_HEADER`, which must name a single-value header that the platform **overwrites** on every request (`Fly-Client-IP`, `CF-Connecting-IP`). Production refuses to start without it. `X-Forwarded-For` is accepted only together with `TRUSTED_PROXY_HOPS`, and is then parsed from the **right** (the entry the platform appended), never the client-seeded left. This keeps Render and Railway, which append XFF, usable ([F-L3-9](reports/reviews/architecture-L3.md)). A test asserts that a client-supplied value of that header, sent through the platform, doesn't change the limiter key ([F-L3-10](reports/reviews/architecture-L3.md)). Argon2 runs in `spawn_blocking` behind a semaphore of 2, with a bounded wait (`try_acquire` with a 2 s timeout, then 503 `unavailable` + `Retry-After`). A global auth bucket of 120/min across all IPs bounds total argon2 work, so a distributed flood degrades into fast 503s instead of an unbounded queue ([F-L3-4](reports/reviews/architecture-L3.md)). Limiter maps expire idle keys and are bounded.
- **Daily token cap and concurrency**: the [Metering](#metering) ledger, exact.
- **Panics**: `CatchPanicLayer` → 500. The panic hook logs only the location, thread name and `request_id`. It is process-wide, so background-task panics also lose their message and backtrace: an accepted NFR-9 trade-off. `CatchPanicLayer` doesn't cover panics while a streaming (SSE) body is polled; stream producers must run in a guarded task. `JsonRejection` → generic `bad_request` that echoes no input (NFR-9 test).
- **DB errors**: pool timeout/closed, I/O and TLS → 503 `unavailable`; `RowNotFound` → 404; anything else → 500 `internal`. Only the variant (`db_error`) and SQLSTATE are logged, never the driver message.
- **`APP_ORIGIN`** is normalized at startup to what browsers send (lowercase, default port dropped); a path, query, fragment or userinfo stops startup.
- **CSRF** ([F-L3-10](reports/reviews/architecture-L3.md)):
  - every non-GET/HEAD request must carry an `Origin` header equal to the app origin; a missing or different Origin → 403 `forbidden_origin`;
  - the cookie is `SameSite=Strict`;
  - there is no Content-Type rule, so bodyless mutations (logout, retry) are covered by the same check. JSON is parsed only where an endpoint defines a body.

## Auth

[ADR 0004](ADR/0004-password-auth-postgres-sessions.md). Email + password, hashed with argon2id (`argon2` crate defaults).
- On login the server creates a 32-byte random token, stores its SHA-256 in `sessions`, and sets the token as a 30-day cookie.
- Lookups use `WHERE token_hash = $1 AND expires_at > now()`. The sweeper deletes expired rows.
- Logout and account deletion remove the rows immediately (AC-20.1).

**Invite-only accounts; no self-signup in phase 1.** Phase 1 is a moderated test with a handful of learners. Self-signup, even behind an allowlist, lets anyone who knows an allowlisted address claim it first, and proving email ownership needs an email provider.
1. The operator runs `api invite <email>`. It creates the `users` row with no password, and prints a single-use link carrying a 32-byte token in the URL **fragment** (`/invite#t=…`). Fragments are never sent in request lines, so the token can't reach proxy or access logs; the page posts it in the body ([F-L3-8](reports/reviews/architecture-L3.md)). Only the token's SHA-256 is stored, in `invites(token_hash, user_id, expires_at, used_at)`, with a 7-day expiry.
2. The operator sends the link to the learner out of band.
3. `POST /api/auth/accept-invite` claims the token atomically with `UPDATE invites SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING user_id`, so two concurrent accepts can't both win. In the same transaction it sets the password and deletes **all** of that user's other invites ([F-L3-1](reports/reviews/architecture-L3.md)).
There is no public signup endpoint, so there is nothing to enumerate.
- Login verifies against a fixed dummy hash for unknown emails and for users who have no password yet.
- `api reset-password <email>` deletes that user's existing invites and sessions, clears the password hash, writes a `password_reset` tombstone, then issues a new invite link. A leaked earlier link is dead at once.
- `api invite` also deletes any earlier unused invites for that user.
- Opening signup to the public needs an email provider and verification, which is a later phase ([ADR 0004](ADR/0004-password-auth-postgres-sessions.md)).

Not in phase 1: self-signup, email password reset, magic links, OAuth. Single role; no sharing.

## Security and data protection

- **Secrets** come only from environment variables. They are never sent to the client and never logged (NFR-9). `.env` is local-only and git-ignored.
- **Input validation** at the boundary: U+0000 replaced with U+FFFD, serde types with length limits, anchor checks, UUID path params, a 1 MB body limit. Markdown is rendered without raw HTML (no `rehype-raw`).
- **PII**: emails and message content. Content goes only to the configured LLM providers named in the privacy notice (FR-21), never to logs (NFR-9 test). Deleted accounts remain in backups for up to 7 days; the notice says so.
- **Tombstones**. Account deletions, canvas deletions and password resets record `(kind, id)` (UUIDs, no other PII) in two places that fail independently:
  1. the `tombstones` table;
  2. a structured log line `event=tombstone kind=… id=…`, shipped to the host's log drain with retention ≥ 8 days, longer than backup retention.
  A database loss can't erase the second copy. The restore runbook under Deployment uses both.
- **Headers**: HSTS, `nosniff`, `Referrer-Policy: same-origin`, and `Content-Security-Policy: default-src 'self'; script-src 'self' <hashes>; style-src 'self' 'unsafe-inline'`. React Flow positions nodes with inline styles; inline *styles* can't run script, and this is recorded as an accepted risk ([F-L3-8](reports/reviews/architecture-L3.md)). The build hashes every inline script in `apps/web/out/*.html` into `csp-hashes.json`, and the static handler serves the matching hashes per page (**assumption**, first web build). If hashing proves unworkable, `'unsafe-inline'` is recorded as an accepted risk in an ADR.

## Failure handling

| Failure | Behaviour |
| --- | --- |
| LLM connect error, 429 or 5xx **before the first token** | One automatic retry after 1–2 s (jittered), with its own reservation (skipped if the cap refuses it). Then `status = error` and SSE `error` `llm_unavailable`; the UI shows "Answer interrupted" + Retry |
| LLM stream breaks **after tokens** | No automatic retry. Partial text saved, `status = error`, usage settled (estimated if needed); Retry replaces the answer (AC-18.1) |
| LLM timeouts | Connect 10 s; first token 30 s; gap between chunks 30 s; whole task `TASK_DEADLINE_SECS` (240 s, includes compaction and the retry). All config. Each ends like the row above |
| Compaction summary missing at send | Normally precomputed. Otherwise computed on demand (30 s timeout, `compaction_on_critical_path` log), then drop-oldest fallback (`compaction_fallback` log). Budget still holds |
| Merge proposal fails | 502; the user types the summary |
| Client disconnects mid-stream | Task finishes and saves. Reopening attaches via `GET /api/nodes/:id/stream`: snapshot, then live deltas |
| Postgres outage of any length mid-stream or after the stream | The task keeps streaming to attached clients from memory. Flushes fail and are skipped. The final write retries until it lands (`finalizing` shown meanwhile). A single instance never sweeps itself; another instance's sweep is reclaimed by the fenced final write. Tests: pause Postgres 70 s mid-stream, and 5 min after stream end; resume; assert `complete` |
| Process crash (SIGKILL, OOM) mid-stream | Up to 1 s of text since the last flush is lost. The lock session ends, so the next sweep (this instance after restart, or the other one during overlap) marks those nodes `interrupted` and settles their reservations. The user sees Retry |
| Host loss or network partition of an instance | Server TCP keepalives end its lock session within about 30 s, then the sweep runs as above. Test: drop one instance's packets with iptables |
| Deploy (SIGTERM) | Readiness fails at once; `DRAIN_SECS` (270 s) drain; then each remaining task marks only its own `(id, attempt)` as interrupted, with partial text, and settles its own reservation |
| Process exits while a final write is still retrying | Nothing more can be done in-process. The sweep marks the node `interrupted` with the last flushed text; `final_write_abandoned` logged. **Explicit NFR-11 exception**: a Postgres outage that outlasts both the answer and the process's remaining life (e.g. it spans a deploy's `DRAIN_SECS` drain) can lose up to the text after the last flush |
| Retry clicked twice, or two tabs | The compare-and-set lets one through; the other gets 409. A superseded task can't write (fence) |
| Capped or rate-limited user | 429 before any row is written (send, retry, merge proposal) |
| Postgres down | `/api/healthz` 503 (readiness only; liveness stays up so in-flight streams survive). Requests fail 503; creation is one transaction, and the client keeps the typed question |
| Postgres slow | Pool acquire timeout 5 s, statement timeout 10 s → 503 |
| Two sends race on the same parent | Partial unique index → 409; the client reloads |
| Data loss | Completed nodes are committed before `done` (NFR-11). The database itself: the managed provider's PITR window, or ≤ 24 h with daily backups only (**assumption** about the host) |

## Observability

- `tracing` JSON logs to stdout, with a `request_id` per request.
- Log fields are an allowlist: route, status, latency_ms, user_id, canvas_id, node_id, model, prompt_tokens, completion_tokens, cached_tokens, ttft_ms, error_code, owner_key, location, thread, db_error, sqlstate, event, and `message` (fixed text only). **Never** content, anchors, summaries or emails (NFR-9, tested).
- Levels: INFO for `nodemind_api`, WARN for dependencies. Dependency events lose `message`, since their text may interpolate data (tested).
- Engine signals: `compaction_on_critical_path`, `compaction_fallback`, `budget_enforced`, `reclaimed_after_sweep`, `final_write_failed`, `usage_estimated`. A cache hit ratio is derived as `cached_tokens / prompt_tokens`.
- NFR-2 (TTFT) and NFR-12 (cost) are computed with SQL over `llm_usage` and logs. There is no metrics stack in phase 1.
- An uptime check on `/api/healthz` emails the owner (NFR-10).

## Testing strategy

- **Property tests** (`proptest`) on the pure `context` module, over random trees, merges, Unicode content and tokenizer pairs:
  - tokens ≤ budget (NFR-7);
  - `context_tokens(X) == assemble(X).tokens` (AC-12.1);
  - no sibling *messages* in a prompt (AC-10.1/10.2), and other branches only as their confirmed summaries (FR-14);
  - a merged branch's own summary never appears in a prompt built inside that branch;
  - U+0000 in input or model output never reaches Postgres;
  - byte-identical prefixes on consecutive sends (AC-10.4);
  - checkpoint fields depend only on ancestors;
  - the parent X is always in the verbatim tail, including an 8,000-char CJK/emoji user message right before a checkpoint, and a block made of hundreds of 1-token messages;
  - the runtime backstop never fires when `r_up` is at its limit, including a maximum-size message with every `doubt_type` template;
  - sum-of-parts token count equals the full recount;
  - trees that branch from a `user_message` still yield alternating roles;
  - over N sends, summary calls ≤ ⌈path tokens / `BLOCK_TOKENS`⌉.
  Anchors are round-tripped through UTF-16 with arbitrary Unicode.
- **Fault-injection integration tests** against a Postgres container and a scripted fake LLM provider:
  - `docker pause` Postgres for 70 s mid-stream, and for 5 min after stream end → node `complete`;
  - SIGKILL one of two instances, during a stream and during a compaction → the other sweeps the node and the reservation within 30 s;
  - iptables-drop one instance's packets → swept within about 60 s;
  - SIGTERM drain;
  - client disconnect then reattach → full text;
  - capped user → 429 with zero rows;
  - errored stream → settled usage row;
  - retry race → one 409.
- The NFR-1/NFR-3 benchmarks run on a seeded 1,000-node canvas.

## Deployment

**Environments**: `local` (Docker Compose with Postgres only; `cargo run` + `pnpm dev`, with the Next dev server proxying `/api` to `localhost:8080`; **assumption**: dev rewrites pass SSE unbuffered, else CORS for `localhost:3000` in dev only) and `production`.

**Build and ship**:
- One multi-stage container image:
  1. `pnpm build` → `apps/web/out`;
  2. `cargo build --release` in SQLx offline mode;
  3. a Debian-slim runtime holding the binary and `out/`.
- CI on every push: `cargo fmt --check`, `clippy`, `cargo test` (Postgres service container, fake provider, the property and fault-injection suites), web lint/typecheck/Playwright, and the ts-rs diff check (NFR-16).
- Images are tagged by git SHA. Pushing to `main` deploys after CI passes.

**Host**: one small instance on a container PaaS (Fly.io, Render or Railway are candidates; **owner decision**), plus managed Postgres 16 with **PITR required** and backups kept ≥ 7 days. Daily backups alone can't keep NFR-11's "a completed node is never lost", so a host without PITR is ruled out ([F-L3-4](reports/reviews/architecture-L3.md)).
- The rate limiters are in-process, so the app runs as one instance; a deploy overlap briefly doubles them, which is accepted.
- The stream engine and the metering ledger are already multi-instance safe.
- Running more than one instance permanently only requires moving the token buckets into Postgres.
- The advisory-lock connection needs a direct (non-transaction-pooled) connection.

**Health checks**: `/api/livez` (no DB) is the liveness check; `/api/healthz` (DB ping) is readiness. The platform's kill timeout must be ≥ `DRAIN_SECS` + 10 s (280 s; Fly allows up to 300 s). At host choice, confirm that the proxy stops routing on failed readiness or on SIGTERM.

**Rollback**: redeploy the previous image tag. Migrations are backward compatible for one release.

**Restore runbook** (drilled before launch, then monthly; results go in `docs/DAILY_LOG.md`):
1. Prefer PITR to just before the incident.
2. Otherwise, restore the newest backup into a fresh database.
3. In **both** cases, collect tombstones newer than the restore point: `tombstones` from the live DB if it is readable or from any newer backup, plus every `event=tombstone` line in the log drain since the restore point.
4. Re-apply them:
   - `DELETE FROM users WHERE id = ANY($deleted_users)` and `DELETE FROM canvases WHERE id = ANY($deleted_canvases)` (both cascade);
   - `UPDATE users SET password_hash = NULL WHERE id = ANY($reset_users)`; those users get a fresh invite link.
5. `DELETE FROM sessions` and `DELETE FROM invites`. Restored rows could include revoked sessions and used links, so everyone signs in again ([F-L3-2](reports/reviews/architecture-L3.md)).
6. Run `sqlx migrate info` and the row-count smoke queries.
7. Only then open traffic.
The monthly drill deletes a synthetic user and a synthetic canvas, and resets a synthetic password, after the backup. It then checks all three stay applied and that an old session cookie is rejected.

**Config (env)**:
- Connections and keys: `DATABASE_URL`, `DATABASE_URL_DIRECT` (required in production), `COOKIE_KEY` (32 bytes; signs the `device` cookie, rotated by redeploy), provider API keys.
- Models: `MODEL_MAIN`, `MODEL_DOUBT`, `MODEL_SUMMARY`.
- Context budget: `CONTEXT_BUDGET_TOKENS=16000`, `REPLY_RESERVE_TOKENS=2000`, `COMPACTION_SUMMARY_MAX_TOKENS=700`, `MERGE_BUDGET_TOKENS=2000`, `MERGE_BUDGET_CANON=1550`, `MERGE_PROPOSAL_INPUT_TOKENS=4000`, and `tokenizers.toml` (per-model `r_up`/`r_down`). `BLOCK_TOKENS`, `PATH_MSG_MAX_TOKENS` and `FRAME_TOKENS` are compile-time constants.
- Auth: `TRUSTED_IP_HEADER`, `APP_ORIGIN` (used in invite links and the Origin check).
- Limits and streaming: `DAILY_TOKEN_CAP`, `STREAM_FLUSH_MS=1000`, `TASK_DEADLINE_SECS=240`, `DRAIN_SECS=270`, `MAX_NODE_CHARS=100000`.
The budget inequality (Context builder step 3) is checked at startup. Changing the model for a mode is a config change only (NFR-13).

## Open questions and assumptions to settle in build

| Item | Default taken here | How to settle |
| --- | --- | --- |
| Providers/models per mode | Not chosen; drives the tokenizer and the privacy notice | Owner decision before the first LLM task. Preferred: local-tokenizer models (OpenAI family) |
| `genai` covers streaming, usage, `cached_tokens` and Anthropic `cache_control` | Streaming + usage yes; caching controls unverified | One-day spike; fallback `async-openai` |
| Recursive CTE meets NFR-1 | Yes (assumption) | Seeded 1,000-node benchmark; `ltree` if not |
| Host allows a direct connection for the advisory lock, and honours session `tcp_keepalives_*` / `tcp_user_timeout` | Yes on Fly/Render/Railway managed Postgres (assumption) | Check at host choice with the iptables partition test |
| DOM selection → markdown source offsets | Renderer source positions (assumption) | Spike with code blocks, lists, inline code |
| Static export with `?id=` route and CSP hashes | Works (assumption) | First web build |
| Hosting provider, PITR and log-drain retention | Container PaaS + managed Postgres; drain ≥ 8 days | Owner decision before release |
| Host deploy strategy overlaps old and new instances (rolling or blue-green), and the app DB role may set `log_error_verbosity` | Yes (assumption); the zero-cut-deploy claim depends on overlap ([F-L3-8](reports/reviews/architecture-L3.md)) | Check at host choice; without overlap, deploys cut answers older than `DRAIN_SECS`, which is accepted |
