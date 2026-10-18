# Design brief

Inputs: [requirements.md](requirements.md) (phase 1 MVP), [discovery.md](discovery.md), `references/catalog.json`.
The reference catalog is empty, so nothing below comes from a visual reference. Every visual choice is a design decision for phase 1, and where it depends on how users will behave it is marked **assumption**.

Phase 1 screens: **Sign in**, **Canvas list**, **Canvas** (main thread, branches, merge, notices), **Account** (delete). There's no marketing site in phase 1.

<!-- factory:style-prefill -->
## Style direction

**Calm editorial workspace**: a reading-first document style placed on a quiet canvas. Answer text gets the same care as a good long-form article. Canvas chrome (edges, toolbars, controls) stays thin and neutral so the content does the talking.

Why this fits:
- The primary users are self-learners in long technical study sessions (discovery). Most of their time goes to **reading answers**, not arranging a board, so typography and line length come before canvas decoration.
- The core interaction is "doubt beside the answer". The only strong visual signal in the system is the **branch** (accent color, anchor highlight). That keeps main vs. detour readable at a glance (FR-15 to FR-17).
- A whiteboard style (hand-drawn, sticky notes) was rejected for phase 1. It competes with the text, and the brief saves the hand-drawn look (Rough.js, Caveat) for phase 2 sketchnotes, where it will stand out *because* the canvas is plain.

## Mood

- **Calm**: low-saturation neutrals, generous whitespace, no attention-grabbing chrome.
- **Focused**: one readable column per node; one input; the current path is always clear.
- **Trustworthy**: precise type, honest states (streaming, failed, partial), visible token count.

Must never feel like: a gamified edtech app (streaks, confetti, mascots), a noisy infinite whiteboard, or a developer dashboard full of metrics.

## Audience and goals

| User | What the design must do |
| --- | --- |
| Self-learner (primary) | Read long answers comfortably; branch a doubt in ≤ 2 clicks after a highlight (NFR-5); get back to the main thread in one action (FR-17) |
| Student | Same as above. Collapse and merge keep a long session scannable for later review |

The design goal follows the phase 1 gate: branching and returning must **feel effortless** (median highlight → sent < 10 s, ease ≥ 6/7; targets, not measurements).

## Typography

| Role | Family | Fallback stack | License |
| --- | --- | --- | --- |
| UI and answer text | Inter (variable) | `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif` | SIL OFL 1.1 |
| Code blocks, token counter digits | JetBrains Mono | `ui-monospace, "SF Mono", Menlo, Consolas, monospace` | SIL OFL 1.1 |

Self-host both through `next/font` (no third-party font requests). Use `font-variant-numeric: tabular-nums` on token counters so the digits don't jump while they update.

Type scale (root 16px):

| Token | Size | Line height | Weight | Use |
| --- | --- | --- | --- | --- |
| `--text-xs` | 12px / 0.75rem | 1.4 | 500 | Token counter, chips, timestamps |
| `--text-sm` | 14px / 0.875rem | 1.45 | 400 / 500 | UI labels, buttons, toolbar, merged summary |
| `--text-base` | 16px / 1rem | 1.6 | 400 | Answer and question body |
| `--text-code` | 14px / 0.875rem | 1.55 | 400 | Code blocks (mono) |
| `--text-lg` | 18px / 1.125rem | 1.4 | 600 | Markdown h3 inside answers, dialog titles |
| `--text-xl` | 20px / 1.25rem | 1.3 | 600 | Markdown h2 inside answers |
| `--text-2xl` | 24px / 1.5rem | 1.25 | 600 | Page titles (Canvas list), empty-canvas prompt |

Weights: 400 body, 500 UI emphasis, 600 headings. No other weights. Answer bodies in main nodes are limited to about 70 characters per line by the node width (see Layout).

## Color

