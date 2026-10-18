---
name: factory-design
description: UI/UX visual design direction. Use in the Design phase to complete docs/product/design-brief.md (mood, typography, color tokens, spacing, layout, motion, states, responsive, accessibility) and when implementing or fixing UI.
---
# Design

Input: requirements, visual references (references/catalog.json, type image/url), the style notes the
factory pre-filled in docs/product/design-brief.md. Complete the brief; replace every `TODO(factory)`.

Five principles (apply all): 1 clear mood, 2 deliberate typography, 3 intentional whitespace,
4 purposeful hero composition, 5 consistent color system.

Deliver concretely:
- Mood: 3 adjectives + what the product must never feel like.
- Typography: font families (with fallbacks and license), type scale (px/rem), weights, line heights.
- Color: semantic tokens (bg, surface, text, muted, primary, accent, success, warning, danger, border) with
  hex values for light and dark; contrast ratio for text pairs (target WCAG AA 4.5:1, 3:1 large text).
  60-30-10 is an optional heuristic, not a rule.
- Spacing: base unit and scale; container widths; section rhythm.
- Layout & components: grid, breakpoints, key components with variants.
- Hero: goal (conversion/action), composition, primary CTA.
- Motion: durations/easing; respect prefers-reduced-motion.
- States: loading, empty, error, success for each key screen.
- Accessibility: keyboard paths, focus styles, labels, alt text, contrast.
From visual references extract properties (palette, type, density, shapes); never clone a site.
When implementing, express tokens as CSS variables / theme config so the system stays consistent.
