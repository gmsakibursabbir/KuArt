//! KuArt desktop application (`design-editor`).
//!
//! Canvas with pan/zoom. Event-driven: no frames are rendered while idle. The CPU renderer is
//! the default; `--gpu` (or `UDE_RENDERER=gpu`) opts into vello on wgpu, see docs/perf-log.md.
//!
//! Controls: wheel = pan (Shift = horizontal), Ctrl+wheel / pinch = zoom at cursor,
//! middle-drag or Space+drag = pan, Shift+1 = zoom to fit, Shift+0 / Ctrl+0 = 100%,
//! Ctrl+= / Ctrl+- = zoom, F3 = frame stats in the title bar, F12 = diagnostics window.
//! Ctrl+C / X / V = copy, cut, paste; Ctrl+D = duplicate; Ctrl+G / Ctrl+Shift+G = group /
//! ungroup; Ctrl+] / Ctrl+[ = forward / backward (with Shift: to front / back).
//!
//! Selection: click = select layer (group-first), Ctrl/Cmd+click = deep select,
//! Shift+click = add/remove, double-click = enter group, drag on empty canvas = marquee,
//! Esc = select parent / clear, Ctrl/Cmd+A = select all at the current level.
//!
//! Editing: drag a selected layer = move (Shift = lock axis, snaps to other layers),
//! drag a handle = resize (Shift = keep ratio, Alt = from centre), drag just outside a
//! corner = rotate (Shift = 15° steps), arrows = nudge 1 (Shift = 10), Delete = delete,
//! Ctrl/Cmd+Z = undo, Ctrl/Cmd+Shift+Z or Ctrl+Y = redo, Esc while dragging = cancel.

// A GUI app: Windows must not open a console window next to it.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod autosave;
mod backend;
mod camera;
mod files;
mod google_fonts;
mod pen;
mod recents;
mod selection_ui;
mod settings;
mod tabs;
mod text_edit;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use backend::{Backend, FrameResult, UiFrame};
use camera::Camera;
use selection_ui::{Marquee, build_overlay};
use ude_core::scene::BoolOpKind;
use ude_core::{Document, History, NodeId, NodeKind, Op, Transaction};
use ude_editor::transform::{self, Modifiers};
use ude_editor::{Arrange, Tool};
use ude_editor::{
    Gesture, HandleIndex, Selection, SelectionMode, Snapper, drill, hit_test, marquee_select,
    resolve_target,
};
use ude_renderer::RenderCaches;
use ude_renderer::View;
use ude_renderer::vello::kurbo::{Point, Rect, Vec2};
use ude_renderer::vello::peniko::Color;
use ude_spatial::SpatialIndex;
use ude_ui::{Action, Panels, Status, ThemeChoice, egui, theme};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

const USAGE: &str = "usage: design-editor [file.krt] [--gpu | --cpu] [--stats] [--demo NODES]";

