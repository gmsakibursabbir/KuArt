# Universal Design Model

Status: **accepted** · Implemented in `crates/core` (Milestone 0)

The Universal Design Model (UDM) is the single in-memory representation that every importer
produces, every tool edits, every renderer draws and every code generator reads. It is **not** a
flattened raster and **not** SVG. It is a typed scene graph that is a superset of the concepts in
Figma, XD and Photoshop.

## 1. Design rules

1. **Compact hot data, sparse cold data.** The fields touched every frame (tree links, transform,
   size, opacity, fills) live inline in `Node`. Rare data (effects, strokes, auto layout,
   constraints, preserved source blobs) lives in `Option<Box<NodeExt>>`, which costs 8 bytes when
   absent.
2. **IDs, not pointers.** Nodes live in a generational arena. `NodeId` is 8 bytes and
   `Option<NodeId>` is also 8 bytes thanks to a `NonZeroU32` generation.
3. **Share immutable payloads.** Paths, gradients, paint lists and text style tables are
   `Arc`-shared, so copy/paste, component instances and undo snapshots cost a refcount instead of
   a deep copy.
4. **Intern every repeated string.** Layer names, font families and style names are stored as
   4-byte `Symbol`s.
5. **Assets are content-addressed and lazy.** An image is an `AssetId` pointing to a byte range in
   the source file plus a codec tag. Pixels are decoded on demand at the resolution needed.
6. **Preserve, never drop.** Anything without a UDM equivalent is stored as a `Preserved` record
   (source format, key, raw bytes as an asset) on the nearest node, and written back on export to
   the same format.

## 2. Size budget (enforced by tests)

| Structure | Budget | Notes |
|---|---|---|
| `Node` | ≤ 128 bytes (measured: 120) | test `node_is_compact` |
| `NodeExt` | ≤ 128 bytes (measured: 128) | test `node_is_compact` |
| `NodeId` / `Option<NodeId>` | 8 / 8 bytes | test `ids_are_compact` |
| `Symbol` | 4 bytes | |
| `Paint` | ≤ 32 bytes | gradients behind `Arc` |
| Per-node average incl. side tables, 100k-node doc | ≤ 250 bytes (measured M1: 215) | `ude-bench`, loaded document |

The M1 benchmark found 324 B/node on its first run. A rounded rectangle needed a whole 216-byte
`NodeExt` just for its radius. Moving the uniform radius into `Node` padding, boxing the rare
`NodeExt` parts and calling `shrink_to_fit` after load brought it to 215 B/node. About half of that
is real content (path points and text), not overhead.

## 3. Scene graph

```
Document (root node, NodeKind::Document)
└── Page*                                   NodeKind::Page
    └── Frame | Group | Shape | Vector | Text | Raster | Component | ComponentSet
        | Instance | BooleanOp | Slice | Preserved   (recursive)
```

* **Artboards** (XD, PSD artboards, Figma top-level frames) are `Frame` nodes that are direct
  children of a `Page`. Code that needs the distinction asks `Document::is_artboard(id)`. A
  separate kind would duplicate every frame code path.
* **Masks** follow the Figma model, which subsumes XD and PSD clipping. A child with
  `flags.IS_MASK` masks its *following* siblings within the same parent, up to the next mask.
  PSD clipping masks (clipping base + clipped layers) map to the same thing with
  `MaskKind::Alpha`. PSD layer and vector masks map to `NodeExt::layer_mask`.
* **Clipping** is `flags.CLIPS_CONTENT` on frames.

### 3.1 Node (hot part)

```rust
pub struct Node {
    pub kind: NodeKind,             // 16 B: small inline payloads, heavy ones behind Box/Arc
    pub name: Symbol,               // 4 B
    pub parent: Option<NodeId>,     // 8 B
    pub children: Vec<NodeId>,      // 24 B (no allocation for leaves)
    pub transform: Affine,          // 24 B, local → parent, f32
    pub size: Size,                 // 8 B, local box (0,0)-(w,h)
    pub opacity: f32,
    pub corner_radius: f32,         // uniform radius, fits in padding; per-corner in ext
    pub blend: BlendMode,           // u8
    pub flags: NodeFlags,           // u16: VISIBLE, LOCKED, CLIPS_CONTENT, IS_MASK, …
    pub fills: Paints,              // Option<Arc<[Paint]>>, 16 B, empty = no allocation
    pub ext: Option<Box<NodeExt>>,  // 8 B
}
```

