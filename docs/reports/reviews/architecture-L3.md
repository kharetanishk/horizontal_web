# Level 3 review: architecture

- Generated: 2026-10-19T14:11:52+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Reservations for deleted nodes may never settle, so in-flight slots leak and the user gets locked out | docs/ARCHITECTURE.md:251-252 ("A stream task whose node is deleted matches no row and stops"), :106 (sweeper settles an answer from assemble(parent) + flushed content), Metering in_flight query |
| F-architecture-L3-2 | medium | Attaching to a stream from the in-memory registry has no stated ownership check | docs/ARCHITECTURE.md:93-98 (Attach), API row GET /api/nodes/:id/stream |
| F-architecture-L3-3 | medium | A merge racing a send into the branch can miss a message and never flag it stale | docs/ARCHITECTURE.md Data model 'Merges are snapshots' (stale = exists(subtree node created after merged_at)); merge confirm 422 branch_streaming |
| F-architecture-L3-4 | medium | The PITR restore path doesn't re-apply deletions made after the restore point | docs/ARCHITECTURE.md Restore runbook steps 1-4 |
| F-architecture-L3-5 | medium | On Fly, a 290 s drain is shorter than the 300 s whole-answer timeout, so deploys can still cut answers | docs/ARCHITECTURE.md:117 (DRAIN_SECS = whole-answer timeout, 'never cuts a normal answer'); Deployment health checks (DRAIN_SECS=290 on Fly) |
| F-architecture-L3-6 | low | The answer reservation may not cover max_tokens output | docs/ARCHITECTURE.md Metering ('an answer reserves the context budget plus a 5% margin (16,800)'); send step 6 (max_tokens = REPLY_RESERVE_TOKENS) |
| F-architecture-L3-7 | low | Progressive delays per email and the global auth bucket let others deny logins | docs/ARCHITECTURE.md API Rules > Rate limits |
| F-architecture-L3-8 | low | Password length has no upper bound | docs/ARCHITECTURE.md API signup row (password ≥ 12 chars); DELETE /api/me {password} |
| F-architecture-L3-9 | low | Canvas GET writes lazily-filled token counts back to the database | docs/ARCHITECTURE.md Token counter ('written back lazily for any missing id') |

## F-architecture-L3-1: Reservations for deleted nodes may never settle, so in-flight slots leak and the user gets locked out

llm_usage.node_id deliberately has no FK, so a reservation outlives canvas deletion. When the canvas is deleted mid-stream, the fenced flush matches no row and the task 'stops'. Nothing says it still settles its reservation. The owner is alive, so the sweeper never touches that row. If the instance later dies, the sweeper settles an answer from `assemble(parent)` and the last flushed content, and both are gone. The doc gives no fallback for that case, so the sweep transaction can error on every 30 s pass. The in_flight count includes every row with `settled_at IS NULL` regardless of day. Scenario: a user deletes 3 canvases while answers are streaming, and from then on every send, retry or merge proposal returns 429 (`in_flight >= 3`).

**Recommendation:** State that a fenced write that matches no row still settles its own reservation with the local estimate. For a missing node, the sweeper should fall back to `reserved_tokens` (or the locally counted completion only). As a backstop, exclude reservations older than the whole-answer timeout plus margin from `in_flight`. Add a test: delete the canvas mid-stream, then assert the reservation is settled and in_flight = 0.

## F-architecture-L3-2: Attaching to a stream from the in-memory registry has no stated ownership check

Attach sends a `snapshot` with all text so far straight from `Mutex<HashMap<NodeId, Arc<Stream>>>`. The `Stream` struct holds only attempt, text and watch: no user_id or canvas_id. The ownership rule says 'every query joins through canvases.user_id', but a registry hit is not a query. An implementation that checks the registry first would stream another user's answer to anyone holding the node UUID. UUIDs are not secret: they are client-generated and appear in logs (node_id is an allowed log field). This breaks NFR-8.

**Recommendation:** Either do the owned-node DB lookup (404 otherwise) before any registry or poller access, or store `user_id` in `Stream` and compare it. Name GET /api/nodes/:id/stream explicitly in the NFR-8 per-endpoint tests, with a live registry entry belonging to another user.

## F-architecture-L3-3: A merge racing a send into the branch can miss a message and never flag it stale

