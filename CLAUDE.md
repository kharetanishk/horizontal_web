# nodemind

Learning-focused AI chat on a canvas: every prompt and answer is a node, doubts branch sideways with path-scoped context, and selected nodes compose into notes and sketchnotes.

This project is driven by the Engineering Factory (`factory status` shows the workflow state).

- Owner's brief (source of truth for scope, stack, data model): docs/product/source-brief.md. Read it first.
- Product context: docs/PRODUCT_BRIEF.md (keep it concise).
- Requirements: docs/product/requirements.md · Architecture: docs/ARCHITECTURE.md · Decisions: docs/DECISIONS.md
- References: references/catalog.json (distinguish `requirement` from `inspiration`).
- Phase skills live in .claude/skills (factory-*). Use the one matching the current phase.

Rules:
- MVP = brief phase 1 only (branching canvas). Phases 2-6 are roadmap; do not build them until phase 1 is accepted.
- Stack is fixed by the brief (pnpm + Turborepo, Next.js + React Flow, FastAPI, Postgres + pgvector, LiteLLM). Add a brief-listed service only when the current phase needs it.
- Do not commit, push, or rewrite git history; the factory checkpoints verified work.
- Never read, print, or commit secrets (.env, keys, tokens).
- Mark unverified claims as assumptions. Never invent users, metrics, benchmarks or test results.
- Prefer the simplest design that meets the requirements.
