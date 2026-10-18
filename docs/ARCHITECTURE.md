# Architecture

Scope: brief phase 1 (branching canvas) only. Inputs: [requirements.md](product/requirements.md) (FR/NFR IDs below refer to it), [source-brief.md](product/source-brief.md), [research.md](product/research.md), [ADR 0002](ADR/0002-rust-backend-supersedes-0001.md).
No code exists yet, so no number here is measured. Library behaviour, provider behaviour and hosting details are **assumptions** until a spike or test checks them.

## Options considered

| | A. One Rust service + Postgres (chosen) | B. API + Redis queue + LLM worker | C. Next.js server (BFF) + Rust API + Postgres |
| --- | --- | --- | --- |
| Shape | Axum binary serves the API, SSE, and the static Next.js export. Postgres is the only dependency | API enqueues each answer; worker calls the LLM and pushes tokens to the API through Redis pub/sub; API relays SSE | Next.js server renders pages and proxies `/api` to Axum |
| Streaming (FR-3, NFR-2) | Direct: LLM stream → SSE in one process | Extra hop through Redis, more latency and code | Extra proxy hop; SSE buffering through the Next proxy is a known pitfall (assumption) |
| Partial text kept on failure (FR-18) | Background task in the API writes the partial text | Worker writes it | Same as A |
| Availability 99% (NFR-10) | One instance plus managed Postgres is enough for a 99% target (≈7 h downtime/month) | Three things to keep up | Two app processes to keep up |
| Ops cost | One image, one process | Redis plus worker deploys | Node runtime plus Rust runtime |
| Needed when | Now | Requests outlive an HTTP connection, or slow jobs arrive (phase 2 composer, phase 3 embeddings) | Pages need SSR or SEO (none in phase 1: the canvas is client-only, behind login) |

**Decision: A** ([ADR 0003](ADR/0003-single-service-architecture.md)). No phase 1 requirement or number calls for a queue, cache, second service, Kubernetes, sharding or multi-region. Expected load is a handful of test users (requirements success metrics: 5 moderated learners), so one instance is enough. Redis/workers come with phase 2/3 (CLAUDE.md rule).

## Components

```
Browser (Next.js static export, React Flow, Zustand)
   │  same-origin HTTPS: JSON + SSE, session cookie
   ▼
nodemind-api (one Rust binary: Axum + Tokio)
   ├── http      routes, auth extractor, ownership checks, error mapping, rate limits
   ├── auth      signup/login/logout, argon2id, Postgres sessions
   ├── canvas    canvases, nodes, edges, positions, collapse, merge
   ├── chat      send/retry handlers, stream task, SSE
   ├── context   path fetch, mode, merged summaries, anchor, compaction, token count
   ├── llm       gateway: model per mode, streaming, usage, timeouts, retries
   ├── static    serves the exported web app (tower-http ServeDir, SPA fallback)
   └── db        SQLx pool, embedded migrations
   │                         │
   ▼                         ▼
Postgres 16 (managed)     LLM provider(s) over HTTPS
```

| Component | Responsibility | Notes |
| --- | --- | --- |
| `apps/web` | Canvas UI: nodes, selection toolbar, quick actions, merge dialog, collapse, "back to main", token counters, notices | Next.js `output: "export"`; no Node server in production. Canvas route is `/canvas?id=…`, because static export needs build-time params for dynamic segments (assumption, verify). React Flow for pan/zoom/`setCenter`; Zustand store holds canvas nodes/edges |
| Layout (in `apps/web`) | Own tree layout (FR-15): main path down; each branch starts right of its parent at the anchor's y; subtrees stacked so bounding boxes never overlap. Uses measured node heights from React Flow | About 100 lines, pure function with a unit test for AC-15.1 (research lean). Positions are saved to the server so reload is exact (FR-19) |
| `packages/schemas` | TypeScript types generated from Rust (ts-rs) | NFR-16: CI regenerates and fails on diff |
| `apps/api` | Single crate, modules above | Cargo workspace with one member. Split crates only when a second binary (worker, phase 2+) needs shared code |
| `apps/api/prompts/` | System prompts per mode (main, doubt, summary) and the 5 quick-action templates, loaded with `include_str!` | Versioned with the code; a `packages/prompts` package only when a second consumer exists |
| `llm` gateway | Only module that talks to providers (NFR-13). Interface: `stream(mode, messages) -> Stream<Delta>` then `Usage`; `complete(mode, messages) -> String`; `count_tokens(model, messages) -> u32` | Built on the `genai` crate (multi-provider, streaming; assumption, spike first). Fallback: `async-openai` against OpenAI-compatible APIs. [ADR 0005](ADR/0005-llm-gateway-and-tokenizer.md) |
| Postgres | All state: users, sessions, canvases, nodes, edges, llm_usage, events | pgvector is **not** enabled in phase 1; enable it with phase 3 |

