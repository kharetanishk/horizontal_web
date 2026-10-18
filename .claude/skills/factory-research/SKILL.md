---
name: factory-research
description: Market, alternative and technical reference research with evidence grading. Use in the Research phase to write docs/product/research.md from the reference catalog.
---
# Research

Input: requirements + `references/catalog.json` (+ assets in references/assets). Run `factory ref list` to see them.
Write `docs/product/research.md` with headings: Alternatives, Evidence, Reference findings, Open questions.

- Alternatives: competitors, substitutes, "do nothing". Table: option | what it does well | gap we address | source (R#).
- Evidence: every factual claim cites a reference ID with retrieval status `ok`. If a reference failed to
  retrieve or was never retrieved, say so; do not describe its contents.
- Grade reliability: official docs/measurements > vendor claims > blog posts > forum posts.
- Reference findings: per reference, the transferable pattern or design property and its trade-offs.
  Respect licenses; never copy code or whole designs from references with unknown or restrictive licenses.
- Distinguish inspiration from requirement. Only the user can promote a reference to `requirement`.
- Open questions: what we still do not know and how to find out cheaply.
