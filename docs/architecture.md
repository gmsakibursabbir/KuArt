# Architecture

Status: **accepted for MVP** · Owner: core team · Last revised: 2026-10-08

This document defines the system architecture, the module dependency rules, the data flow and
the top-level rendering pipeline of the Universal Design Editor (UDE). Detailed strategies live in
sibling documents:

| Document | Scope |
|---|---|
| [universal-model.md](universal-model.md) | Internal scene graph / document model |
| [format-strategy.md](format-strategy.md) | Figma, XD, PSD, SVG, PDF, raster, `.design` |
| [memory-strategy.md](memory-strategy.md) | RAM, VRAM, CPU, disk I/O budgets and techniques |
| [rendering-strategy.md](rendering-strategy.md) | GPU/CPU split, caching, tiling, dirty regions |
| [dev-mode.md](dev-mode.md) | Inspector + code-generation pipeline |
| [benchmark-plan.md](benchmark-plan.md) | Reproducible benchmarks and success criteria |
| [roadmap.md](roadmap.md) | Milestones, each one compilable and testable |
| [compatibility-matrix.md](compatibility-matrix.md) | Honest per-feature support status |
| [ui-design-system.md](ui-design-system.md) | Kuduy premium UI: tokens, typography, layout, overlays |
| [distribution.md](distribution.md) | Windows + Linux targets, packaging, signing, CI |

---

## 1. Non-negotiable constraints

1. **Native, single process.** No Electron, no Chromium, no WebView, no JS runtime in the editor.
2. **Correctness and fidelity come before RAM numbers.** We lower memory with better architecture,
   not by removing features (spec §57).
3. **Imported files are untrusted.** Parsers never panic on input and run under resource limits.
4. **Nothing is silently destroyed.** Anything we cannot model is *preserved* and reported.
5. **Optional systems cost ~0 when unused.** Dev Mode, plugins, the automation API and diagnostics
   allocate nothing until they are first opened.
6. **Idle means idle.** No redraw, no polling and no timers while nothing changes (except autosave,
   which is event-driven).

## 2. Technology choices

Every dependency must justify itself. The core model crate has **zero** third-party dependencies.

| Concern | Choice | Why | Rejected alternative |
|---|---|---|---|
| Language | Rust (edition 2024) | Memory control, no GC, safe parsers | — |
| Windowing / input / IME | `winit` | De-facto standard, small, cross-platform | SDL2 (C dep) |
| GPU abstraction | `wgpu` | DX12 / Vulkan / Metal / GL fallback for old Intel iGPUs | raw Vulkan (too much surface) |
| Vector renderer | `vello` (GPU compute) behind a `RenderBackend` trait | Pure Rust, shares our `wgpu` device, handles paths, gradients, clips, blend modes and glyphs on GPU | `skia-safe`: excellent fidelity but adds 20–40 MB native binary and a C++ toolchain. It stays a **drop-in backend option** if fidelity tests demand it |
| CPU raster (fallback, headless export, reference) | `vello_cpu` / `tiny-skia` | No GPU needed for CLI, CI and old drivers without compute shaders | — |
| Text layout & shaping | `parley` + `fontique` + `swash` (HarfBuzz-class shaping) | Rich text spans, bidi, font fallback, system font enumeration (needed to detect missing fonts), native `vello` glyph path | `cosmic-text`: good, but weaker rich-text API and no direct `vello` integration |
| Panels UI | `egui` (immediate mode) + AccessKit | No retained widget tree, so UI memory scales with *visible* rows. Has virtualized lists (`show_rows`), a `wgpu` backend and screen-reader support | `iced`/`slint`: retained trees and heavier per-widget cost. Web UI is forbidden |
| SVG import | `usvg` | Mature normalizer (CSS, `use`, transforms) | writing our own |
| Containers | `zip` (deflate only), `zstd` | `.fig`/`.xd` are zip-based; `.design` uses zstd | — |
| Raster codecs | `zune-jpeg`, `png`, `image-webp`, `fast_image_resize` | Scaled JPEG decode (1/2, 1/4, 1/8) and SIMD downsampling for lazy mips | full `image` crate with every codec |
| PDF export | `krilla` | High-level, correct PDF writer (fonts, gradients, transparency) | `printpdf` |
| Native serialization | hand-written varint encoding + `zstd` + `blake3` (see format-strategy.md §8) | Deterministic id/string remapping, `ude-core` stays dependency-free, explicit wire contract | serde + postcard (cannot remap ids), protobuf / flatbuffers |
| JSON parsing | `serde_json` (XD / Figma REST JSON importers only) | Mature, streaming | — |
| Parallelism | `rayon` | Decode, import and tile rendering | ad-hoc threads |
| Hashing | `blake3` | Content-addressed assets | SHA-256 (slower) |
| Spatial index | `rstar` | R-tree for hit testing and culling | quadtree written by hand |
| Clipboard | `arboard` | Text and images | — |
| Plugins (optional crate) | `wasmi` (interpreter) | Sandboxed, small, loaded only when a plugin is enabled | `wasmtime` (JIT, much larger RSS) |

