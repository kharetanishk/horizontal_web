# Level 3 review: architecture

- Generated: 2026-10-18T18:57:44+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Compaction cutoff moves on almost every send, so long paths re-summarize before each answer | docs/ARCHITECTURE.md:67 (Context builder step 5) |
| F-architecture-L3-2 | high | After a Postgres outage over 60 s, the sweeper kills live streams and their final writes are silently discarded | docs/ARCHITECTURE.md:57 (send flow step 7), :175 (sweeper row), send flow step 8 |
| F-architecture-L3-3 | medium | Prefix-sum token counting on canvas load can't equal assemble() once compaction, truncation, summary caps or mixed tokenizers apply | docs/ARCHITECTURE.md:71-72 (Context builder step 8, Canvas load) |
| F-architecture-L3-4 | medium | Daily token cap and llm_usage miss calls that error, time out or lose their stream | docs/ARCHITECTURE.md:88 (llm_usage), :145 (Daily token cap), Context builder step 7 |
| F-architecture-L3-5 | medium | Rate-limit and daily-cap checks aren't ordered against the send transaction | docs/ARCHITECTURE.md:52-53 (send flow steps 2-3), :145, retry row in API table |
| F-architecture-L3-6 | medium | The account_deletions re-apply step can't recover deletions made after the restored backup | docs/ARCHITECTURE.md:163 (PII), Deployment > Backups and restore test |
| F-architecture-L3-7 | medium | Behavior when a message is sent inside an already-merged branch is undefined | docs/ARCHITECTURE.md Data model (merge_summary on edge), API PUT /api/edges/:id/merge |
| F-architecture-L3-8 | low | Dropping merged summaries past the 15% cap silently breaks AC-14.1 and prefix stability | docs/ARCHITECTURE.md:65 (Context builder step 3) |
| F-architecture-L3-9 | low | `done`/`error` delivery and the SIGTERM 'mark remaining nodes' scope are underspecified | docs/ARCHITECTURE.md send flow step 6; Failure handling (SIGTERM) |
| F-architecture-L3-10 | low | Signup reveals whether an account exists through timing, and the CSRF Content-Type rule conflicts with bodyless mutations | docs/ARCHITECTURE.md API table (signup), Rules > CSRF |

## F-architecture-L3-1: Compaction cutoff moves on almost every send, so long paths re-summarize before each answer

C is defined as 'the latest path node such that the messages after C fit in 50% of the path budget', and the trigger is 'If the full path fits, send it'. Once the raw path is over budget, each new message pushes that latest qualifying C forward. A new C has no cached compaction_summary, so most sends on a long path make a synchronous summary LLM call (up to 30 s) before the answer call starts. That breaks NFR-2 (TTFT p95 < 3 s), roughly doubles LLM calls and cost per send, and changes the prompt prefix on every send, which undoes the prefix-caching goal behind AC-10.4.

**Recommendation:** Add hysteresis. Keep using the cached C_prev while summary(C_prev) + messages after C_prev fit in the path budget. Only pick a new C, advanced so the tail fits in 50%, when that compacted form overflows. Write down the expected compaction frequency and add a test: on a 200-node path, N consecutive sends trigger at most ceil(N / k) summary calls.

## F-architecture-L3-2: After a Postgres outage over 60 s, the sweeper kills live streams and their final writes are silently discarded

Heartbeats are DB writes. During an outage longer than 60 s, no heartbeat lands, so every streaming row's updated_at goes stale. When the DB comes back, the first sweeper tick (every 30 s, and possibly before the 10 s heartbeat fires) sets status='error', error_code='interrupted' on all of them. The task's final write is `WHERE attempt=$2 AND status='streaming'`, so it matches 0 rows. That counts as 'superseded', not a DB error, so the 30 s retry never runs. A fully generated answer the task still holds in memory is thrown away. NFR-11 ('a completed node is never lost after the stream ends') does not hold, and the failure table claims the final-write retry covers Postgres blips.

**Recommendation:** Let the owning task reclaim a node that was swept but not retried. Make the final write `WHERE id=$1 AND attempt=$2 AND (status='streaming' OR (status='error' AND error_code='interrupted'))`. A retry bumps attempt, so single-writer still holds. Or have the sweeper skip a tick right after the DB recovers. Add a test: pause the DB for 70 s mid-stream, resume, and assert the node ends `complete`.

## F-architecture-L3-3: Prefix-sum token counting on canvas load can't equal assemble() once compaction, truncation, summary caps or mixed tokenizers apply

AC-12.1 requires each node's counter to equal the token count from the same function the context builder uses. The canvas-load pass adds per-message counts down the DFS. That holds only when the whole path fits. Once a path exceeds 14k, assemble() swaps root..C for a summary, middle-truncates oversized messages, caps merged summaries at 15%, and adds framing tokens. None of these are additive. Main and doubt modes can also use different tokenizers ('each node is counted with its own mode's tokenizer'), so a branch node can't reuse the main-mode prefix sum of its ancestors. The doc then states the counter and send path match, which this method can't deliver for long or branched paths.

**Recommendation:** Use prefix sums only as a fast path, and only when the summed raw path fits the budget and the node's mode tokenizer matches the prefix's. Otherwise call assemble() in dry-run for that node. Keep per-tokenizer prefix sums. Extend the AC-12.1 test to an over-budget path and to a branch whose doubt tokenizer differs from main's.

## F-architecture-L3-4: Daily token cap and llm_usage miss calls that error, time out or lose their stream