struct Options {
    file: Option<PathBuf>,
    /// GPU renderer is opt-in until its memory overhead is resolved (docs/perf-log.md).
    use_gpu: bool,
    stats: bool,
    /// `--demo N`: open a synthetic N-layer document instead of the home screen.
    demo: Option<u32>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        file: None,
        use_gpu: std::env::var("UDE_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("gpu")),
        stats: false,
        demo: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--gpu" => o.use_gpu = true,
            "--cpu" => o.use_gpu = false,
            "--stats" => o.stats = true,
            "--demo" => {
                o.demo = args
                    .next()
                    .and_then(|v| v.replace('_', "").parse().ok())
                    .map(Some)
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
        Some(path) => Ok((files::load_path(path)?, files::file_name(path))),
        None => match o.demo {
            Some(nodes) => {
                let doc = ude_bench::generate(ude_bench::SynthSpec { nodes, seed: 42 });
                Ok((doc, format!("Demo ({nodes} nodes)")))
            }
            None => Ok((files::blank_document(), "KuArt".into())),
        },
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
    /// Wakes the event loop from background work (font downloads).
    proxy: Option<EventLoopProxy<AppEvent>>,
    /// Progress or result of the last font download, shown in the status bar.
    font_status: Option<String>,
    /// Copied layers (Ctrl+C / Ctrl+X); valid for the current document only.
    clipboard: ude_editor::Clipboard,
    /// Diagnostics window (F12).
    diagnostics: bool,
    /// Duration and statistics of the last presented frame.
    last_frame: Option<(f64, ude_renderer::FrameStats)>,
    applied_theme: Option<(ThemeChoice, bool)>,
    /// Document revision at the last save (or open); differs when there are unsaved changes.
    saved_revision: u64,
    shown_dirty: bool,
    caches: RenderCaches,
    tool: Tool,
    /// Home screen (recent files) instead of the editor.
    home: bool,
    recents: Vec<ude_ui::Recent>,
    recents_dirty: bool,
    /// Open documents; the active one is `None` here (its state is in the fields above).
    tabs: Vec<Option<tabs::DocTab>>,
    active_tab: Option<usize>,
    /// Autosave file id of the active document.
    doc_id: u64,
    last_input: Instant,
    last_autosave: Instant,
    /// Text layer being edited on the canvas.
    editing: Option<text_edit::Editing>,
    /// Pen drawing or point editing on the canvas.
    path_edit: Option<pen::PathEdit>,
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
            egui: {
                let c = egui::Context::default();
                theme::install_fonts(&c);
                c
            },
            egui_state: None,
            panels: Panels::default(),
            canvas: Rect::ZERO,
            panel_hover: None,
            proxy: None,
            font_status: None,
            clipboard: ude_editor::Clipboard::default(),
            diagnostics: false,
            last_frame: None,
            applied_theme: None,
            saved_revision: 0,
            shown_dirty: false,
            caches: RenderCaches::default(),
            tool: Tool::Move,
            home: false,
            recents: Vec::new(),
            recents_dirty: true,
            tabs: Vec::new(),
            active_tab: None,
            doc_id: autosave::new_id(),
            last_input: Instant::now(),
            last_autosave: Instant::now(),
            editing: None,
            path_edit: None,
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
        let diagnostics = self.diagnostics.then(|| self.diagnostic_rows());
        let tab_infos = self.tab_infos();
        if self.home && self.recents_dirty {
            self.recents = recents::load();
            self.recents_dirty = false;
        }
        if self.home {
            recents::load_textures(&self.egui, &mut self.recents);
        }
        let hint = self.hint();
        let state = self.egui_state.as_mut()?;
        let input = state.take_egui_input(window);
        let system_dark = window.theme() != Some(winit::window::Theme::Light);
        let tokens = self.panels.theme.tokens(system_dark);
        if self.applied_theme != Some((self.panels.theme, system_dark)) {
            theme::apply(&self.egui, tokens);
            if self
                .applied_theme
                .is_some_and(|(t, _)| t != self.panels.theme)
            {
                settings::save_theme(self.panels.theme);
            }
            self.applied_theme = Some((self.panels.theme, system_dark));
        }
        let renderer = self.backend.as_ref().map(Backend::name).unwrap_or_default();
        let missing_owned = self.caches.text.missing_fonts();
        let missing: Vec<&str> = missing_owned.iter().map(String::as_str).collect();
        let families = self.caches.text.missing_families();
        let status = Status {
            title: &self.title,
            dirty: self.doc.revision() != self.saved_revision,
            zoom: self.camera.zoom,
            renderer: &renderer,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            tool: self.tool,
            missing_fonts: &missing,
            missing_families: &families,
            font_status: self.font_status.as_deref(),
            diagnostics: diagnostics.as_deref(),
            recents: &self.recents,
            home: self.home,
            tabs: &tab_infos,
            active_tab: if self.home { None } else { self.active_tab },
            hint,
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
                    let merge_key = if merge_key == 0 {
                        self.next_merge_key()
                    } else {
                        merge_key
                    };
                    let mut tx = Transaction::new("Edit").with_merge_key(merge_key);
                    match tx.apply(&mut self.doc, Op::Set { id, value }) {
                        Ok(()) => self.history.commit(&mut self.doc, tx),
                        Err(e) => eprintln!("edit failed: {e}"),
                    }
                }
                Action::Tool(tool) => {
                    self.end_path_edit();
                    self.tool = tool;
                }
                Action::Open => self.open_dialog(),
                Action::Save => self.save(),
                Action::Export => self.export_dialog(),
                Action::DownloadFont(family) => self.download_font(family),
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
                Action::ZoomIn => self.zoom_center(2.0),
                Action::ZoomOut => self.zoom_center(0.5),
                Action::Zoom100 => self.zoom_100(),
                Action::ToggleDiagnostics => self.diagnostics = !self.diagnostics,
                Action::Rename(id, name) => self.rename(id, &name),
                Action::New => self.new_document(),
                Action::Import => self.import_dialog(),
                Action::OpenRecent(path) => self.open_path(path),
                Action::SwitchTab(i) => self.switch_tab(i),
                Action::CloseTab(i) => {
                    self.close_tab(i);
                }
                Action::RemoveRecent(path) => {
                    recents::remove(&path);
                    self.recents_dirty = true;
                }
                Action::Home => self.go_home(),
                Action::SaveAs => self.save_as(),
                Action::ExportWith(o) => self.export_with(o),
                Action::SelectPage(p) => self.select_page(p),
                Action::AddPage => self.add_page(),
                Action::Edit(e) => self.edit(e),
                Action::Align(how) => self.align(how),
                Action::AddComment(text) => self.add_comment(text),
                Action::ReplyComment(i, text) => {
                    if let Some(c) = self.doc.comments.get_mut(i) {
                        c.replies.push(ude_core::comments::Reply {
                            author: user_name(),
                            text,
                            created: ude_core::comments::now(),
                        });
                        self.doc.mark_changed();
                    }
                }
                Action::ResolveComment(i, resolved) => {
                    if let Some(c) = self.doc.comments.get_mut(i) {
                        c.resolved = resolved;
                        self.doc.mark_changed();
                    }
                }
                Action::DeleteComment(i) => {
                    if i < self.doc.comments.len() {
                        self.doc.comments.remove(i);
                        self.doc.mark_changed();
                    }
                }
                Action::FocusComment(i) => self.focus_comment(i),
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
        if self.path_editing() {
            // Points, not the resize box, are what can be dragged.
            overlay.selection_box = None;
            overlay.selected.clear();
            overlay.hover = None;
        }
        self.selection_box = overlay.selection_box.map(|o| o.quad);
        let po = self.path_overlay(view.transform);
        overlay.path = po.path;
        overlay.path_anchors = po.anchors;
        overlay.path_handles = po.handles;
        overlay.path_rubber = po.rubber;
        overlay.path_closing = po.closing;
        let [ar, ag, ab, _] = self.panels.theme.tokens(system_dark).accent.to_array();
        overlay.accent = Some(Color::from_rgba8(ar, ag, ab, 255));
        let (caret, text_selection) = self.text_overlay(view.transform);
        overlay.caret = caret;
        overlay.text_selection = text_selection;
        let page_index = self.page_index();
        overlay.pins = self
            .doc
            .comments
            .iter()
            .filter(|c| c.page == page_index && (!c.resolved || self.panels.comments_open))
            .map(|c| {
                (
                    view.transform * Point::new(f64::from(c.x), f64::from(c.y)),
                    c.resolved,
                )
            })
            .collect();
        let dirty = self.doc.revision() != self.saved_revision;
        if dirty != self.shown_dirty && !self.options.stats {
            self.shown_dirty = dirty;
            self.update_title();
        }
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
            &mut self.caches,
        ) {
            Ok(FrameResult::Presented(stats)) => {
                self.last_frame = Some((started.elapsed().as_secs_f64() * 1000.0, stats));
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
        if self.editing.is_some() && self.text_key(event) {
            return;
        }
        if self.path_edit.is_some() && self.path_key(event) {
            return;
        }
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
        let alt = self.modifiers.alt_key();
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
        if !ctrl
            && !alt
            && let Some(tool) = tools::tool_for_key(code)
        {
            self.end_path_edit();
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
            KeyCode::KeyO if ctrl => self.open_dialog(),
            KeyCode::KeyN if ctrl => self.new_document(),
            KeyCode::Enter | KeyCode::NumpadEnter
                if matches!(
                    self.selection.ids(),
                    [id] if matches!(self.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Vector(_)))
                ) =>
            {
                let id = self.selection.ids()[0];
                self.begin_path_edit(id);
            }
            KeyCode::Enter | KeyCode::NumpadEnter
                if matches!(
                    self.selection.ids(),
                    [id] if matches!(self.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Text(_)))
                ) =>
            {
                let id = self.selection.ids()[0];
                self.begin_text_edit(id, true);
            }
            KeyCode::KeyW if ctrl => {
                if let Some(i) = self.active_tab.filter(|_| !self.home) {
                    self.close_tab(i);
                }
            }
            KeyCode::Tab if ctrl && !self.tabs.is_empty() => {
                let n = self.tabs.len();
                let cur = self.active_tab.unwrap_or(0);
                self.switch_tab(if shift {
                    (cur + n - 1) % n
                } else {
                    (cur + 1) % n
                });
            }
            KeyCode::KeyA if alt => self.align(ude_editor::Align::Left),
            KeyCode::KeyD if alt => self.align(ude_editor::Align::Right),
            KeyCode::KeyH if alt && shift => self.align(ude_editor::Align::DistributeH),
            KeyCode::KeyH if alt => self.align(ude_editor::Align::HCenter),
            KeyCode::KeyW if alt => self.align(ude_editor::Align::Top),
            KeyCode::KeyS if alt => self.align(ude_editor::Align::Bottom),
            KeyCode::KeyV if alt && shift => self.align(ude_editor::Align::DistributeV),
            KeyCode::KeyV if alt => self.align(ude_editor::Align::VCenter),
            KeyCode::KeyC if ctrl => self.copy(),
            KeyCode::KeyX if ctrl => {
                self.copy();
                self.delete_selection();
            }
            KeyCode::KeyV if ctrl => self.paste(),
            KeyCode::KeyD if ctrl => self.structural("duplicate"),
            KeyCode::KeyU if ctrl && alt => self.boolean(BoolOpKind::Union),
            KeyCode::KeyS if ctrl && alt => self.boolean(BoolOpKind::Subtract),
            KeyCode::KeyI if ctrl && alt => self.boolean(BoolOpKind::Intersect),
            KeyCode::KeyX if ctrl && alt => self.boolean(BoolOpKind::Exclude),
            KeyCode::KeyE if ctrl => self.structural("flatten"),
            KeyCode::KeyG if ctrl && shift => self.structural("ungroup"),
            KeyCode::KeyG if ctrl => self.structural("group"),
            KeyCode::BracketRight if ctrl => self.arrange(if shift {
                Arrange::ToFront
            } else {
                Arrange::Forward
            }),
            KeyCode::BracketLeft if ctrl => self.arrange(if shift {
                Arrange::ToBack
            } else {
                Arrange::Backward
            }),
            KeyCode::KeyS if ctrl && shift => self.save_as(),
            KeyCode::KeyS if ctrl => self.save(),
            KeyCode::KeyE if ctrl && shift => self.export_dialog(),
            KeyCode::KeyZ if ctrl && shift => self.redo(),
            KeyCode::KeyZ if ctrl => self.undo(),
            KeyCode::KeyY if ctrl => self.redo(),
            KeyCode::Delete | KeyCode::Backspace if !self.selection.is_empty() => {
                self.delete_selection();
            }
            KeyCode::Digit1 if shift => self.zoom_to_fit(),
            KeyCode::Digit0 if shift || ctrl => self.zoom_100(),
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
            KeyCode::F12 => {
                self.diagnostics = !self.diagnostics;
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
        if self.editing.is_some() && self.text_press() {
            return;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| {
            now.duration_since(t) < DOUBLE_CLICK && (p - self.cursor).hypot() < 4.0
        });
        self.last_click = Some((now, self.cursor));
        if (self.tool == Tool::Pen || self.path_edit.is_some()) && self.path_press(double) {
            return;
        }
        if self.tool != Tool::Move {
            return self.begin_create();
        }
        if let Some(pick) = self.pick_handle(self.cursor) {
            self.begin_gesture(Some(pick), self.cursor);
            self.redraw();
            return;
        }

        // Double-click on a selected text layer edits it on the canvas.
        if double
            && let [current] = self.selection.ids()
            && self.target_at(self.cursor, SelectionMode::Deep) == Some(*current)
            && matches!(
                self.doc.node(*current).map(|n| &n.kind),
                Some(NodeKind::Text(_))
            )
        {
            let id = *current;
            self.begin_text_edit(id, false);
            return;
        }
        // Double-click on a selected vector edits its points.
        if double
            && let [current] = self.selection.ids()
            && self.target_at(self.cursor, SelectionMode::Deep) == Some(*current)
            && matches!(
                self.doc.node(*current).map(|n| &n.kind),
                Some(NodeKind::Vector(_))
            )
        {
            let id = *current;
            self.begin_path_edit(id);
            return;
        }
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
        self.text_release();
        self.path_release();
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
        if self.text_drag() {
            return;
        }
        if self.panning {
            self.camera.pan += delta;
            self.redraw();
            return;
        }
        if self.path_drag() {
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

impl ApplicationHandler<AppEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let dirty = !self.home && self.doc.revision() != self.saved_revision;
        match autosave::due(dirty, self.last_autosave, self.last_input) {
            Some(at) if Instant::now() >= at && self.drag.is_none() => {
                self.last_autosave = Instant::now();
                if let Err(e) = autosave::write(
                    self.doc_id,
                    &self.doc,
                    &self.title,
                    self.options.file.as_deref(),
                ) {
                    eprintln!("autosave failed: {e}");
                }
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            }
            // Never wait for a time already past (that would spin): defer during a gesture.
            Some(at) => event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                at.max(Instant::now() + autosave::IDLE),
            )),
            None => event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::FontDownloaded { family, result } => self.font_downloaded(family, result),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("{} — KuArt", self.title))
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_visible(false)
            .with_window_icon(window_icon());
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
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        let mut egui_consumed = false;
        // egui-winit asks for a repaint on RedrawRequested itself, which would redraw forever.
        // While text is edited on the canvas, keystrokes belong to it, not to egui.
        let canvas_typing = self.editing.is_some()
            && matches!(
                event,
                WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_)
            );
        if let (Some(state), Some(window)) = (self.egui_state.as_mut(), self.window.as_ref())
            && !matches!(event, WindowEvent::RedrawRequested)
            && !canvas_typing
        {
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
            WindowEvent::DroppedFile(path) => self.dropped(path),
            WindowEvent::Ime(winit::event::Ime::Commit(text)) if !egui_consumed => {
                if self.editing.is_some() {
                    self.text_insert(&text);
                }
            }
            WindowEvent::CloseRequested => {
                if self.close_all() {
                    event_loop.exit();
                }
            }
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
    let event_loop = match EventLoop::<AppEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("error: cannot start the event loop: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let home = options.file.is_none() && options.demo.is_none();
    let mut app = App::new(options, doc, title);
    app.panels.theme = settings::load_theme();
    app.home = home;
    if !home {
        app.tabs.push(None);
        app.active_tab = Some(0);
    }
    app.recover();
    app.proxy = Some(event_loop.create_proxy());
    app.saved_revision = app.doc.revision();
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

/// App icon (generated from assets/icon.png by build.rs).
fn window_icon() -> Option<winit::window::Icon> {
    const SIZE: u32 = 64;
    let rgba = include_bytes!(concat!(env!("OUT_DIR"), "/window_icon.rgba"));
    winit::window::Icon::from_rgba(rgba.to_vec(), SIZE, SIZE).ok()
}

/// Results of background work, delivered to the event loop.
pub(crate) enum AppEvent {
    FontDownloaded {
        family: String,
        result: Result<Vec<PathBuf>, String>,
    },
}

impl App {
    /// Starts a Google Fonts download for `family` on a background thread.
    fn download_font(&mut self, family: String) {
        let (Some(proxy), Some(dir)) = (self.proxy.clone(), ude_core::paths::user_fonts_dir())
        else {
            self.font_status = Some("No user fonts folder on this system".into());
            return;
        };
        self.font_status = Some(format!("Downloading {family} from Google Fonts…"));
        std::thread::spawn(move || {
            let result = google_fonts::download(&family, &dir);
            let _ = proxy.send_event(AppEvent::FontDownloaded { family, result });
        });
        self.redraw();
    }

    fn font_downloaded(&mut self, family: String, result: Result<Vec<PathBuf>, String>) {
        self.font_status = Some(match result {
            Ok(paths) => {
                let faces: usize = paths
                    .iter()
                    .filter_map(|p| std::fs::read(p).ok())
                    .map(|b| self.caches.text.add_font_data(b))
                    .sum();
                // Text re-shapes with the new font: drop the cached frame.
                if let Some(b) = &mut self.backend {
                    b.invalidate();
                }
                format!("Installed {family} ({faces} styles)")
            }
            Err(e) => e,
        });
        self.redraw();
    }
}

impl App {
    /// Rows for the diagnostics window: OS memory counters next to our own accounting.
    fn diagnostic_rows(&self) -> Vec<(&'static str, String)> {
        let mb = |b: usize| format!("{:.1} MB", b as f64 / 1_048_576.0);
        let mut rows = Vec::new();
        match ude_diagnostics::process_memory() {
            Some(m) => {
                rows.push(("Private memory", format!("{:.1} MB", m.private_mb())));
                rows.push(("Working set", format!("{:.1} MB", m.resident_mb())));
            }
            None => rows.push(("Process memory", "unavailable".into())),
        }
        let nodes = self.doc.descendants(self.doc.root()).count();
        rows.push((
            "Document",
            format!("{nodes} nodes · {}", mb(self.doc.memory_report().total())),
        ));
        rows.push(("Assets", format!("{}", self.doc.assets.len())));
        rows.push((
            "Image cache",
            format!(
                "{} / {}",
                mb(self.caches.images.used_bytes()),
                mb(self.caches.images.budget())
            ),
        ));
        rows.push((
            "Text layouts",
            format!("{} cached", self.caches.text.cached_layouts()),
        ));
        rows.push((
            "Undo history",
            format!(
                "{} steps · {}",
                self.history.len(),
                mb(self.history.bytes())
            ),
        ));
        if let Some(index) = &self.index {
            rows.push((
                "Spatial index",
                format!("{} nodes · {}", index.len(), mb(index.heap_bytes())),
            ));
        }
        let renderer = self.backend.as_ref().map(Backend::name).unwrap_or_default();
        rows.push(("Renderer", renderer.to_string()));
        if let Some((ms, s)) = self.last_frame {
            rows.push(("Last frame", format!("{ms:.1} ms")));
            rows.push((
                "Traversal",
                format!(
                    "{} drawn · {} visited · {} culled · {} layers",
                    s.drawn, s.visited, s.culled, s.layers
                ),
            ));
        }
        rows
    }
}

impl App {
    fn delete_selection(&mut self) {
        let ids = self.selection.ids().to_vec();
        match transform::delete(&mut self.doc, &mut self.history, &ids) {
            Ok(()) => self.selection.clear(),
            Err(e) => eprintln!("delete failed: {e}"),
        }
        self.hover = None;
        self.redraw();
    }

    /// Copies the selection in-app and, as SVG markup, to the system clipboard.
    fn copy(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.clipboard = ude_editor::commands::copy(&self.doc, self.selection.ids());
        if let [id] = self.selection.ids()
            && let Ok(svg) = ude_format_svg::export(&self.doc, *id)
        {
            self.egui.copy_text(svg);
        }
    }

    /// Pastes on top of the current page.
    fn paste(&mut self) {
        let Some(page) = self.page else { return };
        if self.clipboard.is_empty() {
            return;
        }
        match ude_editor::commands::paste(&mut self.doc, &mut self.history, &self.clipboard, page) {
            Ok(ids) => self.selection.set(ids),
            Err(e) => eprintln!("paste failed: {e}"),
        }
        self.redraw();
    }

    /// Duplicate, group or ungroup the selection; the result becomes the selection.
    fn structural(&mut self, what: &str) {
        let ids = self.selection.ids().to_vec();
        if ids.is_empty() {
            return;
        }
        use ude_editor::commands as c;
        let (doc, history) = (&mut self.doc, &mut self.history);
        let result = match what {
            "duplicate" => c::duplicate(doc, history, &ids),
            "group" => c::group(doc, history, &ids).map(|g| g.into_iter().collect()),
            "flatten" => c::flatten(doc, history, &ids),
            _ => c::ungroup(doc, history, &ids),
        };
        match result {
            Ok(new) if !new.is_empty() => self.selection.set(new),
            Ok(_) => {}
            Err(e) => eprintln!("{what} failed: {e}"),
        }
        self.redraw();
    }

    fn boolean(&mut self, kind: BoolOpKind) {
        let ids = self.selection.ids().to_vec();
        match ude_editor::commands::boolean(&mut self.doc, &mut self.history, &ids, kind) {
            Ok(Some(id)) => self.selection.set([id]),
            Ok(None) => {}
            Err(e) => eprintln!("boolean failed: {e}"),
        }
        self.redraw();
    }

    fn arrange(&mut self, how: Arrange) {
        let ids = self.selection.ids().to_vec();
        if let Err(e) = ude_editor::commands::arrange(&mut self.doc, &mut self.history, &ids, how) {
            eprintln!("arrange failed: {e}");
        }
        self.redraw();
    }
}

impl App {
    fn zoom_100(&mut self) {
        let (w, h) = self.viewport();
        self.camera
            .set_zoom_at(Point::new(w / 2.0, h / 2.0), 1.0, self.scale_factor());
        self.redraw();
    }

    fn edit(&mut self, e: ude_ui::Edit) {
        use ude_ui::Edit as E;
        match e {
            E::Copy => self.copy(),
            E::Cut => {
                self.copy();
                self.delete_selection();
            }
            E::Paste => self.paste(),
            E::Duplicate => self.structural("duplicate"),
            E::Delete => self.delete_selection(),
            E::Group => self.structural("group"),
            E::Ungroup => self.structural("ungroup"),
            E::Union => self.boolean(BoolOpKind::Union),
            E::Subtract => self.boolean(BoolOpKind::Subtract),
            E::Intersect => self.boolean(BoolOpKind::Intersect),
            E::Exclude => self.boolean(BoolOpKind::Exclude),
            E::Flatten => self.structural("flatten"),
            E::BringForward => self.arrange(Arrange::Forward),
            E::SendBackward => self.arrange(Arrange::Backward),
            E::SelectAll => {
                if let Some(page) = self.page {
                    let all = self.doc.children(page).to_vec();
                    self.selection.set(all);
                }
            }
        }
    }
}

/// Name for new comments: the OS user name.
fn user_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "You".into())
}

impl App {
    /// Index of the current page among the document's pages (comments are keyed by it).
    fn page_index(&self) -> u32 {
        self.page
            .and_then(|p| {
                self.doc
                    .children(self.doc.root())
                    .iter()
                    .position(|&c| c == p)
            })
            .unwrap_or(0) as u32
    }

    /// Pins a new comment at the centre of the visible canvas.
    fn add_comment(&mut self, text: String) {
        let center = self.doc_point(self.canvas.center());
        self.doc.comments.push(ude_core::comments::Comment {
            page: self.page_index(),
            x: center.x as f32,
            y: center.y as f32,
            author: user_name(),
            text,
            created: ude_core::comments::now(),
            ..Default::default()
        });
        self.doc.mark_changed();
        self.redraw();
    }

    /// Centres the view on comment `i` (switching to its page).
    fn focus_comment(&mut self, i: usize) {
        let Some(c) = self.doc.comments.get(i) else {
            return;
        };
        let (page, at) = (c.page as usize, Point::new(f64::from(c.x), f64::from(c.y)));
        if let Some(&p) = self.doc.children(self.doc.root()).get(page)
            && Some(p) != self.page
        {
            self.select_page(p);
            self.fitted = true;
        }
        let s = self.scale_factor();
        let target = self.canvas.center();
        self.camera.pan = target.to_vec2() - at.to_vec2() * (self.camera.zoom * s);
        self.redraw();
    }
}

impl App {
    fn align(&mut self, how: ude_editor::Align) {
        let ids = self.selection.ids().to_vec();
        if let Err(e) = ude_editor::commands::align(&mut self.doc, &mut self.history, &ids, how) {
            eprintln!("align failed: {e}");
        }
        self.redraw();
    }
}

impl App {
    /// Offers to reopen documents an earlier run did not save or close cleanly.
    fn recover(&mut self) {
        let left = autosave::leftovers();
        if left.is_empty() {
            return;
        }
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("KuArt")
            .set_description(format!(
                "KuArt did not close properly. Recover {} unsaved document{}?",
                left.len(),
                if left.len() == 1 { "" } else { "s" }
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        for l in left {
            if answer == rfd::MessageDialogResult::Yes
                && let Ok(doc) = ude_format_native::load_from_path(
                    &l.file,
                    &ude_format_native::Limits::default(),
                )
            {
                let native = l.original.as_ref().filter(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        ude_format_native::is_native_extension(&e.to_ascii_lowercase())
                    })
                });
                self.open_in_new_tab(doc, native.cloned());
                self.title = format!("{} (recovered)", l.title);
                // Unsaved until the user saves it.
                self.doc.mark_changed();
                self.update_title();
            }
            autosave::remove(l.id);
        }
    }

    /// A file dropped on the window: images go onto the canvas, anything else opens.
    fn dropped(&mut self, path: std::path::PathBuf) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !self.home && matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            if let Err(e) = self.place_image(&path) {
                eprintln!("{}: {e}", path.display());
            }
        } else {
            self.open_path(path);
        }
        self.redraw();
    }

    /// Adds an image as a new layer at the pointer, at its natural size.
    fn place_image(&mut self, path: &std::path::Path) -> Result<(), String> {
        use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
        let page = self.page.ok_or("no page")?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let codec = match reader.format() {
            Some(image::ImageFormat::Png) => Codec::Png,
            Some(image::ImageFormat::Jpeg) => Codec::Jpeg,
            Some(image::ImageFormat::WebP) => Codec::WebP,
            _ => return Err("unsupported image".into()),
        };
        let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
        let asset = self.doc.assets.insert(AssetEntry {
            hash: ContentHash(*blake3::hash(&bytes).as_bytes()),
            kind: AssetKind::Image {
                width: w,
                height: h,
            },
            source: AssetSource::Memory {
                bytes: bytes.into(),
                codec,
            },
        });
        let at = self.doc_point(self.cursor);
        let name = self.doc.strings.intern(&files::file_name(path));
        let node = ude_core::Node::new(NodeKind::Raster { asset }, name)
            .with_transform(ude_core::Affine::translate(at.x as f32, at.y as f32))
            .with_size(w as f32, h as f32);
        let id = self.doc.create(node);
        let index = self.doc.children(page).len() as u32;
        let mut tx = Transaction::new("Place image");
        tx.apply(
            &mut self.doc,
            Op::Attach {
                id,
                parent: page,
                index,
            },
        )
        .map_err(|e| e.to_string())?;
        self.history.commit(&mut self.doc, tx);
        self.selection.set([id]);
        Ok(())
    }
}
