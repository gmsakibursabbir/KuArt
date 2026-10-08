//! KuArt desktop application (`design-editor`).
//!
//! Canvas with pan/zoom. Event-driven: no frames are rendered while idle. The CPU renderer is
//! the default; `--gpu` (or `UDE_RENDERER=gpu`) opts into vello on wgpu, see docs/perf-log.md.
//!
//! Controls: wheel = pan (Shift = horizontal), Ctrl+wheel / pinch = zoom at cursor,
//! middle-drag or Space+drag = pan, Shift+1 = zoom to fit, Shift+0 / Ctrl+0 = 100%,
//! Ctrl+= / Ctrl+- = zoom, F3 = frame stats in the title bar.
//!
//! Selection: click = select layer (group-first), Ctrl/Cmd+click = deep select,
//! Shift+click = add/remove, double-click = enter group, drag on empty canvas = marquee,
//! Esc = select parent / clear, Ctrl/Cmd+A = select all at the current level.
//!
//! Editing: drag a selected layer = move (Shift = lock axis, snaps to other layers),
//! drag a handle = resize (Shift = keep ratio, Alt = from centre), drag just outside a
//! corner = rotate (Shift = 15° steps), arrows = nudge 1 (Shift = 10), Delete = delete,
//! Ctrl/Cmd+Z = undo, Ctrl/Cmd+Shift+Z or Ctrl+Y = redo, Esc while dragging = cancel.

mod backend;
mod camera;
mod selection_ui;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use backend::{Backend, FrameResult, UiFrame};
use camera::Camera;
use selection_ui::{Marquee, build_overlay};
use ude_core::{Document, History, NodeId, NodeKind, Op, Transaction};
use ude_editor::Tool;
use ude_editor::transform::{self, Modifiers};
use ude_editor::{
    Gesture, HandleIndex, Selection, SelectionMode, Snapper, drill, hit_test, marquee_select,
    resolve_target,
};
use ude_renderer::TextEngine;
use ude_renderer::View;
use ude_renderer::vello::kurbo::{Point, Rect, Vec2};
use ude_renderer::vello::peniko::Color;
use ude_spatial::SpatialIndex;
use ude_ui::{Action, Panels, Status, ThemeChoice, egui, theme};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

const USAGE: &str = "usage: design-editor [file.design] [--gpu | --cpu] [--stats] [--demo NODES]";

struct Options {
    file: Option<PathBuf>,
    /// GPU renderer is opt-in until its memory overhead is resolved (docs/perf-log.md).
    use_gpu: bool,
    stats: bool,
    demo_nodes: u32,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        file: None,
        use_gpu: std::env::var("UDE_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("gpu")),
        stats: false,
        demo_nodes: 10_000,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--gpu" => o.use_gpu = true,
            "--cpu" => o.use_gpu = false,
            "--stats" => o.stats = true,
            "--demo" => {
                o.demo_nodes = args
                    .next()
                    .and_then(|v| v.replace('_', "").parse().ok())
                    .ok_or("--demo needs a number")?
            }
            "-h" | "--help" => return Err(USAGE.into()),
            path if o.file.is_none() && !path.starts_with('-') => {
                o.file = Some(PathBuf::from(path))
            }
            other => return Err(format!("unexpected argument '{other}'\n{USAGE}")),
        }
    }
    Ok(o)
}

fn load_document(o: &Options) -> Result<(Document, String), String> {
    match &o.file {
        Some(path) => {
            let doc =
                ude_format_native::load_from_path(path, &ude_format_native::Limits::default())
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned());
            Ok((doc, name))
        }
        None => {
            let doc = ude_bench::generate(ude_bench::SynthSpec {
                nodes: o.demo_nodes,
                seed: 42,
            });
            Ok((doc, format!("Demo ({} nodes)", o.demo_nodes)))
        }
    }
}