## 3. Process and thread model

```
┌───────────────────────────── single OS process ─────────────────────────────┐
│                                                                             │
│  Main thread (winit event loop)                                             │
│   input → tool state machine → Ops → Document → ChangeSet                   │
│   ChangeSet → renderer invalidation → request_redraw (only when needed)     │
│   frame: cull → encode → GPU submit → egui overlay → present                │
│                                                                             │
│  rayon pool (N = physical cores − 1, min 1)                                 │
│   import parsing · image decode/downsample · tile rasterization (CPU path)  │
│   codegen (Dev Mode) · export                                               │
│                                                                             │
│  I/O thread                                                                 │
│   memory-mapped reads · autosave journal appends · .design writes           │
│                                                                             │
│  Memory-pressure watcher (OS notification, not polling)                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

* The `Document` is owned by the main thread. Workers receive **immutable snapshots** of what they
  need (for example `Arc` path data and asset handles), never `&mut Document`.
* Import runs on a worker inside `catch_unwind`. A malformed file yields an error report and never
  takes the editor down.

## 4. Workspace layout

Format parsers live in their own crates, so they are compiled and linked only when enabled and
can never reach into the renderer or UI.

```
crates/
  core/            ude-core       document, scene, geometry, typography, layout, components,
                                  assets, styles, history, memory accounting   (no deps)
  renderer/        ude-renderer   gpu/, vector/, raster/, text/, effects/, cache/
  editor/          ude-editor     selection, transform, snapping, guides, tools, commands
  ui/              ude-ui         canvas, toolbar, layers, inspector, assets, panels
  formats/
    native/        ude-format-native   .design read/write, autosave journal
    figma/         ude-format-figma
    xd/            ude-format-xd
    psd/           ude-format-psd
    svg/           ude-format-svg
    pdf/           ude-format-pdf      (export first; import in phase 4)
    raster/        ude-format-raster   PNG/JPG/WebP
  codegen/         ude-codegen    ir/, html/, css/, scss/, tailwind/, react/, vue/, svg/, tokens/
  plugins/         ude-plugins    WASM host (optional feature)
  api/             ude-api        automation surface used by CLI, MCP and plugins (phase 4)
  app/             ude-app        binary `design-editor` (GUI)
  cli/             ude-cli        binary `design-editor-cli` (no winit, no GPU required)
  benchmarks/      ude-bench      synthetic generators + scenarios