Semantic tokens, light and dark. Ratios were computed by hand with the WCAG 2.1 relative-luminance formula, so automated tooling hasn't checked them yet. **Assumption until verified**: the CI accessibility check (axe in Playwright) must confirm them.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--bg` | `#F7F6F3` | `#121314` | Canvas background, page background |
| `--surface` | `#FFFFFF` | `#1C1D1F` | Nodes, top bar, dialogs, toolbar |
| `--text` | `#1C1B19` | `#ECEAE6` | Body text |
| `--muted` | `#5F5B54` | `#A19D96` | Secondary text, main-thread edges, counters |
| `--primary` | `#2F5BD3` | `#8AA9FF` | Primary buttons, links, focus ring, active-node marker |
| `--on-primary` | `#FFFFFF` | `#121314` | Text on primary buttons |
| `--accent` | `#A84B0C` | `#F0A35E` | Branches: branch edges, branch node rule, anchor underline |
| `--highlight` | `#FCE8B2` | `#5A4413` | Anchor highlight background in the origin node |
| `--success` | `#1E7A46` | `#5CC48A` | "Merged" label, saved confirmations |
| `--warning` | `#8A5A00` | `#E7B54A` | Context near budget / compaction notice |
| `--danger` | `#B42318` | `#F97066` | Stream errors, delete actions |
| `--border` | `#DCD8D0` | `#34363A` | Decorative dividers and node card outlines (paired with shadow) |
| `--border-strong` | `#8C877E` | `#6E6A64` | Input borders, selected node outline (meets 3:1 for UI components) |

Contrast (hand-computed):

| Pair | Light | Dark | Requirement |
| --- | --- | --- | --- |
| text / surface | 17.2:1 | 14.0:1 | 4.5:1 ✓ |
| text / bg | 15.9:1 | 15.5:1 | 4.5:1 ✓ |
| muted / surface | 6.8:1 | 6.3:1 | 4.5:1 ✓ |
| muted / bg | 6.3:1 | 6.9:1 | 4.5:1 ✓ |
| primary / surface (links) | 5.9:1 | 7.4:1 | 4.5:1 ✓ |
| on-primary / primary (buttons) | 5.9:1 | 8.2:1 | 4.5:1 ✓ |
| accent / surface (branch labels) | 5.7:1 | 8.1:1 | 4.5:1 ✓ |
| accent / bg (branch edges) | 5.3:1 | 9.0:1 | 3:1 ✓ |
| success / surface | 5.4:1 | 7.8:1 | 4.5:1 ✓ |
| warning / surface | 5.9:1 | 8.9:1 | 4.5:1 ✓ |
| danger / surface | 6.6:1 | 6.1:1 | 4.5:1 ✓ |
| text / highlight | 14.2:1 | 7.7:1 | 4.5:1 ✓ |
| border-strong / surface | 3.6:1 | 3.1:1 | 3:1 ✓ |
| primary focus ring / bg | 5.5:1 | 8.2:1 | 3:1 ✓ |

Rules:
- Proportions are roughly neutral 60 (bg) / 30 (surface) / 10 (primary + accent). This is a heuristic, not a rule.
- **Primary = action, accent = branch.** Never use accent for buttons or primary for branch visuals.
- `--border` (≈1.3:1) is decorative only. Anything a user must perceive as a boundary (inputs, selected node, focus) uses `--border-strong` or `--primary`.
- No tinted status backgrounds in phase 1. Status shows through a 4px left rule, an icon and text, so no extra contrast pairs to verify.
- Theme follows `prefers-color-scheme`. A manual toggle is skipped until users ask for one.

