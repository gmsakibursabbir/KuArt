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
