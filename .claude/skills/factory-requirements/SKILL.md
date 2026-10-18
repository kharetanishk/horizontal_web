---
name: factory-requirements
description: Turn discovery into testable requirements. Use in the Requirements phase to write docs/product/requirements.md (MVP scope, FR/NFR with IDs, acceptance criteria, out of scope, success metrics, roadmap).
---
# Requirements

Input: docs/product/discovery.md, references marked `requirement` in references/catalog.json.
Write `docs/product/requirements.md`; replace every `TODO(factory)`.

Required headings: MVP scope, Functional requirements, Nonfunctional requirements, Acceptance criteria,
Out of scope, Success metrics, Roadmap.

Rules:
- IDs: FR-1.., NFR-1... Each FR is one user-visible capability. Each has acceptance criteria written as
  Given/When/Then or a measurable check the factory can verify by a test.
- NFRs need numbers and units (e.g. p95 < 300 ms at 20 req/s; 99.5% monthly availability). Mark targets
  as targets, not measurements.
- MVP = the smallest set that tests the riskiest assumption. Move everything else to Out of scope or Roadmap.
  Say which features you cut and why.
- Success metrics must be measurable after launch (instrumentation named).
- Link requirements to reference IDs (R#) when a reference informed them.
Summarize the MVP in docs/ROADMAP.md "Now".
