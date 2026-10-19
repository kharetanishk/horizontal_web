# Level 3 review: architecture

- Generated: 2026-10-19T14:15:12+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Allowlisted accounts can be taken by anyone who knows the email (no proof of ownership) | docs/ARCHITECTURE.md Auth (signup), API `POST /api/auth/signup` |
| F-architecture-L3-2 | high | Restore runbook revives revoked sessions and old passwords | docs/ARCHITECTURE.md Deployment → Restore runbook steps 3–6; Auth (logout, `api reset-password`) |
| F-architecture-L3-3 | medium | Account deletion cascades the `llm_usage` ledger, which resets the daily token cap and erases cost records | docs/ARCHITECTURE.md Data model `llm_usage` (FK `user_id → users ON DELETE CASCADE`) |
| F-architecture-L3-4 | medium | Drain duration contradicts itself across sections (270 s vs 20 s vs 300 s) | docs/ARCHITECTURE.md Stream engine → Graceful shutdown step 2 (270 s); Failure handling 'Deploy (SIGTERM)' row ('20 s drain'); 'Process exits while a final write…' row ('300 s drain'); Deployment health checks (kill timeout 280 s) |
| F-architecture-L3-5 | medium | The 'drain always outlasts any answer' bound ignores compaction, the pre-first-token retry and finalizing | docs/ARCHITECTURE.md Graceful shutdown step 2; Failure handling (timeouts, compaction on demand); Stream engine attach ('polling attach closes after 5 min') |
| F-architecture-L3-6 | medium | Concurrent first messages (and any canvas UPDATE during admission) can deadlock under `FOR SHARE` | docs/ARCHITECTURE.md Send-message flow step 3; Data model `canvases` (root_node_id set with `WHERE root_node_id IS NULL`) |
| F-architecture-L3-7 | medium | Device cookie bypasses the global argon2 bucket, which restores the CPU flood path | docs/ARCHITECTURE.md API Rules → Rate limits (device cookie, global auth bucket 120/min, argon2 semaphore of 2) |
| F-architecture-L3-8 | medium | Instance keeps admitting work under its `owner_key` while its liveness lock is lost | docs/ARCHITECTURE.md Stream engine → Instance liveness (lock re-acquire on reconnect), Send-message flow step 3 |
| F-architecture-L3-9 | low | Readiness does not flip on SIGTERM, so the draining instance keeps getting routed sends it rejects | docs/ARCHITECTURE.md Graceful shutdown step 1; Deployment → Health checks |
| F-architecture-L3-10 | low | Metering lock `FOR UPDATE` on `users` blocks FK inserts from login and canvas creation | docs/ARCHITECTURE.md Metering (`SELECT 1 FROM users WHERE id = $u FOR UPDATE`) |
| F-architecture-L3-11 | low | The claim 'the cap can never lock compaction out' is false | docs/ARCHITECTURE.md Metering (startup check reservation ≤ DAILY_TOKEN_CAP/10) |

## F-architecture-L3-1: Allowlisted accounts can be taken by anyone who knows the email (no proof of ownership)

