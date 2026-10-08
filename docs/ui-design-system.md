# UI Design System: "Kuduy" premium

Status: **draft, pending owner confirmation** · Implemented in `ude-ui::theme` (Milestone 5)

The editor chrome follows the Kuduy visual language taken from the *KU Client Hunter UI mockups*:
deep navy surfaces, cool grey text, violet as the primary accent, and cyan/green/pink/amber as
semantic accents, with JetBrains Mono for numbers and code.

> **Assumption to confirm:** the mockups define the colours and mono font but no UI sans-serif.
> This document proposes **Inter** (SIL OFL, bundled as a ~300 KB subset, variable weight). Swap
> it if Kuduy has a brand sans.

## 1. Principles

1. **The design is the hero.** The chrome is quiet and dark so user artwork carries the colour.
   Accents mark only *state* (selection, focus, components, warnings), never decoration.
2. **Dense but calm.** These are professional panels: 28 px rows, 4 px spacing grid, 12 px UI
   text. Hierarchy comes from tone and weight, not boxes inside boxes.
3. **Hairlines, not shadows.** Panels are separated by 1 px borders. Shadows are used only for
   floating layers (menus, popovers, toasts).
4. **Numbers are first-class.** All measurements, coordinates and values use tabular JetBrains
   Mono, so columns align and values don't jitter while dragging.
5. **Motion is functional and cheap.** 120 ms ease-out for panels and menus, no animation on
   canvas content, and none at all when `reduce motion` is on. Nothing animates while idle, so
   idle CPU stays at zero.

## 2. Colour tokens

### Dark (default)

| Token | Hex | Use |
|---|---|---|
| `bg.canvas` | `#070911` | area around artboards |
| `bg.app` | `#0A0D18` | window background, title bar |
| `bg.panel` | `#1A1E30` | side panels, toolbar |
| `bg.raised` | `#1E2336` | inputs, cards, menus |
| `bg.hover` | `#23283C` | row and button hover |
| `border.subtle` | `#2A3048` | panel dividers, input borders |
| `border.strong` | `#3B4160` | focused input border (with ring), separators in menus |
| `text.primary` | `#EEF1FA` | values, titles |
| `text.secondary` | `#C4C9DD` | labels |
| `text.muted` | `#8D94AF` | hints, section headers |
| `text.faint` | `#5F6684` | placeholders, disabled icons |
| `text.disabled` | `#4C5370` | disabled text |
| `accent.primary` | `#A78BFA` | selection, focus ring, primary buttons, active tool |
| `accent.dev` | `#67E8F9` | Dev Mode, measurements, spacing redlines |
| `accent.component` | `#F472B6` | components, instances, variants |
| `state.success` | `#4ADE80` | fully supported, saved, export done |
| `state.warning` | `#FBBF24` | ⚠ partially supported, missing fonts |
| `state.error` | `#F87171` | import errors, broken references |
| `state.preserved` | `#FB923C` | preserved-but-not-editable badges |

### Light

Derived with the same hues at a lower chroma for paper-white surfaces:

| Token | Hex |
|---|---|
| `bg.canvas` | `#E9EBF2` |
| `bg.app` / `bg.panel` | `#F6F7FB` / `#FFFFFF` |
| `bg.raised` / `bg.hover` | `#FFFFFF` / `#EEF1FA` |
| `border.subtle` / `border.strong` | `#E1E4EE` / `#C4C9DD` |
| `text.primary` / `secondary` / `muted` / `faint` | `#0A0D18` / `#2A3048` / `#5F6684` / `#8D94AF` |
| `accent.primary` | `#7C5CF0` (darkened for AA contrast on white) |
| `accent.dev` | `#0891B2` |
| `accent.component` | `#DB2777` |
| `state.success` / `warning` / `error` / `preserved` | `#16A34A` / `#B45309` / `#DC2626` / `#C2410C` |

### High contrast

`text.*` collapses to `#FFFFFF`/`#000000`, borders become 2 px `text.secondary`, and accents use
the light-theme values on dark (or the reverse). It is a token swap only, with no extra rendering
cost.

