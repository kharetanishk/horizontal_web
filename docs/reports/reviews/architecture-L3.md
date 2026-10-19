# Level 3 review: architecture

- Generated: 2026-10-19T14:20:30+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | reset-password and accept-invite leave the user's other invite tokens valid | docs/ARCHITECTURE.md Auth (lines ~331-347); invites table |
| F-architecture-L3-2 | medium | Retry bypasses the canvas lock, so merge's branch_streaming check and the 'exact' staleness claim break | docs/ARCHITECTURE.md Data model 'Merges are snapshots'; API POST /api/nodes/:id/retry |
| F-architecture-L3-3 | medium | Fixed allotments mix tokenizers and leave no room for wrapper tags, so S and merged summaries get truncated | docs/ARCHITECTURE.md Context builder step 3 table, step 7; Config COMPACTION_SUMMARY_MAX_TOKENS, MERGE_BUDGET_TOKENS |
| F-architecture-L3-4 | medium | Nested branches: only one anchor slot is specified, and placement and budget of the other anchors are undefined | docs/ARCHITECTURE.md Context builder step 3 (Anchor 1,500), step 5 layout ('the anchor, then the new message'), step 6 |
| F-architecture-L3-5 | medium | Postgres error DETAIL can put message content into app logs (NFR-9) | docs/ARCHITECTURE.md Send-message flow step 8 (permanent error → persist_failed); Observability allowlist |
| F-architecture-L3-6 | medium | NULL completion_tokens silently drops a settled row from the daily cap | docs/ARCHITECTURE.md Metering SQL; llm_usage CHECK (settled_at IS NULL OR prompt_tokens IS NOT NULL) |
| F-architecture-L3-7 | low | NFR-13 ('model switch is config only') contradicts the compile-time r_up/r_down table | docs/ARCHITECTURE.md Context builder 'Tokenizer ratios'; Config section last line |
| F-architecture-L3-8 | low | The deploy-overlap claim depends on the host's deploy strategy, which isn't listed as an open question | docs/ARCHITECTURE.md Graceful shutdown step 2 ('The new instance serves new requests meanwhile'); Deployment Host |
| F-architecture-L3-9 | low | An 8,000-char new message can exceed its fixed 3,000-token slot and be silently middle-truncated | docs/ARCHITECTURE.md API message validation (content 1–8,000 chars); Context builder step 3 (New message 3,000) |
| F-architecture-L3-10 | low | Boot-time compaction re-enqueue burns caps and provider cost in a crash loop | docs/ARCHITECTURE.md Context builder 'Precompute' (boot re-enqueue); Stream engine sweeper ('A crash loop therefore doesn't burn users' caps') |
| F-architecture-L3-11 | low | Client-chosen ids make 409 a cross-tenant existence oracle, contrary to NFR-8 | docs/ARCHITECTURE.md API POST /api/canvases {id?}, messages {user_node_id, ai_node_id}; 'Idempotency' |
| F-architecture-L3-12 | low | The 'Merge edits' metric relies on the client-supplied `proposed` text | docs/ARCHITECTURE.md API PUT /api/edges/:id/merge {summary, proposed?}; events merge_confirmed |
| F-architecture-L3-13 | low | Startup migrations also need a session advisory lock on a direct connection | docs/ARCHITECTURE.md Data model 'Migrations'; Deployment ('The advisory-lock connection needs a direct connection') |

## F-architecture-L3-1: reset-password and accept-invite leave the user's other invite tokens valid

`api reset-password` 'issues a new invite link', clears the hash and deletes sessions, but nothing deletes the user's existing unused `invites` rows. Accept-invite marks only the presented token as used. The only `DELETE FROM invites` anywhere is in the restore runbook. Scenario: the first link leaks or is sent to the wrong address, so the operator runs reset-password. The leaked link stays valid for up to 7 days. Whoever holds it can set a password and take over the account, including after the real user has already onboarded. The single-use check is also unspecified: if accept-invite validates the token, hashes, and only then writes, two concurrent accepts can both succeed.

**Recommendation:** In the same transaction, both `reset-password` and accept-invite must run `DELETE FROM invites WHERE user_id = $u` (other tokens). Accept-invite must consume the token with a CAS: `UPDATE invites SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING user_id`, inside the transaction that sets the hash. Add tests for an old link after a reset and for two concurrent accepts.

## F-architecture-L3-2: Retry bypasses the canvas lock, so merge's branch_streaming check and the 'exact' staleness claim break

