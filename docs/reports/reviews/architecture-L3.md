# Level 3 review: architecture

- Generated: 2026-10-19T14:07:55+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Final write retries forever on permanent DB errors (e.g. NUL byte), pinning the node and an in-flight slot | docs/ARCHITECTURE.md:82 (send step 8), Failure handling 'Postgres outage' row |
| F-architecture-L3-2 | medium | A merged branch's own summary is injected into prompts on that same branch | docs/ARCHITECTURE.md:165-166 (Layout), :226 'Merges are snapshots', Testing strategy 'no sibling content' |
| F-architecture-L3-3 | medium | Canvas deletions are resurrected by a backup restore (no tombstones) | docs/ARCHITECTURE.md Security 'Deletion tombstones'; Deployment 'Restore runbook' steps 3-4 |
| F-architecture-L3-4 | medium | Unbounded argon2 queue lets anyone degrade login for every user | docs/ARCHITECTURE.md API Rules 'Rate limits' (argon2 semaphore of 2); Auth 'Equal-work responses' |
| F-architecture-L3-5 | medium | Compaction reservation at the summary model's window maximum can exceed the daily cap, disabling compaction | docs/ARCHITECTURE.md:119 (tokenizer constraint), Metering bullet 2 |
| F-architecture-L3-6 | medium | SSE has no keepalive; proxies will cut long silent phases | docs/ARCHITECTURE.md Stream engine 'Attach'; send step 8 'finalizing'; Failure handling timeouts |
| F-architecture-L3-7 | medium | Canvas load recomputes the mode-tokenizer count of every node from scratch after a restart | docs/ARCHITECTURE.md:176-177 (Token counter), :143 (path CTE) |
| F-architecture-L3-8 | low | 'Full reply never truncated' and 'cap holds exactly' claims rest on the wrong ratio | docs/ARCHITECTURE.md:121 (Path message cap), :158-159, Metering bullet 2 |
| F-architecture-L3-9 | low | Sweeper charges the full 16k reservation for crashed answers | docs/ARCHITECTURE.md:101 (sweeper step 2), Metering bullet 5 |

## F-architecture-L3-1: Final write retries forever on permanent DB errors (e.g. NUL byte), pinning the node and an in-flight slot

Step 8 says the final write retries with backoff 'for as long as the process lives', and only process exit ends it. It does not separate transient errors from permanent ones. Postgres `text` rejects U+0000 (`invalid byte sequence for encoding "UTF8": 0x00`), and a model can be prompted to emit it. Each flush fails and is skipped, then the final write fails deterministically forever. Settlement happens in the same transaction, so the reservation stays unsettled, the node stays `streaming`/`finalizing`, and the call counts against the user's in-flight limit of 3. Three such answers lock the user out of sending until the next deploy. Each one also holds the SIGTERM drain for the full DRAIN_SECS. User `content` sent as JSON `\u0000` hits the same failure on insert.

**Recommendation:** Strip or replace U+0000 in model deltas and in user input at the boundary, and add a property test for it. Classify DB errors: retry only connection, timeout or serialization errors (SQLSTATE classes 08, 57P, 40001). On any other error, write `status=error, error_code='persist_failed'` and settle the usage in a separate statement, then publish `error`.

## F-architecture-L3-2: A merged branch's own summary is injected into prompts on that same branch

The rule includes every merged summary whose origin lies on the path. Merged branches can still be continued (line 226). Take branch B→X→Y, merged, then continued at Y. B is on Y's path, so the summary of X..Y gets appended to B's content, ahead of X and Y, which are also in the prompt verbatim. The model sees a summary of the conversation before that conversation happens, the tokens are duplicated, and the merge allotment is spent on redundant text. Summaries of sibling branches from B also land in Y's prompt. That conflicts with the 'no sibling content in a prompt' property test and AC-10.2.

**Recommendation:** Include a merged summary only if its branch edge is not on the current path: `edge.target_id NOT IN path`. State whether sibling-branch summaries belong in branch prompts, and align the AC-10.1/10.2 property test with that rule.

## F-architecture-L3-3: Canvas deletions are resurrected by a backup restore (no tombstones)

Only account deletions are tombstoned, in `account_deletions` and the log drain. `DELETE /api/canvases/:id` (AC-2.3) leaves no record. Restoring from a backup older than the deletion brings back every canvas a user deleted, with its message content. The runbook has no step to re-apply those deletions. This is a data-protection failure for user-initiated deletion.

**Recommendation:** Write `canvas_id` tombstones the same way: a `canvas_deletions` table plus an `event=canvas_deleted canvas_id=…` log line. Extend runbook steps 3-4 to re-apply them, and add a canvas deletion to the monthly drill.

## F-architecture-L3-4: Unbounded argon2 queue lets anyone degrade login for every user

