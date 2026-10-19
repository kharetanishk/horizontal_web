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
   ├── auth      signup/login/logout, argon2id, Postgres sessions
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
| Postgres | All state: users, sessions, canvases, nodes, edges, llm_usage, events, account_deletions | pgvector is **not** enabled in phase 1 |

## Send-message flow (FR-3 – FR-10)

1. The client sends `POST /api/canvases/:id/messages` with client-generated UUIDs for the new user and AI nodes. The input text stays in the client until the `created` event arrives, so a failed request never loses the question (FR-18).
2. **Rate limit first.** The in-process token bucket (20 LLM calls/min per user) is checked before any database work. Over the limit, the handler returns 429 and writes nothing.
3. **Admission transaction**, all or nothing:
   - lock the user's metering row (`SELECT … FROM users WHERE id = $u FOR UPDATE`) and check the daily cap and the in-flight count (see [Metering](#metering));
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
   On a DB error it retries with capped exponential backoff (max 10 s apart) **for as long as the process lives**. The finished text stays in memory, and attached clients see a non-terminal `finalizing` status. `done` is published only after commit. A terminal `error` is never published while the row is still `streaming`. Only process exit can end the retries. The row is then still `streaming` under a dead `owner_key`, so the next sweep marks it `interrupted` with the last flushed text (`final_write_abandoned` logged at shutdown). On a stream error, the task writes the partial text, `status = error` and `error_code` (also retried), settles usage, and publishes `error`.
9. The buffer is capped at `MAX_NODE_CHARS` (100,000 code points). If the cap is hit, the task stops the stream and saves the text with `error_code = 'truncated'` and `status = complete`.

## Stream engine

**Registry.** `Mutex<HashMap<NodeId, Arc<Stream>>>`. A `Stream` holds `attempt`, `text: RwLock<String>` and a `tokio::sync::watch` of `{len, status}`. The writer appends, then bumps the watch. Entries are removed 30 s after a terminal state.

**Attach.** Both `POST …/messages` and `GET /api/nodes/:id/stream` work the same way:
1. Send a `snapshot` event (the node plus all text so far).
2. Loop: on each watch change, send `delta` with the text since the last offset sent.
3. On a terminal status, send exactly one `done` or `error`, then close.
A slow reader skips intermediate watch values and receives one larger delta. Nothing is dropped and the writer never waits, so terminal events always arrive while a client is connected ([F-L3-9](reports/reviews/architecture-L3.md)). Reloading the page, opening a second tab, or reconnecting after a network drop all just attach again. If the node is not in this instance's registry (for example during deploy overlap), attach falls back to reading the row every `STREAM_FLUSH_MS` until it settles. All attachers of one node share **one poller** (an in-process map from node to a `watch` fed by a single poll task). A polling attach closes after 6 min (the 5-min answer timeout plus margin). Concurrent attaches are capped at 10 per user (429 beyond).

**Instance liveness, without heartbeats.**
- At boot, each instance picks a random `owner_key` (bigint). It holds `pg_advisory_lock(owner_key)` on a dedicated connection outside the pool for its whole lifetime. Postgres releases a session lock only when that session ends, which means the process died or its connection did.
- **Dead-peer detection is bounded.** On a host loss or partition no FIN/RST arrives, so the lock session sets server-side `tcp_keepalives_idle = 10`, `tcp_keepalives_interval = 5`, `tcp_keepalives_count = 3` and `tcp_user_timeout = 30000` right after connecting. The client socket uses the same keepalives. Postgres then ends a vanished instance's session, and frees its lock, within about 30 s instead of the kernel default of about 2 h ([F-L3-3](reports/reviews/architecture-L3.md)). Whether the managed host honours these settings is an open question below.
- Streaming nodes and open reservations carry `owner_key`.
- The sweeper runs at boot and then every 30 s:
  1. Collect `SELECT owner_key FROM nodes WHERE status = 'streaming' UNION SELECT owner_key FROM llm_usage WHERE settled_at IS NULL`, minus `$me`. This also catches orphaned compaction and merge-proposal reservations, which have no streaming node.
  2. For each key, try `pg_try_advisory_xact_lock(key)` in a transaction. Success proves that owner is gone. In that transaction, mark its streaming nodes `error`/`interrupted` and settle its open reservations at their reserved amount (`estimated = true`).
- An instance never sweeps its own key, so a Postgres outage of any length can't kill a single instance's live streams.
- If the lock connection drops, the instance re-acquires the same key on reconnect. A sweep by another instance during that gap is undone by the reclaim rule in send step 7. Accepted edge case, which needs both a lock-connection partition and a second live instance, i.e. a deploy overlap:
  - Clients attached on the sweeping instance see `interrupted` and the Retry button. A reload shows the reclaimed answer.
  - Clicking Retry first supersedes it, by design: the user asked for a new answer.
  - Settlement is keyed by reservation id and always overwrites, so the owner's actual usage replaces the sweeper's estimate.
