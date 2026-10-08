//! KuArt desktop application (`design-editor`).
//!
//! Canvas with pan/zoom. Event-driven: no frames are rendered while idle. The CPU renderer is
//! the default; `--gpu` (or `UDE_RENDERER=gpu`) opts into vello on wgpu, see docs/perf-log.md.
//!
//! Controls: wheel = pan (Shift = horizontal), Ctrl+wheel / pinch = zoom at cursor,
//! middle-drag or Space+drag = pan, Shift+1 = zoom to fit, Shift+0 / Ctrl+0 = 100%,
//! Ctrl+= / Ctrl+- = zoom, F3 = frame stats in the title bar.

mod backend;
mod camera;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use backend::{Backend, FrameResult};
use camera::Camera;
use ude_core::{Document, NodeId, NodeKind};
use ude_renderer::View;
use ude_renderer::vello::kurbo::{Point, Rect, Vec2};
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
}

impl App {
    fn new(options: Options, doc: Document, title: String) -> Self {
        let page = doc.children(doc.root()).iter().copied().find(|&id| {
            doc.node(id)
                .is_some_and(|n| matches!(n.kind, NodeKind::Page))
        });
        Self {
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

    fn zoom_to_fit(&mut self) {
        if let Some(bounds) = self.page_bounds() {
            let (w, h) = self.viewport();
            self.camera.fit(bounds, w, h, self.scale_factor());
        }
        self.redraw();
    }

    fn zoom_center(&mut self, factor: f64) {
        let (w, h) = self.viewport();
        self.camera
            .zoom_at(Point::new(w / 2.0, h / 2.0), factor, self.scale_factor());
        self.redraw();
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(window), Some(page)) = (self.window.clone(), self.page) else {
            return;
        };
        if !self.fitted {
            self.fitted = true;
            self.zoom_to_fit();
        }
        let (width, height) = self.viewport();
        let view = View {
            transform: self.camera.transform(window.scale_factor()),
            width,
            height,
        };
        let Some(backend) = self.backend.as_mut() else {
            return;
        };

        let started = Instant::now();
        match backend.render(&self.doc, page, &view) {
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
        match code {
            KeyCode::Digit1 if shift => self.zoom_to_fit(),
            KeyCode::Digit0 if shift || ctrl => {
                let (w, h) = self.viewport();
                self.camera
                    .set_zoom_at(Point::new(w / 2.0, h / 2.0), 1.0, self.scale_factor());
                self.redraw();
            }
            KeyCode::Equal | KeyCode::NumpadAdd if ctrl => self.zoom_center(2.0),
            KeyCode::Minus | KeyCode::NumpadSubtract if ctrl => self.zoom_center(0.5),
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

    fn update_cursor(&self) {
        if let Some(w) = &self.window {
            w.set_cursor(match (self.panning, self.space_down) {
                (true, _) => CursorIcon::Grabbing,
                (false, true) => CursorIcon::Grab,
                _ => CursorIcon::Default,
            });
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(b) = &mut self.backend {
                    b.resize(size.width, size.height);
                }
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.redraw(),
            WindowEvent::RedrawRequested => self.render(event_loop),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
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
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = Point::new(position.x, position.y);
                if self.panning {
                    self.camera.pan += p - self.cursor;
                    self.redraw();
                }
                self.cursor = p;
            }
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