Coordinates are `f32` in local space, matching Figma's own precision. On a 1,000,000 px canvas
`f32` still resolves about 0.06 px. World transforms are computed on demand and cached in the
renderer, not in the model.

### 3.2 NodeKind

| Kind | Payload | Sources |
|---|---|---|
| `Document` | — | root |
| `Page` | — | Figma page, XD document (one page), PSD document (one page) |
| `Frame` | — | Figma frame, XD artboard/group-with-layout, PSD artboard |
| `Group` | — | all |
| `Rectangle` | corner radii in ext | all |
| `Ellipse` | `arc: Option<Box<ArcData>>` | all |
| `Polygon { sides }`, `Star { points, inner_ratio }`, `Line` | inline | Figma, XD |
| `Vector(Arc<VectorData>)` | path list + fill rule | all, PSD vector shapes |
| `Text(Box<TextData>)` | content + styled runs | all |
| `Raster { asset }` | lazy pixels | PSD pixel layers, placed images |
| `Component(Box<ComponentInfo>)` | key, variant properties | Figma, XD |
| `ComponentSet` | — | Figma variants container |
| `Instance(Box<InstanceData>)` | component ref + overrides | Figma, XD |
| `BooleanOp(BoolOpKind)` | union/subtract/intersect/exclude | Figma, XD |
| `Slice` | — | Figma export slices |
| `Preserved { source }` | rendered with a fallback raster asset | anything unknown |

### 3.3 NodeExt (cold part)

```rust
pub struct NodeExt {                           // 128 bytes; large rare parts boxed
    pub strokes: Option<Box<Stroke>>,          // paints, weight, align, cap, join, dashes
    pub effects: Box<[Effect]>,                // drop/inner shadow, layer/background blur
    pub corners: Option<CornerRadii>,          // per-corner + smoothing; overrides Node::corner_radius
    pub auto_layout: Option<Box<AutoLayout>>,  // container behaviour
    pub layout_child: LayoutChild,             // sizing modes, absolute, Option<Box<SizeLimits>>
    pub constraints: Constraints,              // horizontal/vertical: min/max/center/stretch/scale
    pub grids: Box<[LayoutGrid]>,
    pub layer_mask: Option<Box<LayerMask>>,    // PSD raster/vector mask
    pub source_id: Symbol,                     // original id in the source file (round-trip, diagnostics)
    pub preserved: Vec<Preserved>,             // raw unsupported source data
}
```

Planned additions (phase 2): `adjustment` (PSD adjustment layers, rendered via fallback until
supported) and `style_refs` (links to shared styles and variables), both boxed.

### 3.4 Paint, stroke and effects

```rust
pub struct Paint { kind: PaintKind, opacity: f32, blend: BlendMode, visible: bool }
pub enum PaintKind {
    Solid(Color),                                     // linear f32 RGBA, stored sRGB-encoded
    Gradient(Arc<Gradient>),                          // linear | radial | angular | diamond
    Image { asset: AssetId, mode: ImageScaleMode, transform: Option<Box<Affine>> },
}
pub enum Effect {
    DropShadow(Shadow), InnerShadow(Shadow),
    LayerBlur { radius }, BackgroundBlur { radius },
}
```

The order of paints and effects is preserved exactly, because stacking order is visual output.

### 3.5 Typography

```rust
pub struct TextData {
    pub content: Box<str>,
    pub runs: Box<[TextRun]>,               // (byte range, style index), covers the content
    pub styles: Arc<[TextStyle]>,           // deduplicated per text node
    pub sizing: TextSizing,                 // AutoWidth | AutoHeight | Fixed | Truncate
    pub align_h: TextAlignH, pub align_v: TextAlignV,
    pub paragraph_spacing: f32,
}
pub struct TextStyle {
    pub family: Symbol, pub weight: u16, pub italic: bool, pub size: f32,
    pub line_height: LineHeight, pub letter_spacing: LetterSpacing,
    pub decoration: TextDecoration, pub case: TextCase, pub fills: Paints,
}
```

