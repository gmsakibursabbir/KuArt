# Memory Strategy

Status: **accepted** · Measured by `ude-bench` (see benchmark-plan.md)

Goal: professional capability at a small fraction of browser-based tools' footprint, achieved by
*architecture*, not by removing features.

## 1. Targets and budgets

| Scenario | Target (private bytes) |
|---|---|
| Empty application, one blank document | 40–80 MB |
| Normal design (≈1–5k nodes, 20 images) | 100–200 MB |
| Large design (≈20–50k nodes, 100 images) | 200–400 MB |
| Very large design (100k+ nodes, 500 images) | 400–700 MB |

Default budgets per subsystem. All are bounded, and scaled by detected system RAM (4/8/16/32 GB
tiers):

| Subsystem | 8 GB tier default | Bounded by |
|---|---|---|
| Executable + shared libs (code pages) | ~25 MB | build profile: LTO, `opt-level=3`, `panic=abort` in app, `strip` |
| GPU driver user-mode overhead | 15–40 MB | outside our control. Measured and reported separately |
| Document model | ~200 B/node | compact `Node` (≤128 B) + side tables |
| Decoded image cache (CPU) | 128 MB | LRU, resolution-aware |
| Texture cache (VRAM) | 256 MB or 25% of reported VRAM | LRU |
| Tile cache | 64 MB VRAM | LRU per zoom level |
| Glyph cache | 8 MB | vello glyph cache + LRU |
| History | 32 MB | byte budget per document, oldest trimmed |
| UI (egui) | < 10 MB | virtualized lists, no retained widgets |

## 2. Techniques

### 2.1 Compact model (RAM)
* Hot/cold split: `Node` ≤ 128 bytes, rare data in `Option<Box<NodeExt>>`.
* `NodeId` with a `NonZeroU32` niche, so `Option<NodeId>` = 8 bytes. `Symbol` = 4 bytes.
* `Arc<[T]>` for paints, gradients, paths and text styles. An empty paint list is `None` (no
  allocation).
* Import-time hash-consing: identical paint lists and gradients share one `Arc`. Typical UI files
  reuse a handful of colours thousands of times.
* Instances are not expanded (see universal-model.md §3.7).
* Paths are packed as `Box<[PathVerb]>` (1 byte each) + `Box<[Point]>` (8 bytes each), with no
  per-segment enum padding.

### 2.2 Lazy loading (RAM, disk I/O)
* Source files are **memory-mapped**. The OS pages data in and out, and clean mapped pages can be
  dropped under pressure without a write.
* `.design` pages load on navigation. Other formats load all node metadata (small), but never
  pixels.
* Image pipeline: `original bytes (mmap)` → decode **at the required resolution** → GPU texture.
  * JPEG: DCT-domain scaled decode (1/2, 1/4, 1/8) when the displayed size allows.
  * PNG/WebP/PSD channels: decode, then downsample in streaming row bands. PSD RLE is decoded per
    row range for tiles.
  * Mip levels are produced on demand and never all kept. A 6000×4000 image displayed at 300×200
    occupies ~0.25 MB (the next power-of-two level up), not 96 MB.
  * Full resolution is decoded only for 1:1 zoom, pixel preview, export or explicit "open
    original", and the cache evicts it first.

### 2.3 Bounded caches (RAM + VRAM)
One generic `BoundedCache<K, V: Weighed>` is used for decoded images, textures, tiles, glyphs and
encoded scene fragments:
* LRU by last-used frame number, with weight in bytes.
* Resolution-aware keys: `(AssetId, mip_level)`. A request for level *n* may be served by a cached
  higher-resolution level (downscaled on GPU) instead of decoding again.
* Hard cap plus soft target. Over the soft target, items unused for more than 2 s are evicted.
  Over the hard cap, strict LRU applies.
* Every cache reports `(entries, bytes, hits, misses, evictions)` to the diagnostics registry.

### 2.4 Virtualized UI
* Layers panel: a flattened, expanded-aware row index (`Vec<(NodeId, depth)>`, rebuilt only on
  structural change). Only visible rows are laid out each frame. 100k layers cost ~1.6 MB of
  index and ~40 rows of UI.
