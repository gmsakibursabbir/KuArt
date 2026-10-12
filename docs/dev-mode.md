# Developer Mode and Code Generation

Status: **in progress**: `ude-codegen` ships HTML, CSS, Tailwind, React, Vue and tokens emitters
(auto layout → flex, inferred stacks, centred children, semantics, absolute fallback), used by
`design-editor-cli code` and the MCP `generate_code` tool. Not yet: SCSS, grid inference, token
linking to styles, the in-app code panel.

Goal: select any element and get accurate measurements plus production-quality code. Dev Mode is
lazy: `Option<Box<DevMode>>` is `None` until first opened, so there is no baseline RAM cost.

## 1. Pipeline

```
Selection (NodeIds) ─► 1 Normalize ─► 2 Layout inference ─► 3 Semantics ─► 4 Token resolution
                                                                               │
                         ┌─────────────────────── Code IR ◄────────────────────┘
                         ▼
        Emitters: CSS · SCSS · HTML · Tailwind · React(JS/TS, CSS/Tailwind) · Vue(SFC) · SVG · Tokens
```

All stages consume the Universal Design Model only, so Figma, XD, PSD and SVG imports get
identical Dev Mode features. The crate is `ude-codegen`; it depends on `ude-core` and on
`ude-format-svg` (vector layers become inline SVG).

### 1.1 Normalize
* Resolve instances against their components (with overrides) into a temporary view.
* Remove no-op wrappers: a group with a single child and no visual properties is collapsed.
* Compute effective values: opacity (multiplied through pass-through groups), visible paints, and
  resolved text styles per run.

### 1.2 Layout inference

| Input | Output |
|---|---|
| `AutoLayout` present | `display:flex` with direction, gap, padding, align/justify, wrap and child sizing (`fill` → `flex:1 1 0` / `align-self:stretch`, `hug` → auto, fixed → width/height) |
| Children aligned on one axis with consistent spacing (gap variance ≤ 1 px) | inferred flex row/column + gap |
| 2-D alignment in rows and columns with consistent gutters | CSS grid (`grid-template-columns: repeat(n, 1fr)` when column widths are equal) |
| Overlapping children, or deliberate overlays | `position:relative` parent + `position:absolute` child, **only for the overlapping nodes** |
| Constraints left+right / scale | `width:auto` / percentage widths; `max-width` from the frame width when it is a page container |

Absolute positioning is the last resort and is reported in the code panel ("2 elements positioned
absolutely: overlapping layers").

### 1.3 Semantics

| Heuristic | Element |
|---|---|
| Text: largest size in a section/top-level frame, ≥ 24 px | `h1` (descending sizes → `h2`…`h6`) |
| Text: body-sized paragraphs | `p` |
| Frame with one text child, fill or stroke, radius, height 28–64 px, name contains btn/button/cta | `button` (or `a` when the name contains "link" or a prototype link targets a URL) |
| Raster / image fill | `img` with `alt` from the layer name, plus `object-fit` from the scale mode |
| ≥ 3 structurally identical siblings | `ul > li` |
| Top-level frame children named header/nav/footer/main/section/aside, or positional (top bar → header) | landmark elements |
| Vector-only subtree | inline `svg` or an exported asset reference |
| Otherwise | `div` |

Layer names become class names (`kebab-case`, deduplicated, BEM-style `block__element` in SCSS).
Generic names such as "Frame 123" or "Rectangle 4" fall back to the semantic role
(`.card`, `.card__title`).

### 1.4 Token resolution
* When a value links to a style or variable, emit the token (`var(--color-primary)`, Tailwind
  theme key).
* Otherwise, cluster values across the document (colours by ΔE2000 < 1, spacing/radius exact) to
  propose tokens.
* Tailwind mapping: nearest default scale value within tolerance (spacing ±0.5 px, colours
  ΔE < 1.5), else arbitrary values (`p-[13px]`, `bg-[#1d4ed8]`). A user `tailwind.config` (JSON
  export) can be loaded to map custom colours, spacing and breakpoints.

### 1.5 Code IR

```rust
pub struct CodeNode {
    pub tag: Tag,                    // semantic element
    pub class: Symbol,               // generated class name
    pub style: StyleDecls,           // ordered, deduplicated declarations (property, value, token?)
    pub text: Option<RichText>,
    pub attrs: SmallVec<Attr>,       // src, alt, href, aria-*
    pub children: Vec<CodeNode>,
    pub source: NodeId,              // for click-to-select from the code panel
}
```

Before emitting, identical `StyleDecls` are hashed and shared (one class, many elements).
Declarations equal to CSS initial or inherited values are dropped (`flex-direction: row`,
`opacity: 1`, a repeated `font-family` already set on an ancestor).

### 1.6 Emitters

| Emitter | Output |
|---|---|
| CSS | class rules in tree order, shorthand where lossless (`padding: 12px 20px`) |
| SCSS | one block per component root, `&__elem` nesting at most 1 level deep |
| HTML | semantic markup, linked or inline CSS |
| Tailwind | classes sorted in the official order (layout → box → typography → visual) |
| React | function component, `className`, TSX types for props when text/images look like content slots |
| Vue | SFC with `<template>`, `<script setup lang="ts">` and scoped style or Tailwind |
| SVG | the `ude-format-svg` writer for the selection |
| Tokens | CSS variables, SCSS map, JSON (W3C design-tokens draft), TS const, Tailwind `theme.extend` |

Output is deterministic: the same selection always gives byte-identical code, so it can be
snapshot-tested.

## 2. Inspector panels

* **Properties:** position, size, spacing (padding/gap from auto layout or inferred), fill, stroke,
  radius, opacity, blend, typography and effects. Every value has a click-to-copy action.
* **Measurement:** with one object selected, hovering another shows the distances between their
  edges (and to the parent's padding box). Labels are clickable to copy (`24px`).
* **Colour:** HEX, RGB(A), HSL(A), OKLCH (via OKLab: linear sRGB → LMS → cube root → Lab →
  LCh), each with copy.
* **SVG:** path data, fill/stroke, viewBox and dimensions, with Copy SVG / Copy path / Optimize /
  Export.
* **Assets:** source filename, intrinsic vs displayed size, format, byte size, object-fit and
  position, with Export / Copy / Open original.
* **Responsive:** preview widths (Mobile 375, Tablet 768, Desktop 1440, custom), produced by
  running the layout solver at those widths. Shows constraints, sizing modes and the resulting
  rules.

## 3. Code panel

```
┌──────────────────────────────────────┐
│ Inspect | CSS | HTML | Tailwind      │
│ React | Vue | SVG | Tokens           │
├──────────────────────────────────────┤
│  generated code (virtualized lines,  │
│  lightweight highlighter)            │
├──────────────────────────────────────┤
│             Copy Code   ⌃⇧C          │
└──────────────────────────────────────┘
```

Syntax highlighting uses a small hand-written tokenizer per language (CSS, HTML/JSX, Vue, JSON):
about 300 lines each, with no tree-sitter and no web engine. Only visible lines are laid out.

## 4. Quality gates
* Snapshot tests per fixture and emitter (`insta`).
* Visual check in CI only: generated HTML+CSS is rendered with a headless browser **in CI**
  (never in the app) and compared to the design render. Similarity ≥ 0.95 SSIM for the fixture
  set.
* Lint gates: generated CSS passes `stylelint` (standard config), and TSX type-checks with `tsc`.