Signup runs argon2id on every request, even when signup is closed or the email isn't allowlisted. Login does the same for unknown emails. The only limits are 10/min per client IP and a semaphore of 2. Nothing bounds the waiters on that semaphore. A distributed client (many IPs × 10/min) can exceed 2-worker argon2 throughput, and queued requests grow without limit. Legitimate logins then time out, and memory grows with parked requests.

**Recommendation:** Add a global auth token bucket and a bounded wait on the semaphore: `try_acquire` with a short timeout → 503 `unavailable` with Retry-After. When signup is closed (empty allowlist and SIGNUP_OPEN=false), return the 403 without hashing; there is nothing to enumerate.

## F-architecture-L3-5: Compaction reservation at the summary model's window maximum can exceed the daily cap, disabling compaction

For a summary model without a local tokenizer (e.g. Anthropic, explicitly allowed for MODEL_SUMMARY), the reservation is its 'window maximum', around 200k tokens. That is likely larger than any sensible DAILY_TOKEN_CAP. If so, every compaction admission is refused, so every long path silently uses the drop-oldest fallback. Line 138 confirms a refused compaction falls back to drop-oldest. FR-11 summaries never happen, and the precompute queue keeps re-trying. The same section also says the reservation is 'summary input plus output', which contradicts the window maximum.

**Recommendation:** The input to compaction is already bounded (COMPACTION_SUMMARY_MAX_TOKENS + BLOCK_TOKENS + one PATH_MSG_MAX_TOKENS, canonical). Reserve that bound × a measured worst-case ratio for the summary model, plus max output. Add a startup check that this reservation is well below DAILY_TOKEN_CAP.

## F-architecture-L3-6: SSE has no keepalive; proxies will cut long silent phases

Several phases send no bytes for a long time: first-token wait up to 30 s, an on-demand compaction chain up to 2×30 s before the stream starts, a 30 s inter-chunk gap, and a `finalizing` state during a Postgres outage that can last minutes. The doc defines no SSE comment or heartbeat. PaaS edge proxies and load balancers close idle HTTP responses after roughly 60–100 s, depending on the host. The client then has to reconnect and re-attach, and during deploy overlap that falls back to the 1 s DB poller.

**Recommendation:** Send an SSE comment (`:\n\n`) every 15 s on every attach. Add an integration test that holds the fake provider silent for 90 s behind the platform proxy.

## F-architecture-L3-7: Canvas load recomputes the mode-tokenizer count of every node from scratch after a restart

Persisted `ctok` uses the canonical tokenizer, but the counter is computed in the mode tokenizer. Those counts are cached only in an in-process LRU keyed by (node_id, tokenizer). After any restart or eviction, opening a 1,000-node canvas re-tokenizes every node's full content in two tokenizers; middle truncation needs a full tokenization first. Nodes can be up to 100k chars. The doc also doesn't say whether `assemble` per node issues the step-1 recursive CTE. If it does, that is 1,000 CTEs per load. NFR-3 (< 2 s) is at risk, and the doc calls the load 'cheap' without a bound.

**Recommendation:** State that canvas load derives every path in memory from one `nodes`+`edges` fetch, with no per-node CTE. Persist the per-node mode-tokenizer count next to `ctok`, keyed by tokenizer id, and invalidate it on model change. Benchmark NFR-3 with a cold cache and maximum-size nodes.

## F-architecture-L3-8: 'Full reply never truncated' and 'cap holds exactly' claims rest on the wrong ratio

MODE_TOKEN_RATIO is defined as the worst ratio of mode tokens to canonical tokens, which is the right bound for converting canonical → mode. Line 121 uses it the other way: it claims PATH_MSG_MAX_TOKENS (2,500 canonical) ≥ REPLY_RESERVE (2,000 mode) × ratio. Converting mode → canonical needs the inverse worst-case ratio, which is never measured. Even taken as written, the startup check allows r up to 1.26 while 2,000 × 1.26 = 2,520 > 2,500. Separately, provider-reported prompt tokens can exceed local counts by chat framing (NFR-6 allows ±5%). A settled answer can then exceed its 16,000 reservation, so 'the cap holds exactly' is overstated.

**Recommendation:** Measure and check both directions of the ratio at startup, and make PATH_MSG_MAX_TOKENS ≥ REPLY_RESERVE × the worst canonical/mode ratio. Reserve the budget plus a framing margin, or reword 'exactly' as 'up to framing error'.

## F-architecture-L3-9: Sweeper charges the full 16k reservation for crashed answers

The sweeper settles a dead owner's reservations at the reserved amount: 16,000 per answer, even if a few tokens were generated. A crash loop, such as OOM on a large canvas or a bad deploy, therefore burns users' daily caps on system faults. The users get 429 `daily_cap_reached` for failures that weren't theirs.

**Recommendation:** Settle swept answer reservations from the data that exists: a local count of the parent path's prompt plus the last flushed content, marked `estimated=true`. Keep the full reservation only for kinds that have no flushed state.