* Asset browser and font list: same pattern, with lazy thumbnails rendered at row size.

### 2.5 Incremental history
* Undo stores **inverse ops**, never document snapshots. A move of 500 nodes costs 500 ×
  (`Op` ≈ 48 B) ≈ 24 KB.
* Coalescing: continuous gestures (drag, slider scrub, nudge repeat) merge into one entry through
  a merge key. Only the first old value per `(node, property)` is kept.
* Budget: per-document byte cap. The oldest entries are trimmed, and detached nodes they kept
  alive are purged.

### 2.6 Multi-document and memory pressure
Inactive tabs step down through these states:

| State | Kept | Freed |
|---|---|---|
| Active | everything | — |
| Background (> 10 s) | model, history | textures, tiles, glyph cache entries only it used |
| Hibernated (pressure ≥ Elevated, or > 10 min) | journal on disk, tab metadata | model, history (serialized to the temp `.design` + journal) |

The pressure source is OS notifications, not polling:
* Windows: `CreateMemoryResourceNotification(LowMemoryResourceNotification)` plus a cheap check of
  `GlobalMemoryStatusEx` at frame boundaries when it fires.
* Linux: PSI `/proc/pressure/memory` trigger via `poll()`.
* macOS: `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`.

Actions escalate (Normal → Elevated → Critical): shrink image cache to 50% → drop background tab
textures → hibernate background tabs → shrink tile cache → trim history to 25% (with a toast). The
active document's visible content is never evicted, and nothing blocks the UI thread.

### 2.7 Optional systems cost nothing
Dev Mode, codegen, plugins, the automation API and diagnostics are compiled in (or behind features)
but hold **no state until first use**. They are stored as `Option<Box<Subsystem>>` initialised
lazily. Code pages that never execute are never paged in.

### 2.8 Allocator and returning memory
* We measure `mimalloc` against the system allocator in `ude-bench` and pick per platform from
  the data.
* After large frees (closing a document, hibernating), we ask the allocator to return memory
  (`mi_collect(true)`, or `HeapCompact`/`malloc_trim` with the system allocator).
* The diagnostics build wraps the allocator with atomic per-category counters (tagged by the
  thread-local "current subsystem"). It is compiled out of release builds.

## 3. VRAM
* Textures are created at the mip level actually displayed. Glyphs use vello's atlas.
* The tile cache is used only when the direct render can't hold frame time (see
  rendering-strategy.md). It is bounded and freed when idle for 30 s.
* VRAM is read from `wgpu` allocator reports and DXGI `QueryVideoMemoryInfo` on Windows, and our
  budget is clamped to 50% of the OS-reported budget.

## 4. CPU
* Event-driven loop: `ControlFlow::Wait`. No frames are rendered when nothing changed, so idle CPU
  is about 0%.
* Per-frame work is proportional to changes (`ChangeSet`) and visible content (R-tree culling),
  never to document size.
* Heavy work (decode, import, export, codegen) runs on rayon with cancellation tokens. Stale
  requests are dropped when the viewport moves on.

## 5. Disk I/O
* mmap for reads. Chunked `.design` lets us read only the pages and assets we need.
* Autosave appends small journal records (typically < 1 KB per transaction) instead of rewriting
  the document. A full compaction happens on explicit save, or when the journal exceeds 25% of the
  document size.
* Asset bytes already compressed (JPEG/PNG/WebP) are stored as-is, without a second zstd pass.

## 6. Memory diagnostics panel (developer)

```
Memory
──────────────────
Document       42 MB   nodes 48,212 · strings 1.2 MB · paths 18 MB
Textures       31 MB   212 entries · hit 97%
Images         22 MB   decoded CPU cache
Fonts           8 MB   mapped 31 MB (shared, not counted)
History         6 MB   184 entries
UI             12 MB
Renderer       18 MB   encodings, buffers
Caches          4 MB   tiles, glyphs
──────────────────
Total         143 MB   (process private 151 MB · untracked 8 MB)
```

Sources: `HeapSize` implementations (model, history), cache weight counters (renderer), the
allocator category counters (diagnostics build), and the OS process counters for the ground-truth
total. The "untracked" line exposes accounting gaps instead of hiding them.
