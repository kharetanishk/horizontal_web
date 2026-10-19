# Level 3 review: architecture

- Generated: 2026-10-19T14:24:41+05:30
- Verdict: **fail**

| ID | Severity | Title | Location |
|---|---|---|---|
| F-architecture-L3-1 | medium | The new-message slot does not count the quick-action template, so a valid message can push the prompt over budget | docs/ARCHITECTURE.md:320 (validation), :161-167 (slot table), :182 (backstop), :189 (template wrapping) |
| F-architecture-L3-2 | medium | Truncating an oversized anchor can cut out the very text being asked about (breaks AC-7.1/AC-6.1) | docs/ARCHITECTURE.md:166 (anchor 1,500), :177 (middle-truncation), :191 (code anchor = selected lines + rest of block) |
| F-architecture-L3-3 | medium | The token counter is defined two ways that contradict each other (exact full count vs. sum of stored per-message counts) | docs/ARCHITECTURE.md:182, :198-200 |
| F-architecture-L3-4 | medium | DELETE /api/me lets anyone holding a stolen session test passwords and run argon2 with no stated rate limit | docs/ARCHITECTURE.md:298, :328 |
| F-architecture-L3-5 | low | The merge stale check (merged_rev) misses changes that don't add nodes or attempts | docs/ARCHITECTURE.md:253 (merged_rev), :80 (reclaim rule) |
| F-architecture-L3-6 | low | The polling attach doesn't define what happens when a retry resets content and increments attempt | docs/ARCHITECTURE.md:103, :307 |
| F-architecture-L3-7 | low | Each branch's anchor moves to the end of every later send, so the history differs from what was actually sent and prefix caching stops at the branch point | docs/ARCHITECTURE.md:189-191 |
| F-architecture-L3-8 | low | Invite token in the link URL can reach proxy/access logs and the log drain | docs/ARCHITECTURE.md:347 |

## F-architecture-L3-1: The new-message slot does not count the quick-action template, so a valid message can push the prompt over budget

Validation accepts content up to 3,000 tokens in the mode tokenizer. Step 3 says every fixed slot includes its own wrapper tags and framing, and step 5 wraps the new message in the quick-action template (AC-8.x). A 3,000-token message plus a template such as the 'I think this is wrong' re-check instructions is larger than the 3,000-token slot. The runtime backstop then truncates or drops older tail messages, so NFR-7 still holds, but this breaks the stated property 'the runtime backstop never fires when r_up is at its limit'. It also silently removes context that the fit guarantee says is always present.

**Recommendation:** Validate content against 3,000 minus (the largest template overhead + chat framing), computed per mode at startup. Alternatively, give the templates their own fixed slot in the table. Add a proptest case: a 3,000-token message with every doubt_type, with the backstop asserted not to fire.

## F-architecture-L3-2: Truncating an oversized anchor can cut out the very text being asked about (breaks AC-7.1/AC-6.1)

The anchor slot is 1,500 mode tokens. An answer can be up to max_tokens = 2,000 tokens, multiplied by r for the mode tokenizer. So a `text` highlight of most of an answer, or a `code` anchor (selected lines plus the whole block), can be larger than 1,500 tokens. The only overflow rule is generic middle-truncation with an omission marker. For a code anchor, the selected lines 5–9 can sit in the cut middle, so the prompt no longer contains 'the selected lines marked as the part asked about' (AC-7.1). For a long text highlight, the highlight itself is cut.

**Recommendation:** Define truncation that keeps the selection: trim the surrounding context (sentence or rest of the code block) first, and middle-truncate the selection only when the selection alone is over the slot. Then mark it explicitly. Add a test where a code block over 1,500 tokens has the selection in the middle.

## F-architecture-L3-3: The token counter is defined two ways that contradict each other (exact full count vs. sum of stored per-message counts)

Line 198 says context_tokens(X) = assemble(X).tokens, 'the exact same function'. Line 182 says the backstop counts the assembled prompt once in the mode tokenizer. Lines 199-200 say canvas load never re-tokenizes and only sums stored per-message `tok` values. These numbers differ for three reasons: (a) BPE tokenizes a concatenation differently from its parts summed; (b) the `\n\n---\n\n` same-role merge removes per-message framing and adds separators; (c) `<merged_branch>` blocks and outer `<anchor>` markers are appended to tail content. Either the AC-12.1 equality property fails, or the budget check uses a sum-of-parts number that can undercount the real prompt.

