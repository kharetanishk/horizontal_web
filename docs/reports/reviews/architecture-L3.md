# Level 3 review: architecture

- Generated: 2026-10-19T14:03:35+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Context-budget fit guarantee (NFR-7) is arithmetically inconsistent and has no runtime backstop | docs/ARCHITECTURE.md, Context builder steps 3–4 (Fixed allotments, Fit guarantee, Cutoff) |
| F-architecture-L3-2 | medium | Per-canvas merge budget makes merge permanently unavailable on long canvases | docs/ARCHITECTURE.md, Context builder step 7; PUT /api/edges/:id/merge |
| F-architecture-L3-3 | medium | NFR-6 (±5% counter accuracy) is silently weakened for non-local tokenizers | docs/ARCHITECTURE.md, Token counter; Open questions (Providers/models per mode) |
| F-architecture-L3-4 | medium | POST /api/events has no stated ownership check or rate limit | docs/ARCHITECTURE.md, API table (POST /api/events); NFR-8 |
| F-architecture-L3-5 | medium | Delete behaviour of `llm_usage.node_id` is unspecified; a cascade would wipe the cost ledger and reset the daily cap | docs/ARCHITECTURE.md, Data model (llm_usage row); Metering |
| F-architecture-L3-6 | medium | Deploy-on-every-push with a 20 s drain routinely interrupts in-flight answers | docs/ARCHITECTURE.md, Stream engine (Graceful shutdown); Deployment (Health checks, Build and ship) |
| F-architecture-L3-7 | low | Hitting max_tokens (finish_reason=length) is not handled | docs/ARCHITECTURE.md, Send-message flow step 6; Context builder (Path message cap) |
| F-architecture-L3-8 | low | Merge-proposal input over a subtree is unbounded and unspecified | docs/ARCHITECTURE.md, API (POST /api/edges/:id/merge-proposal) |
| F-architecture-L3-9 | low | Compaction precompute is global, small and non-durable, so on-demand chains land on the critical path after deploys | docs/ARCHITECTURE.md, Context builder (Precompute, off the critical path) |
| F-architecture-L3-10 | low | TRUSTED_IP_HEADER can be spoofed unless the doc pins which header and how it is parsed | docs/ARCHITECTURE.md, API Rules (Rate limits) |

## F-architecture-L3-1: Context-budget fit guarantee (NFR-7) is arithmetically inconsistent and has no runtime backstop

Step 3 gives the compaction summary its own fixed 1,000-token allotment inside the 8,300, which leaves 5,700 for the tail. Step 4 then picks j so that `S(ckⱼ)` plus the messages after it fit the tail allotment, which counts S a second time. If S really counts against the tail, the claimed fit breaks: 5,174 + 1,000 = 6,174 > 5,700. With an estimated tokenizer, every count is also multiplied by TOKEN_SAFETY_FACTOR (1.15) on top of TOKENIZER_SLACK (1.15): 4,499 × 1.15 × 1.15 ≈ 5,950 > 5,700. With default config the startup check would then refuse to boot for any non-local-tokenizer provider, and that provider choice is still open. `ctok` counts only content. Per-message chat framing, the `\n\n---\n\n` separators and the `<conversation_summary>`/`<merged_branch>` wrappers are in no allotment. A block can legally hold hundreds of tiny messages (it is bounded by tokens, not by message count), so framing overhead is unbounded relative to the 5,700 tail. TOKENIZER_SLACK = 1.15 between o200k and another provider's tokenizer is unmeasured for CJK and emoji. Nothing says what `assemble` does if the inequality fails at runtime.

**Recommendation:** Write a single explicit inequality that says whether S lives in the fixed part or the tail, includes both factors and a per-message framing bound (cap messages per block, or add k tokens per message to ctok), and recompute the defaults so it holds for both tokenizer classes. Then add a final hard check after assembly in the mode tokenizer: if it is over budget, drop the oldest tail messages or middle-truncate until it fits, and log `budget_enforced`. Property tests should use a block of many 1-token messages and a tokenizer pair at the slack limit.

## F-architecture-L3-2: Per-canvas merge budget makes merge permanently unavailable on long canvases

The total `merge_tokens` per canvas is capped at 2,000. A 200-char summary is roughly 40–60 tokens in English and can reach 200+ in CJK or emoji. A canvas therefore allows about 30–40 merges, or about 10 with CJK, and after that every merge returns 422 `merge_budget_exhausted`, even for a branch whose downstream paths carry no other summaries. Only the summaries on one path ever enter a prompt, so the per-canvas cap is far more restrictive than the budget needs. This conflicts with FR-13/FR-14 and the ≥ 30% merge-use target on the 1,000-node canvases the NFRs assume.

**Recommendation:** Enforce the cap per path. At confirm time, lock the canvas and check the maximum merged-summary tokens over all root-to-leaf paths through the origin, a CTE over the origin's ancestors plus the subtree. Alternatively, fold summaries whose origin lies before ckⱼ into the next S(ck), so they stop consuming the merge allotment.

## F-architecture-L3-3: NFR-6 (±5% counter accuracy) is silently weakened for non-local tokenizers

For non-OpenAI providers the architecture uses a 'calibrated approximation… marked estimated'. NFR-6 and AC-12.2 require ±5% with no exception, and the provider choice is still open. If the owner picks Anthropic, NFR-6 can't pass and the gate test AC-12.2 fails, yet no requirement change or ADR records the trade-off. The same factor also feeds the metering cap ('holds within TOKEN_SAFETY_FACTOR'), so the 'exact' daily cap stops being exact.

