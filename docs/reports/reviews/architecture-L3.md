# Level 3 review: architecture

- Generated: 2026-10-19T13:58:39+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Sweeper never settles orphaned compaction/merge-proposal reservations, so users can end up permanently rate-limited | docs/ARCHITECTURE.md:99-100 (sweeper step 1), :169 (Metering), :106-110 (drain) |
| F-architecture-L3-2 | high | If the final write fails, the node stays 'streaming' forever, the finished answer is lost, and the user cannot retry | docs/ARCHITECTURE.md:82 (send step 8), :101, :308-311 (Failure handling) |
| F-architecture-L3-3 | high | Advisory-lock liveness depends on TCP detection; losing a host keeps a dead instance's lock for hours | docs/ARCHITECTURE.md:96-100, :339 (test only covers SIGKILL) |
| F-architecture-L3-4 | medium | A sweep during a lock-connection blip shows a terminal error that a later reclaim contradicts; Retry then throws away a valid answer | docs/ARCHITECTURE.md:78 (reclaim rule), :93 (attach fallback), :102 |
| F-architecture-L3-5 | medium | The 'cap holds exactly' and NFR-7 claims are false when token counts are approximate | docs/ARCHITECTURE.md Metering (:152-170) and Context builder step 3 / Token counter (approximation path) |
| F-architecture-L3-6 | medium | Background compaction and merge proposals use the user's 3 in-flight slots, so system work can 429 user sends | docs/ARCHITECTURE.md Context builder 'Precompute' and Metering ('in_flight >= 3' → 429, not queued) |
| F-architecture-L3-7 | medium | The 'tail allotment > one block' guarantee is wrong: one block can exceed 5,700 tokens, so the latest message drops out of the verbatim tail | docs/ARCHITECTURE.md Context builder step 3 ('5,700 … more than BLOCK_TOKENS plus tokenizer slack') and step 4 |
| F-architecture-L3-8 | medium | Unbounded SSE attach connections each poll Postgres every second | docs/ARCHITECTURE.md:93 (attach fallback), API GET /api/nodes/:id/stream |
| F-architecture-L3-9 | low | Changing BLOCK_TOKENS or the canonical tokenizer silently invalidates the stored checkpoint fields | docs/ARCHITECTURE.md Context builder 'Checkpoints'; Config (env) BLOCK_TOKENS |
| F-architecture-L3-10 | low | Branching from a user_message puts two user turns in a row, which the layout rule says cannot happen | docs/ARCHITECTURE.md Context builder step 5 ('Roles strictly alternate'); AC-5.1 |
| F-architecture-L3-11 | low | The 'UTC day' depends on the session time zone, which is never set | docs/ARCHITECTURE.md Metering SQL (`date_trunc('day', now())`), 'The day is the UTC day' |

## F-architecture-L3-1: Sweeper never settles orphaned compaction/merge-proposal reservations, so users can end up permanently rate-limited

The sweeper finds dead owners only through `SELECT DISTINCT owner_key FROM nodes WHERE status = 'streaming'`. A `compaction` or `merge_proposal` reservation (`node_id` nullable) has no streaming node, so its owner_key is never found. Background precompute is not durable, and drain step 3 only mentions stream tasks. So a crash, OOM or SIGTERM during a compaction or merge proposal leaves an `llm_usage` row with `settled_at IS NULL` forever. The metering query counts unsettled rows on every day (`OR settled_at IS NULL`). Each leaked row adds its `reserved_tokens` to `spent_today` every day and takes one of the 3 `in_flight` slots. After three leaks the user gets 429 on every send for good.

**Recommendation:** Build the sweeper's key set as a UNION of streaming `nodes.owner_key` and `llm_usage.owner_key WHERE settled_at IS NULL`, using the existing partial index. Make drain cancel and settle compaction and merge-proposal tasks too. Add a fault-injection test: SIGKILL during a compaction, then check the reservation gets settled.

## F-architecture-L3-2: If the final write fails, the node stays 'streaming' forever, the finished answer is lost, and the user cannot retry

