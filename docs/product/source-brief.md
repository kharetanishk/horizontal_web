# Nodemind — project brief (source)

Owner's original brief, exported from the Claude Doc on 2026-10-18
(https://claude.ai/code/artifact/0aaaa5f9-7327-4892-a2cf-bd03ac545a40, as of 2026-09-29).
This is the input for every phase. Do not edit it to match later decisions; record changes in docs/DECISIONS.md.

Backend language changed: the backend is Rust, not Python (docs/ADR/0002). Where this brief names Python tools
(FastAPI, SQLAlchemy, Alembic, LiteLLM, Pydantic, uv, pytest, the MCP Python SDK), ADR 0002 overrides it.

## Overview

Nodemind is a learning-focused AI chat where every prompt and every answer is a node on a canvas, so doubts branch sideways instead of derailing the main discussion.

**The problem.** In linear chat apps, a doubt about part of an answer pushes the conversation down. The learner loses the original answer, scrolls back up, and the side questions pollute the model's context, causing drift.

**The idea.** The main discussion flows vertically. Any doubt about any part of a node (a word, a sentence, a code block, the whole answer) branches horizontally, right beside it. Each node's context is only its own path from the root. Selected nodes can then be joined into a composer node that turns the exploration into notes, sketchnotes, a landing page, and later images and videos.

**Who it's for.** Students and self-learners who explore topics through conversation and want structured study material out of it.

**Second goal.** The project is built to cover the full applied AI engineering stack end to end, as a portfolio piece.

## Core concepts

The app is a conversation graph: typed nodes joined by typed edges, with context scoped to each node's path.

| Element | Types | Purpose |
| --- | --- | --- |
| Node | `user_message`, `ai_message`, `composer`, `source`, `artifact` | A prompt, an answer, a composer, an input like a PDF or link, or a generated output |
| Edge | `reply` | Normal vertical flow of the main discussion |
| Edge | `branch` | Horizontal doubt, stores an anchor to the part of the parent being asked about |
| Edge | `input` | Connects a node into a composer, with an order number the user controls |

**Path-scoped context.** A message at node X is answered using only the path from the root to X. Sibling branches are invisible, so side doubts never pollute the main thread or waste tokens.

**Merge back.** A finished branch can be collapsed into a one-line summary that is injected into the main path, so the learning isn't lost.

**Composer.** Takes ordered input nodes plus a prompt and produces an artifact node: notes, a sketchnote, a landing page, and later images and video.

## Doubt branching

A branch can start from any part of any node, and branches can branch again, so a doubt inside a doubt is a deeper horizontal path.

| Anchor kind | Example |
| --- | --- |
| `none` (whole node) | "Give me an example of this whole answer" |
| `text` (word, phrase, sentence, paragraph) | "What's Firecracker?", "Why does this make it more secure?" |
| `code` (block or line range) | "What do lines 5 to 9 do?" |
| `region` (box on a diagram or image, later) | "Explain this part of the sketchnote" |
| Own prompt node | "What should I have asked instead?" |

The anchor is stored as one flexible JSON column on the edge, so new kinds can be added without a schema change:

```json
{ "kind": "text", "start": 412, "end": 540, "text": "each microVM runs its own kernel..." }
```

**Quick actions** appear on highlight: explain simpler, give an example, why does this work, go deeper, and "I think this is wrong" (which makes the model re-check itself). The user can also type a free question. Each action maps to a prompt template, and tagging branches by doubt type builds data for evals.

## Context builder

For a message at node X, the prompt is assembled in this order, within a token budget:

1. Fetch the path from root to X with one recursive SQL query over `edges`.
2. Pick the system prompt by mode: main thread, doubt, or composer.
3. Add relevant learner memory (what they already understand).
4. Add merged branch summaries that lie on the path.
5. Add RAG chunks if sources are attached.
6. Add path messages: recent ones verbatim, older ones compacted into a summary when over budget.
7. For a branch, add the anchor, scaled to its size: a short highlight plus its surrounding sentence; a long highlight marked as "the part being asked about"; selected code lines plus the rest of the block; nothing extra for a whole-node branch.
8. Add the new message, leaving room for the reply.

Shared path prefixes stay identical across turns, so provider prompt caching cuts latency and cost.

## Composer and sketchnotes

Every composer run follows one pipeline, and a structured JSON plan is the source of truth for every output style.

1. **Classify** the request: notes, sketchnote, diagram, landing page, image.
2. **Plan** the output as structured JSON from the ordered input nodes.
3. **Check** that every bullet is supported by the source nodes (hallucination guardrail).
4. **Render** in the chosen style.
5. **Evaluate** the result and retry on failure.

The quality bar for sketchnotes is a numbered-panel layout: title banner, colored highlighter headers, one icon or mini-diagram per concept, flow arrows, analogy boxes, sticky-note benefit boxes, and a takeaways checklist.

| Style | How it's drawn | Strengths | Weaknesses |
| --- | --- | --- | --- |
| Sketch (default) | JSON rendered as SVG with Rough.js and handwriting fonts | Editable, text always correct, cheap, fast | About 80% of the illustrated look |
| Illustrated | JSON turned into a detailed image-model prompt | Matches the hand-drawn quality bar | Not editable, slower, costlier, text can garble |

For the Illustrated style, OCR the image and compare its text to the JSON; regenerate on mismatch. Every section keeps `source_node_ids`, so clicking a panel jumps back to where it was learned.

```json
{
  "title": "System design",
  "sections": [
    {
      "number": 1,
      "heading": "Single server setup",
      "color": "blue",
      "diagram": { "nodes": ["Client", "Web server"], "edges": [["Client", "Web server", "HTTP request"]] },
      "bullets": ["Simple", "Not scalable", "Single point of failure"],
      "analogy": null,
      "source_node_ids": ["n_12", "n_14"]
    }
  ],
  "takeaways": ["Start simple, scale gradually"]
}
```

## Features by phase

Ship in six layers; each phase works on its own before the next starts.

1. **Branching canvas.** Auth, canvases, nodes, vertical and horizontal branching from any anchor, quick actions, path-scoped context, streaming answers, token counter per path, merge back. Gate: branching a doubt and returning to the main thread feels effortless.
2. **Composer.** Node selection and ordering, normal notes, Sketch-style sketchnotes, then Illustrated style with OCR checks. Gate: notes pass the faithfulness check against source nodes.
3. **Search and memory.** Embeddings for every node, semantic search across canvases, learner memory, RAG over uploaded files.
4. **Landing page builder.** Agentic plan, code, run, fix loop with sandboxed live preview.
5. **Quality layer.** Intent classifier for routing, eval suites, guardrails, tracing, latency and cost optimization.
6. **Extras.** MCP server, concept knowledge graph, image generation, YouTube ingestion (yt-dlp), video uploads (ffmpeg), Remotion videos.

## Architecture

The canvas talks only to the API; the API's context builder assembles every prompt and sends it through the LLM gateway, while slow work (embeddings, memory, composer runs) goes to workers via the queue.

## Tech stack

| Layer | Choice | Why |
| --- | --- | --- |
| Monorepo tooling | pnpm workspaces + Turborepo (JS), uv workspace (Python) | One repo, cached builds for both languages |
| Frontend | Next.js, TypeScript, Tailwind, Zustand | Standard and well documented |
| Canvas | React Flow (`@xyflow/react`) | Built for node graphs: pan, zoom, custom nodes |
| Sketch rendering | SVG + Rough.js + handwriting fonts (Caveat) | Editable sketchnotes, always-correct text |
| Backend API | Python, FastAPI, SQLAlchemy, Alembic | Best AI ecosystem, async streaming |
| Streaming | Server-Sent Events | Token-by-token answers |
| Database | PostgreSQL + pgvector | Nodes, edges and embeddings in one DB |
| Cache and queue | Redis + ARQ (or Celery) | Sessions, caching, background jobs |
| LLM access | Provider SDKs behind LiteLLM | Swap models per task without code changes |
| Token counting | Provider tokenizers (e.g. tiktoken) | Context budgeting |
| Observability | Langfuse | Traces, prompt versions, costs, eval scores |
| Evals | pytest suites + promptfoo | Regression tests for prompts and pipelines |
| Sandbox | E2B first, Firecracker later | Safe execution of generated code |
| Graph DB (later) | Neo4j | Concept knowledge graph |
| Object storage | MinIO locally, S3 in production | Uploads and generated images |
| MCP | Official MCP Python SDK | Expose canvas tools to other AI clients |
| Local infra | Docker Compose | Postgres, Redis, MinIO, Langfuse with one command |

## Data model

Eight tables in PostgreSQL cover the first version; the root-to-node path comes from one recursive query over `edges`.

| Table | Key columns |
| --- | --- |
| `users` | id, email, created_at |
| `canvases` | id, user_id, title, created_at |
| `nodes` | id, canvas_id, type, role, content, model, token_count, position_x, position_y, created_at |
| `edges` | id, source_id, target_id, type, anchor (JSON), order |
| `node_embeddings` | node_id, vector |
| `memories` | id, user_id, fact, source_node_id |
| `composer_runs` | id, composer_node_id, status, plan_json, output_ref, eval_scores |
| `artifacts` | id, type, storage_url, source_node_ids |

Node types, edge types, the anchor shape, composer requests and the sketchnote JSON are defined once as JSON Schema in `packages/schemas`, then generated into TypeScript types and Pydantic models. Frontend, backend and LLM structured outputs stay in sync.

## Monorepo layout

```
nodemind/
├── apps/
│   ├── web/                 # Next.js canvas app
│   ├── api/                 # FastAPI: auth, canvases, chat, context builder
│   ├── worker/              # Background jobs: embeddings, memory, composer
│   └── mcp-server/          # MCP server exposing canvas tools (later)
├── packages/
│   ├── schemas/             # JSON Schemas: source of truth for TS + Pydantic types
│   ├── ui/                  # Shared React components
│   ├── sketch-renderer/     # Sketchnote JSON → SVG (Rough.js)
│   └── prompts/             # Versioned prompt templates, incl. doubt quick actions
├── python/
│   ├── core/                # LLM gateway, context builder, db models
│   └── pipelines/           # Composer pipelines: notes, sketchnote, landing page
├── services/
│   └── sandbox/             # Code execution (E2B → Firecracker)
├── evals/                   # Datasets + suites: routing, faithfulness, OCR checks
├── infra/
│   └── docker-compose.yml   # Postgres+pgvector, Redis, MinIO, Langfuse
├── docs/                    # Architecture notes, decisions
├── turbo.json
├── pnpm-workspace.yaml
└── pyproject.toml           # uv workspace root
```

## AI engineering coverage

| Concept | Where it lives | Phase |
| --- | --- | --- |
| Context window, compaction | Path-scoped context, summarizing old path messages | 1 |
| Tokenization | Live token counter per path | 1 |
| Prompt engineering | Mode-specific system prompts, doubt quick-action templates | 1 |
| Streaming responses | SSE into nodes | 1 |
| Prompt caching, latency | Stable path prefixes, small model for doubts | 1, 5 |
| Structured outputs | Composer plans, sketchnote JSON | 2 |
| Hallucination guardrails | Faithfulness check against source nodes, "I think this is wrong" action | 2, 5 |
| Multimodal generation | Illustrated sketchnotes, images | 2, 6 |
| Embeddings, vector DB | Semantic search across all nodes (pgvector) | 3 |
| RAG | Uploaded files as source nodes | 3 |
| Memory systems | Learner memory of understood concepts | 3 |
| Function and tool calling | Search, create node, generate diagram, build page | 3, 4 |
| Agents, agentic workflows | Landing page plan, code, run, fix loop | 4 |
| Sandboxing, microVMs | E2B, then Firecracker for generated code | 4 |
| Classification, fine-tuning vs RAG | Fine-tuned small intent router; RAG for knowledge | 5 |
| Evals | Routing accuracy, note faithfulness, OCR text match | 5 |
| Guardrails | Input moderation, output checks | 5 |
| MCP | Canvas tools exposed as an MCP server | 6 |
| Graph DB | Concept knowledge graph (Neo4j) | 6 |

## Risks and principles

- **Canvas clutter.** Wide branching gets messy fast; plan auto-layout, collapsible branches and zoom-to-node from phase 1.
- **Cost growth.** Every branch is another model call; route doubts to a small model and cache aggressively.
- **Wrong facts.** A learning app that teaches errors loses trust; every generated note keeps `source_node_ids` and passes a faithfulness check.
- **Scope creep.** The branching experience is the product; don't start phase 4 before phase 1 feels great.
- **Model lock-in.** All model calls go through the LLM gateway so providers can be swapped per task.
