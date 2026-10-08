# KuArt: Universal Design Editor

A native Rust design editor (no Electron, no browser engine) that opens, edits and exports Figma,
Adobe XD, Photoshop, SVG and PDF designs in one GPU-accelerated canvas, with a built-in Dev Mode
for CSS/HTML/Tailwind/React/Vue output. It is built to run well on older PCs with 8 GB of RAM.

**Status: Milestone 1 done.** The core scene graph, undo/redo and memory accounting are in place,
along with the native `.design` format, a headless CLI and benchmarks (100k nodes: save 219 ms,
load 146 ms, 215 bytes/node). There is no GUI yet. See the [roadmap](docs/roadmap.md) and
[perf log](docs/perf-log.md).

## Documents

| | |
|---|---|
| [Architecture](docs/architecture.md) | system, modules, data flow, tech choices |
| [Universal model](docs/universal-model.md) | the internal scene graph |
| [Format strategy](docs/format-strategy.md) | Figma / XD / PSD / SVG / PDF / `.design` |
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
cargo run --release -p ude-bench
```

```bash
cargo run --release -p ude-cli -- generate demo.design --nodes 10000
```
