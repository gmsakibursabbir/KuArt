# Feature checklist (audit of `main`, 2026-10-12)

✓ done · ~ partial (note says what is missing) · ✗ not yet.
Summary at the end. Priorities and order of work: [feature-gaps.md](feature-gaps.md).

## A. Canvas and document management
- ✓ Infinite canvas / workspace
- ✓ Multiple pages and artboards (pages panel, add page; artboards are frames)
- ✓ Frames, nested frames and groups
- ~ Sections and canvas organization (Figma sections import as frames; no section tool)
- ~ Page thumbnails and navigation (pages list; thumbnails only for files on the home screen)
- ~ Zoom, pan, fit-to-screen and zoom-to-selection (no zoom-to-selection yet)
- ~ Rulers, guides and smart guides (smart guides only)
- ✗ Configurable grids and layout grids (imported grids are kept, not drawn)
- ~ Snap to grid, guides and objects (snap to objects)
- ✓ Alignment and distribution (toolbar, Object menu, Alt shortcuts)
- ✓ Layer ordering and grouping
- ✓ Lock, hide, rename and duplicate layers
- ✓ Multi-select and selection tools (click, Shift, marquee, deep select)
- ~ Clipboard, copy/paste and paste-in-place (in-app paste; no cross-document or paste-in-place)
- ✓ Undo and redo history
- ✗ Search layers and objects
- ✗ Canvas minimap / navigator
- ✓ Multiple open documents (tabs)
- ✗ Document templates
- ~ Starter files and presets (New design = one 1440×1024 frame)
- ~ Local documents and cloud documents (local only, by design)
- ✓ Autosave and crash recovery (on pause, recovered at next launch)
- ✗ Version history and restore
- ✗ File duplication and document branching
- ~ Document metadata and ownership (title and source format)
- ✗ Page and layer linking
- ✗ Import and merge documents (import opens a new document)
- ✓ Large-document performance (100k layers: 85 MB, frames < 6 ms)
- ~ Keyboard shortcuts and customization (shortcuts, no customization)

## B. Vector drawing and illustration
- ✓ Selection tool · ✓ point editing (Enter or double-click a vector)
- ✓ Pen tool and Bézier curves (P)
- ✗ Pencil / freehand drawing
- ~ Line, arrow and connector tools (line; arrows/connectors only imported)
- ✓ Rectangle, rounded rectangle, ellipse and polygon
- ~ Stars and custom shapes (star; no custom shape tool)
- ✓ Vector point editing
- ✓ Add (click a segment), delete and convert (double-click) anchor points
- ~ Corner radius and independent corner controls (uniform edit; per-corner imported and drawn)
- ✓ Smooth, sharp and mirrored points (Alt breaks the mirror)
- ✓ Open and closed paths (drawn and imported)
- ✗ Path joining and splitting
- ✓ Boolean union, subtract, intersect and exclude (live, Ctrl+Alt+U/S/I/X; Flatten Ctrl+E)
- ✓ Compound paths (drawn, even-odd and non-zero)
- ✗ Shape expansion and outlining
- ~ Stroke alignment and cap styles (alignment editable; caps imported)
- ~ Join styles and miter limits (imported and drawn)
- ~ Dashed and dotted strokes (imported; drawn solid)
- ~ Multiple fills and strokes per layer (multiple fills editable; one stroke)
- ~ Gradient fills and strokes (drawn and exported; no editor)
- ~ Linear, radial, angular and diamond gradients (all four drawn; diamond approximated)
- ✗ Gradient stop editing
- ✓ Opacity and blend modes
- ~ Masks and clipping masks (frame clipping ✓; masks kept, not composited)
- ✓ Masks (vector outline; Ctrl+Alt+M) · ✗ alpha/luminance masks
- ✗ Vector crop and image crop
- ✓ SVG import and editing (import + edit as layers)
- ✓ SVG path preservation
- ~ Shape conversion and flattening (booleans flatten to a vector)
- ✗ Vector outline and simplify operations
- ✗ Shape builder and freeform polygons
- ✗ Brushes and variable-width strokes
- ✗ Pattern fills (image tile mode only)
- ✗ Vector textures and grain
- ~ Editable vector icons (vectors editable as layers, not by point)
- ✓ Boolean shape editing without destructive flattening (children stay editable)
- ✓ Precision coordinate editing (X/Y/W/H/rotation fields)
- ✗ SVG optimization and cleanup