Staleness is called exact because 'Sends take the same lock' and subtrees only grow. Retry is not a send. Its admission CAS (`UPDATE nodes … WHERE status='error'`) does not lock the canvas row, and it rewrites an existing node's content without adding a node. Two problems follow. (1) TOCTOU: a merge confirm checks 'no node in the subtree is streaming' under the canvas lock, then a retry flips an error node in that subtree to streaming, and the merge commits a summary of text that is being replaced. (2) Retrying an error node inside an already-merged branch changes its content, but `subtree size > merged_node_count` stays false. The downstream summary silently describes text that no longer exists, and the UI never offers 'update summary'.

**Recommendation:** Make the retry admission take the same `UPDATE canvases … RETURNING` row lock as sends (user, then canvas). Either reject merges of subtrees containing `error` nodes, or bump a per-edge content version on retry and compute `stale` from it as well as the node count.

## F-architecture-L3-3: Fixed allotments mix tokenizers and leave no room for wrapper tags, so S and merged summaries get truncated

The table says 'everything is in the mode tokenizer, and every fixed part includes its own wrapper tags'. Yet the summary allotment is 1,000 and `COMPACTION_SUMMARY_MAX_TOKENS` (the summary model's output limit, counted in its own tokenizer) is also 1,000. A maximum-length S plus `<conversation_summary>` tags therefore exceeds its slot, and so does any S re-counted by a mode tokenizer with r_up > 1. The rule 'an oversized piece is middle-truncated' then cuts the middle of S on every send. `merge_tokens` is likewise a single int checked against `MERGE_BUDGET_TOKENS` at confirm time. That is one tokenizer, while the main and doubt modes may use different ones, so 2,000 tokens counted under one can exceed 2,000 under the other (×1.26 = 2,520). Merged summaries then get middle-truncated, which contradicts 'Merged summaries are never dropped … every accepted summary reaches every downstream prompt' (AC-14.1).

**Recommendation:** Set the summary `max_tokens` to ⌊(1,000 − wrapper) / max r_up across modes⌋. Store `merge_tokens` in the canonical tokenizer and enforce the cap as `MERGE_BUDGET_TOKENS / max r_up`, or store a per-tokenizer value and check every mode. Add both cases to the startup inequality check and to the proptest (allotment never truncates S or a merged summary).

## F-architecture-L3-4: Nested branches: only one anchor slot is specified, and placement and budget of the other anchors are undefined

AC-9.1 requires a second-level branch's path to include both branch origins, and AC-10.2 requires B's anchor in prompts sent deeper in the branch. The layout reserves one anchor slot (1,500) placed just before the new message. For a third-level node, the path crosses 2–3 branch edges, each with an anchor. The design doesn't say which anchor fills the slot, whether outer anchors are inlined at their origin, or which budget they come from. If they are inlined, the 5,700 tail inequality no longer bounds the tail. If they are omitted, AC-10.2-style context is lost for nested doubts.

**Recommendation:** Specify it. For example: each branch edge's anchor is inlined after its source message in the tail, with a per-anchor cap counted inside `ctok` at node completion so the block bound still holds; or there is one slot for the innermost anchor and outer anchors are capped at N tokens from a named allotment. Add a proptest over 3-level branches asserting that every anchor on the path appears.

## F-architecture-L3-5: Postgres error DETAIL can put message content into app logs (NFR-9)

Constraint violations such as `CHECK (char_length(content) <= 100000)` and the compaction_summary CHECK return SQLSTATE 23514 with `DETAIL: Failing row contains (…)`. That detail carries the full row, including content and summaries. The permanent-error path ('any other error is permanent') will naturally log the sqlx error, and `Display` of `sqlx::Error` / `PgDatabaseError` includes the message, and the detail is accessible on the same error. The managed host's Postgres server log, often shipped to the same drain, also records the DETAIL. The planned NFR-9 test only checks the normal send path, not DB error paths.

**Recommendation:** Log DB errors only as SQLSTATE plus constraint name, never via Display or Debug of sqlx errors. Add an NFR-9 test that forces a constraint violation with a marker string and asserts the marker is absent from logs. Set the managed Postgres `log_error_verbosity = terse` and avoid parameter logging, or document the server-log exposure.

## F-architecture-L3-6: NULL completion_tokens silently drops a settled row from the daily cap

The cap sums `prompt_tokens + completion_tokens` for settled rows. The CHECK only requires `prompt_tokens`, so a row settled with `completion_tokens` NULL makes the addition NULL, and `sum()` skips it. That row then contributes 0 to `spent_today`. Likely settlement paths have no completion count: a sweeper-settled compaction is 'charged its locally counted input', and a failed compaction or merge proposal has no output. Each one quietly refunds the cap.

**Recommendation:** Use `CHECK (settled_at IS NULL OR (prompt_tokens IS NOT NULL AND completion_tokens IS NOT NULL))`, settle with an explicit 0, and/or use `coalesce(completion_tokens, 0)` in the sum. Add a test that settles a row with no completion and asserts it still counts toward the cap.

## F-architecture-L3-7: NFR-13 ('model switch is config only') contradicts the compile-time r_up/r_down table

`r_up`/`r_down` are compile-time constants per model, and 'Startup refuses unknown models'. Switching a mode to any model not already in the compiled table therefore needs a code change, a fixture run and a release, yet the Config section claims 'Changing the model for a mode is a config change only (NFR-13)'.

**Recommendation:** Either narrow NFR-13 to 'switching among models in the compiled allowlist', or move the ratio table into config and validate it at startup against a bundled fixture corpus.

## F-architecture-L3-8: The deploy-overlap claim depends on the host's deploy strategy, which isn't listed as an open question

The claim that a deploy never cuts an answer and that new requests are served during the 270 s drain assumes the old and new instances overlap. On Fly with a single Machine, the default rolling strategy stops the old Machine before starting the new one. That means either the drain runs with no instance serving new requests (about 4.5 minutes of 503s on every push to main) or no overlap at all. The open-questions table only covers routing on readiness failure.

**Recommendation:** Add 'deploy strategy gives overlap (Fly bluegreen / Render zero-downtime / Railway overlap)' to the open questions, and make it a host-selection criterion. Otherwise document the per-deploy unavailability against NFR-10.

## F-architecture-L3-9: An 8,000-char new message can exceed its fixed 3,000-token slot and be silently middle-truncated

In CJK or emoji-heavy text, 8,000 code points can be well over 3,000 o200k tokens. The quick-action template and anchor wrapper share that slot. 'An oversized piece is middle-truncated', so the model never sees the middle of the user's question, and nothing tells the user.

**Recommendation:** Reject at the boundary (422) when the tokenized new message plus its template exceeds the allotment, and show a counter or warning in the composer. Or derive the char limit from the token slot.

## F-architecture-L3-10: Boot-time compaction re-enqueue burns caps and provider cost in a crash loop

Every boot re-enqueues all `is_checkpoint AND compaction_summary IS NULL` checkpoints on canvases active in the last 24 h and starts metered summary calls. If the process crashes before they finish, the sweeper charges each one its counted input with the provider already billed, and the next boot repeats this. The crash-loop claim only covers answers.

**Recommendation:** Delay the boot re-enqueue (e.g. 60 s of uptime) or back it off per checkpoint (store an attempt count or last-attempt time). Narrow the claim in the doc.

## F-architecture-L3-11: Client-chosen ids make 409 a cross-tenant existence oracle, contrary to NFR-8

A repeat insert hits the PK and returns 409. Submitting another user's canvas or node UUID therefore returns 409 rather than 201, which confirms the id exists. NFR-8 says other users' ids must look like 404/absent. UUIDv7 ids are hard to guess, but ids do leak (screenshots, support, logs), and the server never checks that client ids are actually v7.

**Recommendation:** On a PK conflict, return 409 only if the existing row belongs to the caller, else a generic 409 that doesn't distinguish. Or use `ON CONFLICT DO NOTHING` plus an ownership re-check. Validate the UUID version, and include it in the NFR-8 tests.

## F-architecture-L3-12: The 'Merge edits' metric relies on the client-supplied `proposed` text

The proposal is 'not stored', so the proposed-vs-confirmed comparison uses whatever the client sends. A client bug or a stale dialog corrupts the success metric, and an omitted `proposed` is indistinguishable from 'typed manually after a 502'.

**Recommendation:** Store the last proposal server-side (e.g. on the edge, or as a short-lived row keyed by edge) and compare against it at confirm.

## F-architecture-L3-13: Startup migrations also need a session advisory lock on a direct connection

`sqlx::migrate!` takes `pg_advisory_lock` on a session. Through transaction-mode PgBouncer that lock isn't held reliably, so two overlapping instances could both run migrations. Only the liveness-lock connection is routed through `DATABASE_URL_DIRECT`.

**Recommendation:** Run migrations over `DATABASE_URL_DIRECT` as well, and state it in the doc.