- The lock connection must bypass transaction-mode PgBouncer (`DATABASE_URL_DIRECT`, if the host pools).
- This also replaces the heartbeat column, the 10 s heartbeat writes and the 60 s staleness guess.

**Graceful shutdown (SIGTERM).**
1. New sends and retries get 503 `unavailable`.
2. Running tasks get `DRAIN_SECS` (20 s) to finish.
3. Tasks still running are cancelled: stream tasks, compaction precomputes and merge proposals. Each cancelled stream task writes its own `(id, attempt)` as `error`/`interrupted` with its partial text. Every cancelled task settles its own reservation.
4. The process exits and its lock is released.
No broad `WHERE status = 'streaming'` update exists anywhere.

## Context builder (FR-10 – FR-12, NFR-7)

**Path message cap.** Every path message enters prompts middle-truncated to at most `PATH_MSG_MAX_TOKENS` (2,000). AI replies are already below this through `max_tokens`; this bounds 8,000-char user messages in dense scripts.

**Checkpoints (deterministic, prefix-only).** When a node becomes `complete`, three fields are stored:
- `ctok`: the node's tokens, as truncated above, under the canonical tokenizer (`o200k_base` from `tiktoken-rs`, pinned);
- `block_acc = parent.block_acc + ctok`;
- `is_checkpoint = block_acc >= BLOCK_TOKENS` (2,500); a checkpoint resets `block_acc` to 0.
So a block holds at most `BLOCK_TOKENS − 1 + PATH_MSG_MAX_TOKENS` (4,499) canonical tokens.

`BLOCK_TOKENS`, `PATH_MSG_MAX_TOKENS` and the canonical tokenizer are **compile-time constants**, not env config. Changing one needs a migration that recomputes the checkpoint fields and clears cached summaries ([F-L3-9](reports/reviews/architecture-L3.md)).

These values depend only on the node's ancestors. A node only gets children once it is complete, and error nodes never have children. So every descendant, on every branch, sees the same checkpoints.

**Summary chain.** For checkpoint k with previous checkpoint p, `S(k) = summarize(S(p) + messages after p up to and including k)`. The input is bounded (≤ `COMPACTION_SUMMARY_MAX_TOKENS` + `BLOCK_TOKENS` + one middle-truncated message), so it never approaches the summary model's window. `S(k)` is cached on node k with `UPDATE … SET compaction_summary = $s WHERE id = $k AND compaction_summary IS NULL`; a lost race re-reads the stored value. It is valid forever and shared by every branch below k.

**Precompute, off the critical path.** When a node completes as a checkpoint and its path holds ≥ 50% of the path budget in raw tokens, the scheduler computes the missing `S(·)` in the background:
- in order, with a global semaphore of 2;
- metered as a `compaction` reservation, and skipped if the user is over the cap.
Compaction reservations count toward the daily cap but not the user's in-flight limit, so system work never 429s a user's send ([F-L3-6](reports/reviews/architecture-L3.md)). An on-demand compaction inside a send takes its own `compaction` reservation in its own admission. If the cap refuses it, the send falls back to drop-oldest.
By the time a send needs `S(k)`, it already exists. The scheduler is in-process and not durable. A summary lost to a restart is computed on demand by the next send, with a 30 s timeout and a `compaction_on_critical_path` log. If that fails, the send drops the oldest messages instead.

**Assembly** `assemble(node, mode, new_message?) -> (messages, tokens, estimated)`:

