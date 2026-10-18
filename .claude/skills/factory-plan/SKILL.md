---
name: factory-plan
description: Break approved requirements and architecture into a verified task graph. Used by `factory plan`.
---
# Task planning

Produce small, independently verifiable tasks (each ≤ ~1–2 hours of agent work, one concern).
For each task: title, objective, files/modules, depends_on (task indexes), acceptance criteria tied to
FR/NFR IDs, tests (commands the project already supports, e.g. `npm test`, `uv run pytest -q`,
`go test ./...`; plain commands only, no pipes or shell operators), risk (low/medium/high).

Rules:
- First task sets up the project skeleton and its test command, so later tasks can be verified.
- High risk = auth, payments, data migrations, PII, security boundaries, irreversible operations.
- Tasks touching the same files must depend on each other; independent tasks list disjoint files.
- Cross-module tasks state the interface contract in acceptance criteria.
- No task without a way to verify it.