```css
:root {
  --bg:#F7F6F3; --surface:#FFFFFF; --text:#1C1B19; --muted:#5F5B54;
  --primary:#2F5BD3; --on-primary:#FFFFFF; --accent:#A84B0C; --highlight:#FCE8B2;
  --success:#1E7A46; --warning:#8A5A00; --danger:#B42318;
  --border:#DCD8D0; --border-strong:#8C877E;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg:#121314; --surface:#1C1D1F; --text:#ECEAE6; --muted:#A19D96;
    --primary:#8AA9FF; --on-primary:#121314; --accent:#F0A35E; --highlight:#5A4413;
    --success:#5CC48A; --warning:#E7B54A; --danger:#F97066;
    --border:#34363A; --border-strong:#6E6A64;
  }
}
```

## Spacing

- Base unit **4px**. Scale: `--space-1` 4, `-2` 8, `-3` 12, `-4` 16, `-6` 24, `-8` 32, `-12` 48, `-16` 64 (px).
- Node padding: 16px horizontal, 16px vertical. Paragraph gap inside answers: 12px. Code block margin: 16px.
- Radius: 8px nodes, dialogs and toolbars; 6px buttons and inputs; 999px chips.
- Shadow (nodes, toolbar): `0 1px 2px rgb(0 0 0 / .06), 0 2px 8px rgb(0 0 0 / .04)`; in dark mode, none (border only).
- Page containers (Sign in, Canvas list, Account): max width 720px, centered, 48px top padding. Section rhythm on these pages: 32px between sections, 16px between items.

Canvas layout constants (input to FR-15 auto-layout; tune in build):

| Constant | Value |
| --- | --- |
| Main-thread node width | 640px (≈ 70 ch body at 16px Inter after padding; **assumption**, check rendered measure) |
| Branch node width | 480px |
| Vertical gap between nodes on a path | 32px |
| Horizontal gap, parent → branch column | 96px |
| Gap between sibling branch stacks | 24px |

## Layout

Desktop-only (NFR-14). Supported viewport is ≥ 1024px wide; the design is drawn at 1440 × 900.

**Canvas screen** (the core screen):
- **Top bar**, 48px, `--surface`, bottom `--border`. Left: back to canvas list, canvas title (click to rename, FR-2). Right: "AI answers may contain errors" notice (always visible, `--muted`, `--text-xs`, FR-21), account menu.
- **Canvas**, full-bleed below the top bar, `--bg`, with a subtle dot grid in `--border`. The main thread is a vertical column; branches open to the right of their parent, aligned to the anchor's vertical position (FR-15).
- **Composer**, docked bottom-center, 640px wide, `--surface`. It always targets the **active node**: by default the last node on the main thread, or any node the user selects. A chip above the input says where the reply goes ("Main thread" or "Branch · Simpler"), next to the token counter for that path (FR-12). Enter sends, Shift+Enter adds a newline.
- **Canvas controls**, bottom-left: zoom in/out, fit view. A "Back to main" button appears bottom-right whenever the active node is in a branch.

**Canvas list**: page title "Your canvases", a "New canvas" primary button, then a single-column list of rows (title, last-updated, overflow menu: rename, delete).

**Sign in**: centered card, 400px wide: product line ("Ask any doubt beside the answer, without losing your place."), email field, primary "Continue" button. The exact flow (magic link vs. password) follows the architecture decision (FR-1).

### Hero (empty canvas)

There's no marketing page in phase 1, so the "hero" is the **empty canvas**, the first thing a new user sees after creating a canvas.
- **Goal:** get the first question asked (starts journey J1).
- **Composition:** composer centered vertically (not docked). Above it, `--text-2xl` prompt "What do you want to understand?" and one `--muted` line: "Highlight any part of an answer to ask a doubt beside it." No illustration.
- **Primary CTA:** "Ask" (primary button, Enter). After the first send, the composer animates to its docked position and the first node appears at the top of the column.

## Components

