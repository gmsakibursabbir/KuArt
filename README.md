# KuArt: Universal Design Editor

A native Rust design editor (no Electron, no browser engine) that opens, edits and exports Figma,
Adobe XD, Photoshop, SVG and PDF designs in one GPU-accelerated canvas, with a built-in Dev Mode
for CSS/HTML/Tailwind/React/Vue output. It is built to run well on older PCs with 8 GB of RAM.

**Status: Milestone 9 done.** SVG files open (Ctrl+O or CLI). SVG export now wraps text exactly like the canvas.

M8: Open (Ctrl+O), Save (Ctrl+S), Save As (Ctrl+Shift+S) and Export to PNG/SVG (Ctrl+Shift+E) in the app; `design-editor-cli convert` to .png/.svg.

M7: Images render (decoded at the size on screen, bounded cache) and PNG/JPG/WebP files open directly. App icon: `assets/icon.png`.

M6: Drawing tools (frame, rectangle, ellipse, polygon, star, line, text; V F R O P S L T) and real text rendering with missing-font warnings.

M5: Kuduy-styled panels (layers, inspector, top and status bars, dark/light/system/high-contrast themes) around the canvas.

M4: You can select layers (click, deep select, Shift multi-select,
double-click into groups, marquee) and edit them: move with snapping, resize and rotate with
handles (Shift/Alt modifiers), nudge, delete, and undo/redo. Each drag is one undo step.

Before that, M2 delivered the canvas: A native window renders documents with a multithreaded CPU renderer by
default (about 21 MB for a 10k-node document), or on the GPU with `--gpu`. It supports pan/zoom and
uses zero CPU while idle. Underneath sit the scene graph,
undo/redo, the native `.krt` format, a headless CLI and benchmarks. Editing tools come next (M3+).
See the [roadmap](docs/roadmap.md) and [perf log](docs/perf-log.md), which explains why the CPU renderer is
the default for now.

## Documents

| | |
|---|---|
| [Architecture](docs/architecture.md) | system, modules, data flow, tech choices |
| [Universal model](docs/universal-model.md) | the internal scene graph |
| [Format strategy](docs/format-strategy.md) | Figma / XD / PSD / SVG / PDF / `.krt` |
| [Memory strategy](docs/memory-strategy.md) | RAM, VRAM, CPU, disk I/O |
| [Rendering strategy](docs/rendering-strategy.md) | GPU/CPU, caching, tiling, dirty regions |
| [Dev Mode](docs/dev-mode.md) | inspector and code-generation pipeline |
| [UI design system](docs/ui-design-system.md) | Kuduy premium look: tokens, type, layout |
| [Distribution](docs/distribution.md) | Windows and Linux builds, packages, signing, CI |
| [Benchmark plan](docs/benchmark-plan.md) | reproducible benchmarks and success criteria |
| [Compatibility matrix](docs/compatibility-matrix.md) | honest per-feature status |

## Build

Requires stable Rust (1.85+). Builds on Windows and Linux.

```bash
cargo test --workspace
```

```bash
cargo run --release -p ude-app -- --stats
```

With no file argument, the app opens a 10k-node demo. Pass a `.krt` file to open it. The CPU renderer is the default (lowest memory); `--gpu` opts into
the GPU renderer. Controls: wheel to pan, Ctrl+wheel to zoom, Space+drag or middle-drag to
pan, Shift+1 to fit, Ctrl+0 for 100%, F3 for stats.

```bash
cargo run --release -p ude-bench
```

```bash
cargo run --release -p ude-cli -- generate demo.krt --nodes 10000
```
