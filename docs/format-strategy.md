# Format Strategy

Status: **accepted** · Covers import and export for Figma, XD, PSD, SVG, PDF, raster images and
native `.design`.

## 1. Common importer architecture

```rust
pub trait Importer {
    /// Cheap check on the first bytes / zip directory. Never parses the full file.
    fn sniff(&self, head: &[u8]) -> Confidence;
    fn import(&self, src: &Source, limits: &Limits, out: &mut DocumentBuilder)
        -> Result<ImportReport, ImportError>;
}

pub struct Limits {
    pub max_file_bytes: u64,          // default 4 GiB
    pub max_decompressed_bytes: u64,  // zip/zlib bomb guard, default 8× file size, cap 16 GiB
    pub max_nodes: u32,               // default 2,000,000
    pub max_depth: u16,               // default 256
    pub max_image_pixels: u64,        // default 300 MP per image
}

pub struct ImportReport { pub diagnostics: Vec<Diagnostic>, pub stats: ImportStats }
```

* `Source` is a memory-mapped file plus an optional virtual zip directory. Parsers read through a
  bounds-checked `Reader` (`read_u32_be()? → Result`). Raw slice indexing with lengths taken from
  the file is forbidden.
* Every parser has a `cargo-fuzz` target and a corpus of malformed files in `tests/corrupt/`.
* Import runs on a worker thread inside `catch_unwind`. On failure the user sees which part
  failed, and a partial document is offered when that is meaningful.
* Image payloads are **not decoded at import**. The importer registers `AssetSource::FileRange`
  entries pointing into the mmapped file.

## 2. Figma (`.fig`)

**Container.** Current `.fig` files are zip archives containing `canvas.fig`, `meta.json`,
`thumbnail.png` and `images/<sha1>`. `canvas.fig` begins with the `fig-kiwi` magic, a version,
and length-prefixed chunks. Chunk 0 is the compressed **kiwi schema**; chunk 1 is the compressed
message (zlib/deflate in older files, zstd in newer ones). Older non-zip `.fig` files start
directly with `fig-kiwi`.

**Parsing.** Kiwi is Evan Wallace's open-source binary format, and the schema is embedded in every
file. We decode the schema at runtime, so new Figma versions that add fields do not break the
parser. Unknown fields are skipped and recorded as preserved data. The message is a flat
`nodeChanges` list with `guid` + `parentIndex { guid, position }`. We rebuild the tree by sorting
siblings on the fractional-index `position` string.

**Mapping (MVP → phase 2).**

| Figma | UDM | MVP |
|---|---|---|
| DOCUMENT / CANVAS | Document / Page | ✓ |
| FRAME / GROUP / SECTION | Frame / Group / Frame | ✓ |
| RECTANGLE, ELLIPSE, REGULAR_POLYGON, STAR, LINE | shapes | ✓ |
| VECTOR (`vectorNetworkBlob`, `fillGeometry` blobs) | Vector (precomputed geometry first, vector network later) | ✓ |
| TEXT (`textData`, `characterStyleIDs`, `styleOverrideTable`) | Text with runs | ✓ basic |
| fills / strokes / effects / blend / opacity | Paint / Stroke / Effect | ✓ / phase 2 for effects |
| SYMBOL / INSTANCE (`symbolData`, overrides) | Component / Instance | phase 2 |
| stack* fields (auto layout), constraints | AutoLayout / Constraints | phase 2 |
| BOOLEAN_OPERATION | BooleanOp | phase 2 |
| Variables, styles | Document.styles | phase 2 |

**Alternative path.** Optionally import the Figma REST API JSON (`GET /v1/files/:key`) that the
user saved to disk. It is documented and stable, and it is useful for fidelity cross-checks. The
app never calls the network on its own (offline first).

**Legal note.** The `.fig` format is undocumented. We implement it clean-room for
interoperability, from public knowledge of the kiwi format and inspection of files the user owns.
No Figma code or assets are used. Compatibility claims are made only from automated tests on real
files (§8).

**Export.** `.fig` writing is not planned before phase 4, and only if round-trip tests prove
Figma opens the files correctly.

## 3. Adobe XD (`.xd`)

**Container.** A UCF zip: `mimetype`, `manifest` (JSON describing artboards and resources),
`META-INF/`, `artwork/artboard-<uuid>/graphics/graphicContent.agc`,
`resources/graphics/graphicContent.agc` (shared symbols, gradients, colour swatches),
`resources/<uid>` (images) and `interactions/`.

