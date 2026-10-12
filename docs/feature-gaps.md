# Feature gaps vs. popular design tools

Status of KuArt (0.1 + current `main`) against what designers expect from Figma, Adobe XD,
Sketch, Penpot, Framer and Affinity Designer. ✓ done · ~ partial · ✗ missing.
Priority: **P0** blocks everyday use · **P1** expected by professionals · **P2** nice to have.

## 1. Workspace and navigation

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| Home screen with recent files, thumbnails, search | ✓ | ✗ (in progress) | P0 |
| Main menu (File / Edit / View / Object / Text / Arrange / Help) | ✓ | ✗ | P0 |
| Icon toolbar with tool groups and flyouts | ✓ | ~ text buttons | P0 |
| Pages panel (add, rename, reorder, switch pages) | ✓ | ~ first page only | P0 |
| Layer panel: type icons, show/hide and lock toggles, rename, drag to reorder | ✓ | ~ list + select | P0 |
| Right panel tabs: Design / Prototype / Inspect (Dev Mode) | ✓ | ~ inspector only | P0 |
| Rulers, guides from rulers, pixel grid, layout grids | ✓ | ~ smart guides only | P1 |
| Multiple document tabs | ✓ | ✗ | P1 |
| Command palette / quick actions (Ctrl+K, Ctrl+/) | ✓ | ✗ | P1 |
| Zoom to selection, minimap/navigator | ✓ | ~ fit and 100% | P1 |
| Keyboard shortcut reference and customisation | ✓ | ✗ | P2 |
| Dark and light themes | ✓ | ✓ (Kuduy lime/blue redesign in progress) | — |
| Localisation (UI languages) | ✓ | ✗ | P2 |

## 2. Drawing and editing

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| Frame, rectangle, ellipse, polygon, star, line, text tools | ✓ | ✓ | — |
| Pen tool, Bézier node editing, pencil | ✓ | ✓ pen and point editing (no pencil yet) | P0 |
| Arrow tool, line end caps (arrows, circles) | ✓ | ✗ | P1 |
| Boolean operations (union, subtract, intersect, exclude) | ✓ | ✓ live, with Flatten | P0 |
| Flatten, outline stroke, path offset | ✓ | ✗ | P1 |
| Align and distribute, tidy up, smart spacing | ✓ | ✗ | P0 |
| Copy / paste / duplicate / group / ungroup / arrange | ✓ | ✓ | — |
| Paste between documents, paste SVG/images from other apps | ✓ | ~ copy out as SVG | P0 |
| Drag and drop images and files onto the canvas | ✓ | ✗ | P0 |
| Lock / hide from the canvas, select matching layers | ✓ | ✗ | P1 |
| Rotation, flip, constraints-aware resize | ✓ | ~ rotate/resize, no flip shortcuts | P1 |
| Corner radius per corner, corner smoothing (squircle) | ✓ | ~ data imported, edit uniform only | P1 |
| Masks (layer, clip groups, alpha masks) | ✓ | ✓ vector masks rendered and exported · ✗ alpha masks | P0 |
| Image crop, fill/fit/tile, adjustments (exposure, contrast…) | ✓ | ~ fill/fit only | P1 |
| Scale tool (scale strokes and effects) | ✓ | ✗ | P2 |
| Slices / export regions | ✓ | ✗ | P1 |

## 3. Text

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| On-canvas text editing (caret, selection, IME) | ✓ | ✗ (inspector only) | P0 |
| Rich text per range (font, weight, colour) in the editor | ✓ | ~ rendered, not editable per range | P1 |
| Lists, paragraph spacing, indent, vertical trim | ✓ | ~ spacing imported | P2 |
| OpenType features (ligatures, tabular numbers) | ✓ | ✗ | P2 |
| Font picker with previews, Google Fonts | ✓ | ~ name field + download missing | P1 |
| Text on path | ✓ (Affinity, Illustrator) | ✗ | P2 |

## 4. Styling and effects

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| Multiple fills and strokes per layer | ✓ | ~ rendered, single fill edit | P1 |
| Colour picker with eyedropper, HEX/RGB/HSL, document colours | ✓ | ~ basic | P0 |
| Gradient editor (linear, radial, angular, diamond) | ✓ | ✗ (rendered when imported) | P1 |
| Drop shadow | ✓ | ✓ | — |
| Layer blur · inner shadow, background blur | ✓ | ✓ layer blur (CPU canvas, raster export) · ✗ inner shadow, background blur (data kept) | P1 |
| Blend modes per layer and per fill | ✓ | ✓ render, ~ edit | P2 |
| Shared styles (colour, text, effect, grid) | ✓ | ✗ | P1 |
| Variables / design tokens with modes (light/dark) | ✓ | ~ tokens exported, not edited | P1 |

