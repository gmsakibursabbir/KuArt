# Compatibility Matrix

Legend: ✓ fully supported · ~ partial · P preserved (displayed and kept, not editable) ·
✗ unsupported · – not applicable · ○ planned, not implemented yet

**Current status (M0): no importer is implemented yet.** Every entry is ○ until an automated
corpus test proves otherwise. This file is regenerated from `tests/compat/*/manifest.toml` once
the compatibility suite exists (M9+).

| Feature | Figma | XD | PSD | SVG | Target milestone |
|---|---|---|---|---|---|
| Pages / artboards | ○ | ○ | ○ | – | M10–M12 |
| Groups / layer hierarchy | ○ | ○ | ○ | ○ | M9–M12 |
| Basic shapes | ○ | ○ | – | ○ | M9–M12 |
| Vectors / paths | ○ | ○ | ○ (shape layers, phase 2) | ○ | M9–M12 |
| Text (display) | ○ | ○ | ○ | ○ | M9–M12 |
| Text (editable) | ○ | ○ | ○ | ○ | M10–M12 |
| Images / raster layers | ○ | ○ | ○ | ○ | M7–M12 |
| Solid fills / strokes | ○ | ○ | ○ | ○ | M9–M12 |
| Gradients | ○ | ○ | ○ | ○ | phase 2 |
| Opacity / blend modes | ○ | ○ | ○ | ○ | M9–M12 |
| Masks / clipping | ○ | ○ | ○ | ○ | M10 / phase 2 |
| Shadows / blur | ○ | ○ | ○ (P until phase 2) | ○ | phase 2 |
| Components / instances | ○ | ○ | – | – | phase 2 |
| Variants | ○ | ~ (states) | – | – | phase 2 |
| Auto layout / stacks | ○ | ○ | – | – | phase 2 |
| Constraints / responsive resize | ○ | ○ | – | – | phase 2 |
| Smart objects | – | – | ○ (P) | – | M10 |
| Adjustment layers | – | – | ○ (P) | – | M10 |
| Layer effects | ○ | ○ | ○ (P) | – | phase 2 |
| Repeat grid | – | ○ (~) | – | – | phase 2 |
| Native export (round trip) | ✗ | ✗ | ○ (phase 4) | ○ | — |
