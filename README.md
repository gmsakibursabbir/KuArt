# KuArt: Universal Design Editor

A native Rust design editor (no Electron, no browser engine) that opens, edits and exports Figma,
Adobe XD, Photoshop, SVG and PDF designs in one GPU-accelerated canvas, with a built-in Dev Mode
for CSS/HTML/Tailwind/React/Vue output. It is built to run well on older PCs with 8 GB of RAM.

**Status: 0.1.0, the first release (MVP milestones M0–M14).** Downloads for Windows (installer,
MSI, portable) and Linux (AppImage, deb, rpm, Arch) are on the
[releases page](https://github.com/gmsakibursabbir/KuArt/releases).

## What it does

* **Opens** KuArt `.krt` documents, Figma files (`.fig`, `.jam`, `.deck`, `.buzz`, `.site`,
  `.make`), Sketch (`.sketch`), Adobe XD (`.xd`), Photoshop (`.psd`, `.psb`), Illustrator (`.ai`),
  PDF, SVG and PNG/JPEG/WebP images. Unsupported features are listed in the
  [compatibility matrix](docs/compatibility-matrix.md); the importers report what they skip.
* **Edits:** select (click, marquee, deep select), move with snapping, resize and rotate, frame,
  rectangle, ellipse, polygon, star, line and text tools, layers panel, inspector, copy/cut/paste,
  duplicate, group/ungroup, layer order, undo/redo.
* **Exports** PNG and SVG, and **turns designs into code**: HTML, CSS, Tailwind, React, Vue and
  design tokens (`design-editor-cli code`, or the MCP `generate_code` tool).
* **Works with AI agents:** `design-editor-cli mcp` is an [MCP server](docs/mcp.md) that can
  create, edit, preview, save and code-generate designs.
* **Fonts:** system fonts plus a KuArt fonts folder; missing fonts are named in the status bar,
  with a one-click Google Fonts download.
* **Light:** 34 MB private memory with an empty document, 85 MB with 100k layers, 0% CPU when
  idle, about 0.1 s to the first frame ([perf log](docs/perf-log.md)). Offline first: it only goes
  online when you click a font download.

Shortcuts: V F R O P S L T (tools), Ctrl+O / S / Shift+S (open, save, save as), Ctrl+Shift+E
(export), Ctrl+Z / Y (undo, redo), Ctrl+C / X / V / D (copy, cut, paste, duplicate), Ctrl+G /
Ctrl+Shift+G (group, ungroup), Ctrl+] / [ (forward, backward), Shift+1 (fit), Ctrl+0 (100%), F12
(diagnostics). Known gaps and what comes next: [roadmap](docs/roadmap.md).

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
| [MCP server](docs/mcp.md) | tools for AI agents |

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
pan, Shift+1 to fit, Ctrl+0 for 100%, F3 for stats, F12 for diagnostics.

```bash
cargo run --release -p ude-bench
```

```bash
cargo run --release -p ude-cli -- generate demo.krt --nodes 10000
```
