# MVP Roadmap

Each milestone is small, compiles on its own, ships tests, and leaves `cargo test --workspace`
green. Status is updated in this file as milestones land.

## Phase 1: MVP

| # | Milestone | Deliverable | Acceptance | Status |
|---|---|---|---|---|
| M0 | Core model | workspace, `ude-core`: arena, ids, interner, geometry, paints, nodes, assets, ops, history with coalescing and byte budget, memory report | unit tests; `Node` ≤ 128 B | **done** |
| M1 | Native format + CLI skeleton | `ude-format-native` (`.design` v0: chunked, varint encoding, zstd, blake3), `design-editor-cli inspect/convert/generate`, `ude-bench` synthetic generator + benchmark | round-trip is byte-identical; 100k-node doc saves/loads; deterministic output | **done**: 100k nodes save 219 ms / load 146 ms, 215 B/node (see [perf-log.md](perf-log.md)) |
| M2 | GPU canvas | `ude-app` window (winit + wgpu + vello): render shapes, paths, solid fills; pan/zoom; idle = no frames; CPU fallback backend | 10k rects pan at 60 FPS on machine A; idle CPU < 0.5% | **done, with one open issue**: GPU 16 ms / CPU 24 ms for a worst-case 10k frame, idle 0% CPU. GPU-path committed memory is ~600 MB, tracked in [perf-log.md](perf-log.md) |
| M3 | Spatial index + selection | R-tree, hit testing, click/marquee/multi-select, overlays | select under the cursor in < 1 ms at 100k nodes | **done**: 2.3 µs avg / 13.6 µs worst at 100k ([perf-log.md](perf-log.md)) |
| M4 | Transform tools | move/resize/rotate with handles, Shift/Alt modifiers, basic snapping, undo/redo wired to the UI | drag coalesces into one undo entry | **done**: unit-tested (50-move drag = 1 undo entry); interactive check pending |

| M5 | Panels | egui shell styled per [ui-design-system.md](ui-design-system.md) (Kuduy tokens, Inter + JetBrains Mono bundled): toolbar, virtualized layers panel, basic inspector, dark/light/system/high-contrast | 100k layers: panel frame < 2 ms; token contrast test passes AA | **done**: panels frame 0.098 ms with 60k rows; AA contrast test passes; CPU renderer only; app 34 MB private |
| M6 | Shape & text tools | rect/ellipse/polygon/line/frame tools; text via parley; missing font warnings | text renders identically on GPU and CPU backends (golden) | next |
| M7 | Images | asset store, lazy scaled decode, image cache + texture LRU | 6000×4000 shown at 300×200 holds ≤ 1 MB decoded | |
| M8 | Export | PNG (CPU backend) and SVG writer | golden-image tests | |
| M9 | SVG import | `usvg` mapping | W3C SVG subset corpus ≥ 90% at SSIM ≥ 0.98 | |
| M10 | PSD import | layer tree, groups, raster (lazy RLE/ZIP), opacity/blend/visibility, clipping, raster masks, text (data + pixel fallback), preserved blocks | corpus renders match the merged image at SSIM ≥ 0.97 | |
| M11 | Figma import | `.fig` container, kiwi schema decode, tree rebuild, shapes/vectors/text/fills/images | corpus vs reference PNGs | |
| M12 | XD import | UCF zip, AGC → model, artboards, shapes, text, images | corpus vs reference PNGs | |
| M13 | Diagnostics + baselines | memory panel, debug panel, benchmark scenarios, first published numbers | MVP gate: [benchmark-plan.md](benchmark-plan.md) §5 | |
| M14 | Release packaging | release workflow: Windows msi/zip (signed), Linux AppImage/deb/rpm, CLI archives; Flatpak + winget manifests ([distribution.md](distribution.md)) | clean-machine install smoke test on the §7 matrix | |

CI runs `fmt`, `clippy` and `test` on **Windows and Linux** for every change from M0 onward.

## Phase 2: Professional editing
Vector/Bézier editing and pen tool · boolean operations (`flo_curves` or a `kurbo`-based
implementation, evaluated first) · gradients editor · masks · effects (shadow/blur passes) ·
components/instances/variants · auto layout solver · constraints · better PSD (shapes, effects,
vector masks) · better XD (masks, symbols, repeat grid) · better Figma (instances, auto layout,
variables) · autosave journal + crash recovery · multi-document tabs + memory pressure system ·
PDF export.

## Phase 3: Dev Mode
Inspector, measurements, colour inspector → codegen IR and layout inference → CSS/HTML →
Tailwind → React/TS → SCSS → Vue → design tokens → SVG/asset inspectors → responsive preview.
See [dev-mode.md](dev-mode.md).

## Phase 4: Ecosystem
Round-trip export (PSD first) · advanced PSD/XD/Figma features · WASM plugins · automation API
(`ude-api`) + MCP server + REST · VS Code integration · PDF import · web exporters
(WordPress/Elementor/Next.js), each as a separate optional crate.