After `FINAL_WRITE_RETRY_SECS` (120 s) the task logs `final_write_failed`, publishes `error` and drops the in-memory text. The row still has `status = 'streaming'` and this instance's owner_key, and an instance never sweeps its own key. Until the process restarts: retry is refused (its CAS needs `status = 'error'`), attach falls back to polling a row that never settles, and the reservation holds an in-flight slot. Only the last 1 s flush of text survives. The 'Postgres outage of any length' claim only holds if the stream outlasts the outage by less than 120 s. A 5-minute outage during a 20 s answer loses that answer, which contradicts NFR-11.

**Recommendation:** Keep the finished text and retry the final write without a deadline, or with a bounded queue, while the process lives; liveness is already DB-independent. Alternatively, when retries run out, hand the node to an in-process 'pending finalize' list that is retried on each sweeper tick. Never publish a terminal `error` while the row stays `streaming`. Add a test: pause Postgres for longer than 120 s past stream end, resume, check the node is `complete`.

## F-architecture-L3-3: Advisory-lock liveness depends on TCP detection; losing a host keeps a dead instance's lock for hours

`pg_advisory_lock` is released only when the Postgres backend sees its session end. On SIGKILL or OOM the kernel closes the socket, so the planned test passes. On VM loss, a network partition or a PaaS host migration, no FIN or RST arrives. The backend keeps the session, and the lock with it, until server TCP keepalives fire (Linux default about 2 h 11 min). During that window the nodes stay 'streaming' and cannot be retried, and their reservations block in-flight slots. The doc's claim of 'no clock-based guessing' moves the guess into kernel defaults the team does not control.

**Recommendation:** Set server-side `tcp_keepalives_idle/interval/count` (and `tcp_user_timeout` where available) on the lock session, e.g. `SET tcp_keepalives_idle = 10` after connecting. Check that the managed host honours this, and add it to the open-questions table. Add a test that drops packets to one instance with iptables, not just SIGKILL.

## F-architecture-L3-4: A sweep during a lock-connection blip shows a terminal error that a later reclaim contradicts; Retry then throws away a valid answer

When instance A's lock connection drops, instance B sweeps A's nodes to `error/interrupted` and settles their reservations at the reserved amount. Clients attached on B through polling see the terminal `error` and close, and the UI offers Retry. A's task then reclaims the row with its fenced write and finishes it, but nothing tells B's clients, who already closed. If the user clicks Retry, the CAS bumps `attempt` and fences out A's nearly finished answer. The reclaim's final 'settle the reservation' also runs on a row the sweeper already settled. The doc doesn't say whether settlement is `WHERE settled_at IS NULL`, which would drop the real usage, or an overwrite.

**Recommendation:** State that the final settlement overwrites the sweeper's estimated amount (`SET … estimated = false` unconditionally for that reservation id). Have polling attachers treat `interrupted` as non-terminal for a short grace period, or re-check before closing. Alternatively, have A drain and stop streaming when its lock connection drops, so a reclaim never happens.

## F-architecture-L3-5: The 'cap holds exactly' and NFR-7 claims are false when token counts are approximate

For providers without a local tokenizer, the doc uses a 'calibrated approximation' and marks it `estimated`. The prompt is cut to 14,000 tokens under that estimate. If the provider's real tokenizer counts more, the request goes over the input budget (NFR-7: 'No request sends more input tokens than the configured budget'). Settlement then records the provider's larger usage, more than the 16,000 reserved, so the daily cap overshoots. The jittered automatic retry before the first token also runs 'inside the same reservation'; any provider that bills an aborted attempt doubles usage under one reservation.

**Recommendation:** For estimated tokenizers, apply a safety margin (e.g. scale counts by the measured worst-case ratio from the AC-12.2 run) and validate it in the startup inequality. Change the wording from 'exactly' to 'exactly for local tokenizers, within margin otherwise', or give each provider attempt its own reservation.

## F-architecture-L3-6: Background compaction and merge proposals use the user's 3 in-flight slots, so system work can 429 user sends