1. **Path**: one recursive CTE walks up from X through `edges.target_id` and returns the path nodes, root first, with their parent edges and checkpoint fields.
2. **Mode**: `doubt` if any edge on the path is a `branch`, else `main` (AC-10.3). The mode picks the system prompt, the model and the tokenizer.
3. **Fixed allotments**, all in the mode tokenizer: system prompt ≤ 800; anchor ≤ 1,500; new message ≤ 3,000; merged summaries ≤ `MERGE_BUDGET_TOKENS` (2,000); compaction summary ≤ 1,000. An oversized piece is middle-truncated with a `[… N tokens omitted …]` marker. The path tail gets the rest: 14,000 − 8,300 = 5,700.
- **Fit guarantee.** One whole block, (`BLOCK_TOKENS` − 1 + `PATH_MSG_MAX_TOKENS`) × `TOKENIZER_SLACK` = 4,499 × 1.15 ≈ 5,174, fits the tail. So when the parent X has just closed a block, the cutoff j = m − 1 still fits, and **X is always in the verbatim tail** (AC-11.1, [F-L3-7](reports/reviews/architecture-L3.md)). A prompt within budget always exists (NFR-7).
- `TOKENIZER_SLACK` bounds the mode tokenizer against the canonical one.
- The server checks this inequality at startup and refuses to start if it doesn't hold. With an estimated (non-local) mode tokenizer, every count is first multiplied by `TOKEN_SAFETY_FACTOR`, the worst ratio measured in the AC-12.2 run (default 1.15; 1.0 for local tokenizers), and the inequality includes it.
4. **Cutoff with built-in hysteresis.** Let ck₁…ckₘ be the checkpoints on the path, with ck₀ meaning "no compaction". Pick the smallest j such that `S(ckⱼ)` plus the messages after ckⱼ fit the tail allotment. The new message is reserved at its full 3,000 allotment whatever its real size. So j doesn't depend on what the user types, and the counter and the send pick the same j. j changes only when the path crosses a checkpoint, at most once per ~2,500 tokens of conversation, never on every send ([F-L3-1](reports/reviews/architecture-L3.md)).
5. **Layout, ordered for prefix caching:**
   - the system prompt;
   - if j > 0, `<conversation_summary>S(ckⱼ)</conversation_summary>`, followed by a `<merged_branches>` block holding the merged summaries whose origin lies in root..ckⱼ (ordered by origin depth, then `merged_at`);
   - the tail messages verbatim; a merged summary whose origin is in the tail is appended to that origin's content in a `<merged_branch>` block;
   - the anchor, then the new message wrapped by the quick-action template (AC-7.1, AC-8.x).
   Roles strictly alternate user/assistant. Adjacent same-role items are concatenated into one turn with a fixed `\n\n---\n\n` separator; this covers a summary before a user message and a branch from a `user_message` (U1 →branch→ U2, AC-5.1). The merge is deterministic, so it stays byte-stable ([F-L3-10](reports/reviews/architecture-L3.md)). Two consecutive sends on the same path share every byte up to the new message (AC-10.4). Sibling branches share every byte up to their fork. Explicit Anthropic `cache_control` breakpoints go after the summary block and at the end of the tail, if the gateway exposes them (spike); OpenAI-style providers cache stable prefixes automatically. The provider's `cached_tokens` is recorded.
6. **Anchor** (branches): for `text`, the highlight plus its surrounding sentence, or for long highlights the text marked "the part being asked about"; for `code`, the selected lines marked plus the rest of the block; nothing extra for `none`.
7. **Merged summaries are never dropped.** The total `merge_tokens` per canvas is capped at `MERGE_BUDGET_TOKENS`. A merge that would exceed it returns 422 `merge_budget_exhausted`, and the UI says so. Every accepted summary therefore reaches every downstream prompt (AC-14.1, [F-L3-8](reports/reviews/architecture-L3.md)).

**Token counter** (FR-12, AC-12.1): `context_tokens(X) = assemble(X, mode, None).tokens`, the exact same function. It does not use the prefix-sum trick ([F-L3-3](reports/reviews/architecture-L3.md)).
- Per-message token counts are memoized for `complete` nodes in a bounded LRU keyed by `(node_id, tokenizer)`. Because of step 4, each node only sums its tail (≤ ~5,700 tokens, a few dozen messages), so computing all 1,000 counters on canvas load is cheap. It runs in `spawn_blocking` and is part of the NFR-3 benchmark.
- If `S(ckⱼ)` isn't computed yet, its allotment (1,000) is used and the count is marked `estimated`.
- A merge returns updated counts for the affected nodes, and `done` carries the new node's count. Canvas load never calls a provider.
- With a local tokenizer (OpenAI family) counts match the provider's up to chat framing, checked against reported usage (AC-12.2, ±5%). Otherwise a calibrated approximation is used and marked `estimated` ([ADR 0005](ADR/0005-llm-gateway-and-tokenizer.md)).

NFR-1 (path fetch + assembly < 200 ms p95 at 200-deep paths) is expected to hold with the CTE over `edges(target_id)` (**assumption**, seeded benchmark; `ltree` if not).

## Metering

`llm_usage` is a ledger of reservations. Every LLM call (answer, retry, compaction, merge proposal) is admitted by one transaction:

```sql
SELECT 1 FROM users WHERE id = $u FOR UPDATE;            -- serializes this user's metering only
SELECT coalesce(sum(CASE WHEN settled_at IS NULL THEN reserved_tokens
                         ELSE prompt_tokens + completion_tokens END), 0) AS spent_today,
       count(*) FILTER (WHERE settled_at IS NULL AND kind <> 'compaction') AS in_flight
FROM llm_usage WHERE user_id = $u AND (created_at >= date_trunc('day', now()) OR settled_at IS NULL);
-- reject (429, rollback) if spent_today + $reserve > DAILY_TOKEN_CAP or in_flight >= 3 (user-initiated kinds only)
INSERT INTO llm_usage (…, reserved_tokens, owner_key) VALUES (…);
```

- Every pool connection and the lock connection set `TimeZone = 'UTC'` in `after_connect`, so `date_trunc('day', now())` is the UTC day on every connection ([F-L3-11](reports/reviews/architecture-L3.md)).
- The reservation is an upper bound on the call: the whole context budget (16,000) for an answer, and summary input plus output for compaction. Each provider attempt has its own reservation, including the automatic pre-first-token retry. With a local tokenizer the cap therefore holds **exactly**. With an estimated tokenizer it holds within `TOKEN_SAFETY_FACTOR`, the margin calibrated in the AC-12.2 run ([F-L3-5](reports/reviews/architecture-L3.md)).
- A 4th concurrent call is rejected with 429, not queued ([F-L3-5](reports/reviews/architecture-L3.md)).
- Settlement writes provider usage when the provider reports it. Otherwise it writes the locally counted prompt plus locally counted streamed completion, with `estimated = true`. That covers broken streams, timeouts and failed compaction calls, so every billed call leaves a settled row ([F-L3-4](reports/reviews/architecture-L3.md)).
- An unsettled reservation from a dead instance is settled by the sweeper at its reserved amount. Settlement is `UPDATE … WHERE id = $reservation_id` and always overwrites, so a late owner's real usage replaces that estimate.
- The day is the UTC day.

## Data model

Postgres 16. All ids are UUID v7. Text enums use `CHECK` constraints. The brief's `node_embeddings`, `memories`, `composer_runs` and `artifacts` tables are phase 2/3 and not created.

| Table | Columns | Constraints / indexes |
| --- | --- | --- |
| `users` | `id`, `email citext`, `password_hash`, `created_at` | `UNIQUE(email)`. The row doubles as the per-user metering lock |
| `sessions` | `token_hash bytea` (SHA-256 of the cookie token), `user_id`, `expires_at`, `created_at` | PK `token_hash`; FK `user_id → users ON DELETE CASCADE`; index `user_id` |
| `canvases` | `id`, `user_id`, `title`, `root_node_id` (nullable), `created_at`, `updated_at` | FK `user_id → users ON DELETE CASCADE`; index `(user_id, updated_at DESC)`. The first message sets `root_node_id` with `WHERE root_node_id IS NULL`; no row updated → 409. Merges lock the canvas row to check the merge budget |
| `nodes` | `id`, `canvas_id`, `type` (`user_message`/`ai_message`), `content text`, `status` (`streaming`/`complete`/`error`), `error_code`, `attempt int`, `owner_key bigint` (instance streaming it), `model`, `prompt_tokens`, `completion_tokens`, `cached_tokens`, `ctok int`, `block_acc int`, `is_checkpoint bool`, `compaction_summary`, `compaction_tokens int`, `position_x`, `position_y`, `created_at`, `updated_at` | FK `canvas_id → canvases ON DELETE CASCADE`; `UNIQUE(canvas_id, id)`; index `canvas_id`; partial index `(owner_key) WHERE status = 'streaming'` (sweeper); `CHECK (char_length(content) <= 100000)`; `CHECK (compaction_summary IS NULL OR is_checkpoint)` |
| `edges` | `id`, `canvas_id`, `source_id`, `target_id`, `type` (`reply`/`branch`), `anchor jsonb`, `doubt_type` (`simpler`/`example`/`why`/`deeper`/`wrong`/`free`), `merge_summary`, `merge_tokens int`, `merged_at`, `collapsed bool`, `created_at` | Composite FKs `(canvas_id, source_id)` and `(canvas_id, target_id) → nodes(canvas_id, id) ON DELETE CASCADE`; `UNIQUE(target_id)` (tree, path-walk index); partial `UNIQUE(source_id) WHERE type = 'reply'`; index `source_id`; index `canvas_id`; `CHECK ((type = 'branch') = (anchor IS NOT NULL AND doubt_type IS NOT NULL))`; `CHECK (type = 'branch' OR (merge_summary IS NULL AND NOT collapsed))` |
| `llm_usage` | `id bigserial`, `user_id`, `node_id` (nullable), `kind` (`answer`/`compaction`/`merge_proposal`), `model`, `owner_key`, `reserved_tokens`, `prompt_tokens`, `completion_tokens`, `cached_tokens`, `estimated bool`, `created_at`, `settled_at` | FK `user_id → users ON DELETE CASCADE`; index `(user_id, created_at)`; partial index `(user_id) WHERE settled_at IS NULL`; `CHECK (settled_at IS NULL OR prompt_tokens IS NOT NULL)` |
| `account_deletions` | `user_id`, `deleted_at` | No FK. Restore runbook input (see Deployment) |
| `events` | `id bigserial`, `user_id`, `canvas_id`, `kind` (`branch_created`/`merge_confirmed`/`merge_updated`/`back_to_main`/`collapse`), `payload jsonb`, `created_at` | FKs `ON DELETE CASCADE`; index `(user_id, created_at)`. Payload is a typed Rust enum per `kind`; never message text except proposed vs. confirmed merge summaries |

