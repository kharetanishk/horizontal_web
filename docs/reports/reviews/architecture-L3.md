# Level 3 review: architecture

- Generated: 2026-10-19T14:27:27+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | high | Advisory-lock liveness depends on a direct connection, but DATABASE_URL_DIRECT is optional | docs/ARCHITECTURE.md:106 (owner lock); Deployment > Config (`DATABASE_URL_DIRECT` (optional)); Migrations bullet (F-L3-13) |
| F-architecture-L3-2 | medium | The 15-minute reservation safety net assumes tasks end by TASK_DEADLINE_SECS, but the final write retries without limit | Data model > 'Every reservation settles exactly once' (safety net > TASK_DEADLINE_SECS); Failure handling row 'Postgres outage of any length'; ARCHITECTURE.md:80 reclaim fence |
| F-architecture-L3-3 | medium | Device-cookie signing key is missing from config | API > Rate limits (signed HttpOnly `device` cookie); Deployment > Config (env) |
| F-architecture-L3-4 | medium | NFR-11 'a completed node is never lost' can't hold with daily backups only | Failure handling > 'Data loss' row; Deployment > Host ('PITR preferred') |
| F-architecture-L3-5 | medium | Canvas load recomputes the full prompt assembly for every node | ARCHITECTURE.md:203 ('context_tokens(X) = assemble(X, mode, None).tokens … does not use the prefix-sum trick'); API `GET /api/canvases/:id` returns context_tokens per node |
| F-architecture-L3-6 | low | The merge-proposal spec contradicts itself on whether the proposal is stored | API table: `POST /api/edges/:id/merge-proposal` ('{summary} (≤ 200 chars, not stored)') vs `PUT /api/edges/:id/merge` ('edges.last_proposal, which the merge-proposal endpoint stores server-side') |
| F-architecture-L3-7 | low | 'interrupted' is used as a node state but is not in the status enum | Data model `nodes.status` (`streaming`/`complete`/`error`); ARCHITECTURE.md:80, 115, 124, 387–390 |
| F-architecture-L3-8 | low | The CSP has no style-src, so inline styles fall under default-src 'self' | Security and data protection > Headers (`default-src 'self'; script-src 'self' <hashes>`) |
| F-architecture-L3-9 | low | TRUSTED_IP_HEADER rule may rule out two of the three candidate hosts | API > Rate limits (production refuses X-Forwarded-For/Forwarded); Deployment > Host (Fly.io, Render or Railway) |

## F-architecture-L3-1: Advisory-lock liveness depends on a direct connection, but DATABASE_URL_DIRECT is optional

Several things rely on a session-level `pg_advisory_lock(owner_key)` held on a dedicated non-pooled connection: the sweeper, reclaim and reservation settlement, and the migration lock. Config marks `DATABASE_URL_DIRECT` optional. If it is left unset and `DATABASE_URL` points at a transaction-mode pooler (PgBouncer, Supavisor, Neon pooled), one of two things happens. The lock lands on a backend that is reused or released between statements. Or `pg_locks` shows no lock for a live owner. Either way, another instance's sweep (line 110) sees a live `owner_key` as dead. It marks in-flight answers `interrupted` and settles their reservations while they are still streaming. Concurrent migrations also lose mutual exclusion. Nothing in the doc detects this misconfiguration.

**Recommendation:** In production, make `DATABASE_URL_DIRECT` required, or refuse to start unless a startup probe proves the lock connection is session-pinned: take the lock, then check across two separate statements that `pg_locks` shows it for the same `pg_backend_pid()`. Add a pooled-URL misconfiguration case to the fault-injection suite.

## F-architecture-L3-2: The 15-minute reservation safety net assumes tasks end by TASK_DEADLINE_SECS, but the final write retries without limit

The startup check (15 min > `TASK_DEADLINE_SECS`) is the only reason the safety net can't settle a live task's reservation. But the final write 'retries until it lands' for an outage 'of any length', so a task can outlive both the deadline and the 15 minutes. After a long outage the sweeper settles that reservation at its reserved amount. The late task then reclaims the node through the fence at line 80 and its `Reservation` guard settles again. The doc never says settlement is `UPDATE … WHERE settled_at IS NULL`. Without that condition the usage is counted twice. With it, the real counts are silently dropped in favour of the reserved estimate, so NFR-12 cost records are wrong without any signal.

**Recommendation:** State that settlement is `UPDATE llm_usage SET … WHERE id = $1 AND settled_at IS NULL`. Let a late owner overwrite an `estimated` safety-net settlement with its real counts (for example `WHERE settled_at IS NULL OR estimated`). Either cap the final-write retry at the deadline, or drop the claim that the 15-minute check is sufficient. Add a test: pause Postgres for more than 15 min after stream end, then assert one settlement with the real counts.

