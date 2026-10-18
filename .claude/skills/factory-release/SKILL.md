---
name: factory-release
description: Deployment preparation. Use in the Release phase to write docs/product/release.md (environment, deployment steps, migrations, secrets, observability, monitoring, rollback, backups, readiness).
---
# Release preparation

Write `docs/product/release.md` (replace `TODO(factory)`), headings: Environment, Deployment steps,
Migrations, Secrets, Observability, Monitoring, Rollback, Backups, Readiness.

- Match infrastructure to scale: a static site → static hosting; a small app → one container/PaaS +
  managed DB. Justify anything heavier with a requirement or estimate.
- Deployment steps must be copy-pasteable and reversible; include health-check verification.
- Rollback: exact steps and how long it takes; how migrations are rolled back or made backward-compatible.
- Secrets: where they live (never in git), how they are rotated.
- Observability: logs, metrics, traces, alerts with thresholds tied to NFRs.
- Backups: frequency, retention, and a tested restore procedure (say if not yet tested).
- Readiness: checklist referencing actual evidence (`factory report`, gate results, benchmark reports).
  Unmet items are listed as unmet. The factory never deploys without explicit approval.
Optionally add Dockerfile / compose / CI config if the project lacks them and the plan needs them.