| Component | Variants / states | Notes |
| --- | --- | --- |
| **Question node** (`user_message`) | main, branch; selected; focused | `--text-sm` 500 label "You"; body `--text-base`. Branch variant shows a doubt-type chip ("Simpler", "Example", "Why", "Deeper", "Check", "Question") |
| **Answer node** (`ai_message`) | streaming, complete, error; main, branch; selected; focused; has-anchor | Markdown + code blocks. Footer: token counter, node actions (Branch, Collapse/Expand, Merge on branch roots, Back to main on branch nodes). Branch variant has a 4px `--accent` left rule plus a "Branch" label, so the variant doesn't depend on color alone |
| **Anchor highlight** | text, code line range | `--highlight` background + 2px `--accent` underline in the origin node; code ranges get a left gutter bar in `--accent` |
| **Selection toolbar** | text selection, code selection | Floats 8px above the selection. Five quick actions as text buttons + an "Ask…" input. One click on a quick action sends the branch (NFR-5). Disabled with a tooltip while the answer is streaming (AC-6.2) |
| **Edge** | reply, branch | Reply: 1.5px solid `--muted`, straight vertical. Branch: 1.5px solid `--accent`, smooth step from the anchor's y-position on the parent's right edge |
| **Collapsed branch stub** | collapsed, collapsed + merged | Pill on the origin node's right edge: "N hidden" (+ merged summary if merged). Click/Enter expands (FR-16) |
| **Merged summary row** | shown on origin node | `--success` "Merged" label + summary text in `--text-sm`. Click expands the branch |
| **Merge dialog** | generating, editing, saving, error | Modal: proposed summary in a single-line text field (≤ 200 chars, live count), "Merge" primary, "Cancel" (FR-13) |
| **Token counter** | normal, near budget, compacting | Mono `--text-xs` `--muted`, e.g. "3,412 tokens". Above 80% of the budget, it turns `--warning` with "older messages will be summarized" (FR-11; the 80% threshold is an **assumption**) |
| **Buttons** | primary, secondary (border-strong outline), ghost, danger | Height 32px (toolbar) / 36px (pages). Visible focus on all |
| **Inputs** | default, focus, disabled, invalid | 1px `--border-strong`, 2px `--primary` focus ring; invalid = `--danger` border + message text |
| **Confirm dialog** | delete canvas, delete account | Danger button. Account deletion requires typing "delete" (FR-20) |

## Motion

| Interaction | Duration | Easing |
| --- | --- | --- |
| Hover, focus, color changes | 120ms | `ease-out` |
| Toolbar appear, dialog open, collapse/expand | 200ms | `cubic-bezier(0.2, 0, 0, 1)` |
| Back to main / center on node (React Flow `setCenter`) | 400ms | `cubic-bezier(0.2, 0, 0, 1)` |
| New node placement | 200ms fade + 8px slide from its edge direction | same |

- Streaming text appears as it arrives, with a static caret at the end. Tokens don't animate in one by one.
- `prefers-reduced-motion: reduce`: every duration drops to 0 (pan/zoom jumps straight there), no slides, no skeleton pulse.

## States

| Screen / element | Loading | Empty | Error | Success |
| --- | --- | --- | --- | --- |
| Sign in | Button shows spinner + "Signing in…", disabled | n/a | Inline message under the field (invalid email / failed request), field keeps its value | Redirect to Canvas list |
| Canvas list | 3 skeleton rows | "No canvases yet" + "New canvas" primary button | Banner "Couldn't load canvases" + Retry | List shown; new canvas opens straight away |
| Canvas (load) | Centered spinner + "Opening canvas…" (target < 2 s, NFR-3) | Empty-canvas hero (see Hero) | "Couldn't open this canvas" + Retry + back to list; 404 shows "Canvas not found" | Canvas restored to saved positions, collapse state and viewport (FR-19) |
| Answer node | Before the first token: "Thinking…" in `--muted` + caret; then streaming text | n/a | 4px `--danger` left rule, icon, "Answer interrupted", partial text kept, "Retry" button (FR-18) | Footer actions and token counter appear; highlight-branching becomes possible |
| Sending a question | Composer disabled while the request starts; the question node appears right away | Send disabled while input is empty | Question stays in the composer with an error line + Retry; never lost | Composer clears |
| Selection toolbar | Disabled while streaming | n/a | Branch request fails → the new branch node shows the answer error state | Branch node appears to the right, gets focus, and the viewport pans to it if it's off-screen |
| Merge dialog | "Writing summary…" skeleton in the field | n/a | "Couldn't write a summary" + Retry, or let the user write one by hand | Dialog closes, branch collapses, origin shows the summary row |
| Token counter | "—" while computing | n/a | "—" with tooltip "Count unavailable" | Number shown; updates when the path changes |
| Rename / delete | Inline spinner | n/a | Toast with message; title reverts | Toast "Canvas deleted" (no undo in phase 1) |
| Account deletion | Button spinner | n/a | Inline error, account unchanged | Signed out, back to Sign in with "Account deleted" |