**Parsing.** AGC is JSON. We stream it with `serde_json` into typed structs, plus
`serde_json::Value` catch-alls for unknown keys. Unknown keys are preserved per node.

| XD | UDM | MVP |
|---|---|---|
| artboard | Frame (artboard) | ✓ |
| group, shape (rect/ellipse/path/line/polygon) | Group / shapes / Vector | ✓ |
| text (`paragraphs`, `rangedStyles`) | Text with runs | ✓ basic |
| image fill → `resources/<uid>` | Paint::Image (lazy) | ✓ |
| fills, strokes, gradients, opacity, blend | Paint / Stroke | ✓ / gradients phase 2 |
| dropShadow, blur, backgroundBlur | Effect | phase 2 |
| mask groups (`meta.ux.clipPathResources`) | IS_MASK child | phase 2 |
| symbols / components + states | Component / variants | phase 2 |
| repeat grid | Frame of instances + preserved grid meta | phase 2 (partial) |
| responsive resize, stacks, padding | Constraints / AutoLayout | phase 2 (partial) |

Adobe XD has been in maintenance mode since 2023, so its format is effectively frozen. That makes
it a stable target.

## 4. Photoshop (`.psd` / `.psb`)

**Structure.** Header → colour mode data → image resources → layer & mask info → merged image.
PSB uses 8-byte lengths in specific places. We support both through a `Width` abstraction in
the reader.

**Layer tree.** Layer records are stored bottom-to-top. Groups are delimited by `lsct` section
divider blocks (open/closed folder, bounding divider). We rebuild the hierarchy with a stack, so
**nothing is flattened**.

**Lazy rasters.** For each layer we record channel offsets and compression (raw, RLE/PackBits,
ZIP, ZIP-with-prediction) and register a `Raster { asset }` whose source is a `FileRange` with a
PSD channel codec. Decoding happens per tile and per mip level on demand in the renderer's image
cache.

| PSD | UDM | MVP |
|---|---|---|
| pixel layer | Raster | ✓ |
| group (`lsct`) | Group (pass-through blend preserved) | ✓ |
| opacity, fill opacity, blend modes, visibility | node fields | ✓ |
| clipping mask (`clipping` flag) | IS_MASK on clipping base | ✓ |
| layer mask, vector mask (`vmsk`/`vsms`) | `LayerMask` | ✓ raster mask / phase 2 vector |
| text (`TySh` + EngineData) | Text (editable) + stored pixels as fallback | ✓ display, basic edit |
| shape layers (`vogk`, `vscg`, `SoCo`) | Vector with fill | phase 2 |
| effects (`lfx2`) | Effect (shadows, strokes) + preserved block | phase 2 / preserved |
| adjustment layers (`levl`, `curv`, `hue2`, …) | `Adjustment` (preserved, fallback render) | preserved |
| smart objects (`SoLd`, `lnk2`) | Raster (placed pixels) + preserved embedded file | preserved |
| artboards (`artb`) | Frame | ✓ |
| unknown tagged blocks | `Preserved { key: "xxxx" }` | ✓ |

**Fidelity fallback.** Photoshop stores rendered pixels for text and shape layers. When we can't
render a feature exactly (adjustments, some effects), we draw the layer's stored pixels and show a
"Partially supported" badge:

> This Photoshop adjustment layer can be displayed, but cannot currently be edited. The original
> data will be preserved during export.

**Export.** Phase 4: write PSD with raster layers, groups, masks and verbatim preserved blocks.
Edited text layers get regenerated pixels, and their EngineData is rewritten only for the fields
we model.

## 5. SVG

* **Import:** `usvg` resolves CSS, `<use>`, units and transforms into a simplified tree, which we
  map to Group / Vector / Text / Raster nodes. Scripts, external `href`s and `foreignObject` are
  ignored and reported.
