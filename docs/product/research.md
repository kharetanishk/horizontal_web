# Research

Inputs: [requirements.md](requirements.md), [discovery.md](discovery.md), `references/catalog.json`.

**Status of evidence.** `references/catalog.json` is empty (`[]`), so no reference has retrieval status `ok`. Web search and the `factory ref` commands were not permitted in this session, so no new reference was retrieved either. Every statement below about a third-party product, library or license comes from the author's prior knowledge, is **unverified**, and is marked **assumption**. None of it can be cited as evidence until it is added to the catalog and retrieved. Nothing here is a `requirement` reference; only the owner can promote one.

## Alternatives

What a self-learner can use today instead of nodemind (discovery: "existing tools don't already solve this well enough" has no evidence yet). Source column: no R# exists, so every row is an assumption.

| Option | What it does well | Gap we address | Source |
| --- | --- | --- | --- |
| **Do nothing: linear follow-ups** in ChatGPT, Claude, Gemini | Zero friction to ask; strong models; the learner already uses it | Lost place, context pollution, hesitation (discovery problem 1-3). Side questions stay in the context of every later answer | None (assumption) |
| **Quote a selection into the next message** (some chat apps offer "ask about this" or quote-reply on selected text) | Anchors the doubt to exact text, one click | Answer still goes into the linear thread; detour stays in main context; no way back to the place | None (assumption: feature exists in at least ChatGPT; unverified) |
| **Edit a past message to fork** (ChatGPT, Claude: edited message creates a new version with arrows to switch) | Real branching of the conversation; earlier context reused | Only one branch visible at a time; fork point is a whole message, not a highlight; no merge back; switching versions hides the other path | None (assumption) |
| **Branch / fork into a new chat** (ChatGPT "branch in new chat"; open-source chat UIs such as LibreChat offer conversation forking) | Keeps the original chat clean; new chat has prior context | Branch lives in a separate chat list, not beside the answer; no anchor; nothing comes back to the main chat | None (assumption: names and behaviour unverified) |
| **Second tab with context pasted by hand** (discovery workaround) | Works with any tool; full control of context | Manual, slow; two places to track; learning from the detour is lost | discovery.md (owner's experience, not measured) |
| **Canvas-style AI chat tools** (node-based LLM canvases; Obsidian Canvas with LLM plugins; whiteboards with AI such as Miro AI or tldraw experiments) | Spatial layout; several threads visible at once; some pass parent-node context to the model | Built for brainstorming or general prompting, not learning: assumed to lack highlight/code-line anchors, doubt quick actions, merge-back summaries and a per-path token counter | None (assumption: exact features of each tool unverified, the core risk to check) |
| **Study tools** (NotebookLM, Khanmigo and similar tutors) | Source-grounded answers or tutoring pedagogy; study guides | Linear chat; no branching; no control over which context the model sees | None (assumption) |
| **Notes app + chat** (copy fragments into Notion/Obsidian) | Durable notes the learner owns | Manual; the chat itself stays linear (note composition is our phase 2, not MVP) | discovery.md target-users table |

Read of the table (assumption until verified): branching itself is not new; the combination nodemind bets on is **anchor-level branching beside the answer + path-scoped context + merge-back summary + visible token cost**, aimed at learning. If a canvas tool already ships that combination, the MVP's differentiation is weak and the scope should be revisited before build.

### Technical alternatives for MVP building blocks

Stack is fixed by ADR 0002; these are the open choices inside it. All facts, including licenses, are assumptions to verify.

| Need (FR) | Options | Lean (assumption) | Why / trade-off |
| --- | --- | --- | --- |
| Canvas (FR-15-17) | React Flow `@xyflow/react` (brief choice); tldraw; custom SVG | React Flow | Brief choice; custom nodes, pan/zoom, `fitView`/`setCenter` for "back to main". Assumed MIT. tldraw is a whiteboard SDK with its own license terms, more than we need |
| Auto-layout (FR-15, AC-15.1) | Own tree layout (main path down, branches right); dagre; elkjs; d3-hierarchy | Own layout, ~100 lines | The layout rule is fixed and simple (vertical main, branches to the right of parent). A generic DAG layouter would fight the "branch next to anchor" rule. dagre/d3-hierarchy assumed permissive (MIT/ISC); elkjs assumed EPL-2.0, check before use |
| Path fetch (FR-10, NFR-1) | Recursive CTE on `edges`; Postgres `ltree` materialized path; app-side walk | Recursive CTE | Nodes are immutable (requirements cut node edits), so the path never changes; a 200-deep CTE is assumed well under 200 ms. `ltree` only if NFR-1 fails |
| Streaming (FR-3, FR-18) | SSE from Axum; WebSocket | SSE | Brief choice; one-way server→client fits; Axum assumed to ship SSE support (`axum::response::sse`) |
| Token count (FR-12, NFR-6) | `tiktoken-rs`; provider count-tokens endpoint; char/4 heuristic | Provider-matched tokenizer locally, check against provider-reported usage | `tiktoken-rs` covers OpenAI encodings only (assumption); non-OpenAI models need the provider's count endpoint or reported usage, else NFR-6 (±5%) may fail |
| Prompt caching (AC-10.4) | Provider automatic prefix caching; explicit cache markers | Keep prefixes byte-stable; measure in phase 5 | Caching rules differ per provider (min length, explicit vs. automatic, TTL); assumption until provider docs are retrieved |
| Merge summary (FR-13) | LLM one-liner + user edit; extractive (first sentence of last answer) | LLM + user edit | Requirements already chose this; extractive is the fallback if cost or quality is poor |

## Evidence

Reliability grading used: official docs / measurements > vendor claims > blog posts > forum posts.

| Claim the product depends on | Current evidence | Grade | Reference that would settle it |
| --- | --- | --- | --- |
| Linear chat derails learners (discovery assumption 1) | Owner's experience | None (anecdote) | 5-10 learner interviews; no public source expected to settle it |
| No existing tool combines anchor branching + path context + merge back | None | None | Product pages / docs / hands-on test of each canvas tool in the Alternatives table |
| ChatGPT and Claude fork by editing or "branch in new chat" only | Author's prior knowledge | None (unverified) | OpenAI and Anthropic help-center articles |
| React Flow handles 200 nodes at ≥ 30 fps (NFR-4) | None | None | React Flow performance docs + our own benchmark (NFR-4 test) |
| Licenses of React Flow, dagre, elkjs, d3-hierarchy, tiktoken-rs allow our use | None | None | Each project's LICENSE file (official) |
| `tiktoken-rs` count within ±5% of provider usage (NFR-6) | None | None | AC-12.2 measurement on 20 real requests |
| Stable prefixes get provider cache hits | None | None | Provider prompt-caching docs (official) |
| Path-scoped context gives better answers than full context (discovery assumption 3) | None | None | Our own blind replay test; no external source substitutes |

Nothing in this table is graded above "None": no reference was retrieved. No metrics, benchmarks or user results exist yet.

## Reference findings

The catalog has no references, so there are no per-reference findings. Candidates to add (all as `inspiration` unless the owner promotes them), with the pattern to look for and the trade-off to record:

| Candidate | Pattern to extract | Trade-off to note | License check |
| --- | --- | --- | --- |
| React Flow docs (sub-flows, `setCenter`, performance guide) | Custom node rendering, viewport control for "back to main", memoized nodes for large graphs | Library-imposed state model vs. our server-owned positions | Confirm MIT |
| ChatGPT / Claude help articles on editing and branching chats | How mainstream tools present forks (version arrows, new chat) | Hidden branches are simple but lose "keep your place" | n/a (docs, cite only) |
| 2-3 canvas AI chat tools (to be picked after a search) | How they pass parent context, lay out branches, avoid clutter | Generality vs. learning-specific actions | Do not copy UI or code; inspiration only |
| Provider prompt-caching and token-counting docs (OpenAI, Anthropic) | Prefix rules, minimum cacheable length, how to count tokens per model | Provider-specific behaviour behind the gateway (NFR-13) | n/a |
| Postgres docs: recursive CTE, `ltree` | Path query shape for FR-10 | CTE simplicity vs. `ltree` speed | PostgreSQL license (permissive) |
| Axum SSE example | Streaming handler with client disconnect handling (FR-18) | — | Confirm MIT |

Rule for all of them: record the transferable property, never copy code or whole designs from references with unknown or restrictive licenses.

## Open questions

| Question | Why it matters | Cheapest way to find out |
| --- | --- | --- |
| Does any existing tool already do anchor-level branching with merge back for learning? | If yes, the MVP's reason to exist weakens | 1-2 hours: search, try the top 3 canvas chat tools, add them to the catalog with findings |
| Will learners branch instead of typing a follow-up? (phase 1 gate) | Core bet | Clickable prototype or the phase 1 build with 5 learners (requirements success metrics) |
| Which LLM providers and models for main and doubt modes? | Drives tokenizer choice (NFR-6), caching behaviour, privacy notice (FR-21) | Owner decision in architecture; then retrieve those providers' docs |
| Can a local tokenizer meet ±5% for the chosen provider? | NFR-6 / AC-12.2 | Spike: 20 requests, compare local count vs. reported usage |
| Does the fixed layout rule stay readable at 30+ nodes and 3 branch levels? | Canvas clutter risk | Seeded canvas from AC-15.1; time a find-and-return task |
| Is a one-line summary enough to carry a branch's learning? | Lossy merge risk | Discovery test: downstream question that depends on the branch; check the answer uses it |
| Are all canvas/layout dependencies licensed for our use? | Legal | Read each LICENSE file before adding the dependency |
| Does path-scoped context measurably beat full-chat context? | Value claim "clean context" | Blind replay of 10 conversations, both modes |

Next step to unblock evidence: add the candidates above to `references/catalog.json` via the factory reference commands, retrieve them, then replace the "assumption" rows with R# citations.