Choices behind the model:

- **Branch state lives on the branch edge** (`merge_summary`, `collapsed`). An origin can have several branches, each merged or collapsed separately. The origin in "stored against the origin node" (AC-13.2) is the edge's `source_id`.
- **Content is immutable once a node is `complete`.** Only `streaming`/`error` AI nodes change content. An `error` node has no children, because sends need a complete parent. So checkpoint fields, summaries and anchors are never invalidated.
- **Merges are snapshots** ([F-L3-7](reports/reviews/architecture-L3.md)). A merged branch can still be continued. Downstream prompts keep the confirmed summary. The edge exposes `stale = exists(subtree node created after merged_at)`, and the UI offers "N new messages since merge · update summary". Re-merging replaces the summary and writes `merge_updated`, not a second `merge_confirmed`, so the metrics don't double-count. The same summary again is a no-op. Merge proposal and merge confirm return 422 `branch_streaming` while any node in the branch subtree is streaming.
- **Anchor JSON** (brief shape, `kind` discriminated; the Rust enum is the source for TS via ts-rs): `{"kind":"none"}` · `{"kind":"text","start":412,"end":540,"text":"…"}` · `{"kind":"code","start":900,"end":1340,"start_line":5,"end_line":9,"text":"…"}`.
  - `start`/`end` are UTF-16 code-unit offsets into the raw markdown `content`, matching JS string indexing. For a `code` anchor they cover the whole code block, and `text` is the selected lines.
  - The server never slices Rust strings by those offsets. It converts with `encode_utf16()`, checks `start <= end <= len`, and compares `String::from_utf16(&u[start..end])` with `text` (AC-6.1). A `code` slice must also start with a fence line, and lines `start_line..=end_line` must equal `text`. Any failure → 422 `invalid_anchor`.
  - Mapping a DOM selection back to source offsets uses the renderer's source positions (**assumption**, spike).
- **Ownership**: every query joins through `canvases.user_id = $current_user`.
- **Deletes** cascade from `users` and `canvases` (AC-2.3, AC-20.1). A stream task whose node is deleted matches no row and stops.

**Migrations**:
- SQLx migrations are embedded with `sqlx::migrate!` and run at startup under SQLx's advisory lock.
- `set_ignore_missing(true)`, so a rolled-back binary boots on a newer schema.
- Forward-only, using expand → deploy → contract.
- `.sqlx/` offline data is committed.

## API

JSON over HTTPS under `/api`, same origin as the web app. Auth is an `HttpOnly; Secure; SameSite=Strict` session cookie. Every endpoint except signup/login/livez/healthz/meta needs a session (401 otherwise, AC-1.2). Any canvas, node or edge not owned by the caller returns **404**, never 403 (NFR-8).

**Error format** (all non-2xx and the SSE `error` event):

```json
{ "error": { "code": "not_found", "message": "Canvas not found" } }
```

Codes:
- 400 `bad_request`
- 401 `unauthorized`
- 403 `forbidden_origin`, `signup_unavailable`
- 404 `not_found`
- 409 `conflict`
- 422 `invalid_anchor`, `parent_not_complete`, `branch_streaming`, `merge_budget_exhausted`
- 429 `rate_limited`, `daily_cap_reached` (both with `Retry-After`)
- 502 `llm_unavailable`
- 503 `unavailable`

