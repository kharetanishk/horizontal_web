# ADR 0004: Email + password auth with Postgres sessions and a signup allowlist

- Status: accepted
- Date: 2026-10-18

## Context

FR-1 asks for email sign-in and leaves the method open (magic link vs. password). Magic links need an email provider, a secret for it and deliverability work. Phase 1 is a moderated test with a few learners, and every account spends LLM tokens.

## Decision

Email + password, hashed with argon2id. Sessions are random 32-byte tokens in an HttpOnly, Secure, SameSite=Lax cookie; only their SHA-256 is stored in a `sessions` table (30-day expiry). Signup is restricted to `SIGNUP_ALLOWLIST` and fails closed: no allowlist means no signup, and open signup needs `SIGNUP_OPEN=true`. Signup returns the same response for not-allowlisted and already-registered emails, so it doesn't reveal which emails exist. There is no email-based password reset; the operator resets passwords with a CLI subcommand.

## Consequences

No external auth or email dependency, and logout and account deletion invalidate sessions immediately. Users cannot reset their own password. That is acceptable for allowlisted testers, but a reset flow with an email provider is required before signup opens to the public.