## C. Raster and image editing
- ✓ Image import and placement (open images; image fills from imports)
- ✓ Drag-and-drop images (placed at the pointer; design files open in a tab)
- ~ Crop, resize and rotate images (resize/rotate; no crop)
- ~ Image fill and fit modes (fill/fit/tile drawn; not editable)
- ✗ Image adjustment controls
- ✗ Brightness, contrast, saturation and exposure
- ✗ Hue and color adjustments
- ✗ Blur and sharpening
- ✗ Background removal
- ✗ Image masking
- ✗ Image replacement while preserving layout
- ✗ Image filters and effects
- ✓ Image opacity and blending
- ✗ Basic retouching tools
- ~ Image clipping and compositing (frame clipping, PSD clipping baked)
- ✓ High-resolution image support (mip-level decode, 128 MB cache)
- ✗ Rasterization controls
- ✗ Image asset relinking
- ~ Embedded versus linked images (always embedded)
- ✓ Support for transparent images
- ✓ Raster export quality controls (scale, JPEG quality, palette compression)

## D. Typography and text
- ✓ Point text and area text (auto width, auto height, fixed)
- ✓ Text editing on canvas (caret, mouse and keyboard selection, word/line navigation, clipboard, IME)
- ✓ Font family, weight and style
- ~ Font size and line height (size editable; line height imported)
- ~ Letter spacing and paragraph spacing (imported and drawn)
- ~ Text alignment and justification (imported and drawn; not editable)
- ~ Text decoration (drawn; not editable)
- ~ Text transformation and case (imported; exported to code)
- ~ Text color, fill and stroke (fill; no text stroke)
- ✓ Text opacity · ✗ text effects
- ✗ Text on a path
- ✓ Paragraph wrapping and resizing
- ✓ Text box auto-sizing
- ✓ Vertical text alignment
- ✓ Font substitution warnings (face-level names in the status bar)
- ✓ Font fallback handling (+ Google Fonts download)
- ✗ OpenType features
- ~ Variable fonts (rendered by weight; no axis controls)
- ✗ Text styles
- ~ Reusable typography tokens (exported as tokens)
- ~ Rich text runs with different styles (imported and drawn; edits collapse to one style)
- ✗ Bulleted and numbered lists
- ✗ Text clipping and overflow detection
- ✗ Text outline conversion (PDF export outlines text)
- ✗ Text measurement and baseline controls
- ✓ Multilingual text and font support (parley shaping + fallback)
- ✓ Right-to-left text support (bidi in parley)
- ~ Emoji and color-font support (shaped; color glyphs depend on font)
- ✗ Text content replacement in components (only Sketch text overrides on import)
- ✗ Dynamic text and content placeholders

