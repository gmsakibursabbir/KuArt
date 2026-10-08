# Rendering Strategy

Status: **accepted** · Implemented from Milestone 2

## 1. Backends

```rust
pub trait RenderBackend {
    fn render(&mut self, scene: &FrameScene, target: &mut RenderTarget) -> Result<FrameStats, RenderError>;
}
```

| Backend | When | Notes |
|---|---|---|
| `VelloGpu` | default | compute-shader renderer on `wgpu` (DX12 / Vulkan / Metal) |
| `VelloCpu` / `tiny-skia` | no compute support (DX11-only / GL 3.3 iGPUs), CLI, export, CI reference | deterministic, multi-threaded |
| `Skia` (optional feature) | only if fidelity tests show gaps we can't close | not built by default |

At startup the adapter is probed. If compute shaders are unavailable or the device is lost, we
fall back to the CPU backend and present via a texture upload. The visual-regression suite runs
on **both** GPU and CPU backends and requires them to match within a threshold.

## 2. Frame pipeline

```
 ChangeSet ──► invalidate: encoding cache entries, R-tree entries, dirty tiles
 viewport  ──► R-tree query (world-space viewport + effect outset margin)
           ──► visible list in paint order (tree order, masks/clips resolved)
           ──► for each visible node: cached encoding (vello Scene fragment) or encode now
           ──► compose: push_layer for opacity / blend / clip / mask groups, append fragments
           ──► GPU render to the canvas texture
           ──► overlay scene (screen space): selection, handles, guides, measurements, snapping
           ──► egui panels
           ──► present (vsync, or mailbox when available)
```

* **Encoding cache.** Each node's own drawing commands (fills, strokes, text glyph runs) are
  encoded once into a fragment and reused. They are re-encoded only when the node's revision
  changes. Component encodings are shared by all instances that have no visual override.
* **Transform-only changes** (moving, panning, zooming) never re-encode. Fragments are appended
  with a new transform.
* **Group compositing.** `push_layer` is used only when it changes the result (opacity < 1 on a
  group with more than one overlapping child, non-normal blend, clip, mask, layer effects).
  Otherwise children draw directly with multiplied opacity. This is the biggest saving on typical
  UI designs.

## 3. Effects

| Effect | Implementation |
|---|---|
| Drop shadow on rect / rounded rect | vello's analytic blurred rounded rect (no offscreen pass) |
| Drop/inner shadow on arbitrary shapes | offscreen mask → separable Gaussian blur compute pass (`wgpu`), downsampled 2–4× for radius > 8 px |
| Layer blur | offscreen layer → blur pass, cached by (node revision, zoom bucket) |
| Background blur | read back the composed region below → blur → clip to shape; cached while the content beneath is unchanged |
| Blend modes | vello's native blend modes (all 16 separable/non-separable modes) |

The blur radius scales with zoom. At small zoom levels blurs are computed at reduced resolution,
since visually that is identical.

## 4. Text

`parley` lays out text (shaping, bidi, line breaking, fallback) and produces glyph runs, which are
cached per text node revision. vello renders the glyph outlines on the GPU. Below about 4 px on
screen, text is "greeked" (drawn as grey bars) during interaction, and real glyphs return once
interaction settles.

## 5. Images

The renderer asks the image cache for `(asset, level)` where
`level = floor(log2(source_px / displayed_px))`. Until that is ready it draws the nearest cached
level, upscaled. Decoding is asynchronous on rayon, and stale requests are cancelled when the
viewport moves on. The canvas never waits for a decode.

## 6. Level of detail

| Zoom / situation | LOD |
|---|---|
| Node screen size < 0.5 px | skipped |
| Node screen size < 2 px | drawn as a filled bbox with average colour |
| Text < 4 px | greeked during interaction |
| Effects with screen radius < 0.5 px | skipped |
| Images | mip level matched to screen size |

## 7. Tiling (huge documents and weak GPUs)

Direct rendering is the default. A frame-time governor switches to **tile mode** when p95 frame
time over the last 30 frames exceeds 14 ms (and switches back below 8 ms).

* Tiles are 512×512 device pixels at quantized zoom levels (powers of √2).
* Visible tiles render in priority order: centre first, then outward.
* **Pan** reuses existing tiles. Only newly exposed tiles render.
* **Zoom** draws the nearest cached level scaled, then refines with crisp tiles after 100 ms
  without zoom input.
* **Dirty regions.** On change, `dirty = bounds_before ∪ bounds_after` (including effect outsets)
  for each changed node. Only intersecting tiles are invalidated, so editing a button on a
  50,000-node canvas re-renders one or two tiles.
* During a drag, the moving selection is rendered live as a separate layer above static tiles that
  exclude it. The static tiles stay valid for the whole gesture.

## 8. Spatial index

An `rstar` R-tree of world-space bounding boxes (including effect and stroke outsets) is used for
culling, hit testing, marquee selection and snapping candidates. It is updated incrementally from
the ChangeSet. A bulk load on import takes ~30 ms for 100k nodes.

## 9. Hit testing

1. R-tree candidates under the cursor (top-most first, in paint order).
2. Precise test: rect/ellipse analytically, paths with a flattened-path winding test, text by line
   boxes.
3. Selection rules (deep select with Ctrl, group-first by default) live in `ude-editor`, not the
   renderer.

## 10. Profiling

* `puffin` scopes (feature `profile`) on every pipeline stage. The diagnostics panel shows FPS,
  frame time (p50/p95/p99), visible and total nodes, encoded fragments, draw/dispatch counts,
  cache usage and VRAM.
* Rule: no GPU-side optimisation without a captured profile showing the bottleneck. Each one is
  recorded in `docs/perf-log.md` with before/after numbers.