struct App {
    options: Options,
    doc: Document,
    title: String,
    page: Option<NodeId>,
    window: Option<Arc<Window>>,
    backend: Option<Backend>,
    camera: Camera,
    fitted: bool,
    modifiers: ModifiersState,
    cursor: Point,
    space_down: bool,
    panning: bool,
    shown: bool,
    index: Option<SpatialIndex>,
    selection: Selection,
    hover: Option<NodeId>,
    marquee: Option<Marquee>,
    last_click: Option<(Instant, Point)>,
    history: History,
    drag: Option<Drag>,
    /// Snap guides of the running gesture, in document coordinates.
    guides: Vec<(Point, Point)>,
    /// Handle box of the last drawn selection (device space), for handle picking.
    selection_box: Option<[Point; 4]>,
    /// Source of unique merge keys: one per gesture or nudge sequence.
    merge_seq: u64,
    nudge_key: Option<u64>,
    cursor_icon: CursorIcon,
    egui: egui::Context,
    /// Present only with the CPU backend (egui has no renderer on the vello GPU path yet).
    egui_state: Option<egui_winit::State>,
    panels: Panels,
    /// Canvas area left between the panels, in device pixels.
    canvas: Rect,
    panel_hover: Option<NodeId>,
    applied_theme: Option<(ThemeChoice, bool)>,
    text: TextEngine,
    tool: Tool,
}

enum Drag {
    /// Drawing a new layer with a creation tool.
    Create(ude_editor::Create),
    /// Pressed on a selected layer; becomes a move once the pointer travels far enough.
    Pending { start: Point },
    Active {
        gesture: Gesture,
        snapper: Option<Snapper>,
    },
}

#[derive(Clone, Copy)]
enum Pick {
    Resize(HandleIndex),
    Rotate,
}

/// Pointer tolerance for hit testing, in device pixels.
const HIT_TOLERANCE_PX: f64 = 3.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Pointer travel before a press on a selected layer starts moving it (logical pixels).
const DRAG_THRESHOLD: f64 = 3.0;
const HANDLE_RADIUS: f64 = 6.0;
/// Rotation zone just outside the corner handles (logical pixels).
const ROTATE_RADIUS: f64 = 20.0;
/// Snap distance in logical pixels.
const SNAP_PX: f64 = 6.0;

impl App {
    fn new(options: Options, mut doc: Document, title: String) -> Self {
        let page = doc.children(doc.root()).iter().copied().find(|&id| {
            doc.node(id)
                .is_some_and(|n| matches!(n.kind, NodeKind::Page))
        });
        let index = page.map(|p| SpatialIndex::build(&doc, p));
        let _ = doc.take_changes();
        Self {
            history: History::default(),
            drag: None,
            guides: Vec::new(),
            selection_box: None,
            merge_seq: 0,
            nudge_key: None,
            cursor_icon: CursorIcon::Default,
            egui: egui::Context::default(),
            egui_state: None,
            panels: Panels::default(),
            canvas: Rect::ZERO,
            panel_hover: None,
            applied_theme: None,
            text: TextEngine::default(),
            tool: Tool::Move,
            index,
            selection: Selection::default(),
            hover: None,
            marquee: None,
            last_click: None,
            options,
            doc,
            title,
            page,
            window: None,
            backend: None,
            camera: Camera::default(),
            fitted: false,
            modifiers: ModifiersState::empty(),
            cursor: Point::ORIGIN,
            space_down: false,
            panning: false,
            shown: false,
        }
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn scale_factor(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor())
    }

    fn viewport(&self) -> (f64, f64) {
        self.window.as_ref().map_or((1.0, 1.0), |w| {
            let s = w.inner_size();
            (f64::from(s.width), f64::from(s.height))
        })
    }

    fn page_bounds(&self) -> Option<Rect> {
        let page = self.page?;
        self.doc
            .children(page)
            .iter()
            .filter_map(|&id| self.doc.world_bounds(id))
            .map(|r| Rect::new(r.x0.into(), r.y0.into(), r.x1.into(), r.y1.into()))
            .reduce(|a, b| a.union(b))
    }