## E. Layout and responsive design
- ~ Auto layout / stack layout (imported, exported to flexbox; no reflow)
- ~ Horizontal and vertical layout flows (same)
- ~ Nested layouts (same)
- ~ Padding and spacing controls (data only)
- ~ Gap and distribution settings (data only)
- ~ Hug contents sizing (data only)
- ~ Fill container sizing (data only)
- ✓ Fixed dimensions
- ✗ Min/max width and height
- ~ Alignment within containers (data only)
- ~ Absolute positioning within layouts (data only)
- ✗ Relative positioning
- ~ Constraints and resizing behavior (imported, not applied)
- ✗ Responsive resize
- ✗ Breakpoint-based layouts
- ✗ Responsive frames
- ~ Flexbox-like layout behavior (in generated code)
- ✗ Grid-based layout
- ✗ CSS Grid-like row and column systems
- ✗ Aspect-ratio constraints
- ✓ Overflow and clipping (frame clip content)
- ✗ Scrollable frames
- ✗ Sticky and fixed-position elements
- ~ Wrap and flow layouts (wrap imported, exported)
- ✗ Content-aware resizing
- ✗ Layout presets
- ~ Repeat Grid / repeated-content generation (XD repeat grids import as groups)
- ✗ Data-driven repeated cards and lists
- ✗ Responsive variants
- ✗ Device presets for mobile, tablet and desktop

## F. Components and design systems
- ~ Reusable components (imported; no create-component command)
- ~ Component instances (imported, expanded, linked to main)
- ~ Nested components (imported)
- ✗ Component properties
- ✗ Boolean properties
- ✗ Text properties
- ✗ Instance swapping
- ✗ Component variants
- ✗ Variant sets and property combinations
- ~ Component overrides (single-level text/props on import)
- ✗ Reset and detach instance
- ✗ Component inheritance (instances do not update from the main)
- ✗ Component slots or replaceable content areas
- ✗ Interactive components
- ✗ Component states
- ✗ State-specific styling
- ✗ Reusable icon components
- ✗ Shared color styles
- ✗ Shared text styles
- ✗ Effect styles
- ✗ Grid and layout styles
- ~ Design tokens (exported from designs)
- ✗ Color variables
- ✗ Number and spacing variables
- ✗ String and Boolean variables
- ✗ Variable collections
- ✗ Variable modes for light and dark themes
- ✗ Brand and platform themes
- ✗ Variable aliases and references
- ✗ Component libraries
- ✗ Published shared libraries
- ✗ Library updates and change review
- ✗ Cross-document component reuse
- ✗ Design-system documentation
- ✗ Component usage guidelines
- ✗ Component search
- ✗ Component usage analytics
- ✗ Unused component and token detection
- ✗ Design-system consistency checks
- ~ Token export and import (export only)
- ✗ Component versioning
- ✗ Branching and merging design systems

## G. UI/UX design and wireframing
General-purpose drawing covers wireframes and high-fidelity screens of every kind (web,
mobile, tablet, desktop, dashboards, SaaS, e-commerce, forms, navigation, tables): ✓ by hand.
Not built in: ✗ UI kits and starter kits, iOS/Material libraries, chart elements, state
templates, accessibility annotations, flow/journey/sitemap diagram tools, A/B variant tools,
localization tooling.

## H. Prototyping and interactions
✗ Not started (connections, triggers, transitions, overlays, smart animate, variables and
logic, media playback, presentation and sharing). Prototype data in imported files is ignored.

## I. Motion and advanced animation
✗ Not started.

## J. Collaboration and feedback
- ✗ Real-time multiplayer editing (P2P WebRTC designed: [collaboration.md](collaboration.md))
- ✗ Live collaborator cursors · presence · follow · selection indicators
- ~ Comments on canvas (pins, Comments tab, add/reply/resolve/delete, saved in .krt; read from PDF, Illustrator, Photoshop notes, SVG and PNG/JPEG metadata; not live-shared yet)
- ~ Threaded discussions (replies) · ✓ resolve/reopen · ✗ mentions · ✗ notifications · ✗ audio
- ✗ Shared documents, workspaces, libraries, roles, guests, permissions, links, passwords
- ✗ Feedback mode · presentation mode · review links · approvals · status labels
- ✗ Activity feed · change notifications · conflict handling · offline sync · team assets

