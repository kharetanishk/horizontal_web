---
name: factory-architecture
description: System design and architecture decisions. Use in the Architecture phase to write docs/ARCHITECTURE.md and ADRs (components, data model, API, failure handling, deployment) choosing the simplest architecture that meets requirements.
---
# Architecture

Input: requirements (FR/NFR), docs/product/estimation.md if present. Write `docs/ARCHITECTURE.md`
(replace `TODO(factory)`); headings: Components, Data model, API, Failure handling, Deployment.
Also include: Options considered, Auth, Observability, Backup & recovery.

Method:
1. List 2–3 options (e.g. single app + managed Postgres vs. app + queue + worker). Table of trade-offs
   against the NFRs. Pick the simplest that satisfies them. Microservices, Kubernetes, sharding,
   multi-region need a requirement or a number that forces them; say which.
2. Data model: entities, keys, relationships, indexes derived from the main query patterns; migrations strategy.
3. API: resources/endpoints or RPCs with request/response shapes, error format, auth, pagination, idempotency
   for writes that can be retried.
4. Caching, queues, async work only where a requirement or estimate justifies it.
5. Failure handling: timeouts, retries with backoff, what happens when each dependency is down, data-loss scenarios.
6. Security: authn/authz model, secret management, input validation boundaries, PII handling.
7. Deployment: environments, how it ships, health checks, rollback, backups and restore test.
Record each significant choice as an ADR: `factory decide "title" --context .. --decision .. --consequences ..`.