| Method & path | Request | Response |
| --- | --- | --- |
| `POST /api/auth/signup` | `{email, password}` (password ≥ 12 chars) | 201 `{user}` + cookie. Not allowlisted and already registered both return the same 403 `signup_unavailable` after the same argon2 work (see Auth) |
| `POST /api/auth/login` | `{email, password}` | 200 `{user}` + cookie; 401 on bad credentials (same message and same argon2 work for unknown email) |
| `POST /api/auth/logout` | – | 204, session row deleted |
| `GET /api/me` | – | `{id, email}` |
| `DELETE /api/me` | `{password}` (re-auth) | 204; cascades everything, writes tombstones, cookie cleared (FR-20) |
| `GET /api/meta` | – | `{providers: ["…"], context_budget}` for the privacy notice (FR-21) |
| `GET /api/canvases` | – | `[{id, title, updated_at}]` newest first. No pagination in phase 1 (**assumption**: tens of canvases per user) |
| `POST /api/canvases` | `{id?, title?}` | 201 canvas |
| `GET /api/canvases/:id` | – | `{canvas, nodes: [Node & {context_tokens, estimated}], edges: [Edge & {stale}]}`, compressed |
| `PATCH /api/canvases/:id` | `{title}` | 200 canvas |
| `DELETE /api/canvases/:id` | – | 204 |
| `POST /api/canvases/:id/messages` | `{user_node_id, ai_node_id, parent_id?, edge: "reply" \| "branch", anchor?, doubt_type?, content}` | `text/event-stream`: `created` {nodes, edges} → `snapshot` → `delta`* → `done` {node, context_tokens} or `error` {error, node} |
| `GET /api/nodes/:id/stream` | – | `text/event-stream`: `snapshot` {node} → `delta`* → `done`/`error`. For a settled node: `snapshot` then the terminal event. Used on reload, second tab and reconnect |
| `POST /api/nodes/:id/retry` | – | Rate limit, then one transaction: admission + compare-and-set `UPDATE nodes SET status = 'streaming', content = '', attempt = attempt + 1, owner_key = $me WHERE id = $1 AND status = 'error' AND <owned> RETURNING attempt`. No row → 409 and nothing written, including the reservation and the partial text (AC-18.1). Then the same stream as a send |
| `PUT /api/canvases/:id/positions` | `[{id, x, y}]` (≤ 2,000) | 204. One `UPDATE … FROM unnest(…)` scoped to the canvas after the ownership check. Duplicate ids → 400; row count ≠ ids → 404, rolled back |
| `POST /api/edges/:id/merge-proposal` | – | `{summary}` (≤ 200 chars, not stored). Admitted and metered like a send; the subtree goes through the same truncation path. 422 `branch_streaming`; 502 if the LLM fails (the UI lets the user type it) |
| `PUT /api/edges/:id/merge` | `{summary, proposed?}` (1–200 chars each) | 200 edge (`collapsed = true`, AC-13.2) plus updated `context_tokens` for affected nodes. Same summary → no-op 200. 404 unless a `branch`; 422 `branch_streaming` / `merge_budget_exhausted` |
| `PATCH /api/edges/:id` | `{collapsed}` | 200 edge. 404 unless a `branch` |
| `POST /api/events` | `{canvas_id, kind, payload}`, UI-only kinds (`back_to_main`, `collapse`) | 204. Server-side kinds are never accepted from the client; payload is typed per kind (422 otherwise) |
| `GET /api/livez` | – | 200 while the process runs |
| `GET /api/healthz` | – | 200 `{db: "ok"}` / 503 |

Rules:

- **Length unit**: every length limit counts Unicode code points (`chars().count()`, `char_length`).
- **Message validation**:
  - `content` is 1–8,000 chars.
  - `parent_id` may be absent only for the first question, enforced through `canvases.root_node_id`.
  - The parent must be `complete` (AC-6.2).
  - A `reply` parent has at most one reply child (partial unique → 409).
  - `branch` needs an `anchor` and a `doubt_type`; `text`/`code` anchors only on `ai_message` parents.
- **Idempotency**: retried writes carry client-generated ids. A repeat insert hits the PK → 409, and the client reloads.
- **Rate limits** (in-process token buckets, checked before any DB write):
  - 20 LLM calls/min per user;
  - auth endpoints 10/min per client IP;
  - progressive delays on failed logins per (email, IP) and per email, which delay but never lock out.
  The client IP comes only from `TRUSTED_IP_HEADER`, and production refuses to start without it. Argon2 runs in `spawn_blocking` behind a semaphore of 2. Limiter maps expire idle keys and are bounded.