## K. Developer handoff and design-to-code
- ✓ Developer inspection mode (Dev Mode tab)
- ✓ Layer and component inspection
- ✓ CSS property inspection
- ✓ Computed dimensions and spacing
- ✓ Color value inspection (with copy)
- ✓ Typography specifications
- ~ Spacing and padding inspection (in generated code; no redlines)
- ~ Border, radius and shadow inspection (in code; radius in properties)
- ~ Constraints and layout inspection (flex in code)
- ✓ Asset downloads (export selection)
- ✓ Multiple asset export formats (PNG, JPG, WebP, SVG, PDF)
- ✓ SVG and PNG exports
- ✓ Export at multiple scales (0.5×–4×)
- ✗ Platform-specific asset presets
- ~ Export naming rules (name@2x)
- ✗ Asset slicing
- ✓ Export selected layers
- ✓ Design tokens for developers
- ✓ Code snippets (copy code)
- ✓ CSS, HTML and platform-specific code generation (HTML, CSS, Tailwind, React, Vue)
- ~ React component mapping (one component per selection)
- ✗ Component playground
- ✗ Component property inspection
- ✓ Design-to-code integrations (CLI + MCP)
- ✗ Code-to-design workflows
- ✗ Code Connect-style component mapping
- ✗ Design annotations
- ✗ Redline and measurement tools
- ✗ Version comparison for frames
- ✗ Ready-for-development queues
- ✗ Links to GitHub and issue trackers
- ✗ Storybook and documentation links
- ✗ VS Code / IDE integrations (MCP works with IDE agents)
- ✓ Design API access (MCP tools, CLI)
- ✓ MCP server integration for AI coding agents
- ✗ Live implementation preview
- ✗ Design-to-code consistency checks
- ~ Developer-friendly component specifications

## L. Import, export and interoperability
- ✓ PNG import and export · ✓ JPG import and export · ✓ WebP import and export
- ✓ SVG import and export · ✓ PDF import and export
- ✗ EPS · ✗ TIFF · ✗ HEIC · ~ GIF (import first frame inside design files) · ✗ video/audio · ✗ Lottie
- ✗ Clipboard import from other design applications
- ✗ Paste SVG from browsers and vector editors
- ✗ Copy and paste between documents
- ✓ Export complete pages · ✓ frames and artboards · ✓ individual layers · ✓ selected objects
- ✗ Batch export · ✗ Export presets · ✗ Custom filename templates
- ✓ Transparent backgrounds
- ✗ Color profile handling · ✗ wide-gamut color
- ✓ PDF page ordering · ✓ PDF multi-page export (one page per artboard, CLI/MCP)
- ✓ Design-token JSON export
- ✗ Asset manifest generation
- ✓ Import from Figma, XD and Sketch (+ FigJam, Slides, Buzz, Sites, Make)
- ✓ PSD and AI asset interoperability (import)
- ✓ Preserve text and vectors when importing (PSD text as pixels)
- ✗ Linked asset updates
- ✓ Loss-aware conversion warnings (each importer reports skipped features)
- ✗ Round-trip import and export to other apps' formats
- ✓ Open document recovery and compatibility (versioned .krt, legacy .design)

## M. Plugins, extensions and automation
- ✗ Plugin marketplace, plugins, SDK, permissions, sandboxing, updates
- ✓ Automation scripts / document transformation APIs (CLI + MCP)
- ✗ Webhooks, batch operations UI, custom shortcuts, custom panels/tools

## N. AI-powered design
- ✓ AI agent canvas editing (MCP: create, update, delete, render, save)
- ✓ MCP support for external agents
- ✓ Design-to-code generation
- ~ Text-to-design / prompt-to-layout (through an external agent over MCP; nothing built in)
- ✗ Built-in AI features (image generation/editing, background removal, writing, critique,
  naming, alt text, accessibility detection, semantic search, …)
- ~ Privacy controls for AI features (no built-in AI; nothing leaves the machine)