**Recommendation:** State one definition. Recommended: assemble().tokens = sum of stored part counts + fixed per-part framing constants, used by both the counter and admission. The backstop does a full re-count and logs the delta. Bound sum-vs-full drift in a proptest (e.g. ≤ 1 token per boundary), and include that margin in the budget inequality.

## F-architecture-L3-4: DELETE /api/me lets anyone holding a stolen session test passwords and run argon2 with no stated rate limit

DELETE /api/me takes {password} and verifies it with argon2. The rate-limit rules cover 'auth endpoints 10/min per client IP' and give progressive delays only for failed logins. Re-auth on DELETE /api/me is in neither list, and nothing says it shares the argon2 semaphore or the global 120/min auth bucket. With a stolen session cookie, an attacker gets an unthrottled oracle for the user's password, which may be reused elsewhere. They also get an argon2 CPU path outside the bounded auth budget.

**Recommendation:** State explicitly that the DELETE /api/me password check goes through the same argon2 semaphore, the global auth bucket, the per-IP auth bucket and the per-email failure delays as login. Add it to the rate-limit tests.

## F-architecture-L3-5: The merge stale check (merged_rev) misses changes that don't add nodes or attempts

merged_rev = count(subtree nodes) + sum(attempts). Two changes to the merge input don't move it: (1) re-merging or newly merging a nested branch changes the nested summaries that are inlined into the outer branch's proposal input; (2) a node swept as `interrupted` is reclaimed by its original owner (status error→streaming→complete, same attempt). An `error` node does not block a merge (only `streaming` does), and the reclaim fenced write does not take the canvas lock. So the outer summary can describe partial or outdated content and is never flagged as stale.

**Recommendation:** Add a per-subtree monotonic counter that is bumped under the canvas lock by any nested merge, by a reclaim, and by a status transition of a subtree node. Alternatively, store a hash of the proposal input and compare it. Block merges while any subtree node is `error`/`interrupted`, or make the reclaim take the canvas lock.

## F-architecture-L3-6: The polling attach doesn't define what happens when a retry resets content and increments attempt

The polling fallback (used during deploy overlap) sends deltas from the last offset it sent. A retry sets content = '' and attempt = attempt + 1 on the same node id. A poller that compares only length or offsets will slice past the end of the new, shorter content, or interleave two attempts' text. The registry path carries `attempt`, but the poller is not specified to.

**Recommendation:** The poller must read `attempt` with `content`. When attempt changes, send a fresh `snapshot` (or a terminal event plus `reattach`) and reset the offset. Add an integration test: retry during a polling attach.

## F-architecture-L3-7: Each branch's anchor moves to the end of every later send, so the history differs from what was actually sent and prefix caching stops at the branch point

The innermost anchor and the quick-action template are placed just before the new message on every send. On the second send in a branch, U1 appears in the tail as its stored content, not as it was originally sent (anchor + template). The anchor moves after U1/A1. So the model sees a different conversation than it answered, and consecutive sends share a cache prefix only up to the fork, not up to the previous turn. This weakens the caching goal behind AC-10.4.

**Recommendation:** Persist the exact wrapped user turn (template + anchor) as sent, or render the anchor once at the branch's first message, and keep it there verbatim in later prompts. Then add a test that send N+1's prefix equals send N's full prompt plus its reply.

## F-architecture-L3-8: Invite token in the link URL can reach proxy/access logs and the log drain

The 32-byte invite token is carried in a URL. If it is in the path or query, the PaaS edge proxy and access logs record it. Those logs ship to the log drain, which is kept ≥ 8 days, longer than the 7-day invite expiry. Anyone with log access can claim an unused invite. The NFR-9 allowlist covers app logs, not platform proxy logs.

**Recommendation:** Put the token in the URL fragment (#token=…), which browsers never send to servers. The page reads it and POSTs it to /api/auth/accept-invite. Document this choice.
