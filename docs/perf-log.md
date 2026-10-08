# Performance Log

Every optimisation and baseline is recorded here with before/after numbers
(rendering-strategy.md §10, benchmark-plan.md §5).

Run with `cargo run --release -p ude-bench`. Results are written to `target/bench/m1.json`.

## 2026-10-08 · M1 baseline

Machine: Windows 10 Pro (developer workstation, not reference machine A/B), release build, seed 42.
Memory is measured on the **loaded** document (`Document::memory_report`). OS-level RAM counters
arrive with the GUI in M13.

| nodes | generate ms | model MB | B/node | save ms | file KB | load ms | re-save | move 500 ms | undo 500 ms |
|---:|---:|---:|---:|---:|---:|---:|:--:|---:|---:|
| 1,000 | 0.6 | 0.22 | 213 | 2.2 | 59 | 1.2 | identical | 0.03 | 0.03 |
| 5,000 | 2.9 | 1.13 | 216 | 11.6 | 294 | 6.1 | identical | 0.03 | 0.03 |
| 10,000 | 6.4 | 2.24 | 214 | 22.6 | 579 | 12.5 | identical | 0.04 | 0.03 |
| 50,000 | 29.9 | 11.23 | 214 | 96.0 | 2,883 | 60.1 | identical | 0.06 | 0.03 |
| 100,000 | 58.4 | 22.57 | 215 | 218.9 | 5,831 | 146.0 | identical | 0.04 | 0.03 |

### Fix: node footprint 324 → 215 B/node (−34%)

* **Before:** 324 B/node at 100k (32.9 MB). Every rounded rectangle allocated a 216-byte
  `NodeExt` for one radius, and Vec growth slack added up to about 30%.
* **Changes:**
  * Uniform `corner_radius: f32` in `Node`. It fits in padding, so `Node` stays 120 B.
  * `NodeExt` 216 → 128 B, with stroke, auto layout, layer mask and size limits boxed.
  * `Document::shrink_to_fit()` after load.
* **After:** 215 B/node (22.6 MB). About half is real content (vector path points, text). The
  remainder is the node slot (128 B) plus the child-list entry.

## 2026-10-08 · M2 first canvas measurements

Machine: Windows 10 Pro, NVIDIA GTX 1050 Ti (the spec's reference GPU class), release build,
1280×800 window, synthetic 10k-node demo zoomed to fit (every node on screen: the worst case for
direct rendering). Frame time is CPU-side (scene encoding + submit), read from the `--stats` title.

| Backend | Frame | Working set | Private bytes |
|---|---:|---:|---:|
| GPU · Vulkan, 10k nodes | 16.3 ms | 138 MB | 642 MB |
| GPU · Vulkan, empty document | 1.3 ms | 131 MB | 633 MB |
| GPU · DX12, empty document | — | 218 MB | 517 MB |
| GPU · GL, empty document | — | 138 MB | 543 MB |
| CPU (`vello_cpu`, multithreaded), 10k nodes | 24.3 ms | 28 MB | 20 MB |

* **Idle:** 0 ms CPU time over 3 s with the window open. Event-driven rendering works.
* **Traversal:** 10,000 visited, 8,884 drawn, 132 culled in the fit-all view. Zoomed in, culling
  removes most work. The R-tree (M3) will also skip the traversal of off-screen subtrees.

### Open issue: GPU-path committed memory (≈520–640 MB private)

It is independent of the document (an empty document costs the same) and appears on all three
wgpu backends. So it comes from the GPU stack, either driver allocations or vello's
renderer/buffer setup, not from our model. The working set (115–220 MB) is much lower than
private bytes, so much of it is committed but not resident. That still counts against
low-memory machines.

Plan: break it down in M13 with the diagnostics build (per-step measurements: device creation →
`vello::Renderer` → first render → steady state). Then evaluate:
1. Vello renderer options / bump-buffer sizing.
2. A lighter GPU path for weak machines.
3. Making the CPU renderer the default when the GPU stack's cost exceeds its benefit. At 20 MB
   private and 24 ms for a worst-case 10k frame, it is already viable for normal documents.

The user-facing budget in memory-strategy.md (empty app 40–80 MB) is **not met on the GPU path
yet**, and is met on the CPU path.

### Root cause and decision (2026-10-08)

**Cause.** Read from the vello 0.11 source (`vello_encoding::config::BufferSizes::new`). Every
frame allocates fixed-size GPU scratch buffers, whatever the scene contains, and pools them for
reuse:

| Buffer | Size |
|---|---|
| `lines` | 2²¹ × 24 B ≈ 48 MB |
| `segments` | ≈ 48 MB |
| `ptcl` | 32 MB |
| `tiles` + `seg_counts` | 32 MB |

That is about 165 MB, plus the driver's matching allocations. The `bump_estimate` feature only
reports an estimate; it does not size the buffers. None of this is configurable through the
public API.

**Options considered:**
1. Switch the GPU backend to `vello_hybrid`. It runs the CPU path pipeline with GPU raster and
   compositing through render pipelines (no compute), shares `vello_cpu`'s API (one painter for
   both backends) and suits old iGPUs. Today it is only available as 0.2 (`vello_common` 0.2,
   `wgpu` 29), one release behind our `vello_cpu` 0.3 / `wgpu` 30.
2. Patch vello's buffer sizes in a vendored fork. Rejected: we would maintain a fork, and
   undersized buffers break complex scenes.

**Decision.** The CPU renderer is the default, and the GPU (vello) is opt-in via `--gpu` or
`UDE_RENDERER=gpu`. When a `vello_hybrid` release matches `vello_common` 0.3, we adopt it as the
GPU backend and re-measure. Until then, a scene that is too heavy for the CPU path at 60 FPS is
what the tile cache (rendering-strategy.md §7) is for.

| Default launch, 10k-node demo, fit-all | Frame | Working set | Private |
|---|---:|---:|---:|
| Before (GPU, vello) | 16.3 ms | 138 MB | 642 MB |
| After (CPU, vello_cpu) | 25.4 ms | 37 MB | 21 MB |