tests/             cross-crate integration, visual regression, compatibility corpus
examples/          small sample documents
docs/              this folder
```

## 5. Module dependency rules

```
             ┌──────────┐
             │ ude-app  │──────────────────────────────┐
             └────┬─────┘                              │
                  ▼                                    ▼
             ┌──────────┐     ┌──────────────┐   ┌──────────────┐
             │  ude-ui  │────►│ ude-renderer │   │ ude-format-* │
             └────┬─────┘     └──────┬───────┘   └──────┬───────┘
                  ▼                  │                  │
             ┌──────────┐            │                  │
             │ude-editor│            │                  │
             └────┬─────┘            │                  │
                  ▼                  ▼                  ▼
             ┌─────────────────────────────────────────────┐
             │                  ude-core                   │
             └─────────────────────────────────────────────┘
                  ▲                  ▲
             ┌────┴─────┐      ┌─────┴──────┐
             │ude-codegen│     │ ude-plugins│ (optional, via ude-api)
             └──────────┘      └────────────┘

 ude-cli ──► ude-core, ude-format-*, ude-renderer (CPU backend), ude-codegen   (never ude-ui)
```

Enforced rules (checked in CI with `cargo tree` assertions):

* `ude-core` depends on nothing outside `std` (serde support is an optional feature).
* `ude-format-*` depends only on `ude-core` plus parsing crates. It never depends on the renderer,
  editor or UI.
* `ude-codegen` depends only on `ude-core`. Every format gets the same Dev Mode for free (spec §35).
* `ude-renderer` never depends on `ude-editor` or `ude-ui`. Overlays are passed in as plain data.
* `ude-cli` must build with `--no-default-features` and without `winit`.

## 6. Data flow

```
 file on disk
     │  mmap (no copy)
     ▼
 Importer::sniff → Importer::import(limits) ── diagnostics ──► ImportReport (shown to the user)
     │  DocumentBuilder (direct, non-undoable construction)
     ▼
 Document  (arena of compact Nodes + side tables, interned strings, AssetStore of lazy sources)
     │
     │  user input → Tool → Transaction{ Op, Op, … } → Document::apply → inverse Ops → History
     ▼
 ChangeSet { changed node ids, structure flag }   (drained once per frame)
     ├──► SpatialIndex update (only changed subtrees)
     ├──► Renderer: invalidate encoded fragments + dirty tiles
     ├──► Layers panel: invalidate only if structure/name/visibility changed
     ├──► Dev Mode (if open): recompute only for the current selection
     └──► Autosave journal: append the transaction's forward Ops (debounced)
```

The `Op` log is the single mutation path. Undo/redo, autosave, change notification, the future
automation API (MCP/REST) and plugins all reuse it, so there is exactly one place where
invariants are checked.

## 7. Rendering pipeline (summary)

```
Document ─► per-node encoding cache (keyed by node id + revision)
          ─► viewport culling via R-tree
          ─► LOD selection (greeking, image mip level, effect skipping at tiny zoom)
          ─► scene composition with the view transform
          ─► GPU backend (vello/wgpu)  ─┐
              or tile cache (huge docs) ─┼─► canvas texture
          ─► editor overlays (selection, guides, measurements) in screen space
          ─► egui panels
          ─► present
```

See [rendering-strategy.md](rendering-strategy.md) for details.

## 8. Error-handling policy

* Library crates return `Result<_, ThisCrateError>`. No `unwrap` on data derived from input.
* Unsupported features produce `Diagnostic { level, feature, node, message }`. The UI shows these
  as "⚠ Partially supported" badges with an explanation, and the original data is kept.
* Panics inside import, export or codegen are caught at the worker boundary, turned into an
  error dialog, and the document stays open.

## 9. Security policy

* No embedded script is ever executed (XD plugins data, PDF JavaScript, SVG `<script>`).
* No external resource is fetched (SVG `href="http…"`, linked smart objects) without an explicit
  per-document user permission prompt.
* Plugins run in WASM with a capability-based API. They have no filesystem or network access
  unless the user grants it.
* Every parser has a `cargo-fuzz` target that runs in CI.