Precompute is 'metered as a compaction reservation' and is admitted by the same transaction that counts in-flight calls. A user with one answer streaming whose node triggers background compaction (up to 2 by the semaphore) reaches 3 in flight. Their next send then gets 429, from work they never started. An on-demand compaction inside a send needs a second admission while the send already holds a slot; when that is refused, the send drops to drop-oldest even though budget remains.

**Recommendation:** Leave system-initiated `compaction` reservations out of the `in_flight` count (keep them in `spent_today`), or give compaction a separate concurrency limit. Spell out how an on-demand compaction inside a send is admitted.

## F-architecture-L3-7: The 'tail allotment > one block' guarantee is wrong: one block can exceed 5,700 tokens, so the latest message drops out of the verbatim tail

`block_acc` resets only after reaching 3,000, so a block can hold up to 2,999 plus the last message's `ctok`. A stored user message can be 8,000 code points; the 3,000-token cap applies only at send time, not to stored path messages. CJK text or emoji can tokenize at 1 token per char or more, so one block can be well over 5,700 tokens. Suppose the parent X is the checkpoint that closes such a block. Then j = m−1 doesn't fit, so j = m, and the tail after ckₘ is empty. The answer being followed up appears only inside the summary, which breaks AC-11.1 ('the latest messages are verbatim'). The startup inequality checks only config constants, not this data-dependent bound.

**Recommendation:** Store message content with ctok counted at its truncated length (cap each path message at the new-message allotment). Alternatively, always keep X verbatim, truncated, as its own tail item outside the j selection. Add a proptest case with an 8,000-char CJK or emoji user message just before a checkpoint, asserting that X's content is present verbatim.

## F-architecture-L3-8: Unbounded SSE attach connections each poll Postgres every second

The 20/min limit applies only to LLM calls, and auth endpoints are limited per IP. `GET /api/nodes/:id/stream` has no per-user cap on concurrent connections. Each attach to a node not in the local registry runs a DB read every `STREAM_FLUSH_MS`. One signed-in user, or a page that keeps reconnecting, can open hundreds of attaches and use up the pool, whose acquire timeout gives 503 for everyone. Together with the stuck-'streaming' rows above, these polls never end.

**Recommendation:** Cap concurrent SSE attaches per user (e.g. 10) and per node, close polling attaches after the 5-minute whole-answer timeout plus a margin, and share a single poller per node across attachers.

## F-architecture-L3-9: Changing BLOCK_TOKENS or the canonical tokenizer silently invalidates the stored checkpoint fields

`ctok`, `block_acc`, `is_checkpoint` and `compaction_summary` are written once and treated as 'valid forever', but they were computed with the current `BLOCK_TOKENS` and tokenizer. `BLOCK_TOKENS` is an env var, and the doc says config changes need no code change. After a change, old nodes keep old checkpoint spacing while new ones use the new spacing. The startup check verifies only the inequality, so the 5,700-tail guarantee and the summary-call bound no longer hold for existing canvases.

**Recommendation:** Make BLOCK_TOKENS a compile-time constant, or store `checkpoint_version` on nodes and refuse to start (or recompute lazily) when it differs from config.

## F-architecture-L3-10: Branching from a user_message puts two user turns in a row, which the layout rule says cannot happen

AC-5.1 allows a whole-node branch from a `user_message`, so a path can contain U1 →branch→ U2. U1's answer is a sibling and is left off the path. The assembled prompt then has two consecutive user messages. The doc only handles the case where the tail starts with an assistant message, so the 'strictly alternate' claim fails here. Some provider APIs reject such a prompt or merge the turns, which changes the byte layout AC-10.4 depends on.

**Recommendation:** Define how adjacent same-role messages are merged (concatenate them with a separator inside one user turn) and add this tree shape to the proptest generator.

## F-architecture-L3-11: The 'UTC day' depends on the session time zone, which is never set

`date_trunc('day', timestamptz)` uses the session's `TimeZone`. Managed Postgres defaults vary, and per-connection settings can drift. On a non-UTC session the cap resets at the wrong time, and different pool connections could disagree.

**Recommendation:** Use `date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'`, or set `TimeZone=UTC` in the pool's `after_connect`.