Sends don't take the canvas row lock. Only merges do. Under READ COMMITTED, a send transaction that started before the merge transaction but has not committed is invisible to the merge's `branch_streaming` check. Its nodes get `created_at = now()` (transaction start), which is earlier than `merged_at`. After both commit, the branch holds a new Q/A pair that the summary doesn't cover, `stale` is false forever, and the UI never offers 'update summary'. Downstream prompts silently lack that content.

**Recommendation:** Have sends whose parent is inside a branch subtree take `SELECT … FROM canvases WHERE id=$c FOR SHARE` in the admission transaction (merges already take FOR UPDATE). Or compute stale from a monotonic sequence assigned at commit instead of created_at vs merged_at. Add a race test.

## F-architecture-L3-4: The PITR restore path doesn't re-apply deletions made after the restore point

Step 1 says that with PITR 'deletions up to that point are already applied'. Steps 3-4 (collect and re-apply tombstones) are written only for the backup path ('Otherwise…'). Restoring to a point before the incident rolls back every account or canvas deletion made between that point and the restore, which breaks FR-20 / AC-20.1. The same gap exists in step 2 for tombstones whose live `deletions` rows were lost in the incident window.

**Recommendation:** Make tombstone collection and re-application (steps 3-4) unconditional. Use as the cutoff the recovery target time, or the backup time, whichever is older. Extend the monthly drill to the PITR path too.

## F-architecture-L3-5: On Fly, a 290 s drain is shorter than the 300 s whole-answer timeout, so deploys can still cut answers

The claim that a deploy never cuts a normal answer holds only if drain ≥ answer timeout plus the final-write time. On Fly, DRAIN_SECS is 290 against a 300 s answer timeout, so an answer that started within 10 s before SIGTERM and runs long is cancelled. The final write and settlement must also fit inside the kill timeout. That margin is unspecified, and under a slow DB the cancelled task's own error write can be SIGKILLed too.

**Recommendation:** Tie the values together and check them at startup: whole-answer timeout + final-write margin ≤ DRAIN_SECS < platform kill timeout. For example, on Fly use a 270 s answer timeout with DRAIN_SECS=285. Or downgrade the claim to 'answers shorter than DRAIN_SECS'.

## F-architecture-L3-6: The answer reservation may not cover max_tokens output

The reservation is `CONTEXT_BUDGET_TOKENS × 1.05` = 16,800. If the 16,000 budget is input-only and the 2,000 reply is extra, a call can bill up to about 18,800 tokens, and the cap is exceeded by about 2k per in-flight call. The excerpt available doesn't show whether the budget includes the reply reserve. The compaction and merge reservations count output explicitly, but the answer reservation doesn't.

**Recommendation:** Write the answer reservation as (input_budget × 1.05) + REPLY_RESERVE_TOKENS, or state that CONTEXT_BUDGET_TOKENS already includes the reply reserve. Add the reservation ≥ max billable check to the startup assertions.

## F-architecture-L3-7: Progressive delays per email and the global auth bucket let others deny logins

Failed-login delays are applied 'per email' as well as per (email, IP). Anyone who knows a user's email can keep that user's own logins delayed. The global 120/min auth bucket can be exhausted from about 12 IPs at 10/min each, which turns every user's login and signup into 503. The doc accepts the distributed case but not the targeted per-email one.

**Recommendation:** Apply the per-email delay only to IPs with no recent successful login for that account, or cap the per-email delay low (≤ 2 s). Keep the per-(email, IP) delay. Exempt requests that carry a valid session from the global bucket.

## F-architecture-L3-8: Password length has no upper bound

Only a minimum length is set, so the 1 MB body limit is the effective maximum. Argon2 work grows with input length, and that work sits behind a semaphore of 2. Large passwords make each slot more expensive and lengthen the 2 s queue for legitimate users.

**Recommendation:** Cap passwords at 128-1,024 code points in the serde type (400 beyond).

## F-architecture-L3-9: Canvas GET writes lazily-filled token counts back to the database

GET /api/canvases/:id can issue writes for up to 1,000 nodes when a tokenizer id is missing. That happens after any model config change, and the doc doesn't say whether the write is in the request path or bounded. Concurrent loads race on the same rows, and a read-only failure such as the DB in recovery would fail the GET. That works against NFR-3, which is measured on a cold process.

**Recommendation:** Return the computed counts, and write them back in a single bounded background `UPDATE … FROM unnest` that ignores failures. Or fill the counts once at startup when the model config changes.
