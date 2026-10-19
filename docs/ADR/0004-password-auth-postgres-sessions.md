# ADR 0004: Email + password auth with Postgres sessions and invite-only accounts

- Status: accepted
- Date: 2026-10-18

## Context

FR-1 asks for email sign-in and leaves the method open (magic link vs. password). Magic links need an email provider, a secret for it and deliverability work. Phase 1 is a moderated test with a few learners, and every account spends LLM tokens.

## Decision

Email + password, hashed with argon2id. Sessions are random 32-byte tokens in an HttpOnly, Secure, SameSite=Lax cookie; only their SHA-256 is stored in a `sessions` table (30-day expiry). Accounts are invite-only (revised 2026-10-19 after the L3 review). An allowlisted self-signup lets whoever knows an address claim it first. The operator runs `api invite <email>`, which creates the user and prints a single-use, 7-day link token; only its hash is stored. The learner sets their password through `POST /api/auth/accept-invite`. There is no public signup endpoint. Password reset is the operator issuing a new invite (`api reset-password`), which clears the hash and the sessions.

## Consequences

No external auth or email dependency, and logout and account deletion invalidate sessions immediately. Users cannot reset their own password. That is acceptable for invited testers. Public signup needs an email provider with verification and a self-service reset flow.