Signup is gated only by `SIGNUP_ALLOWLIST` membership. Nothing checks that the person signing up controls that email: there is no verification, invite token or magic link. Anyone who knows or guesses an allowlisted address (for example a test user's work email) can register it first, choose the password and receive the session. When the real person tries to sign up, they get the generic 403 `signup_unavailable`. With `SIGNUP_OPEN=true`, anyone can register any address. The equal-work design hides which emails exist, but it does nothing about this.

**Recommendation:** Use the simplest fix: drop self-signup for phase 1. Have the operator create users with a CLI (`api create-user <email>`, next to the existing `api reset-password`), which prints a one-time set-password token, or issue single-use invite tokens tied to the allowlisted email. If self-signup stays, require email verification before a session is issued.

## F-architecture-L3-2: Restore runbook revives revoked sessions and old passwords

Tombstones are written only for user and canvas deletions. Logout, session expiry cleanup and `api reset-password`, which deletes sessions and changes the hash (the documented response to a compromised account), leave no tombstone. A PITR or backup restore to before those events brings back the deleted session rows and the old password hash. A stolen cookie that was revoked becomes valid again, and the compromised password works again. Step 6 opens traffic without dealing with this.

**Recommendation:** Add a step before opening traffic: `DELETE FROM sessions;`, which forces everyone to log in again. Also record `password_reset` tombstones (user id plus timestamp) in the `deletions` table and the log drain, and in the drill either re-apply them or flag those users so they must reset their password.

## F-architecture-L3-3: Account deletion cascades the `llm_usage` ledger, which resets the daily token cap and erases cost records

`node_id` was left without an FK so that 'deleting a canvas never refunds the daily cap or erases cost records'. But `user_id` cascades. An allowlisted user can run `DELETE /api/me`, sign up again with the same email (allowed: the row is gone and the email is still allowlisted) and get a fresh `DAILY_TOKEN_CAP`, as many times as they like. Each deletion also wipes the NFR-12 / cost-per-session data the success metrics depend on.

**Recommendation:** Key the ledger by something that survives deletion. One option: drop the FK, keep `user_id` as a plain UUID, and look up the cap by `email_hash` (SHA-256 of the normalized email) stored on `llm_usage`. Another: keep deleted users' ledger rows anonymized and check the cap per email hash at signup and admission. Add a test: delete the account, sign up again, and assert `spent_today` is unchanged.

## F-architecture-L3-4: Drain duration contradicts itself across sections (270 s vs 20 s vs 300 s)

Graceful shutdown sets `DRAIN_SECS=270` and claims 'a deploy on push never cuts a normal answer'. The Failure handling table says a 20 s drain. The NFR-11 exception says a 300 s drain. Someone configuring the platform's kill timeout, or writing the SIGTERM test, from the table will get answers cut at 20 s. That breaks the 'never cuts a normal answer' claim and AC-18.1 expectations.

**Recommendation:** Make `DRAIN_SECS` the only stated value: change the Failure handling rows to reference `DRAIN_SECS` (270 s) and kill timeout ≥ 280 s, and remove the literal 20 s and 300 s.

## F-architecture-L3-5: The 'drain always outlasts any answer' bound ignores compaction, the pre-first-token retry and finalizing

The startup check is `DRAIN_SECS ≥ 240 + 30`. But one stream task can run an on-demand compaction (30 s timeout), then a first attempt that fails at the first-token timeout (10 s connect + 30 s), then a 1–2 s jittered retry, then a full 240 s answer. That is about 312 s, longer than 270 s, plus a final-write retry loop with no limit. Such a task is cancelled and marked `interrupted` during a deploy, which contradicts the stated guarantee. The 5-minute cap on polling attach has the same flaw: it can close before the terminal event, so the client never sees `done`.

**Recommendation:** Give the whole task one deadline (`TASK_DEADLINE` = compaction + 2 × first-token + answer + margin) and derive `DRAIN_SECS`, the polling-attach lifetime and the 15-min reservation safety net from it in the startup check. Otherwise, apply the 4-min timeout to the whole task, compaction and retry included. Make the poller send a final `snapshot` and tell the client to re-attach instead of closing silently.

## F-architecture-L3-6: Concurrent first messages (and any canvas UPDATE during admission) can deadlock under `FOR SHARE`

Admission takes `FOR SHARE` on the canvas row, and then for the first message it runs `UPDATE canvases SET root_node_id … WHERE root_node_id IS NULL`. Two concurrent first sends (double-click, two tabs) both hold the share lock, and each UPDATE waits on the other: classic lock upgrade, Postgres reports 40P01. The doc promises a 409 for this case. 40P01 is classed as transient for final writes but not mapped for admission, so it most likely surfaces as 500 or 503. The same applies if a send bumps `canvases.updated_at`, which the newest-first list implies but the doc never specifies. In that case every pair of concurrent sends on one canvas deadlocks.

**Recommendation:** For a send with no `parent_id`, take `FOR UPDATE` (or `FOR NO KEY UPDATE`) on the canvas instead of `FOR SHARE`. Say explicitly where `updated_at` is bumped: outside the admission transaction, or as the lock itself (`UPDATE canvases SET updated_at = now() … RETURNING`). Merges would then also serialize against it. Map 40P01/40001 in admission to 409 and add a concurrent-first-send test.

## F-architecture-L3-7: Device cookie bypasses the global argon2 bucket, which restores the CPU flood path

The global bucket exists to bound total argon2 work. A login that presents a valid device cookie skips both the global bucket and the per-email delay. The cookie is a bearer value that works from any IP. An attacker who signs in once to their own account can replay the cookie from N IPs, limited only by the per-IP bucket (10/min each). That gives 10N argon2 runs per minute outside the global limit, and it fills the semaphore of 2, so legitimate users get the 503s the design meant to avoid.

**Recommendation:** Let the device cookie skip only the per-email delay. Keep it inside total argon2 accounting by giving device-cookie logins their own small bucket per cookie id (e.g. 5/min) and a reserved semaphore share, instead of no limit.

## F-architecture-L3-8: Instance keeps admitting work under its `owner_key` while its liveness lock is lost

When the dedicated lock connection drops, the instance re-acquires the key 'on reconnect'. If the old server session isn't detected as dead for about 30 s, or about 2 h if the host ignores `tcp_*` settings (an open question), `pg_advisory_lock` blocks for that long. During the gap the instance still inserts `streaming` nodes and reservations with an `owner_key` it doesn't hold. During a deploy overlap the other instance's sweeper will mark them `interrupted` and settle the reservations. That lowers `in_flight`, so the user can run more concurrent calls than the limit of 3. The 'accepted edge case' covers existing streams but not new admissions made while the lock is down.

**Recommendation:** Track lock-held state with an `AtomicBool`. While it is false, return 503 `unavailable` for sends, retries and merge proposals, and fail readiness (`/api/healthz`). Use `pg_try_advisory_lock` in a retry loop instead of the blocking `pg_advisory_lock`.

## F-architecture-L3-9: Readiness does not flip on SIGTERM, so the draining instance keeps getting routed sends it rejects

During the 270 s drain, step 1 returns 503 for new sends and retries. `/api/healthz` only pings the DB, so it stays 200. Whether the platform proxy stops routing to the old instance then depends on platform behaviour, which is not stated. Users can see 503 on send for up to 4.5 min after each push to main (**assumption**: depends on the host's routing).

**Recommendation:** Return 503 from `/api/healthz` as soon as SIGTERM is received, and confirm at host choice that the proxy drains on readiness or SIGTERM.

## F-architecture-L3-10: Metering lock `FOR UPDATE` on `users` blocks FK inserts from login and canvas creation

Inserts into `sessions`, `canvases`, `events` and `llm_usage` take `FOR KEY SHARE` on the referenced `users` row for their FK check. That conflicts with `FOR UPDATE`, so every admission briefly blocks the same user's logins, canvas creates and event writes, and any slow admission (pool or statement timeout of 5–10 s) stretches that wait. The stated intent is only to serialize metering.

**Recommendation:** Use `FOR NO KEY UPDATE`: it serializes admissions with each other but does not conflict with FK `KEY SHARE` locks.

## F-architecture-L3-11: The claim 'the cap can never lock compaction out' is false

Compaction is admitted against the same `spent_today + reserve > cap` check. A user who still has room for an answer but not for an extra 6,800-token compaction reservation is refused compaction. Bounding the reservation by cap/10 doesn't change that. The drop-oldest fallback keeps the budget, but the guarantee as written is wrong and may mislead implementers into skipping the fallback path.

**Recommendation:** Reword it to: 'compaction may be refused near the cap; the drop-oldest fallback (`compaction_fallback`) handles it'. Add a test where the user is at cap − 1 answer reservation and assert the answer is sent with the fallback.