### Send-message flow (FR-3 – FR-10)

1. Client sends `POST /api/canvases/:id/messages` with client-generated UUIDs for the new user and AI nodes. The input text stays in the client until the `created` event arrives, so a failed request never loses the question (FR-18).
2. One transaction: check canvas ownership, check the parent node is `complete`, validate the anchor against the parent's content, insert `user_message` (complete), `ai_message` (`streaming`, empty, `updated_at = now()`) and the two edges. Commit.
3. **Right after the commit, spawn a detached Tokio task** that owns everything else for this answer. The SSE response only reads from the task's `mpsc` channel. If the client disconnects at any point, the task keeps running and saves the full answer (test: disconnect right after `created`, assert the node reaches `complete`).
4. The response sends SSE `created` with both nodes and edges.
5. In the task: start the heartbeat (`updated_at = now()` every 10 s, always the database clock, until the task ends). Then `context::build(ai_node)` (see below), which may include a compaction-summary call with its own 30 s timeout.
6. Call the LLM with `max_tokens = REPLY_RESERVE_TOKENS`. Forward deltas to a bounded channel with `try_send`; if it is full or closed, the delta is dropped for the client only (the buffer keeps it, and the client recovers through `done` or polling). The heartbeat runs on its own timer, so a slow reader never stalls it. Append to an in-memory buffer, and flush `content` to Postgres every 1 s (configurable).
7. **Single writer.** The node row carries `attempt`. Every heartbeat, flush and final write is `UPDATE … WHERE id = $1 AND attempt = $2 AND status = 'streaming'`. A task whose row was swept or superseded by a retry matches no row and stops.
8. On end: write the final `content`, `status = complete` and usage (`prompt_tokens`, `completion_tokens`, `model`), retrying the write with backoff (5 attempts over about 30 s) on a DB error. Send SSE `done` only after that commit succeeds. If every attempt fails, log `final_write_failed` with `node_id` (no content) and send SSE `error`. On a stream error: write the partial text, `status = error`, `error_code`; send SSE `error`.
9. The buffer is capped at `MAX_NODE_CHARS` (100,000 code points). `max_tokens` already keeps a normal answer far below it (2,000 tokens ≈ 8,000 chars). If the cap is hit anyway, the task stops the stream and saves the text with `error_code = 'truncated'` and `status = complete`.

### Context builder (FR-10 – FR-12, NFR-7)

