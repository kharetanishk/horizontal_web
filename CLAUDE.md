# nodemind

Learning-focused AI chat on a canvas: every prompt and answer is a node, doubts branch sideways with path-scoped context, and selected nodes compose into notes and sketchnotes.

This project is driven by the Engineering Factory (`factory status` shows the workflow state).

- Product context: docs/PRODUCT_BRIEF.md (read first; keep it concise).
- Requirements: docs/product/requirements.md · Architecture: docs/ARCHITECTURE.md · Decisions: docs/DECISIONS.md
- References: references/catalog.json (distinguish `requirement` from `inspiration`).
- Phase skills live in .claude/skills (factory-*). Use the one matching the current phase.

Rules:
- Do not commit, push, or rewrite git history; the factory checkpoints verified work.
- Never read, print, or commit secrets (.env, keys, tokens).
- Mark unverified claims as assumptions. Never invent users, metrics, benchmarks or test results.
- Prefer the simplest design that meets the requirements.
