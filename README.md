<div align="center">

<img src="assets/icon.png" alt="KuArt" width="112" height="112">

# KuArt

**A fast, native design editor for Windows and Linux.**
Open Figma, Sketch, Adobe XD, Photoshop, Illustrator, PDF and SVG files, edit them, export them,
and turn them into code. No browser engine, no account, no cloud.

[![CI](https://github.com/gmsakibursabbir/KuArt/actions/workflows/ci.yml/badge.svg)](https://github.com/gmsakibursabbir/KuArt/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/gmsakibursabbir/KuArt?include_prereleases&label=release)](https://github.com/gmsakibursabbir/KuArt/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux-informational)
![Rust](https://img.shields.io/badge/rust-1.88%2B-orange)

[Download](#download) · [Features](#features) · [File formats](#file-formats) ·
[Code generation](#design-to-code) · [AI agents (MCP)](#ai-agents-mcp) · [Build](#building-from-source) · [Docs](#documentation)

</div>

---

## Why KuArt

Design tools have become browser apps that need a fast machine, a login and a network
connection. KuArt is a native Rust application built for the opposite: it opens your design files
on any PC, including older 8 GB machines, fully offline.

| | KuArt 0.1 (measured) |
|---|---|
| Memory, empty document | **34 MB** private |
| Memory, 100,000 layers | **85 MB** private |
| CPU while idle | **0%**, event-driven rendering |
| Startup to first frame | **≈ 0.1 s** |
| Open a 100k-layer document | **0.3 s** |
| Installer size | ≈ 20 MB GUI + 7 MB CLI |

Numbers from the [performance log](docs/perf-log.md) (release build, Windows 10 developer
workstation). Benchmarks are reproducible with `cargo run --release -p ude-bench`.

## Download

Get the latest build from the [**Releases**](https://github.com/gmsakibursabbir/KuArt/releases) page.

| Platform | Package | Notes |
|---|---|---|
| **Windows 10/11** (x64) | `KuArt-x.y.z-x64-setup.exe` | Installer; per-user, no admin rights needed |
| | `KuArt-x.y.z-x64.msi` | For managed deployment |
| | `KuArt-x.y.z-x64.exe` / `…-portable.zip` | Single executable, nothing to install |
| **Linux** (x86_64, glibc ≥ 2.31) | `KuArt-x.y.z-x86_64.AppImage` | Runs on most distributions |
| | `kuart_x.y.z-1_amd64.deb` | Debian, Ubuntu, Mint, Pop!_OS |
| | `kuart-x.y.z-1.x86_64.rpm` | Fedora, RHEL, openSUSE |
| | `kuart-bin-x.y.z-1-x86_64.pkg.tar.zst` | Arch, Manjaro, EndeavourOS |
| **Command line** | `design-editor-cli-…` archives | Headless tools for CI and servers |

Every release includes `SHA256SUMS`. Installers register KuArt for `.krt`, `.fig`, `.sketch`,
`.xd`, `.psd` and `.ai` files.

## Features

**Open almost any design file.** Import is native and offline, and each importer reports what it
could not reproduce.

**Edit.** Selection (click, marquee, deep select into groups), move with smart snapping, resize
and rotate, frame, rectangle, ellipse, polygon, star, line and text tools, a layers panel, an
inspector, copy/cut/paste, duplicate, group/ungroup, layer order, and unlimited undo/redo.

**Export.** PNG at any scale and SVG that keeps text wrapping exactly as on the canvas. Documents
save as `.krt`: a compact, chunked, zstd-compressed format that saves 100k layers in about 0.2 s.

**Fonts done right.** System fonts and a KuArt fonts folder are both used. Missing fonts are named
in the status bar (for example *"Inter SemiBold Italic"*), and a single click downloads the
family from Google Fonts. That click is the only time KuArt goes online.

**Built-in diagnostics.** Press **F12** for live memory (as the OS reports it), cache, undo
history and frame statistics.

**Polished UI.** Dark, light, system and high-contrast themes on the Kuduy design system.

## File formats

| Format | Extensions | Open | Save / export |
|---|---|:---:|:---:|
| KuArt | `.krt` (legacy `.design`) | ✓ | ✓ |
| Figma (Design, FigJam, Slides, Buzz, Sites, Make) | `.fig` `.jam` `.deck` `.buzz` `.site` `.make` | ✓ | — |
| Sketch | `.sketch` | ✓ | — |
| Adobe XD | `.xd` | ✓ | — |
| Adobe Photoshop | `.psd` `.psb` | ✓ | — |
| Adobe Illustrator (PDF-compatible) | `.ai` | ✓ | — |
| PDF | `.pdf` | ✓ | planned |
| SVG | `.svg` | ✓ | ✓ |
| Images | `.png` `.jpg` `.webp` | ✓ | PNG |

Support is honest and itemised per feature in the
[compatibility matrix](docs/compatibility-matrix.md). The Figma, Sketch, XD and Illustrator
importers are tested against files built in each format's real structure. Bug reports with
sample files are very welcome.

## Design to code

Turn any frame into production-ready code. Auto layout and evenly stacked layers become flexbox,
and buttons, headings and images get semantic tags.

```bash
design-editor-cli code landing.fig react  > Landing.jsx     # React + CSS
design-editor-cli code landing.fig tailwind > landing.html  # Tailwind utilities
design-editor-cli code landing.fig tokens                   # CSS variables + W3C tokens JSON
```

Targets: `html`, `css`, `tailwind`, `react`, `vue`, `tokens`.

## AI agents (MCP)

KuArt ships a [Model Context Protocol](https://modelcontextprotocol.io) server, so agents such as
Claude can create designs, inspect and edit them, render previews, save `.krt` files and
generate code.

```bash
claude mcp add kuart -- design-editor-cli mcp
```

Tools include `new_document`, `open_document`, `get_tree`, `create_node`, `update_node`,
`render_preview`, `generate_code`, `export` and `save_document`. See [docs/mcp.md](docs/mcp.md).

## Command line

```bash
design-editor-cli inspect design.fig               # structure, node counts, memory
design-editor-cli convert design.sketch out.png    # or .svg / .krt
design-editor-cli code design.krt vue              # code generation
design-editor-cli mcp                              # MCP server on stdio
```

## Keyboard shortcuts

| Action | Shortcut | Action | Shortcut |
|---|---|---|---|
| Move / Frame / Rectangle | V / F / R | Open / Save / Save as | Ctrl+O / Ctrl+S / Ctrl+Shift+S |
| Ellipse / Polygon / Star | O / P / S | Export PNG/SVG | Ctrl+Shift+E |
| Line / Text | L / T | Undo / Redo | Ctrl+Z / Ctrl+Y |
| Copy / Cut / Paste | Ctrl+C / X / V | Duplicate | Ctrl+D |
| Group / Ungroup | Ctrl+G / Ctrl+Shift+G | Forward / Backward | Ctrl+] / Ctrl+[ (Shift: front / back) |
| Pan | Space+drag, wheel | Zoom | Ctrl+wheel, Ctrl+= / Ctrl+- |
| Zoom to fit / 100% | Shift+1 / Ctrl+0 | Diagnostics | F12 |

## Building from source

Requires stable Rust 1.88 or newer. There are no system library requirements at build time on
Windows or Linux.

```bash
git clone https://github.com/gmsakibursabbir/KuArt.git
cd KuArt
cargo run --release -p ude-app                 # the editor (opens a demo document)
cargo run --release -p ude-app -- file.fig     # open a file
cargo test --workspace                         # tests
```

The CPU renderer is the default because it uses the least memory. Pass `--gpu` to opt into the
GPU renderer (wgpu + vello). Release packages are built by
[`.github/workflows/release.yml`](.github/workflows/release.yml); see
[docs/distribution.md](docs/distribution.md).

### Architecture at a glance

```
crates/
  core         Universal Design Model: arena scene graph, ops/undo, interning (no dependencies)
  renderer     vello / vello_cpu painter, text (parley), image cache
  app          window, event loop, tools, files            ui        egui panels (Kuduy theme)
  editor       selection, transforms, snapping, commands   spatial   R-tree hit testing
  codegen      design → HTML/CSS/Tailwind/React/Vue/tokens  cli       headless CLI + MCP server
  formats/     native (.krt) · fig · sketch · xd · psd · pdf (+ai) · svg · svg-import · raster · zip
```

## Roadmap

0.1 completes the MVP (milestones M0–M14). Since then: on-canvas text editing, live boolean
operations, Dev Mode, PDF export, document tabs and autosave. Next up:
blur rendering, component editing, the auto-layout solver and P2P collaboration. The full plan and the list of known gaps are in
[docs/roadmap.md](docs/roadmap.md).

## Documentation

| | |
|---|---|
| [Architecture](docs/architecture.md) | system design, modules, data flow, technology choices |
| [Universal model](docs/universal-model.md) | the internal scene graph |
| [Format strategy](docs/format-strategy.md) | how each file format is read and written |
| [Compatibility matrix](docs/compatibility-matrix.md) | per-feature import status |
| [Dev Mode](docs/dev-mode.md) | inspector and code-generation pipeline |
| [MCP server](docs/mcp.md) | tools for AI agents |
| [Feature gaps](docs/feature-gaps.md) | what is missing vs Figma, XD, Sketch, Penpot |
| [Rendering](docs/rendering-strategy.md) · [Memory](docs/memory-strategy.md) | GPU/CPU rendering, caching, RAM budget |
| [Benchmarks](docs/benchmark-plan.md) · [Performance log](docs/perf-log.md) | targets and measured results |
| [UI design system](docs/ui-design-system.md) | Kuduy tokens, type and layout |
| [Distribution](docs/distribution.md) | packages, signing, CI |

## Contributing

Issues and pull requests are welcome. Please run `cargo fmt --all`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo test --workspace` before opening a PR. CI runs the same
checks on Windows and Linux. Sample files that import incorrectly are the most valuable bug
reports.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Figma, Sketch, Adobe XD, Photoshop and Illustrator are trademarks of their respective owners.
KuArt is not affiliated with them.
