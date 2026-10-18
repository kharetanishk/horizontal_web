# ADR 0001: Python backend

- Status: accepted
- Date: 2026-10-18

## Context

Brief left open whether the backend is Python (FastAPI) or all-TypeScript.

## Decision

Backend API, context builder and LLM gateway are Python: FastAPI, SQLAlchemy, Alembic, LiteLLM, managed as a uv workspace. Frontend stays Next.js + TypeScript.

## Consequences

Two languages in one monorepo; shared types come from JSON Schema in packages/schemas, generated to TS and Pydantic. Python gives the strongest AI library ecosystem.
