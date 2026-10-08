# Benchmark Plan

Status: **accepted** · Harness: `crates/benchmarks` (`ude-bench`), micro-benches with `criterion`

## 1. Principles
* **Reproducible:** synthetic documents come from a seeded generator, so the same seed gives the
  same bytes. Real-world files are pinned by blake3 hash in `tests/corpus/manifest.toml`.
* **Measured, not estimated:** RAM comes from OS counters, not our own accounting. (Our accounting
  is validated *against* the OS counters.)
* **Tracked over time:** results are written as JSON (`target/bench/<git-sha>/<machine>.json`),
  and CI compares them with the last main-branch baseline.

## 2. Test documents

| Name | Nodes | Composition |
|---|---|---|
| `syn-1k` | 1,000 | 10 frames, mixed rect/ellipse/text/vector, 5 images |
| `syn-5k` | 5,000 | 50 frames, 20 images, 200 instances |
| `syn-10k` | 10,000 | 100 frames, 50 images, 1,000 instances |
| `syn-50k` | 50,000 | 500 frames, 200 images, effects on 5% |
| `syn-100k` | 100,000 | 1,000 frames, 500 images (2–24 MP), effects on 5%, 20% text |
| `real-*` | varies | anonymized real Figma/XD/PSD files (with permission) |

Distribution: 35% rect, 10% ellipse, 20% text, 20% vector, 10% group/frame, 5% raster. These
are roughly the proportions in typical UI design files.

## 3. Metrics and how they are measured

| Metric | Method |
|---|---|
| Startup time | process start → first presented frame (timestamp in an env-gated log), cold and warm cache |
| File open time | open request → first full-quality frame of the visible page |
| RAM idle / after open / after close | Windows `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` + working set; Linux `smaps_rollup` Pss + Private; macOS `phys_footprint`. Sampled after a 5 s settle |
| VRAM | DXGI `QueryVideoMemoryInfo` / Vulkan memory budget ext / wgpu allocator report |
| CPU | process CPU time per second, idle and during scenarios |
| GPU | GPU timestamp queries around render passes |
| FPS / frame time | p50 / p95 / p99 over scripted scenarios |
| Export time | PNG at 1×/2×, SVG, PDF of the largest frame and the whole page |
| Undo/redo time | 1 op, a 500-node move, a 10k-node delete |
| Save time | full save, incremental journal append |
| Model footprint | `Document::memory_report()` vs OS delta (micro) |

## 4. Scenarios (scripted input, deterministic timing)
1. **Idle:** 30 s idle after open. Expect zero frames rendered and CPU < 0.5%.
2. **Pan:** constant-speed pan across the page for 10 s.
3. **Zoom:** zoom 5% ↔ 800% in steps around a dense area.
4. **Drag:** select 1 / 50 / 500 nodes and drag for 3 s.
5. **Edit:** change the fill of 1,000 nodes, then undo and redo.
6. **Tabs:** open 5 documents, switch through them, close 4, and measure memory returned.
7. **Pressure:** simulate a low-memory notification and verify the degradation steps and that the
   active document remains responsive.

## 5. Success criteria (reference machine A: i5-7500, 8 GB, HD 630; machine B: i5-8400 + GTX 1050 Ti, 16 GB)

| Metric | 1k | 10k | 100k |
|---|---|---|---|
| Open time (A) | < 0.3 s | < 1.0 s | < 5 s |
| RAM after open (A) | < 150 MB | < 250 MB | < 700 MB |
| Pan p95 frame time (A) | < 16.7 ms | < 16.7 ms | < 33 ms (tile mode) |
| Drag 50 nodes p95 (A) | < 16.7 ms | < 16.7 ms | < 20 ms |
| Undo 500-node move | < 5 ms | < 5 ms | < 10 ms |
| Idle CPU | < 0.5% | < 0.5% | < 0.5% |
| RAM after closing the document | within 15 MB of the empty-app baseline | | |

Empty app: startup < 400 ms warm, < 80 MB private.

**Regression gate:** CI fails when any metric regresses by > 10% (RAM) or > 15% (time) against the
baseline on the same runner class. Overriding requires a recorded justification in
`docs/perf-log.md`.

## 6. Micro-benchmarks (criterion)
* `Document::apply`, history commit/merge/undo
* R-tree bulk load and query
* path flattening and bounds
* PSD RLE decode at MB/s, JPEG scaled decode
* kiwi decode at MB/s, AGC JSON parse at MB/s
* codegen for a 200-node selection