* **Export:** our own writer in `ude-format-svg` (also used by Dev Mode's SVG tab). It emits
  minimal path data, shares gradients through `<defs>`, and supports the precision setting.
  "Optimize SVG" applies path simplification and attribute deduplication without external tools.

## 6. PDF

* **Export (MVP+1):** `krilla` maps vectors, gradients, text (embedded subset fonts) and images.
  Transparency groups handle opacity and blend modes. Effects without PDF equivalents (blur) are
  rasterized at export DPI and reported.
* **Import (phase 4):** vector-level import is planned with a pure-Rust PDF parser. Until then,
  PDF opens as rasterized pages.

## 7. Raster (PNG / JPEG / WebP)

Import creates a Frame sized to the image with one Raster child. Export renders through the CPU
backend (deterministic, no GPU needed) at a chosen scale, with sRGB tagging.

## 8. Native `.krt`

KuArt documents use the `.krt` extension (MIME `application/x-kuart-design`). Files named
`.design` (the pre-0.1 name) open unchanged; the bytes are the same format.

Implemented in `crates/formats/native` (M1). Format version **0**.

```
┌──────────────────────────────────────────────────────────────────────┐
│ "UDESIGN\0" · version u16 LE · flags u16 LE · chunk count u32 LE     │
│ chunk table, 61 B per entry:                                         │  ← read first; enables streaming
│   tag[4] · offset u64 · stored len u64 · raw len u64 · codec u8 ·    │
│   blake3(raw)[32]                                                    │
├──────────────────────────────────────────────────────────────────────┤
│ META   title, source format, node count, page count                  │
│ STRS   string table, only strings that are referenced, first-use order│
│ ASET   asset table: blake3 content hash, kind, codec (i-th ↔ i-th BLOB)│
│ ROOT   the document node record                                      │
│ PAGE   one chunk per page: pre-order node records with child counts  │  ← lazy page loading later
│ BLOB   one chunk per asset; already-compressed codecs stored as-is   │
│ ????   unknown chunks from newer versions, preserved verbatim        │
└──────────────────────────────────────────────────────────────────────┘
```

**Encoding.** This is a small hand-written binary encoding (`wire.rs`): LEB128 varints,
little-endian `f32`, and length-prefixed strings and records. We chose it over serde + postcard for
three reasons:
1. `ude-core` stays dependency-free.
2. Every `NodeId`, `Symbol` and `AssetId` must be **remapped** to dense indices (pre-order /
   first-use) for determinism, which serde derives can't express.
3. The format contract stays explicit in one file (`model.rs`), with encoder and decoder side by
   side.

* **Deterministic:** chunk order is fixed, nodes are written in pre-order, and ids, strings and
  assets are renumbered by first use. Two documents with the same content produce byte-identical
  files regardless of edit history, undo or interner contents (test
  `same_content_same_bytes_regardless_of_history`). One caveat: bytes are identical *per zstd
  library version*. The hashes cover the raw bytes, so integrity checks are unaffected.
* **Forward compatible:**
  * Unknown chunk kinds are preserved on re-save (`Metadata::preserved_sections`).
  * Node, ext, asset and meta records are length-prefixed, so a reader skips trailing fields it
    doesn't know. **Rule after the first release:** new fields are only *appended* to a record,
    never inserted. Enum wire codes are append-only (`wire_enum!`, which also fails to compile if
    a core enum gains an unlisted variant).
  * A newer major layout bumps the header version, and older apps refuse such files with a clear
    message.
* **Sharing restored on load:** identical paint lists and vector paths are length-prefixed blobs,
  and the decoder hash-conses them, so `Arc` sharing (and its memory savings) survives a round trip.
* **Safety:**
  * All reads are bounds-checked, and forged counts are rejected when they exceed the remaining
    input.
  * zstd output is capped at the declared raw length (decompression-bomb guard).
  * Each chunk's blake3 hash is verified.
  * Node count, chunk count, chunk size and override nesting depth are limited (`Limits`).
  * The test suite flips bytes and truncates files at hundreds of positions and checks that the
    loader returns an error and never panics.
* **Atomic save:** the file is written to `*.design.tmp`, then renamed.
* **Not yet:**
  * Assets still backed by an unloaded source file (`AssetSource::FileRange`) are refused with
    `UnresolvedAsset`. Importers will resolve them when saving (M10–M12).
  * Lazy page loading and mmap-backed BLOBs come with the GUI.
* **Streamable (planned):** opening reads META plus the visible page, and other pages load when
  navigated to. The chunk table already supports this.
* **Autosave journal:** `project.design.journal` is an append-only log of transactions (forward
  ops, postcard + CRC per record). Recovery replays the journal over the last full save.
  Compaction rewrites only dirty page chunks.

## 9. Testing per format

For every format: corpus of real files → import → render (CPU backend) → compare with a reference
PNG (exported from the source app) → edit → export → re-import → render → compare. Metrics are
pixel-diff percentage and SSIM. Thresholds per file live in `tests/compat/<format>/manifest.toml`
and drive [compatibility-matrix.md](compatibility-matrix.md).