Missing fonts: the renderer asks `fontique` for each `(family, weight, italic)`. A miss produces
a diagnostic such as `⚠ Inter SemiBold is not installed` and a fallback face. The style keeps the
original family, so saving never rewrites the font name.

For PSD text layers, the importer keeps the layer's pre-rendered pixels as a fallback asset, so the
document looks right before the text is edited, even without the font installed.

### 3.6 Layout

`AutoLayout` covers Figma auto layout, XD stacks/padding and (for code generation) flexbox:
`direction (H | V | Wrap)`, `gap`, `cross_gap`, `padding[t,r,b,l]`, `main_align`, `cross_align`,
`primary_sizing (Fixed | Hug)` and `counter_sizing`. Layout grids (`LayoutGrid`: columns, rows,
gutter, margin) are stored for display and for codegen.

The layout **solver** lives in `ude-core::layout` and is a pure function
`(Document, root) → Vec<(NodeId, Rect)>`. It is independent of the UI, so the CLI, codegen and
tests can run it headless.

### 3.7 Components, instances and variants

* `Component` nodes own their subtree. `ComponentSet` groups variants. Each component's
  `ComponentInfo.variant_props` holds `(Symbol, Symbol)` pairs.
* `Instance` stores `component: ComponentRef` (a local `NodeId` or an external library key) and
  `overrides: Box<[Override]>`, where `Override { path: Box<[Symbol]>, prop: Prop }` addresses a
  node inside the component by its stable source ids.
* Instances are **not expanded** in the stored model. The renderer and codegen resolve them
  lazily, and the renderer caches each component's encoding once. A design with 2,000 identical
  buttons pays for one button plus 2,000 small instance records.

### 3.8 Styles and tokens

`Document.styles` holds named shared styles (paint, text, effect, grid) and variables/tokens
(color, number, string, boolean with modes). Nodes link to them through `StyleRefs`. Codegen uses
these names to emit `var(--color-primary)` instead of raw values.

### 3.9 Assets

```rust
pub struct AssetEntry { pub hash: ContentHash, pub kind: AssetKind, pub source: AssetSource }
pub enum AssetSource {
    Memory(Arc<[u8]>),                                    // small or generated
    FileRange { file: SourceFileId, offset: u64, len: u64, codec: Codec }, // lazy, mmap-backed
}
```

Assets are deduplicated by `ContentHash` (blake3, computed by the importer). Decoded pixels never
live in the model. They live in the renderer's bounded image cache (see memory-strategy.md).

### 3.10 Preservation

```rust
pub struct Preserved { pub source: SourceFormat, pub key: Symbol, pub data: AssetId }
```

Examples: a PSD `lfx2` effects block we cannot fully edit, the original XD `meta` of a repeat
grid, or an unknown Figma kiwi field. The exporter for the same `SourceFormat` writes these back
verbatim when the node they belong to has not been edited in a way that invalidates them. If it
has, the exporter reports it explicitly.

### 3.11 Support level reporting

Every importer reports per-feature levels through `SupportLevel { Full, Partial, Preserved,
Unsupported }`, which feeds both the UI warnings and the compatibility matrix.

## 4. Mutation model

All edits go through `Document::apply(Op) -> Result<Op /* inverse */, DocError>`:

| Op | Inverse |
|---|---|
| `Attach { id, parent, index }` | `Detach { id }` |
| `Detach { id }` | `Attach { id, old_parent, old_index }` |
| `Set { id, value: Prop }` | `Set { id, previous value }` |

`Prop` is one replaceable field (name, transform, size, opacity, blend, flags, fills, ext, kind).
Values are swapped with `mem::replace`, so applying an op never clones a document subtree. Shared
data (`Arc`) makes the stored old values cheap.

Deleting a node *detaches* it. The node stays in the arena while some history entry can bring it
back, and is purged when that entry is dropped. This keeps `NodeId`s stable across undo/redo,
which selection, the autosave journal and the automation API all depend on.

Importers use `Document::build_child`, which skips history.

## 5. Change tracking

Every applied op records the touched ids in a `ChangeSet` and bumps `Document::revision`. The
frame loop drains it once per frame (`take_changes`) and fans it out to the spatial index, render
caches, layers panel and Dev Mode, so subsystems never diff the whole document.