- **Daily token cap and concurrency**: the [Metering](#metering) ledger, exact.
- **Panics**: `CatchPanicLayer` → 500. The panic hook logs only the location and `request_id`. `JsonRejection` → generic `bad_request` that echoes no input (NFR-9 test).
- **CSRF** ([F-L3-10](reports/reviews/architecture-L3.md)):
  - every non-GET/HEAD request must carry an `Origin` header equal to the app origin; a missing or different Origin → 403 `forbidden_origin`;
  - the cookie is `SameSite=Strict`;
  - there is no Content-Type rule, so bodyless mutations (logout, retry) are covered by the same check. JSON is parsed only where an endpoint defines a body.

## Auth

[ADR 0004](ADR/0004-password-auth-postgres-sessions.md). Email + password, hashed with argon2id (`argon2` crate defaults).
- On login the server creates a 32-byte random token, stores its SHA-256 in `sessions`, and sets the token as a 30-day cookie.
- Lookups use `WHERE token_hash = $1 AND expires_at > now()`. The sweeper deletes expired rows.
- Logout and account deletion remove the rows immediately (AC-20.1).

**Equal-work responses.**
- Login verifies against a fixed dummy hash for unknown emails.
- Signup always computes the argon2id hash of the submitted password before the allowlist check and the insert. "Not allowlisted", "already registered" and "created" therefore cost the same, and the first two return the same 403.

Signup is limited to `SIGNUP_ALLOWLIST` and fails closed: an empty or missing list means signup is closed. Opening it to everyone needs `SIGNUP_OPEN=true`, and the effective mode is logged at startup.

Not in phase 1: email password reset (the operator uses `api reset-password <email>`, which also deletes that user's sessions), magic links, OAuth. Single role; no sharing.

## Security and data protection

- **Secrets** come only from environment variables. They are never sent to the client and never logged (NFR-9). `.env` is local-only and git-ignored.
- **Input validation** at the boundary: serde types with length limits, anchor checks, UUID path params, a 1 MB body limit. Markdown is rendered without raw HTML (no `rehype-raw`).
- **PII**: emails and message content. Content goes only to the configured LLM providers named in the privacy notice (FR-21), never to logs (NFR-9 test). Deleted accounts remain in backups for up to 7 days; the notice says so.
- **Deletion tombstones** ([F-L3-6](reports/reviews/architecture-L3.md)). Account deletion records the `user_id` (a UUID, no other PII) in two places that fail independently:
  1. the `account_deletions` table;
  2. a structured log line `event=account_deleted user_id=…`, shipped to the host's log drain with retention ≥ 8 days, longer than backup retention.
  A database loss can't erase the second copy. The restore runbook under Deployment uses both.
- **Headers**: HSTS, `nosniff`, `Referrer-Policy: same-origin`, and `Content-Security-Policy: default-src 'self'; script-src 'self' <hashes>`. The build hashes every inline script in `apps/web/out/*.html` into `csp-hashes.json`, and the static handler serves the matching hashes per page (**assumption**, first web build). If hashing proves unworkable, `'unsafe-inline'` is recorded as an accepted risk in an ADR.

## Failure handling

| Failure | Behaviour |
| --- | --- |
| LLM connect error, 429 or 5xx **before the first token** | One automatic retry after 1–2 s (jittered), with its own reservation (skipped if the cap refuses it). Then `status = error` and SSE `error` `llm_unavailable`; the UI shows "Answer interrupted" + Retry |
| LLM stream breaks **after tokens** | No automatic retry. Partial text saved, `status = error`, usage settled (estimated if needed); Retry replaces the answer (AC-18.1) |
| LLM timeouts | Connect 10 s; first token 30 s; gap between chunks 30 s; whole answer 5 min. All config. Each ends like the row above |
| Compaction summary missing at send | Normally precomputed. Otherwise computed on demand (30 s timeout, `compaction_on_critical_path` log), then drop-oldest fallback (`compaction_fallback` log). Budget still holds |
| Merge proposal fails | 502; the user types the summary |
| Client disconnects mid-stream | Task finishes and saves. Reopening attaches via `GET /api/nodes/:id/stream`: snapshot, then live deltas |
| Postgres outage of any length mid-stream or after the stream | The task keeps streaming to attached clients from memory. Flushes fail and are skipped. The final write retries until it lands (`finalizing` shown meanwhile). A single instance never sweeps itself; another instance's sweep is reclaimed by the fenced final write. Tests: pause Postgres 70 s mid-stream, and 5 min after stream end; resume; assert `complete` |
| Process crash (SIGKILL, OOM) mid-stream | Up to 1 s of text since the last flush is lost. The lock session ends, so the next sweep (this instance after restart, or the other one during overlap) marks those nodes `interrupted` and settles their reservations. The user sees Retry |
| Host loss or network partition of an instance | Server TCP keepalives end its lock session within about 30 s, then the sweep runs as above. Test: drop one instance's packets with iptables |
| Deploy (SIGTERM) | 20 s drain; then each remaining task marks only its own `(id, attempt)` as interrupted, with partial text, and settles its own reservation |
| Process exits while a final write is still retrying | Nothing more can be done in-process. The sweep marks the node `interrupted` with the last flushed text; `final_write_abandoned` logged |
| Retry clicked twice, or two tabs | The compare-and-set lets one through; the other gets 409. A superseded task can't write (fence) |
| Capped or rate-limited user | 429 before any row is written (send, retry, merge proposal) |
| Postgres down | `/api/healthz` 503 (readiness only; liveness stays up so in-flight streams survive). Requests fail 503; creation is one transaction, and the client keeps the typed question |
| Postgres slow | Pool acquire timeout 5 s, statement timeout 10 s → 503 |
| Two sends race on the same parent | Partial unique index → 409; the client reloads |
| Data loss | Completed nodes are committed before `done` (NFR-11). The database itself: the managed provider's PITR window, or ≤ 24 h with daily backups only (**assumption** about the host) |

## Observability

- `tracing` JSON logs to stdout, with a `request_id` per request.
- Log fields are an allowlist: route, status, latency_ms, user_id, canvas_id, node_id, model, prompt_tokens, completion_tokens, cached_tokens, ttft_ms, error_code, owner_key. **Never** content, anchors, summaries or emails (NFR-9, tested).
- Engine signals: `compaction_on_critical_path`, `compaction_fallback`, `reclaimed_after_sweep`, `final_write_failed`, `usage_estimated`. A cache hit ratio is derived as `cached_tokens / prompt_tokens`.
- NFR-2 (TTFT) and NFR-12 (cost) are computed with SQL over `llm_usage` and logs. There is no metrics stack in phase 1.
- An uptime check on `/api/healthz` emails the owner (NFR-10).

## Testing strategy

- **Property tests** (`proptest`) on the pure `context` module, over random trees, merges, Unicode content and tokenizer pairs:
  - tokens ≤ budget (NFR-7);
  - `context_tokens(X) == assemble(X).tokens` (AC-12.1);
  - no sibling content in a prompt (AC-10.1/10.2);
  - byte-identical prefixes on consecutive sends (AC-10.4);
  - checkpoint fields depend only on ancestors;
  - the parent X is always in the verbatim tail, including an 8,000-char CJK/emoji user message right before a checkpoint;
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

**Host**: one small instance on a container PaaS (Fly.io, Render or Railway are candidates; **owner decision**), plus managed Postgres 16 with daily backups kept ≥ 7 days, PITR preferred (NFR-11).
- The rate limiters are in-process, so the app runs as one instance; a deploy overlap briefly doubles them, which is accepted.
- The stream engine and the metering ledger are already multi-instance safe.
- Running more than one instance permanently only requires moving the token buckets into Postgres.
- The advisory-lock connection needs a direct (non-transaction-pooled) connection.

**Health checks**: `/api/livez` (no DB) is the liveness check; `/api/healthz` (DB ping) is readiness. The platform's kill timeout must be ≥ 25 s (`DRAIN_SECS=20`).

**Rollback**: redeploy the previous image tag. Migrations are backward compatible for one release.

**Restore runbook** (drilled before launch, then monthly; results go in `docs/DAILY_LOG.md`):
1. Prefer PITR to just before the incident; deletions up to that point are already applied.
2. Otherwise, restore the newest backup into a fresh database.
3. Collect tombstones newer than the backup: `account_deletions` from the live DB if it is readable or from any newer backup, plus every `event=account_deleted` line in the log drain since the backup time.
4. Re-apply them as `DELETE FROM users WHERE id = ANY($ids)` (cascades).
5. Run `sqlx migrate info` and the row-count smoke queries.
6. Only then open traffic.
The monthly drill deletes a synthetic user after the backup and checks it stays deleted.

**Config (env)**:
- Connections and keys: `DATABASE_URL`, `DATABASE_URL_DIRECT` (optional), provider API keys.
- Models: `MODEL_MAIN`, `MODEL_DOUBT`, `MODEL_SUMMARY`.
- Context budget: `CONTEXT_BUDGET_TOKENS=16000`, `REPLY_RESERVE_TOKENS=2000`, `COMPACTION_SUMMARY_MAX_TOKENS=1000`, `MERGE_BUDGET_TOKENS=2000`, `TOKEN_SAFETY_FACTOR` (per mode). `BLOCK_TOKENS` and `PATH_MSG_MAX_TOKENS` are compile-time constants.
- Auth: `SIGNUP_ALLOWLIST`, `SIGNUP_OPEN=false`, `TRUSTED_IP_HEADER`.
- Limits and streaming: `DAILY_TOKEN_CAP`, `STREAM_FLUSH_MS=1000`, `DRAIN_SECS=20`, `MAX_NODE_CHARS=100000`.
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