The gateway writes 'one row per LLM call' from provider-reported usage. That usage normally arrives only at the end of a stream. Three cases are not covered: streams that break after tokens (no automatic retry, status=error), calls that hit the 5-min, first-token or gap timeouts, and compaction calls that time out at 30 s and fall back. All of these are billed by the provider and may write no llm_usage row. The cap sums llm_usage, so a user can keep spending through retries of failing or long answers without being capped. The '≤ 48k overshoot' bound and NFR-12 cost accounting both rely on every call writing a row.

**Recommendation:** Always write an llm_usage row, success or failure. When the provider sends no usage, use the locally counted prompt tokens plus the tokens streamed so far, and mark the row estimated. Add an `estimated bool` column, and a test that an errored stream still produces a row.

## F-architecture-L3-5: Rate-limit and daily-cap checks aren't ordered against the send transaction

Steps 2-3 commit the user_message, a streaming ai_message and two edges, then spawn the task. 'The check runs before each call', which puts the cap check inside the task, after the commit. A capped or rate-limited user therefore gets persistent nodes plus an error node, instead of a clean 429 with no rows written. The client also keeps the typed text only 'until created arrives'. Retry is worse: its compare-and-set has already set content='' (wiping the partial answer AC-18.1 kept) before any limit is checked. The doc also doesn't say whether a 4th concurrent call queues on the per-user semaphore or is rejected.

**Recommendation:** Check the rate-limit token bucket and the daily cap in the handler before the transaction (send) or before the compare-and-set (retry), and return 429 with no DB writes. Keep the per-call check in the task only for compaction. State whether the semaphore queues or rejects. Add an API test: a capped user's send returns 429 and creates no nodes.

## F-architecture-L3-6: The account_deletions re-apply step can't recover deletions made after the restored backup

The doc says the account_deletions row 'survives in later backups, so a restore can re-apply deletions made after the backup was taken'. The backup being restored was taken before those deletions, so it doesn't contain their rows. They exist only in the live database (or a newer backup). The usual reason to restore is losing or corrupting that live database, and then the rows are gone too, so deleted users come back. This undermines FR-20 and the privacy notice's 7-day promise.

**Recommendation:** Make the restore runbook explicit. Before restoring, export account_deletions from the live DB or the newest available backup (or PITR). Better, also record each deletion user_id outside the database (an append-only structured log line or object storage) so the list survives database loss. Add the re-apply step to the monthly restore test.

## F-architecture-L3-7: Behavior when a message is sent inside an already-merged branch is undefined

Merging sets merge_summary and collapsed=true, but nothing blocks later sends to nodes in that branch, or an expand-then-continue flow. New messages in the branch aren't reflected in the summary that downstream prompts receive (AC-14.1). PUT merge is described as 'naturally idempotent' but overwrites the summary and writes another merge_confirmed event, which double-counts the 'Merge use' and 'Merge edits' metrics. Merge-proposal on a branch that still has a streaming node summarizes partial text.

**Recommendation:** Pick one rule and enforce it on the server. Option 1: reject sends into a merged branch (409) until the user unmerges. Option 2: clear merged_at/merge_summary on the first new send in the branch. Reject merge while any branch node is streaming (422 parent_not_complete). Write merge_confirmed only on the first merge, or record re-merges as a separate kind.

## F-architecture-L3-8: Dropping merged summaries past the 15% cap silently breaks AC-14.1 and prefix stability

Past the cap, older summaries are dropped and the closest ones kept. AC-14.1 says the downstream prompt contains the summary. As the path grows, the set of kept summaries shifts, which changes bytes early in the prompt (on origin messages) and contradicts the byte-stable prefix claim (AC-10.4). The only signal is a log line, and the user isn't told.

**Recommendation:** Fold summaries that are over the cap into the compaction summary, or into one combined 'earlier branches' block, instead of dropping them. At minimum, flag the node's token counter as 'summaries_capped' in the UI. Add a test with enough merges to hit the cap.

## F-architecture-L3-9: `done`/`error` delivery and the SIGTERM 'mark remaining nodes' scope are underspecified

Deltas use try_send and may be dropped. If `done` or `error` goes through the same full channel, the client never learns the node has settled and keeps showing a stream that is still open. On SIGTERM the server 'marks the remaining nodes error'. If that is a broad `WHERE status='streaming'` update rather than one scoped to this instance's own (id, attempt) pairs, it kills the new instance's live streams while the two instances overlap during a deploy.

**Recommendation:** Send terminal events with an awaited send that has a short timeout, then close the channel. Mark only the nodes owned by this process's tasks, using `WHERE id = ANY($ids) AND attempt = ANY(...)` per task (or have each task mark its own node on cancellation).

## F-architecture-L3-10: Signup reveals whether an account exists through timing, and the CSRF Content-Type rule conflicts with bodyless mutations

Signup returns the same 403 for 'already registered' and 'not allowlisted'. For an allowlisted email, though, the existence check runs before argon2 hashing, so response time still shows whether the account exists. Login has a dummy-hash defense, but signup has none. Separately, 'mutating requests must have Content-Type: application/json', yet POST /api/auth/logout and POST /api/nodes/:id/retry have no body. The rule will either break them or get exempted ad hoc.

**Recommendation:** Run argon2 hashing (or the dummy verify) on every signup path before returning. Base CSRF protection on the Origin check for all mutating methods, and require application/json only when a body is present, as an explicit rule.
