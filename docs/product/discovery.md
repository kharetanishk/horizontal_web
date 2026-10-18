# Discovery

Source: [source-brief.md](source-brief.md). Scope here is phase 1 (branching canvas) only; phases 2-6 are roadmap.
`references/catalog.json` is empty, so every claim about demand, users or competitors below is an **assumption**.

## Problem

People who learn a topic by chatting with an AI model hit the same snag again and again: an answer contains a term, a step or a code line they don't understand. In a linear chat, the only way to ask is to append a follow-up question. That causes three costs:

1. **Lost place.** The original answer scrolls away; the learner scrolls back up to keep going, or loses the thread.
2. **Context pollution.** Every side question stays in the model's context, so later answers drift toward the detour and spend tokens on it.
3. **Hesitation.** Because asking derails the main thread, learners skip small doubts, or open a new chat and lose all prior context.

How often: the brief says this happens at nearly every non-trivial answer while studying. *Assumption: owner's own experience; not measured with other users.*

## Target users

| Profile | Context | Goal | Current workaround |
| --- | --- | --- | --- |
| Self-learner (primary) | Learns technical topics (e.g. system design, programming) through long AI chat sessions | Understand a topic end to end without losing the main explanation | Follow-ups in the same chat, scroll back; or a second chat tab with context pasted in by hand |
| Student | Studies course material and wants study notes from what they explored | Clear up doubts, then turn the session into revision material | Copies chat fragments into a notes app by hand (note composition is phase 2) |
| Owner as builder (secondary) | Builds the app as an applied AI engineering portfolio piece | Cover context management, streaming and token budgeting for real | n/a (the brief's second goal; it shapes the stack, not the product's value) |

No interviews have been done. The profiles come from the brief, not from user research.

## Value proposition

Ask any doubt about any part of an answer right beside it, without losing your place or polluting the main thread's context.

- **Place kept:** the main discussion stays vertical; doubts branch sideways from the exact word, sentence or code lines, so the original answer stays in view.
- **Clean context:** each node is answered from its root-to-node path only, so sibling detours never reach the model, and the per-path token counter shows it.
- **Learning kept:** a finished branch merges back as a one-line summary on the main path, so the answer it produced isn't lost.

## User journeys

**J1. Main thread.**
1. Learner signs in and creates a canvas.
2. Types a question; the answer streams into a new node below it.
3. Asks a follow-up; it appears vertically under the answer, continuing the main path.

**J2. Branch a doubt from a highlight.**
1. In an answer, learner highlights a phrase (e.g. "each microVM runs its own kernel").
2. Picks a quick action (explain simpler, give an example, why does this work, go deeper, "I think this is wrong") or types a free question.
3. A branch node opens beside the answer, anchored to the highlight; the reply streams in using the main path plus the anchor only.
4. Learner asks a deeper doubt from that branch; it branches again sideways.

**J3. Merge back and continue.**
1. Learner marks the branch as done and merges it back.
2. A one-line summary is attached to the main path; the branch collapses.
3. Learner continues the main thread; the next answer includes the summary but not the full branch.

**J4. Code doubt.**
1. Learner selects lines 5-9 in a code block in an answer and asks "what do these lines do?".
2. The branch reply receives the selected lines plus the rest of the block as context.

Phase 1 gate (from the brief): branching a doubt and returning to the main thread feels effortless.

## Assumptions

| Assumption | Evidence | How to test cheaply |
| --- | --- | --- |
| Linear chat derails learners often enough to make them switch tools | Owner's experience only (brief) | Ask 5-10 learners to describe their last study chat; count side questions and workarounds |
| Learners will highlight and branch instead of just typing a follow-up | None yet | Clickable prototype or phase 1 build; watch 5 users do a study task; count branches vs. linear follow-ups |
| Path-scoped context gives noticeably better answers than full-chat context | None yet; plausible from how context windows work | Replay the same conversation with full vs. path context; compare answers blind on a small set |
| A one-line merge summary keeps enough of a branch's learning | None yet | After merge, ask the main thread a question that depends on the branch; check the answer uses it |
| A canvas stays usable as branches grow (with auto-layout, collapse, zoom-to-node) | None yet; brief lists clutter as a risk | Build a 30+ node canvas; time how long it takes to find and return to a given node |
| Existing tools don't already solve this well enough | None; no competitors in the reference catalog | Research phase: list branching-chat / canvas-chat tools and what they do and don't do |
| Doubt answers can use a small, cheap model without hurting quality | None yet | Run the same doubts through a small and a large model; compare blind |
| Learners want the session turned into notes later (justifies phase 2) | None yet | Ask test users at the end of phase 1 whether they'd export; don't build until asked |

Challenged: the "who it's for" is broad ("students and self-learners"). Phase 1 should be judged with the self-learner on technical topics, the profile the brief's examples (Firecracker, system design) actually describe.

## Risks

**Product**
- *Canvas clutter:* wide branching gets messy and defeats "keep your place". Auto-layout, collapsible branches and zoom-to-node are core to phase 1, not polish.
- *Friction worse than linear chat:* if branching takes more steps than typing a follow-up, users won't use it. The phase 1 gate tests this directly.
- *Lossy merge:* a bad summary silently drops what was learned.
- *Wrong answers:* a learning tool that teaches errors loses trust; "I think this is wrong" helps but doesn't guarantee correctness.

**Technical**
- *Anchor drift:* text offsets on the edge must stay valid against the stored node content (streamed or edited content can shift them).
- *Path context growth:* long main paths exceed the token budget; compaction of older messages must keep answers coherent.
- *Cost and latency:* every branch is another model call; mitigated by small models for doubts and stable path prefixes for prompt caching (provider-dependent; *assumption* until measured).
- *Stack weight for phase 1:* the brief lists Redis, workers, Langfuse, MinIO etc. Phase 1 needs only web, API, Postgres and the LLM gateway; the rest should wait until a phase needs them.

**Legal / compliance**
- Users' chat content and emails are personal data: needs a privacy notice, account deletion and no logging of message content to third parties without disclosure.
- Content goes to third-party LLM providers via LiteLLM; their data-use terms must be disclosed to users.
- Generated answers may be wrong; add a "may contain errors" notice for a learning context.

**Features that don't serve the core problem in phase 1** (defer): composer, notes, sketchnotes, landing page builder, RAG, learner memory, semantic search, MCP server, knowledge graph, image/video generation. Learner memory (context builder step 3) and RAG (step 5) are phase 3 even though they appear in the context builder order.
