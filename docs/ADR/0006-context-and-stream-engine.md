# ADR 0006: Context and stream engine — checkpoint compaction, attachable streams, lock-based liveness, reservation ledger

- Status: accepted
- Date: 2026-10-19

## Context

Four rounds of L3 review on the architecture kept finding defects in the same four mechanisms:

- **Compaction cutoff.** It moved on almost every send, so long paths were re-summarized before each answer. That cost TTFT and money, and broke prefix caching.
- **Heartbeat sweeper.** It killed live streams after a Postgres outage and discarded their final writes.
- **Counting and metering.** Prefix-sum token counting couldn't equal `assemble()`. Usage metering missed failed calls and wasn't ordered against the send transaction.
- **Edge cases.** Merged summaries could be silently dropped, and terminal SSE events could be lost to a full channel.

Patching each finding on its own kept producing new edge cases. The owner asked for the strongest technical design.

## Decision

Rebuild the four mechanisms on one property: **the conversation is an immutable, append-only tree**.

1. **Deterministic checkpoint compaction.**
   - Each complete node stores `ctok` (canonical `o200k_base` tokens) and `block_acc`. A node becomes a checkpoint when `block_acc` reaches `BLOCK_TOKENS`.
   - Checkpoints depend only on ancestors, so the rolling summary chain `S(k) = summarize(S(prev) + block)` is computed once per checkpoint and shared by every branch below it.
   - Summaries are precomputed in the background when a checkpoint closes on a long path.
   - The cutoff is the smallest checkpoint that fits, so it moves at most once per `BLOCK_TOKENS` of conversation. Prompt prefixes stay byte-stable for provider caching.
   - Fixed per-part allotments plus a startup-checked budget inequality guarantee a fitting prompt.
   - Merged summaries are bounded per canvas (`MERGE_BUDGET_TOKENS`) and are never dropped.
2. **One truth for counting.** The token counter is `assemble()` in dry run. Because the tail is bounded by `BLOCK_TOKENS`, that is cheap enough to run for every node on canvas load.
3. **Attachable streams.**
   - An answer is an in-memory buffer plus a `watch` state.
   - Any client attaches through `GET /api/nodes/:id/stream` and gets a snapshot, then deltas, then exactly one terminal event.
   - Slow readers coalesce; nothing is dropped. Polling is replaced.
4. **Liveness by Postgres session advisory lock**, not heartbeats.
   - Each instance holds `pg_advisory_lock(owner_key)` for its lifetime. A sweep only reclaims nodes whose owner's lock is acquirable, meaning the owner is dead.
   - An instance never sweeps itself.
   - Flushes and final writes are fenced by `attempt`, and they may reclaim an `interrupted` node that was not retried.
   - On SIGTERM, each task cancels and marks only its own `(id, attempt)`.
5. **Reservation ledger for metering.**
   - Every LLM call is admitted in one transaction that locks the user row, checks the daily cap and the in-flight count against `sum(reserved or actual)`, and inserts a reservation.
   - For sends and retries, that transaction is the same one that creates the nodes or runs the retry compare-and-set.
   - Settlement always writes a row, estimated when the provider reports no usage.
   - The cap is exact. The in-flight limit rejects; it does not queue.

Rejected:
- CRDT/local-first sync (no collaboration);
- event sourcing (the tree already is the log);
- Redis or a worker for streams (one instance is enough);
- crypto-shredding (it needs a key store outside the backups);
- a shared WASM core in the browser (server counts are enough);
- heartbeat-plus-grace-period sweeping (it guesses liveness from clocks and fails during DB outages).

## Consequences

- At most one compaction call per `BLOCK_TOKENS` of conversation per path, usually off the critical path. Sibling branches share summaries and prompt-cache prefixes.
- No periodic heartbeat writes. A Postgres outage of any length doesn't kill a single instance's streams.
- The advisory-lock connection must bypass transaction-mode PgBouncer (`DATABASE_URL_DIRECT`).
- The daily cap can't be overshot. Every billed call has a settled `llm_usage` row.
- Merges are snapshots. Continuing a merged branch marks its edge `stale`, and the UI offers to update the summary. A canvas has a merge budget; when it is full, merges return 422.
- New invariants are covered by property tests on the pure `context` module and by fault-injection tests (Postgres pause, SIGKILL, SIGTERM).
- The precompute scheduler is in-process and not durable. A summary lost to a restart is computed on demand by the next send that needs it.
