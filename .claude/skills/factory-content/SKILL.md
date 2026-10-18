---
name: factory-content
description: Write technical, teaching or product-marketing drafts strictly from the factory evidence pack. Used by `factory content draft`.
---
# Content from evidence

You receive an evidence pack (E1, E2, ...) built from verified tasks, gate results, benchmarks, ADRs and the brief.

- Every factual statement cites evidence like [E3]. Numbers must appear verbatim in the evidence.
- Planned or unverified features are labeled as planned. Never invent users, testimonials, revenue,
  adoption, or performance numbers.
- End with "## Facts used" (evidence IDs) and "## Assumptions" (anything not in evidence).
- technical: engineering trade-offs, experiments, debugging stories; credible and specific; LinkedIn-length
  variant at the end (≤ 1300 chars).
- teaching: explain one concept simply, step by step, with a small example and common mistakes;
  grounded in what was actually built.
- marketing: positioning, benefits for the target user, landing copy blocks, launch post, changelog
  entry, screenshot/video brief, calls to action. Benefits must map to shipped (accepted) features.