    /// Visible canvas area in device pixels: between the panels, or the whole window without them.
    fn canvas_rect(&self) -> Rect {
        if self.canvas.area() > 0.0 {
            self.canvas
        } else {
            let (w, h) = self.viewport();
            Rect::new(0.0, 0.0, w, h)
        }
    }

    fn zoom_to_fit(&mut self) {
        if let Some(bounds) = self.page_bounds() {
            let c = self.canvas_rect();
            self.camera
                .fit(bounds, c.width(), c.height(), self.scale_factor());
            self.camera.pan += c.origin().to_vec2();
        }
        self.redraw();
    }

    fn zoom_center(&mut self, factor: f64) {
        self.camera
            .zoom_at(self.canvas_rect().center(), factor, self.scale_factor());
        self.redraw();
    }

    /// Runs the panels and applies their actions; returns egui output for the backend.
    fn run_panels(
        &mut self,
        window: &Window,
        page: NodeId,
    ) -> Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32)> {
        let state = self.egui_state.as_mut()?;
        let input = state.take_egui_input(window);
        let system_dark = window.theme() != Some(winit::window::Theme::Light);
        let tokens = self.panels.theme.tokens(system_dark);
        if self.applied_theme != Some((self.panels.theme, system_dark)) {
            theme::apply(&self.egui, tokens);
            self.applied_theme = Some((self.panels.theme, system_dark));
        }
        let renderer = self.backend.as_ref().map(Backend::name).unwrap_or_default();
        let missing = self.text.missing_fonts();
        let status = Status {
            title: &self.title,
            zoom: self.camera.zoom,
            renderer: &renderer,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            tool: self.tool,
            missing_fonts: &missing,
        };
        let mut result = None;
        let out = self.egui.run_ui(input, |ui| {
            result = Some(self.panels.ui(
                ui,
                &self.doc,
                page,
                &self.selection,
                self.hover,
                &status,
                tokens,
            ));
        });
        state.handle_platform_output(window, out.platform_output);
        if out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero())
        {
            window.request_redraw();
        }
        let ppp = f64::from(out.pixels_per_point);
        let (actions, canvas) = result?;
        self.canvas = Rect::new(
            f64::from(canvas.left()) * ppp,
            f64::from(canvas.top()) * ppp,
            f64::from(canvas.right()) * ppp,
            f64::from(canvas.bottom()) * ppp,
        );
        self.apply_actions(actions);
        let jobs = self.egui.tessellate(out.shapes, out.pixels_per_point);
        Some((jobs, out.textures_delta, out.pixels_per_point))
    }

    fn apply_actions(&mut self, actions: Vec<Action>) {
        let hover = self.panel_hover;
        self.panel_hover = None;
        for action in actions {
            match action {
                Action::Select(ids) => self.selection.set(ids),
                Action::Toggle(id) => self.selection.toggle(id),
                Action::Hover(id) => self.panel_hover = id,
                Action::Set {
                    id,
                    value,
                    merge_key,
                } => {
                    let mut tx = Transaction::new("Edit").with_merge_key(merge_key);
                    match tx.apply(&mut self.doc, Op::Set { id, value }) {
                        Ok(()) => self.history.commit(&mut self.doc, tx),
                        Err(e) => eprintln!("edit failed: {e}"),
                    }
                }
                Action::Tool(tool) => self.tool = tool,
                Action::SetText {
                    id,
                    content,
                    family,
                    size,
                    weight,
                    merge_key,
                } => {
                    self.set_text(id, content, family, size, weight, merge_key);
                }
                Action::Undo => self.undo(),
                Action::Redo => self.redo(),
                Action::ZoomToFit => self.zoom_to_fit(),
            }
            self.redraw();
        }
        if self.panel_hover != hover {
            self.redraw();
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(window), Some(page)) = (self.window.clone(), self.page) else {
            return;
        };
        // Panels first: their actions edit the document before this frame is drawn.
        let ui = self.run_panels(&window, page);
        let system_dark = window.theme() != Some(winit::window::Theme::Light);
        let [r, g, b, _] = self.panels.theme.tokens(system_dark).bg_canvas.to_array();
        let bg = Color::from_rgba8(r, g, b, 255);
        if !self.fitted {
            self.fitted = true;
            self.zoom_to_fit();
        }
        // Keep derived state in sync with document edits.
        let changes = self.doc.take_changes();
        if let Some(index) = &mut self.index {
            index.update(&self.doc, &changes);
        }
        if !changes.is_empty() {
            self.selection.retain_valid(&self.doc);
        }

        let (width, height) = self.viewport();
        let view = View {
            transform: self.camera.transform(window.scale_factor()),
            width,
            height,
        };
        let dragging = matches!(self.drag, Some(Drag::Active { .. }));
        let mut overlay = match &self.index {
            Some(index) => build_overlay(
                &self.doc,
                index,
                view.transform,
                window.scale_factor(),
                &self.selection,
                // No hover outline while dragging: it would trail the moving layer.
                if dragging {
                    None
                } else {
                    self.panel_hover.or(self.hover)
                },
                self.marquee.as_ref(),
            ),
            None => Default::default(),
        };
        overlay.guides = self
            .guides
            .iter()
            .map(|&(a, b)| (view.transform * a, view.transform * b))
            .collect();
        self.selection_box = overlay.selection_box.map(|o| o.quad);
        let Some(backend) = self.backend.as_mut() else {
            return;
        };

        let started = Instant::now();
        let ui = ui
            .as_ref()
            .map(|(jobs, textures, pixels_per_point)| UiFrame {
                jobs,
                textures,
                pixels_per_point: *pixels_per_point,
            });
        match backend.render(
            &self.doc,
            page,
            &view,
            &overlay,
            bg,
            ui.as_ref(),
            &mut self.text,
        ) {
            Ok(FrameResult::Presented(stats)) => {
                if self.options.stats {
                    window.set_title(&format!(
                        "{} — KuArt — {:.0}% — {:.1} ms — drawn {} / visited {} / culled {} — {}",
                        self.title,
                        self.camera.zoom * 100.0,
                        started.elapsed().as_secs_f64() * 1000.0,
                        stats.drawn,
                        stats.visited,
                        stats.culled,
                        backend.name()
                    ));
                }
            }
            Ok(FrameResult::Skipped) => window.request_redraw(),
            Err(e) if matches!(backend, Backend::Gpu(_)) => {
                eprintln!("GPU rendering failed ({e}); switching to the CPU renderer");
                match Backend::new(window.clone(), true) {
                    Ok(cpu) => {
                        self.backend = Some(cpu);
                        window.request_redraw();
                    }
                    Err(e) => fatal(event_loop, &format!("no renderer available: {e}")),
                }
            }
            Err(e) => fatal(event_loop, &format!("rendering failed: {e}")),
        }
    }

    fn on_key(&mut self, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let pressed = event.state == ElementState::Pressed;
        if code == KeyCode::Space {
            self.space_down = pressed;
            self.update_cursor();
            return;
        }
        if !pressed {
            return;
        }
        let (shift, ctrl) = (
            self.modifiers.shift_key(),
            self.modifiers.control_key() || self.modifiers.super_key(),
        );
        if self.drag.is_some() {
            // Esc cancels a running gesture; other keys wait until it ends.
            if code == KeyCode::Escape {
                self.cancel_drag();
            }
            return;
        }
        let arrow = match code {
            KeyCode::ArrowLeft => Some(Vec2::new(-1.0, 0.0)),
            KeyCode::ArrowRight => Some(Vec2::new(1.0, 0.0)),
            KeyCode::ArrowUp => Some(Vec2::new(0.0, -1.0)),
            KeyCode::ArrowDown => Some(Vec2::new(0.0, 1.0)),
            _ => None,
        };
        if let Some(dir) = arrow {
            self.nudge(dir * if shift { 10.0 } else { 1.0 });
            return;
        }
        self.nudge_key = None;
        if !ctrl && let Some(tool) = tools::tool_for_key(code) {
            self.tool = tool;
            self.update_cursor();
            return self.redraw();
        }
        if code == KeyCode::Escape && self.tool != Tool::Move {
            self.tool = Tool::Move;
            self.update_cursor();
            return self.redraw();
        }
        match code {
            KeyCode::KeyZ if ctrl && shift => self.redo(),
            KeyCode::KeyZ if ctrl => self.undo(),
            KeyCode::KeyY if ctrl => self.redo(),
            KeyCode::Delete | KeyCode::Backspace if !self.selection.is_empty() => {
                let ids = self.selection.ids().to_vec();
                match transform::delete(&mut self.doc, &mut self.history, &ids) {
                    Ok(()) => self.selection.clear(),
                    Err(e) => eprintln!("delete failed: {e}"),
                }
                self.hover = None;
                self.redraw();
            }
            KeyCode::Digit1 if shift => self.zoom_to_fit(),
            KeyCode::Digit0 if shift || ctrl => {
                let (w, h) = self.viewport();
                self.camera
                    .set_zoom_at(Point::new(w / 2.0, h / 2.0), 1.0, self.scale_factor());
                self.redraw();
            }
            KeyCode::Equal | KeyCode::NumpadAdd if ctrl => self.zoom_center(2.0),
            KeyCode::Minus | KeyCode::NumpadSubtract if ctrl => self.zoom_center(0.5),
            KeyCode::Escape => {
                // Select the parents of the selection; at the top level, clear it.
                let page = self.page;
                let parents: Vec<NodeId> = self
                    .selection
                    .ids()
                    .iter()
                    .filter_map(|&id| self.doc.node(id).and_then(|n| n.parent))
                    .filter(|&p| Some(p) != page)
                    .collect();
                self.selection.set(parents);
                self.redraw();
            }
            KeyCode::KeyA if ctrl => {
                // All layers at the level of the current selection (or the page's top level).
                let parent = self
                    .selection
                    .ids()
                    .first()
                    .and_then(|&id| self.doc.node(id))
                    .and_then(|n| n.parent)
                    .or(self.page);
                if let Some(parent) = parent {
                    let all: Vec<NodeId> = self.doc.children(parent).to_vec();
                    self.selection.set(all);
                }
                self.redraw();
            }
            KeyCode::F3 => {
                self.options.stats = !self.options.stats;
                if !self.options.stats
                    && let Some(w) = &self.window
                {
                    w.set_title(&format!("{} — KuArt", self.title));
                }
                self.redraw();
            }
            _ => {}
        }
    }

    fn next_merge_key(&mut self) -> u64 {
        self.merge_seq += 1;
        self.merge_seq
    }

    /// Consecutive nudges share a merge key: one undo step per nudge sequence.
    fn nudge(&mut self, delta: Vec2) {
        if self.selection.is_empty() {
            return;
        }
        let key = match self.nudge_key {
            Some(k) => k,
            None => {
                let k = self.next_merge_key();
                self.nudge_key = Some(k);
                k
            }
        };
        let ids = self.selection.ids().to_vec();
        if let Err(e) = transform::nudge(&mut self.doc, &mut self.history, &ids, delta, key) {
            eprintln!("nudge failed: {e}");
        }
        self.redraw();
    }

    fn undo(&mut self) {
        match self.history.undo(&mut self.doc) {
            Ok(true) => {
                self.selection.retain_valid(&self.doc);
                self.redraw();
            }
            Ok(false) => {}
            Err(e) => eprintln!("undo failed: {e}"),
        }
    }

    fn redo(&mut self) {
        match self.history.redo(&mut self.doc) {
            Ok(true) => {
                self.selection.retain_valid(&self.doc);
                self.redraw();
            }
            Ok(false) => {}
            Err(e) => eprintln!("redo failed: {e}"),
        }
    }

    fn gesture_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
        }
    }

    /// Resize handle or rotation zone of the current selection under `device`.
    fn pick_handle(&self, device: Point) -> Option<Pick> {
        let quad = self.selection_box?;
        let s = self.scale_factor();
        let handles = ude_renderer::overlay::handle_points(&quad);
        if let Some(i) = handles
            .iter()
            .position(|&h| (h - device).hypot() <= HANDLE_RADIUS * s)
        {
            return Some(Pick::Resize(i as HandleIndex));
        }
        let bounds = quad.iter().fold(
            Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |r, &p| r.union_pt(p),
        );
        let near_corner = [0, 2, 4, 6]
            .iter()
            .any(|&i| (handles[i] - device).hypot() <= ROTATE_RADIUS * s);
        (near_corner && !bounds.contains(device)).then_some(Pick::Rotate)
    }

    /// Starts a resize/rotate (`pick`) or a move (`None`) of the selection at `start` (device).
    fn begin_gesture(&mut self, pick: Option<Pick>, start: Point) {
        let key = self.next_merge_key();
        let start = self.doc_point(start);
        let ids = self.selection.ids().to_vec();
        let (gesture, snapper) = match pick {
            Some(Pick::Resize(h)) => (Gesture::begin_resize(&self.doc, &ids, h, start, key), None),
            Some(Pick::Rotate) => (Gesture::begin_rotate(&self.doc, &ids, start, key), None),
            None => {
                let snapper = self.index.as_ref().map(|index| {
                    let (w, h) = self.viewport();
                    let inv = self.camera.transform(self.scale_factor()).inverse();
                    let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
                    Snapper::new(&self.doc, index, &ids, visible, SNAP_PX / self.camera.zoom)
                });
                (Gesture::begin_move(&self.doc, &ids, start, key), snapper)
            }
        };
        if !gesture.is_empty() {
            self.drag = Some(Drag::Active { gesture, snapper });
        }
    }

    fn drag_to(&mut self) {
        if matches!(self.drag, Some(Drag::Create(_))) {
            return self.create_drag_to();
        }
        let pointer = self.doc_point(self.cursor);
        let mods = self.gesture_modifiers();
        let Some(Drag::Active { gesture, snapper }) = &self.drag else {
            return;
        };
        match gesture.update(
            &mut self.doc,
            &mut self.history,
            pointer,
            mods,
            snapper.as_ref(),
        ) {
            Ok(feedback) => self.guides = feedback.guides,
            Err(e) => eprintln!("transform failed: {e}"),
        }
        self.redraw();
    }

    fn end_drag(&mut self) {
        self.drag = None;
        self.guides.clear();
        self.redraw();
    }

    fn cancel_drag(&mut self) {
        if matches!(self.drag, Some(Drag::Active { .. } | Drag::Create(_))) {
            // The gesture is one coalesced history entry; undoing it restores the start state.
            self.undo();
        }
        self.end_drag();
    }

    fn doc_point(&self, device: Point) -> Point {
        self.camera.transform(self.scale_factor()).inverse() * device
    }

    /// The layer a click at `device` would select (applying the selection rules).
    fn target_at(&self, device: Point, mode: SelectionMode) -> Option<NodeId> {
        let (index, page) = (self.index.as_ref()?, self.page?);
        let p = self.doc_point(device);
        let tolerance = HIT_TOLERANCE_PX / (self.camera.zoom * self.scale_factor());
        let hit = hit_test(&self.doc, index, p.x, p.y, tolerance)?;
        resolve_target(&self.doc, page, hit, mode)
    }

    fn mode(&self) -> SelectionMode {
        if self.modifiers.control_key() || self.modifiers.super_key() {
            SelectionMode::Deep
        } else {
            SelectionMode::GroupFirst
        }
    }

    fn on_left_press(&mut self) {
        self.nudge_key = None;
        if self.tool != Tool::Move {
            return self.begin_create();
        }
        if let Some(pick) = self.pick_handle(self.cursor) {
            self.begin_gesture(Some(pick), self.cursor);
            self.redraw();
            return;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| {
            now.duration_since(t) < DOUBLE_CLICK && (p - self.cursor).hypot() < 4.0
        });
        self.last_click = Some((now, self.cursor));

        // Double-click on a selected container drills one level towards the pointer.
        if double
            && let [current] = self.selection.ids()
            && let Some(deep) = self.target_at(self.cursor, SelectionMode::Deep)
            && let Some(next) = drill(&self.doc, *current, deep)
        {
            self.selection.set([next]);
            self.redraw();
            return;
        }

        match self.target_at(self.cursor, self.mode()) {
            Some(target) => {
                if self.modifiers.shift_key() {
                    self.selection.toggle(target);
                } else if !self.selection.contains(target) {
                    self.selection.set([target]);
                }
                // Pressing on a selected layer arms a move; it starts past the drag threshold.
                if self.selection.contains(target) {
                    self.drag = Some(Drag::Pending { start: self.cursor });
                }
            }
            None => {
                let base = if self.modifiers.shift_key() {
                    self.selection.ids().to_vec()
                } else {
                    Vec::new()
                };
                if !self.modifiers.shift_key() {
                    self.selection.clear();
                }
                let start = self.doc_point(self.cursor);
                self.marquee = Some(Marquee {
                    start,
                    current: start,
                    base,
                });
            }
        }
        self.redraw();
    }

    fn on_left_release(&mut self) {
        if self.marquee.take().is_some() {
            self.redraw();
        }
        if matches!(self.drag, Some(Drag::Create(_))) {
            self.finish_create();
        }
        if self.drag.is_some() {
            self.end_drag();
        }
        self.update_cursor();
    }

    fn on_cursor_moved(&mut self, delta: Vec2) {
        if self.panning {
            self.camera.pan += delta;
            self.redraw();
            return;
        }
        if let Some(Drag::Pending { start }) = self.drag {
            if (self.cursor - start).hypot() >= DRAG_THRESHOLD * self.scale_factor() {
                self.begin_gesture(None, start);
                self.drag_to();
                self.update_cursor();
            }
            return;
        }
        if self.drag.is_some() {
            self.drag_to();
        } else if self.marquee.is_some() {
            self.update_marquee();
        } else {
            self.update_hover();
            self.update_cursor();
        }
    }

    fn update_marquee(&mut self) {
        let current = self.doc_point(self.cursor);
        let mode = self.mode();
        let (Some(m), Some(index)) = (self.marquee.as_mut(), self.index.as_ref()) else {
            return;
        };
        m.current = current;
        let hits = marquee_select(&self.doc, index, m.doc_rect(), mode);
        self.selection.set(m.base.iter().copied().chain(hits));
        self.redraw();
    }

    fn update_hover(&mut self) {
        let hover = if self.panning || self.space_down {
            None
        } else {
            self.target_at(self.cursor, self.mode())
        };
        if hover != self.hover {
            self.hover = hover;
            self.redraw();
        }
    }

    fn update_cursor(&mut self) {
        let icon = if self.panning {
            CursorIcon::Grabbing
        } else if self.space_down {
            CursorIcon::Grab
        } else if matches!(self.drag, Some(Drag::Active { .. })) {
            self.cursor_icon
        } else if self.tool != Tool::Move {
            CursorIcon::Crosshair
        } else {
            match self.pick_handle(self.cursor) {
                Some(Pick::Resize(0 | 4)) => CursorIcon::NwseResize,
                Some(Pick::Resize(2 | 6)) => CursorIcon::NeswResize,
                Some(Pick::Resize(1 | 5)) => CursorIcon::NsResize,
                Some(Pick::Resize(_)) => CursorIcon::EwResize,
                Some(Pick::Rotate) => CursorIcon::Crosshair,
                None => CursorIcon::Default,
            }
        };
        if icon != self.cursor_icon
            && let Some(w) = &self.window
        {
            w.set_cursor(icon);
            self.cursor_icon = icon;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("{} — KuArt", self.title))
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_visible(false);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return fatal(event_loop, &format!("cannot create window: {e}")),
        };
        match Backend::new(window.clone(), !self.options.use_gpu) {
            Ok(b) => {
                eprintln!("renderer: {}", b.name());
                self.backend = Some(b);
            }
            Err(e) => return fatal(event_loop, &format!("no renderer available: {e}")),
        }
        if matches!(self.backend, Some(Backend::Cpu(_))) {
            self.egui_state = Some(egui_winit::State::new(
                self.egui.clone(),
                egui::ViewportId::ROOT,
                &window,
                Some(window.scale_factor() as f32),
                window.theme(),
                None,
            ));
        }
        self.window = Some(window.clone());
        // Render once before showing the window to avoid a blank flash. Hidden windows get
        // no paint events on Windows, so this frame is drawn directly, and the window is
        // shown whatever its outcome.
        self.render(event_loop);
        if !self.shown {
            self.shown = true;
            window.set_visible(true);
        }
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let mut egui_consumed = false;
        if let (Some(state), Some(window)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            let r = state.on_window_event(window, &event);
            if r.repaint {
                window.request_redraw();
            }
            egui_consumed = r.consumed;
        }
        let mut cursor_delta = Vec2::ZERO;
        if let WindowEvent::CursorMoved { position, .. } = &event {
            let p = Point::new(position.x, position.y);
            cursor_delta = p - self.cursor;
            self.cursor = p;
        }
        // Pointer over a panel belongs to egui, unless a canvas gesture is in progress.
        let canvas_busy = self.drag.is_some() || self.marquee.is_some() || self.panning;
        let over_panel =
            self.egui_state.is_some() && !canvas_busy && !self.canvas.contains(self.cursor);
        match event {
            WindowEvent::KeyboardInput { .. }
                if egui_consumed || self.egui.egui_wants_keyboard_input() => {}
            WindowEvent::MouseWheel { .. } if over_panel => {}
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } if over_panel => {}
            WindowEvent::CursorMoved { .. } if over_panel => {
                // egui sets the cursor icon over panels.
                self.cursor_icon = CursorIcon::Default;
                if self.hover.take().is_some() {
                    self.redraw();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(b) = &mut self.backend {
                    b.resize(size.width, size.height);
                }
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.redraw(),
            WindowEvent::RedrawRequested => self.render(event_loop),
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                // Modifiers change the gesture (axis lock, aspect ratio, from centre) mid-drag.
                if matches!(self.drag, Some(Drag::Active { .. })) {
                    self.drag_to();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 60.0, f64::from(y) * 60.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                if self.modifiers.control_key() || self.modifiers.super_key() {
                    // Exponential zoom: equal wheel distance = equal zoom ratio.
                    let factor = (dy * 0.0025).exp();
                    self.camera
                        .zoom_at(self.cursor, factor, self.scale_factor());
                } else if self.modifiers.shift_key() {
                    self.camera.pan += Vec2::new(dy + dx, 0.0);
                } else {
                    self.camera.pan += Vec2::new(dx, dy);
                }
                self.redraw();
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.camera
                    .zoom_at(self.cursor, 1.0 + delta, self.scale_factor());
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pan_button = button == MouseButton::Middle
                    || (button == MouseButton::Left && self.space_down);
                if pan_button {
                    self.panning = state == ElementState::Pressed;
                    self.update_cursor();
                } else if button == MouseButton::Left {
                    match state {
                        ElementState::Pressed => self.on_left_press(),
                        ElementState::Released => self.on_left_release(),
                    }
                }
            }
            WindowEvent::CursorMoved { .. } => self.on_cursor_moved(cursor_delta),
            WindowEvent::CursorLeft { .. } if self.hover.take().is_some() => self.redraw(),
            _ => {}
        }
    }
}

fn fatal(event_loop: &ActiveEventLoop, msg: &str) {
    eprintln!("error: {msg}");
    event_loop.exit();
}

fn main() -> std::process::ExitCode {
    let options = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}");
            return std::process::ExitCode::from(2);
        }
    };
    let (doc, title) = match load_document(&options) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let event_loop = match EventLoop::new() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("error: cannot start the event loop: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut app = App::new(options, doc, title);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