1. **Path**: one recursive CTE that walks up from X through `edges.target_id` (each node has exactly one parent edge) and returns the path nodes with their parent edges (type, anchor, doubt type), root first.
2. **Mode**: `doubt` if any edge on the path is a `branch`, else `main` (AC-10.3). Picks the system prompt and model.
3. **Merged summaries**: one query, `edges` where `type = 'branch' AND merge_summary IS NOT NULL AND source_id = ANY(path_ids) AND id <> ALL(path_edge_ids)`. Summaries are not separate chat messages. Each one is appended to its origin node's message content in a delimited block (`<merged_branch>…</merged_branch>`), so roles still strictly alternate user/assistant and the prompt prefix before the origin stays byte-stable (AC-10.4). The brief lists summaries before path messages; this placement is a deliberate change for prefix caching. When an origin lies inside the compacted span root..C, its summary goes into a delimited block after the compaction text instead (step 5). So a branch merged after C was compacted still reaches every downstream prompt (AC-14.1). Merged summaries may use at most 15% of the budget. Past that cap, the ones closest to the new message are kept verbatim and the rest are dropped with a `summaries_capped` log.
4. **Anchor** (branches): for `text`, the highlight plus its surrounding sentence, or for long highlights the text marked "the part being asked about"; for `code`, the selected lines marked plus the rest of the block (AC-7.1); nothing extra for `none`. The quick-action template wraps the user's question (AC-8.1, AC-8.2).
5. **Budget and compaction**: `input budget = CONTEXT_BUDGET_TOKENS (16000) - REPLY_RESERVE_TOKENS (2000) = 14000` (FR-11's 16k is the whole context; 14k is what the prompt may use). Reserve the fixed parts first: system prompt, anchor, new message, capped merged summaries, and `COMPACTION_SUMMARY_MAX_TOKENS` when compaction will be needed. The rest is the path budget. If the full path fits, send it. If not, pick the cutoff node C, the latest path node such that the messages after C fit in 50% of the path budget. Replace root..C with C's `compaction_summary`.
   **Compaction is incremental.** C's summary = summarize(the summary of the previous cutoff C_prev on the same path, if any, + messages C_prev+1..C). That input goes through the same budget and truncation function, so the summary call never exceeds the model's context window, whatever the path length. The summary sits in the first user message, in a delimited block before the first kept message. It is cached on C with `UPDATE nodes SET compaction_summary = $s WHERE id = $c AND compaction_summary IS NULL`. On a lost race the request re-reads and uses the stored summary, so concurrent requests share one prefix. The summary covers only root..C content, which never changes, so the cache never goes stale. Test (AC-11.x): a 200-node path produces a real summary, not the fallback.
6. **Oversized single message**: any one message (parent answer, the rest of a code block, the new message) that is larger than the budget left for it is middle-truncated with a `[… N tokens omitted …]` marker. Together with the summary cap, this means a prompt within budget always exists (NFR-7). Tests: AC-11.1 with one message larger than the budget, and with many merged summaries.
7. If the summary call fails or times out (30 s), fall back to dropping the oldest messages until the prompt fits, and log `compaction_fallback`. The request still respects NFR-7.
8. **Token counter** (FR-12): one pure function `assemble(path, summaries, mode, tokenizer) -> (messages, tokens)` serves both the send path and the counter; the counter calls it in dry-run mode, without the new message, LLM calls or summary generation. A test asserts the counter and the send path return the same number for the same node. Per-message counts are memoized only for `complete` nodes (their content never changes), in a bounded LRU keyed by `(node_id, tokenizer)`. Main and doubt modes may use different tokenizers; each node is counted with its own mode's tokenizer. If compaction is due and C has no cached summary, the counter uses `COMPACTION_SUMMARY_MAX_TOKENS` (1000) and marks it `estimated: true`.
   **Canvas load** computes every node's `context_tokens` in one depth-first pass over the already-fetched graph, inside `spawn_blocking`, carrying the token sums of the shared prefix down each path (per-message counts add up; see above). This is part of the NFR-3 benchmark. A merge changes counts below its origin, so the merge response returns the updated counts, and a `done` event carries the new node's count. **Local counts only.** Canvas load never calls a provider. If the chosen model has a local tokenizer (`tiktoken-rs` for OpenAI models), counts are exact up to chat framing, which is checked against provider-reported usage (AC-12.2, ±5%, NFR-6). If not, a local approximation is used, marked `estimated`, and calibrated against reported usage. Choosing a model with a local tokenizer for main and doubt modes is preferred (ADR 0005).

NFR-1 (path fetch + assembly < 200 ms p95 at 200-deep paths) is expected to hold with a CTE over `edges(target_id)`. This is an **assumption** to check with the seeded benchmark. If it fails, add a materialized path column (`ltree`) on nodes.

## Data model

Postgres 16. All ids are UUID v7 (time-ordered, client-generatable for idempotency). Text enums use `CHECK` constraints, so adding a value is a cheap migration. The brief's `node_embeddings`, `memories`, `composer_runs` and `artifacts` tables are phase 2/3 and not created.

| Table | Columns | Constraints / indexes |
| --- | --- | --- |
| `users` | `id`, `email citext`, `password_hash`, `created_at` | `UNIQUE(email)` |
| `sessions` | `token_hash bytea` (SHA-256 of the cookie token), `user_id`, `expires_at`, `created_at` | PK `token_hash`; FK `user_id → users ON DELETE CASCADE`; index `user_id` |
| `canvases` | `id`, `user_id`, `title`, `root_node_id` (nullable), `created_at`, `updated_at` | FK `user_id → users ON DELETE CASCADE`; index `(user_id, updated_at DESC)` for the list. The first message sets `root_node_id` with `WHERE root_node_id IS NULL`; no row updated → 409 (one root per canvas) |
| `nodes` | `id`, `canvas_id`, `type` (`user_message`/`ai_message`), `content text`, `status` (`streaming`/`complete`/`error`), `error_code`, `attempt int` (stream generation), `model`, `prompt_tokens`, `completion_tokens` (provider usage, AI nodes), `compaction_summary`, `position_x`, `position_y`, `created_at`, `updated_at` (stream heartbeat) | FK `canvas_id → canvases ON DELETE CASCADE`; `UNIQUE(canvas_id, id)` (target of the composite FKs); index `canvas_id`; partial index `updated_at WHERE status = 'streaming'` (sweeper); `CHECK (char_length(content) <= 100000)` |
| `edges` | `id`, `canvas_id`, `source_id`, `target_id`, `type` (`reply`/`branch`), `anchor jsonb`, `doubt_type` (`simpler`/`example`/`why`/`deeper`/`wrong`/`free`), `merge_summary`, `merged_at`, `collapsed bool`, `created_at` | Composite FKs `(canvas_id, source_id)` and `(canvas_id, target_id) → nodes(canvas_id, id) ON DELETE CASCADE`, so both ends sit on the edge's canvas; FK `canvas_id → canvases ON DELETE CASCADE`; `UNIQUE(target_id)` (tree: one parent; path walk index); partial `UNIQUE(source_id) WHERE type = 'reply'` (one continuation per node); index `source_id`; index `canvas_id`; `CHECK ((type = 'branch') = (anchor IS NOT NULL AND doubt_type IS NOT NULL))`; `CHECK (type = 'branch' OR (merge_summary IS NULL AND NOT collapsed))` |
| `account_deletions` | `user_id`, `deleted_at` | No FK (the user row is gone). Used only to re-apply deletions after a backup restore |
| `llm_usage` | `id bigserial`, `user_id`, `kind` (`answer`/`compaction`/`merge_proposal`), `model`, `prompt_tokens`, `completion_tokens`, `created_at` | FK `user_id → users ON DELETE CASCADE`; index `(user_id, created_at)`. The gateway writes one row per LLM call; the daily token cap reads this table |
| `events` | `id bigserial`, `user_id`, `canvas_id`, `kind` (`branch_created`/`merge_confirmed`/`back_to_main`/`collapse`), `payload jsonb`, `created_at` | FKs `ON DELETE CASCADE`; index `(user_id, created_at)`. Success-metric instrumentation only. Payload is a typed Rust enum per `kind` (ids, timestamps); it never holds message text except the proposed vs. confirmed merge summary |

Choices behind the model:

- **Branch state lives on the branch edge** (`merge_summary`, `collapsed`). An origin node can have several branches, each merged or collapsed separately, and "stored against the origin node" (AC-13.2) is the edge's `source_id`.
- **Node content is immutable once `complete`.** Only `streaming`/`error` AI nodes change content (stream flushes, retry). A complete node's `position_x/y` and its one-time `compaction_summary` cache can still be written, never its `content`. This keeps anchors valid (requirements cut node edits) and makes compaction summaries cacheable.
- **Anchor JSON** (brief shape, `kind` discriminated, Rust enum is the source for TS via ts-rs):
  `{"kind":"none"}` · `{"kind":"text","start":412,"end":540,"text":"…"}` · `{"kind":"code","start":900,"end":1340,"start_line":5,"end_line":9,"text":"…"}`, where `start`/`end` cover the whole code block in `content` and `text` is the selected lines.
  `start`/`end` are UTF-16 code-unit offsets into the raw markdown `content`, matching JS string indexing. Rust strings index by UTF-8 byte, so the server never slices `content[start..end]`. It converts explicitly: `let u: Vec<u16> = content.encode_utf16().collect()`, checks `start <= end <= u.len()`, and compares `String::from_utf16(&u[start..end])` with `text` (AC-6.1). Any failure returns 422 `invalid_anchor`. A `code` anchor is checked the same way, with no markdown parsing on the server: the slice `start..end` must start with a fence line, and lines `start_line..=end_line` of it must equal `text`. The context builder takes "the rest of the block" from the same slice. Tests cover emoji and CJK content, and code blocks inside lists. Mapping a DOM selection in rendered markdown back to source offsets uses the renderer's source positions (**assumption**: remark/rehype position data is enough, spike in build).
- **Ownership**: every query joins through `canvases.user_id = $current_user`; `canvas_id` is denormalized onto `edges` so this is one join.
- **Deletes** cascade from `users` and `canvases` (AC-2.3, AC-20.1).

**Migrations**: SQLx migrations in `apps/api/migrations`, embedded with `sqlx::migrate!` and run at startup before the server binds. SQLx holds a Postgres advisory lock while migrating, so instances that overlap during a deploy don't race. The migrator is built with `set_ignore_missing(true)`, so a rolled-back binary boots on a database that already has newer migrations. A release check starts image N-1 against the schema from image N. Forward-only. Destructive changes use expand → deploy → contract across two releases so the previous image can still run (rollback). SQLx offline query data (`.sqlx/`) is committed so CI compiles without a database.

## API

JSON over HTTPS under `/api`, same origin as the web app. Auth by an `HttpOnly; Secure; SameSite=Lax` session cookie. Every endpoint except signup/login/healthz/meta needs a session (401 otherwise, AC-1.2). Any canvas, node or edge not owned by the caller returns **404**, never 403 (NFR-8).

**Error format** (all non-2xx and the SSE `error` event):

```json
{ "error": { "code": "not_found", "message": "Canvas not found" } }
```

Codes: `bad_request` 400, `unauthorized` 401, `not_found` 404, `conflict` 409, `invalid_anchor` / `parent_not_complete` 422, `rate_limited` 429 (with `Retry-After`), `llm_unavailable` 502, `unavailable` 503.

| Method & path | Request | Response |
| --- | --- | --- |
| `POST /api/auth/signup` | `{email, password}` (password ≥ 12 chars) | 201 `{user}` + cookie. Not allowlisted and already registered both return the same 403 `signup_unavailable`, so signup doesn't reveal which emails exist |
| `POST /api/auth/login` | `{email, password}` | 200 `{user}` + cookie; 401 on bad credentials (same message for unknown email) |
| `POST /api/auth/logout` | – | 204, session row deleted |
| `GET /api/me` | – | `{id, email}` |
| `DELETE /api/me` | `{password}` (re-auth) | 204; cascades everything, cookie cleared (FR-20) |
| `GET /api/meta` | – | `{providers: ["…"], context_budget}` for the privacy notice (FR-21) |
| `GET /api/canvases` | – | `[{id, title, updated_at}]` newest first. No pagination: a phase 1 user has tens of canvases (**assumption**); add `?cursor=` (keyset on `updated_at, id`) when a list passes 100 |
| `POST /api/canvases` | `{id?, title?}` | 201 canvas (default title "Untitled canvas") |
| `GET /api/canvases/:id` | – | `{canvas, nodes: [Node & {context_tokens, estimated}], edges: [Edge]}`, the full graph, gzip/brotli compressed. Expected fine for 200–1,000 nodes; the NFR-3 benchmark records response size as well as load time |
| `GET /api/nodes/:id` | – | One node. The client polls this (not the canvas) while a node is `streaming` |
| `PATCH /api/canvases/:id` | `{title}` | 200 canvas |
| `DELETE /api/canvases/:id` | – | 204 |
| `POST /api/canvases/:id/messages` | `{user_node_id, ai_node_id, parent_id?, edge: "reply" \| "branch", anchor?, doubt_type?, content}` | `text/event-stream`: `created` {nodes, edges} → `delta` {text}* → `done` {node} or `error` {error, node} |
| `POST /api/nodes/:id/retry` | – | Same SSE stream (AC-18.1). Compare-and-set: `UPDATE nodes SET status = 'streaming', content = '', attempt = attempt + 1, updated_at = now() WHERE id = $1 AND status = 'error' AND <owned> RETURNING attempt`; no row → 409 `conflict`. Counts as a message send for rate limits |
| `PUT /api/canvases/:id/positions` | `[{id, x, y}]` (≤ 2,000, above the largest supported canvas) | 204. Client sends it debounced after layout. One statement: `UPDATE nodes n SET … FROM unnest($ids, $xs, $ys) v WHERE n.id = v.id AND n.canvas_id = $canvas` after the canvas ownership check. Duplicate ids → 400. Row count ≠ number of ids → 404, transaction rolled back |
| `POST /api/edges/:id/merge-proposal` | – | `{summary}` (≤ 200 chars, not stored). The branch subtree goes through the same budget and truncation path as a send. Rate-limited and counted in `llm_usage`. 502 if the LLM fails; the UI then lets the user type the summary |
| `PUT /api/edges/:id/merge` | `{summary, proposed?}` (1–200 chars each) | 200 edge with `merge_summary`, `collapsed = true` (AC-13.2), plus updated `context_tokens` for the affected nodes. Writes the `merge_confirmed` event (proposed vs. confirmed text) server-side. 404 unless the edge is a `branch` |
| `PATCH /api/edges/:id` | `{collapsed}` | 200 edge. 404 unless the edge is a `branch` |
| `POST /api/events` | `{canvas_id, kind, payload}`, UI-only kinds (`back_to_main`, `collapse`) | 204. `branch_created` and `merge_confirmed` are written by the server inside the send and merge handlers, never by the client. `canvas_id` ownership checked (404 otherwise); `payload` must match the typed shape for `kind` (422 otherwise) |
| `GET /api/livez` | – | 200 while the process runs (liveness) |
| `GET /api/healthz` | – | 200 `{db: "ok"}` / 503 |

Rules:

- **Length unit**: every length limit counts Unicode code points (`chars().count()` in Rust, `char_length` in Postgres).
- **Messages validation**: `content` 1–8,000 chars; `parent_id` absent only for the first question, enforced by setting `canvases.root_node_id` in the same transaction (409 if already set); parent must be `complete` (AC-6.2). A `reply` parent must not already have a reply child (DB partial unique → 409); `branch` needs an `anchor`, and `text`/`code` anchors only on `ai_message` parents; `doubt_type` required for branches.
- **Idempotency**: writes that a client may retry carry client-generated ids (`user_node_id`, `ai_node_id`, canvas `id`). A repeat insert hits the PK and returns 409 `conflict`; the client then reloads the canvas instead of creating a duplicate. `PUT` merge/positions and `PATCH` are naturally idempotent.
- **Rate limits** (in-process token buckets; fine for one instance): 20 LLM calls/min per user, covering sends, retries and merge proposals; auth endpoints 10/min per client IP, plus a progressive delay on failed logins keyed by (email, IP), and a slower per-email delay across all IPs against distributed guessing. Both delay, never lock out, so nobody can lock another user out. Login runs argon2 verify against a fixed dummy hash for unknown emails, so timing doesn't reveal which emails exist. Limiter maps expire idle keys and are bounded in size. In production the server refuses to start if `TRUSTED_IP_HEADER` is unset. The client IP comes only from the platform's trusted header (`TRUSTED_IP_HEADER`, e.g. `Fly-Client-IP`), never from the peer address (that is the proxy) or a client-supplied `X-Forwarded-For`. Argon2 hashing runs in `spawn_blocking` behind a semaphore of 2, so login floods can't exhaust the CPU.
- **Daily token cap** per user (`DAILY_TOKEN_CAP`, sum of today's `llm_usage` rows, which covers answers, compaction and merge proposals) → 429 `rate_limited`. This bounds LLM cost (brief risk "cost growth"). At most 3 LLM calls per user run at once (a per-user semaphore). The check runs before each call, so a user can overshoot the cap by at most 3 × `CONTEXT_BUDGET_TOKENS` (48k tokens). That is accepted.
- **Panics**: `tower_http::catch_panic::CatchPanicLayer` turns a handler panic into a 500, so one bad request can't drop other connections. A custom panic hook logs only the location and `request_id`, never the panic message. Axum's `JsonRejection` is mapped to a generic `bad_request` that echoes no input. The NFR-9 test covers a panicking handler and a malformed body that contains a marker string.
- **CSRF**: SameSite=Lax cookie, and mutating requests must have `Content-Type: application/json` and a matching `Origin` header.

## Auth

[ADR 0004](ADR/0004-password-auth-postgres-sessions.md). Email + password (FR-1 leaves the method open). Passwords hashed with argon2id (`argon2` crate defaults). On login the server creates a 32-byte random token, stores its SHA-256 in `sessions`, and sets it as the cookie, which expires after 30 days. The auth extractor looks sessions up with `WHERE token_hash = $1 AND expires_at > now()`, and the sweeper loop deletes expired rows. Logout and account deletion remove the rows, so the session is invalid immediately (AC-20.1).

Signup is limited to `SIGNUP_ALLOWLIST` (comma-separated emails) because phase 1 is a moderated test with a few learners and every user costs LLM tokens. It fails closed: a missing or empty allowlist means signup is closed. Opening signup to everyone needs an explicit `SIGNUP_OPEN=true`. The effective mode is logged at startup.

Not in phase 1: password reset by email (no email provider yet; the operator resets a password with an `api reset-password <email>` CLI subcommand, which also deletes all of that user's sessions), magic links, OAuth. Add a reset flow with an email provider when signup opens.

Authorization model: single role. A user owns their canvases; there is no sharing (out of scope).

## Security and data protection

- **Secrets** come only from environment variables (`DATABASE_URL`, provider API keys), are never sent to the client, and are never logged (NFR-9). The host's secret store sets them; `.env` is local-only and git-ignored.
- **Input validation** happens at the API boundary: serde types with length limits, anchor check against stored content, UUID path params, a 1 MB body limit. Markdown is rendered client-side without raw HTML (no `rehype-raw`), so model output can't inject script.
- **PII**: emails and message content. Content goes to the configured LLM providers (named in the privacy notice, FR-21). It never appears in logs (NFR-9 test). Deleted accounts remain in backups for up to 7 days; the privacy notice states this. Account deletion writes a row to `account_deletions(user_id, deleted_at)` (no other PII), which survives in later backups, so a restore can re-apply deletions made after the backup was taken.
- **Headers**: HSTS, `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`, and `Content-Security-Policy: default-src 'self'; script-src 'self' <hashes>`. A Next.js App Router static export emits inline `<script>` tags (RSC payload, bootstrap), so `'self'` alone would leave a blank page (**assumption**, checked by the first web build). The build step hashes every inline script in `apps/web/out/*.html` into `csp-hashes.json`, and the static handler serves the matching `script-src` hashes per page. Inline styles may still need `style-src 'unsafe-inline'`. If hashing proves unworkable, `script-src 'unsafe-inline'` is recorded as an accepted risk in an ADR. `rehype-raw` stays banned either way.

## Failure handling

| Failure | Behaviour |
| --- | --- |
| LLM connect error, 429 or 5xx **before the first token** | One automatic retry after 1–2 s (jittered). Then `ai_message.status = error`, SSE `error` `llm_unavailable`; the UI shows "Answer interrupted" + Retry |
| LLM stream breaks **after tokens** | No automatic retry (it would duplicate text). Partial text saved, `status = error`, Retry replaces the answer (AC-18.1) |
| LLM timeouts | Connect 10 s; first token 30 s; gap between chunks 30 s; whole answer 5 min. All config. Each ends as the row above |
| Merge proposal or compaction summary fails | Merge: 502, user types the summary. Compaction: drop-oldest fallback (Context builder step 7) |
| Client disconnects mid-stream | Stream task finishes and saves; reopening the canvas shows the full answer. A node still `streaming` on load is polled via `GET /api/nodes/:id` every 2 s until it settles (ponytail: polling, add stream resume if users notice) |
| API process crashes or redeploys mid-stream | Up to 1 s of text since the last flush is lost. A sweeper task runs every 30 s on each instance (it also deletes expired sessions): `UPDATE nodes SET status = 'error', error_code = 'interrupted' WHERE status = 'streaming' AND updated_at < now() - interval '60 seconds'`. It relies only on the stream heartbeat, so it is correct while old and new instances overlap during a deploy. On SIGTERM the server stops accepting sends, gives stream tasks a `DRAIN_SECS` (20 s) drain window, then marks the remaining nodes `error`. The user sees Retry, never a stuck node |
| Final write fails (Postgres blip) | Retried with backoff for about 30 s (send flow step 8). Only then is the answer lost, with `final_write_failed` logged |
| Retry clicked twice, or two tabs | The retry compare-and-set lets one through; the other gets 409. A superseded stream task can't write (send flow step 7) |
| Postgres down | `/api/healthz` 503; requests fail with 503 `unavailable`. Nothing is half-written: node/edge creation is one transaction, and the client keeps the typed question until `created` arrives |
| Postgres slow | Pool acquire timeout 5 s, statement timeout 10 s → 503 |
| Two sends race on the same parent | Partial unique index → 409; the client reloads |
| Data loss | Completed nodes are committed before `done` is sent (NFR-11). Worst case for the database itself is the managed provider's point-in-time window, or else up to 24 h with daily backups only (**assumption** about the chosen host) |

## Observability

- `tracing` + `tracing-subscriber` JSON logs to stdout, collected by the host. Each request gets a `request_id`.
- Log fields are an allowlist: route, status, latency_ms, user_id, canvas_id, node_id, model, prompt_tokens, completion_tokens, ttft_ms, error_code. **Never** content, anchors, summaries or emails (NFR-9, tested).
- NFR-2 TTFT p95 and per-model cost (NFR-12) are computed from logs and `nodes` with SQL. There is no metrics stack in phase 1. Langfuse/tracing arrives with phase 5.
- An uptime check on `/api/healthz` from the host or a free external pinger alerts the owner by email (NFR-10).

## Deployment

**Environments**: `local` (Docker Compose with Postgres only; `cargo run` + `pnpm dev`. In dev the Next dev server proxies `/api` to `localhost:8080`; **assumption**: Next dev rewrites pass SSE through without buffering, else point the client at the API with CORS for `localhost:3000` in dev only) and `production`. A staging environment can be added when there is more than one person deploying.

**Build and ship**: one container image, multi-stage. (1) `pnpm build` → `apps/web/out`. (2) `cargo build --release` (SQLx offline mode). (3) Debian-slim runtime with the binary plus `out/`. CI (GitHub Actions) on every push runs `cargo fmt --check`, `clippy`, `cargo test` (against a Postgres service container, fake LLM provider), web lint/typecheck/Playwright (fake provider), and a ts-rs regeneration diff check (NFR-16). Images are tagged by git SHA. Pushing to `main` deploys after CI passes. The factory checkpoints commits; nothing deploys without a commit.

**Host**: one small instance on a container PaaS that terminates HTTPS and holds secrets (Fly.io, Render or Railway are candidates; **owner decision, assumption**), plus a managed Postgres 16 with daily backups kept ≥ 7 days (NFR-11). One instance is also what makes the in-process rate limits correct (an overlap during a deploy briefly doubles them, which is accepted). The streaming sweeper is heartbeat-based and safe with several instances. Running more than one instance permanently requires moving the rate limits into Postgres first.

**Health checks**: `/api/livez` (process up, no DB) is the liveness check, so a Postgres blip never restarts the process and kills in-flight streams. `/api/healthz` (pings the DB) is the readiness check. A new release only receives traffic once ready. The platform's kill timeout must be ≥ 25 s (a release-phase check), which covers the configurable drain window (`DRAIN_SECS=20`).

**Rollback**: redeploy the previous image tag. Migrations are backward compatible for one release (expand/contract), so the old binary runs on the new schema. Never edit an applied migration.

**Backups and restore test**: managed daily backups, 7-day retention. Before launch, then monthly: restore the latest backup into a scratch database, run `sqlx migrate info` and a row-count smoke query on `users`/`canvases`/`nodes`, and record the result and duration in `docs/DAILY_LOG.md`. A backup that has not been restored is unverified. A production restore re-applies every `account_deletions` row newer than the backup before traffic returns.

**Config (env)**: `DATABASE_URL`, provider API keys, `MODEL_MAIN`, `MODEL_DOUBT`, `MODEL_SUMMARY`, `CONTEXT_BUDGET_TOKENS=16000`, `REPLY_RESERVE_TOKENS=2000`, `COMPACTION_SUMMARY_MAX_TOKENS=1000`, `SIGNUP_ALLOWLIST`, `SIGNUP_OPEN=false`, `TRUSTED_IP_HEADER`, `DAILY_TOKEN_CAP`, `STREAM_FLUSH_MS=1000`, `MAX_NODE_CHARS=100000`. Changing a model per mode is a config change only (NFR-13).

## Open questions and assumptions to settle in build

| Item | Default taken here | How to settle |
| --- | --- | --- |
| Providers/models per mode | Not chosen; it drives the tokenizer and the privacy notice | Owner decision before the first implementation task that calls an LLM. Preferred: models with a local tokenizer (OpenAI family, `tiktoken-rs`). Otherwise counters are local estimates calibrated to reported usage (Context builder step 8). AC-12.2 checks ±5% (NFR-6) |
| `genai` crate covers streaming + usage for the chosen providers | Yes (assumption) | One-day spike; fallback `async-openai` |
| Recursive CTE meets NFR-1 | Yes (assumption) | Seeded 1,000-node benchmark; `ltree` if not |
| DOM selection → markdown source offsets | Renderer source positions (assumption) | Spike with code blocks, lists, inline code |
| Static export with `?id=` route and CSP | Works (assumption) | First web build |
| Hosting provider and its Postgres backup window | Container PaaS + managed Postgres | Owner decision before the release phase |