## F-architecture-L3-3: Device-cookie signing key is missing from config

Returning users skip the per-email login delay by presenting a signed `device` cookie. The Config list has no signing secret (e.g. `DEVICE_COOKIE_KEY`). If the key is generated at boot, every deploy and every overlapping instance invalidates the other instances' cookies, so returning users lose the protection the pattern exists for. If the key is a compiled-in constant, the cookie can be forged and the per-email throttle bypassed.

**Recommendation:** Add a required env secret (at least 32 bytes) for device-cookie signing, shared by all instances. Production refuses to start without it. Document how to rotate it, for example by accepting the old key for a while.

## F-architecture-L3-4: NFR-11 'a completed node is never lost' can't hold with daily backups only

The doc concedes that without PITR a database loss loses up to 24 h, and it marks the host as an assumption. NFR-11 says a completed node is never lost after the stream ends. With daily backups only, every node completed since the last backup is lost when the database is lost. The doc lists only the drain/outage exception to NFR-11, not this one.

**Recommendation:** Either make PITR (or WAL archiving) a hard requirement for host selection, or record a second explicit NFR-11 exception with an RPO of 24 h and get the owner to sign off on it.

## F-architecture-L3-5: Canvas load recomputes the full prompt assembly for every node

`GET /api/canvases/:id` returns `context_tokens` for every node, and each one is computed by a full `assemble` along that node's path. The cost is O(nodes × path length) assembly work per load, and that includes compaction and summary resolution. A 1,000-node canvas with deep paths can blow the NFR-3 target (render in under 2 s), and the NFR-1 benchmark covers only one path build, not N of them.

**Recommendation:** Extend the NFR-3 benchmark to time server-side `context_tokens` for every node on the seeded 1,000-node canvas with 200-deep paths. If it misses the target, compute counts lazily (only for visible or selected nodes), or cache them per node keyed by `(path checkpoint, merged_rev set)`.

## F-architecture-L3-6: The merge-proposal spec contradicts itself on whether the proposal is stored

One row says the proposal is not stored; the next relies on `edges.last_proposal` being written by that endpoint. The 'merge edits' metric (F-L3-12) only works if the proposal is stored. An implementer following the first row breaks the metric. It also means a mutating, LLM-metered endpoint writes to `edges` without the doc saying whether it takes the canvas lock. That matters because `merged_rev` and merge confirm rely on that lock.

**Recommendation:** Change the merge-proposal row to say it stores `last_proposal`. Specify whether that write takes the canvas lock, and what happens to `last_proposal` when a proposal fails (502) or the user types the summary by hand.

## F-architecture-L3-7: 'interrupted' is used as a node state but is not in the status enum

The fence at line 80 shows `interrupted` is meant as `error_code` with `status = 'error'`. The failure table and drain steps say 'marks those nodes `interrupted`' as if it were a status. `error_code` has no CHECK or enum listing its allowed values. A typo or a status-based implementation would quietly break the reclaim fence, which matches on `error_code = 'interrupted'`.

**Recommendation:** List the `error_code` values (`interrupted`, `llm_unavailable`, …) with a CHECK constraint, and write every reference as `status = 'error', error_code = 'interrupted'`.

## F-architecture-L3-8: The CSP has no style-src, so inline styles fall under default-src 'self'

Only script hashes are planned. With no `style-src`, inline `<style>` elements and markup `style=` attributes are blocked. Sources of those include Next static-export inlined CSS or font CSS, syntax-highlighter output rendered as HTML, and any library-injected styles. Styling could break in production only, since the dev server doesn't send this CSP.

**Recommendation:** Settle `style-src` in the same first-web-build spike: hashes, or `'unsafe-inline'` for styles recorded as an accepted risk. Add a Playwright check that runs against the production CSP headers.

## F-architecture-L3-9: TRUSTED_IP_HEADER rule may rule out two of the three candidate hosts

Production refuses to start unless `TRUSTED_IP_HEADER` names a single-value header that the platform overwrites. Fly has one (`Fly-Client-IP`). Whether Render or Railway provide one is not established (**assumption**). If they don't, the app can't start on those hosts. The doc also forbids the standard safe pattern of taking the right-most XFF entry appended by a known proxy hop.

**Recommendation:** Add this to the host-choice open questions. Alternatively, support a 'right-most XFF entry at trusted hop N' mode as a vetted option, covered by the same spoofing test.
