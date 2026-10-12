# Compatibility Matrix

Legend: ✓ fully supported · ~ partial · P preserved (displayed and kept, not editable) ·
✗ unsupported · – not applicable · ○ planned, not implemented yet

**Current status (M10):** SVG (M9), PSD (M10) and Figma (M11) importers are implemented and
covered by unit tests on synthetic files; XD is not started. Corpus tests against real files are still
to come, so ✓ means "implemented and unit-tested". This file is regenerated from `tests/compat/*/manifest.toml` once
the compatibility suite exists (M9+).

| Feature | Figma | XD | PSD | SVG | Target milestone |
|---|---|---|---|---|---|
| Pages / artboards | ✓ | ○ | ✓ (artboards → frames) | – | M10–M12 |
| Groups / layer hierarchy | ✓ | ○ | ✓ | ✓ | M9–M12 |
| Basic shapes | ✓ | ○ | – | ✓ (as paths) | M9–M12 |
| Vectors / paths | ✓ (computed geometry) | ○ | P (shape layers shown as pixels; phase 2) | ✓ | M9–M12 |
| Text (display) | ✓ (styled runs) | ○ | ✓ (stored pixels) | ~ (as outlines) | M9–M12 |
| Text (editable) | ✓ | ○ | ○ phase 2 | ✗ | M10–M12 |
| Images / raster layers | ✓ (image fills) | ○ | ✓ (lazy, 8/16-bit RGB/gray) | ✓ | M7–M12 |
| Solid fills / strokes | ✓ | ○ | P (fill layers as pixels) | ✓ | M9–M12 |
| Gradients | ✓ | ○ | P (as pixels) | ✓ | phase 2 |
| Opacity / blend modes | ✓ | ○ | ✓ (linear burn/hard light approximate) | ✓ | M9–M12 |
| Masks / clipping | ~ (clipping ✓, masks flagged; compositing phase 2) | ○ | ✓ raster masks, clipping · P vector masks | ✗ (reported) | M10 / phase 2 |
| Shadows / blur | ✓ data · drawn: drop shadows | ○ | P | ✗ (filters reported) | phase 2 |
| Components / instances | ~ (instances expanded; nested overrides reported) | ○ | – | – | phase 2 |
| Variants | ○ | ~ (states) | – | – | phase 2 |
| Auto layout / stacks | ✓ (data; solver phase 2) | ○ | – | – | phase 2 |
| Constraints / responsive resize | ○ | ○ | – | – | phase 2 |
| Smart objects | – | – | P (placed pixels) | – | M10 |
| Adjustment layers | – | – | P (hidden, reported) | – | M10 |
| Layer effects | ○ | ○ | P | – | phase 2 |
| Repeat grid | – | ○ (~) | – | – | phase 2 |
| Native export (round trip) | ✗ | ✗ | ○ (phase 4) | ○ | — |