**Contrast rule:** each `text.*` token on `bg.panel` and `bg.raised` must meet WCAG AA (4.5:1 for
`primary`/`secondary`, 3:1 for `muted`). A unit test in `ude-ui` computes this from the token
table, so a palette change can't silently break accessibility.

## 3. Typography

| Role | Font | Size / line | Weight |
|---|---|---|---|
| Panel title | Inter | 13 / 20 | 600 |
| Label | Inter | 12 / 16 | 500 |
| Body / menu | Inter | 12 / 18 | 400 |
| Section header | Inter, uppercase, +4% tracking | 11 / 16 | 600 |
| Values, coordinates | JetBrains Mono (tabular) | 12 / 16 | 400 |
| Code panel | JetBrains Mono | 12 / 18 | 400 |

Both fonts are bundled, so the UI looks identical on Windows and Linux regardless of installed
system fonts. User design fonts still come from the system.

## 4. Spacing, shape and elevation

* **Spacing scale:** 2, 4, 8, 12, 16, 24, 32. Panel padding is 12, between property rows 4, and
  between sections 16.
* **Row height:** 28 (layers, inputs). Compact mode is 24, for a user setting and small screens.
* **Radii:** 4 (chips, checkboxes), 6 (inputs, buttons), 8 (menus, cards), 12 (dialogs).
* **Elevation:** panels have none and use borders. Menus get `0 8px 24px rgba(0,0,0,.45)` plus a
  1 px `border.subtle`. Dialogs get `0 16px 48px rgba(0,0,0,.55)`.
* **Focus:** a 2 px `accent.primary` ring with 1 px offset on every focusable control, visible
  for keyboard navigation only.

## 5. Window layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ◆ kuArt   File Edit View Object   [Website.fig][Landing.xd][Design.psd] ⚙ ─□×│  bg.app, 40 px
├────┬────────────────┬───────────────────────────────────────┬────────────────┤
│ ▢  │ Layers   Assets│                                       │ Design │ Dev   │
│ ◯  │ ▾ Header       │                                       │ X 120  Y 48    │
│ ✎  │   ▸ Logo       │            canvas (bg.canvas)         │ W 160  H 48    │
│ T  │   ▸ Navigation │                                       │ ─ Fill ─────   │
│ ▣  │ ▾ Hero         │                                       │ ■ #2563EB 100% │
│ ⌗  │                │                                       │ ─ Typography ─ │
│    │                │                                       │                │
├────┴────────────────┴───────────────────────────────────────┴────────────────┤
│ 100% · 48,212 nodes · 143 MB                    ⚠ 2 partially supported      │  status bar, 24 px
└──────────────────────────────────────────────────────────────────────────────┘
  tool rail 44 px · left panel 240 px (resizable) · right panel 280 px (resizable)
```

* The title bar is custom drawn (tabs inside it) on Windows. On Linux we keep server-side
  decorations by default, with client-side decorations optional on Wayland (see distribution.md).
* The Dev tab in the right panel switches the inspector to Dev Mode and tints its header line with
  `accent.dev`.

## 6. Canvas overlay language

| Element | Style |
|---|---|
| Hover outline | 1 px `accent.primary` |
| Selection box | 1 px `accent.primary`; 8 px square white handles with a 1 px accent border |
| Component selection | same, with `accent.component` |
| Measurements / redlines | 1 px `accent.dev` lines; value pill in `accent.dev` with `bg.app` text, mono 11 px |
| Smart guides / snapping | 1 px `accent.component`, matching Figma users' expectation of a contrasting guide colour |
| Auto layout padding/gap | hatched `accent.primary` at 15% opacity |
| Pixel grid (≥ 800%) | `#FFFFFF` at 6% |

Overlays are drawn in screen space at device pixel ratio, so lines stay 1 physical pixel crisp at
any zoom.

## 7. Implementation notes

* One token table in `crates/ui/src/theme/tokens.rs` produces `egui::Visuals`/`Style` and the
  overlay palette. Themes are plain data, so switching costs one restyle and no extra memory.
* Icons are a single monochrome SVG icon set rasterized into one atlas at the current DPI scale
  (16/20 px), and tinted by token at draw time.
* UI scale (90–200%) multiplies all metrics. High DPI comes from `winit`'s scale factor.