**Recommendation:** Either make a local-tokenizer model a hard constraint for phase 1 (record it in an ADR and the privacy notice), or amend NFR-6/AC-12.2 through the requirements process before build, for example by using the provider's count-tokens endpoint off the critical path for calibration.

## F-architecture-L3-4: POST /api/events has no stated ownership check or rate limit

The endpoint takes a client-supplied `canvas_id` and returns 204. The doc lists ownership checks for canvas, node, edge and stream endpoints but not this one. Without the check, any user can insert `back_to_main`/`collapse` events against another user's canvas. That corrupts the success metrics (return-to-main) that gate phase 2. The FK-violation-versus-204 difference also leaks whether a canvas id exists. The endpoint is also outside the LLM and auth rate limits, so the `events` table can grow without bound per user.

**Recommendation:** Join `canvases.user_id = $me` and return 404 otherwise (add it to the NFR-8 per-endpoint test list). Apply a per-user token bucket, for example 60/min, and cap the payload size.

## F-architecture-L3-5: Delete behaviour of `llm_usage.node_id` is unspecified; a cascade would wipe the cost ledger and reset the daily cap

Only `user_id` has a declared FK. If an implementer adds the natural `node_id → nodes ON DELETE CASCADE`, deleting a canvas (AC-2.3 cascades nodes) deletes that day's settled usage rows. `spent_today` then drops and the user regains the full DAILY_TOKEN_CAP, which bypasses the cost control. NFR-12 cost records are lost too. Deleting a canvas mid-stream would also delete the open reservation the sweeper relies on.

**Recommendation:** Declare `node_id` with no FK, or `ON DELETE SET NULL`, explicitly in the data model. Add an integration test: spend tokens, delete the canvas, and assert the cap still refuses.

## F-architecture-L3-6: Deploy-on-every-push with a 20 s drain routinely interrupts in-flight answers

Answers allow 2,000 output tokens and a 5-minute timeout, so long explanations commonly stream for more than 20 s. Every push to main deploys, and after DRAIN_SECS = 20 every remaining stream is cut to `interrupted`. The user has to Retry and pays for a second full reservation, even though the design already claims to be multi-instance safe during overlap. During a DB outage at deploy time, the final write can't land: a fully generated answer reverts to its last flushed text (`final_write_abandoned`). That contradicts NFR-11's 'completed node is never lost'.

**Recommendation:** Set DRAIN_SECS to at least the realistic maximum answer duration (e.g. 120 s) with a matching platform kill timeout (Fly allows up to 300 s), relying on overlap. Alternatively, batch deploys outside active sessions. Document the DB-outage-plus-SIGTERM loss as an explicit NFR-11 exception.

## F-architecture-L3-7: Hitting max_tokens (finish_reason=length) is not handled

The LLM is called with `max_tokens = REPLY_RESERVE_TOKENS` (2,000), but no behaviour is defined when the provider stops on length. The answer ends mid-sentence with `status = complete` and no UI signal, unlike the `truncated` path for MAX_NODE_CHARS. Also, max_tokens is enforced in the mode tokenizer while `ctok`/PATH_MSG_MAX_TOKENS use o200k. A 2,000-token reply in another tokenizer can exceed 2,000 canonical tokens and will be silently middle-truncated in later prompts, contrary to the claim that AI replies are 'already below this'.

**Recommendation:** Map finish_reason=length to `error_code = 'truncated'` (status complete) with a 'continue' affordance. Either set PATH_MSG_MAX_TOKENS ≥ REPLY_RESERVE_TOKENS × TOKENIZER_SLACK, or drop the claim.

## F-architecture-L3-8: Merge-proposal input over a subtree is unbounded and unspecified

'The subtree goes through the same truncation path' doesn't define anything: the truncation and checkpoint machinery is defined for root-to-node paths, not for a branch subtree with sub-branches. Nothing says which messages are included or in what order, how input is bounded against the summary model's window, or what the reservation is. A large branch could overflow the model's window or the reservation.

**Recommendation:** Specify the input: for example, the branch's main path plus its existing nested merge summaries, capped at N tokens with drop-oldest. State the reservation size and add a property test that keeps the input within the cap.

## F-architecture-L3-9: Compaction precompute is global, small and non-durable, so on-demand chains land on the critical path after deploys

One global semaphore of 2 is shared by all users, the queue is lost on every restart (which happens on every push to main), and S(k) depends on S(p) recursively. After a restart, a send may need several sequential summary calls inside a single 30 s on-demand window. It then falls back to drop-oldest, which loses context and breaks AC-10.4 prefix stability for that send, and TTFT (NFR-2) degrades.

**Recommendation:** At boot, enqueue missing summaries for checkpoints on recently active canvases (one query: `is_checkpoint AND compaction_summary IS NULL`). Use a per-user fair queue instead of a single global FIFO, and bound the on-demand chain length explicitly.

## F-architecture-L3-10: TRUSTED_IP_HEADER can be spoofed unless the doc pins which header and how it is parsed

The auth rate limits and login backoff key on the client IP from TRUSTED_IP_HEADER, and production only checks that the header is configured. If it is set to `X-Forwarded-For` and the client-supplied leftmost value is used, or the PaaS appends to the header rather than overwriting it, an attacker can rotate the IP freely and bypass the per-IP login/signup limits.

**Recommendation:** Require a header the platform overwrites (e.g. Fly-Client-IP), or parse XFF from the right with a configured hop count. Add a test that a client-supplied header value doesn't change the rate-limit key.