## O. Search, assets and libraries
- ✗ Global search · page/frame/layer/component/color/token search
- ✗ Asset, icon, illustration, image and font browsers
- ✓ Recent files (home screen) · ✗ recent assets · ✗ favorites
- ✗ Team/local/cloud libraries · ✗ unused/duplicate asset detection · ✗ tags · ✗ templates · ✗ marketplace

## P. Presentation and review
✗ Not started (presentation mode, presenter notes, review links). ✓ PDF export of artboards.

## Q. APIs, storage and enterprise
- ✓ Local-first storage · ✓ Offline editing
- ✗ Cloud sync · ✗ automatic backups · ✓ document recovery (autosave) · ✗ revision history
- ✗ Team/organization management, SSO, roles, audit logs, SCIM, admin
- ~ Large-file handling (100k layers; lazy PSD channels)
- ~ Public API (CLI + MCP; no REST/webhooks)
- ✗ GitHub / Jira integrations

## R. Accessibility and quality assurance
- ~ Keyboard-only navigation (shortcuts; panels not fully keyboard-navigable)
- ✗ Customizable shortcuts
- ✗ Screen-reader-friendly interface (egui accessibility not enabled yet)
- ~ Accessible canvas controls
- ✓ Contrast checking of the app's own theme (unit-tested) · ✗ contrast checks for designs
- ✗ Color-blindness previews · text size / target size checks · alt text · focus order
- ✗ Reduced-motion settings
- ✗ Overflow / text clipping detection
- ✓ Missing font warnings · ✓ Missing asset warnings (import reports)
- ✗ Broken component references · broken prototype links · unused variables
- ✗ Design linting and automated checks · accessibility reports

## S. Professional workflow and productivity
- ✗ Quick actions and command palette
- ✓ Global menu bar (File, Edit, View, Object, Help)
- ~ Context menus (home screen cards only)
- ~ Custom toolbars (floating tool pill) · ✗ dockable panels
- ✓ Panel resizing (left and right panels)
- ✗ Workspace presets · ✗ customizable inspector
- ✓ Recent documents · ✗ pinned documents
- ~ Keyboard shortcut reference (Help menu and tooltips)
- ✗ Shortcut customization
- ~ Multi-object editing (move/resize/delete; no shared property editing)
- ✗ Batch property editing · multi-layer renaming · find and replace · replace colors
- ✗ Bulk export · bulk component updates · templates · presets
- ✗ Smart selection · select similar · select matching layers
- ✗ Paste properties · copy visual style
- ✗ Measurement overlays
- ✗ History navigation panel
- ✗ Focus mode · distraction-free canvas
- ✗ Autosave indicators
- ✓ Performance diagnostics (F12)
- ~ Crash recovery (autosave) · ✗ crash reports
- ✗ Tutorials and onboarding
- ~ In-app help (Help menu)

## Summary

| Area | Status |
|---|---|
| A. Canvas and documents | core, align/distribute, tabs and autosave done; missing search, version history |
| B. Vector drawing | shapes, pen, point editing and booleans done; missing gradient editor, path join/split |
| C. Raster | placement and export done; no image editing |
| D. Typography | rendering and on-canvas editing done; missing text styles, lists |
| E. Layout | data imported and exported to code; no live auto-layout or constraints |
| F. Components | import only; no component editing, variables or libraries |
| G. UI/UX | drawable by hand; no kits or diagram tools |
| H. Prototyping, I. Motion | not started |
| J. Collaboration | not started (P2P WebRTC and comments planned) |
| K. Dev handoff | strong: Dev Mode, code, tokens, exports, MCP; missing redlines/annotations |
| L. Interop | strongest area: widest import support of any editor |
| M. Plugins | MCP/CLI automation only |
| N. AI | external agents via MCP; no built-in AI |
| O. Search/assets, P. Presentation, Q. Enterprise | mostly not started (local-first by design) |
| R. Accessibility/QA | font/asset warnings; no linting or a11y checks |
| S. Productivity | diagnostics, recents, menus; missing command palette and bulk tools |
