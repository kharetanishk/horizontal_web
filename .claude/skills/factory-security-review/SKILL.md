---
name: factory-security-review
description: Security review checklist for factory projects (authn/authz, input validation, secrets, dependencies, data protection). Use when reviewing or hardening code.
---
# Security review

Check, with file:line evidence:
- AuthN/AuthZ: every endpoint/action checks identity and ownership; no IDOR; admin paths protected.
- Input: validation at trust boundaries; parameterized queries; no shell/eval with user input; path traversal; SSRF on URL fetches.
- Output: XSS escaping; secure headers; CORS scope.
- Secrets: none in code, logs, or client bundles; .env ignored.
- Sessions/tokens: expiry, rotation, secure cookie flags, CSRF where relevant.
- Data: PII minimization, encryption in transit/at rest where required, backups.
- Dependencies: known-vulnerable packages (see `factory run` security phase audit output).
- Failure modes: errors do not leak internals; rate limiting on auth and expensive endpoints.
Report only concrete issues with severity and a fix. Do not declare code secure.