Toasts: bottom-center above the composer, 4s, `role="status"`; errors use `role="alert"` and stay until dismissed.

## Responsive

- Supported: ≥ 1024px wide, latest Chrome, Firefox, Safari on desktop (NFR-14). Mobile is out of scope.
- 1024–1279px: composer width shrinks to `min(640px, 100vw - 96px)`. The top-bar notice shortens to "May contain errors".
- ≥ 1280px: as designed.
- The canvas scales with zoom (min 0.25, max 1.5). Text at low zoom isn't meant to be readable; "fit view" and "back to main" restore zoom 1.0 on the target node.

## Accessibility

Target WCAG 2.1 AA (NFR-15).

**Keyboard paths**
- Tab order: top bar → canvas nodes (main thread top to bottom, each node followed by its branches) → canvas controls → composer.
- On a focused node: `Enter` makes it the active node and moves focus to the composer; `B` opens "branch from whole node" (quick actions + Ask); `C` collapses/expands its branches; `M` merges (branch roots); `H` goes back to main. These single-key shortcuts only work while a node has focus, never inside text inputs (WCAG 2.1.4). They're listed in a "Keyboard shortcuts" dialog opened with `?`.
- Arrow keys on a focused node: ↑/↓ previous/next on the same path, → first branch, ← parent. **Assumption**: worth the build cost for keyboard users; drop it if React Flow focus handling makes it costly, since Tab order alone meets NFR-15.
- Text highlight anchors work by keyboard selection (Shift+arrows with caret browsing, F7 in Chrome/Firefox). Once text is selected, the selection toolbar is the next Tab stop. Whole-node branching (`B`) is always available as the keyboard fallback.
- Dialogs trap focus, close on `Esc`, and return focus to the element that opened them.

**Focus styles**: 2px `--primary` outline with 2px offset on every interactive element and on nodes (`:focus-visible`). Never removed.

**Labels and semantics**
- Each node: `role="article"` with `aria-label`, e.g. "Answer 4, main thread" / "Question, branch from answer 2: Simpler".
- Icon-only buttons (zoom, collapse, overflow) have `aria-label`s; toolbar quick actions are text buttons.
- Streaming answer: `aria-busy="true"` while streaming. A polite live region announces "Answer complete" / "Answer interrupted", not every token.
- Token counter: `aria-label="Context: 3,412 tokens"`.
- Anchor highlight has visually hidden text "(asked about in a branch)", so it doesn't rely on color.

**Alt text**: phase 1 has no images. Decorative icons are `aria-hidden`. Model-produced images in markdown (unlikely) render their alt text, or "Image from answer" if there is none.

**Contrast**: see the Color table. All text pairs ≥ 4.5:1, UI boundaries and focus ≥ 3:1. Status and branch never rely on color alone (label, icon or rule shape too). Verify with axe in Playwright for both color schemes.

## References

`references/catalog.json` is empty, so no visual reference was used and no palette, type or density was extracted from one. If references get added later, record extracted properties here (palette, type, density, shapes) and tag each `requirement` or `inspiration`. Never clone a site.