## 5. Components and systems

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| Create component, instances update from main | ✓ | ✗ (instances imported, expanded) | P0 |
| Overrides, swap instance, reset overrides | ✓ | ✗ | P1 |
| Variants and component properties | ✓ | ✗ | P1 |
| Team / shared libraries | ✓ | ✗ | P2 |
| Assets panel (components, images, icons) | ✓ | ✗ | P1 |

## 6. Layout

| Feature | Figma / others | KuArt | Priority |
|---|---|---|---|
| Auto layout (direction, gap, padding, hug/fill, wrap) | ✓ | ~ imported + exported to code, no reflow | P0 |
| Constraints (left/right/center/scale) on resize | ✓ | ~ imported only | P1 |
| Layout grids (columns, rows, baseline) | ✓ | ✗ | P1 |
| Responsive breakpoints / preview | ✓ (Framer, Penpot) | ✗ | P2 |

## 7. Prototyping and motion

| Feature | Figma / XD | KuArt | Priority |
|---|---|---|---|
| Links between frames, triggers, transitions | ✓ | ✗ | P1 |
| Smart animate / auto-animate | ✓ | ✗ | P2 |
| Presentation / prototype player, device frames | ✓ | ✗ | P1 |
| Scrolling, overlays, interactive components | ✓ | ✗ | P2 |

## 8. Dev handoff

| Feature | Figma Dev Mode / others | KuArt | Priority |
|---|---|---|---|
| Inspect panel: CSS/Tailwind/React/Vue code for the selection | ✓ | ~ CLI + MCP only (panel in progress) | P0 |
| Redlines: distances between layers, spacing, sizes | ✓ | ✗ | P0 |
| Copy values (colour, font, size) with one click | ✓ | ✗ | P1 |
| Export assets per layer (1x/2x/3x, formats, suffixes) | ✓ | ~ one export at 2x | P1 |
| Code connect / component-to-code mapping | ✓ | ✗ | P2 |
| SwiftUI / Flutter / Compose code | ~ plugins | ✗ | P2 |
| AI agent access (MCP) | ✓ (Figma MCP) | ✓ create, edit and generate code locally | — |

## 9. Files, import and export

| Feature | Others | KuArt | Priority |
|---|---|---|---|
| Open Figma, Sketch, XD, PSD, AI, PDF, SVG, images | partial in each tool | ✓ (broadest of any editor) | — |
| Export PNG / JPEG / WebP / SVG / PDF | ✓ | ✓ | — |
| Export compression (quality slider, palette PNG, lossy WebP) | ✓ (TinyPNG-like in some) | ✗ (next) | P1 |
| Export to Sketch, layered PSD | ~ | ✗ | P2 |
| Autosave, crash recovery, version history | ✓ | ✗ | P0 |
| Recent files | ✓ | ✗ (in progress) | P0 |

## 10. Collaboration

| Feature | Figma / Penpot | KuArt | Priority |
|---|---|---|---|
| Real-time multiplayer editing | ✓ (cloud) | ✗ (P2P WebRTC planned: no server, no account) | P1 |
| Live cursors and selections of others | ✓ | ✗ | P1 |
| Comments and annotations | ✓ | ✗ | P1 |
| Share links, permissions | ✓ (cloud) | ✗ (local-first by design) | P2 |
| Branching and merging | ✓ | ✗ | P2 |

## 11. Platform

| Feature | Others | KuArt | Priority |
|---|---|---|---|
| Windows and Linux native apps | ~ (Figma: Electron) | ✓ | — |
| macOS | ✓ | ✗ (no design blocker) | P1 |
| Plugins / scripting API | ✓ | ✗ (MCP covers automation) | P2 |
| Works offline, no account | ✗ Figma, ✓ Sketch/Affinity | ✓ | — |
| Low RAM / old hardware | ✗ Figma (browser) | ✓ (34 MB idle) | — |

## Where KuArt is already ahead

* Opens Figma, Sketch, XD, PSD, Illustrator and PDF files in one app, offline.
* Native and light: about 0.1 s to start, 34 MB idle, 85 MB with 100k layers, 0% idle CPU.
* MCP server that creates, edits and code-generates designs locally, with no cloud account.
* No account, no cloud, no subscription; files are local `.krt`.

## Plan (in order)

1. UI redesign: Lucide icons, Kuduy lime (dark) and blue (light), menu, pages, layer toggles,
   home screen with recent files, Dev Mode panel with code and copy.
2. Export compression options.
3. P0 editing: align/distribute, on-canvas text,
   components that update, auto-layout reflow, autosave and recovery, drag-and-drop.
4. P2P WebRTC collaboration (design in `docs/collaboration.md`).
