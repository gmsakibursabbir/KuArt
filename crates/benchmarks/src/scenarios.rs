//! M13 scenarios (benchmark-plan.md §4–5), headless on the CPU renderer at 1280×800:
//! open (load + index + first frame), RAM after open and after close (OS counters), pan and
//! drag-50 frame times (p50/p95), and undo of a 500-node move. Results are compared with the
//! §5 targets and written to `target/bench/<git-sha>/<machine>.json`.

use std::time::Instant;

use ude_bench::{SynthSpec, generate};
use ude_core::{Affine, Document, History, NodeId, Op, Prop, Transaction};
use ude_renderer::vello::kurbo;
use ude_renderer::{CpuPainter, RenderCaches, View, paint_page, vello_cpu};

const W: u16 = 1280;
const H: u16 = 800;
const PAN_FRAMES: usize = 120;
const DRAG_STEPS: usize = 60;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn private_mb() -> f64 {
    ude_diagnostics::process_memory().map_or(f64::NAN, |m| m.private_mb())
}

struct Canvas {
    cx: vello_cpu::RenderContext,
    resources: vello_cpu::Resources,
    pixmap: vello_cpu::Pixmap,
}

impl Canvas {
    fn new() -> Self {
        Self {
            cx: vello_cpu::RenderContext::new(W, H),
            resources: vello_cpu::Resources::new(),
            pixmap: vello_cpu::Pixmap::new(W, H),
        }
    }

    /// One full frame (traversal, encoding, rasterisation). Returns milliseconds.
    fn frame(
        &mut self,
        doc: &Document,
        page: NodeId,
        view: &View,
        caches: &mut RenderCaches,
    ) -> f64 {
        let t = Instant::now();
        self.cx.reset();
        paint_page(
            doc,
            page,
            view,
            &mut CpuPainter {
                cx: &mut self.cx,
                resources: &mut self.resources,
            },
            caches,
        );
        self.cx.flush();
        self.cx.render(&mut self.pixmap, &mut self.resources);
        ms(t)
    }
}

fn view(transform: kurbo::Affine) -> View {
    View {
        transform,
        width: f64::from(W),
        height: f64::from(H),
    }
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(i).copied().unwrap_or(f64::NAN)
}

pub struct Scenario {
    pub nodes: u32,
    pub open_ms: f64,
    pub ram_after_open_mb: f64,
    pub pan_p50: f64,
    pub pan_p95: f64,
    pub drag50_p95: f64,
    pub undo500_ms: f64,
    pub ram_after_close_delta_mb: f64,
}

pub fn run(nodes: u32, baseline_mb: f64) -> Scenario {
    let bytes = ude_format_native::save(&generate(SynthSpec { nodes, seed: 42 })).expect("save");
    let mut canvas = Canvas::new();

    // Open: parse, build the spatial index, render the first full frame (fit to page).
    let t = Instant::now();
    let mut doc =
        ude_format_native::load(&bytes, &ude_format_native::Limits::default()).expect("load");
    let page = doc.children(doc.root())[0];
    let mut index = ude_spatial::SpatialIndex::build(&doc, page);
    let mut caches = RenderCaches::default();
    let bounds = ude_renderer::export_bounds(&doc, page).expect("page bounds");
    let fit = (f64::from(W) / bounds.width()).min(f64::from(H) / bounds.height());
    let fit_view =
        view(kurbo::Affine::scale(fit) * kurbo::Affine::translate((-bounds.x0, -bounds.y0)));
    canvas.frame(&doc, page, &fit_view, &mut caches);
    let open_ms = ms(t);
    let ram_after_open_mb = private_mb();

    // Pan at 100% across the first artboards, 8 px per frame.
    let mut pan: Vec<f64> = (0..PAN_FRAMES)
        .map(|i| {
            let v = view(kurbo::Affine::translate((
                -bounds.x0 - 8.0 * i as f64,
                -bounds.y0,
            )));
            canvas.frame(&doc, page, &v, &mut caches)
        })
        .collect();
    let at_100 = view(kurbo::Affine::translate((-bounds.x0, -bounds.y0)));

    // Drag 50 visible leaf nodes: one merged transaction per step, index update, frame.
    let targets: Vec<NodeId> = index
        .query_rect(ude_core::Rect::new(
            bounds.x0 as f32,
            bounds.y0 as f32,
            bounds.x0 as f32 + f32::from(W),
            bounds.y0 as f32 + f32::from(H),
        ))
        .filter(|&id| doc.node(id).is_some_and(|n| !n.kind.is_container()))
        .take(50)
        .collect();
    let mut history = History::default();
    let mut drag: Vec<f64> = (0..DRAG_STEPS)
        .map(|_| {
            let t = Instant::now();
            let mut tx = Transaction::new("Move").with_merge_key(1);
            for &id in &targets {
                let moved = Affine::translate(2.0, 1.0)
                    * doc.node(id).map_or(Affine::IDENTITY, |n| n.transform);
                tx.apply(
                    &mut doc,
                    Op::Set {
                        id,
                        value: Prop::Transform(moved),
                    },
                )
                .expect("move");
            }
            history.commit(&mut doc, tx);
            let changes = doc.take_changes();
            index.update(&doc, &changes);
            canvas.frame(&doc, page, &at_100, &mut caches);
            ms(t)
        })
        .collect();

    // Undo of a 500-node move.
    let many: Vec<NodeId> = doc
        .descendants(doc.root())
        .filter(|&id| doc.node(id).is_some_and(|n| !n.kind.is_container()))
        .take(500)
        .collect();
    let mut tx = Transaction::new("Move 500");
    for &id in &many {
        let moved =
            Affine::translate(10.0, 10.0) * doc.node(id).map_or(Affine::IDENTITY, |n| n.transform);
        tx.apply(
            &mut doc,
            Op::Set {
                id,
                value: Prop::Transform(moved),
            },
        )
        .expect("move");
    }
    history.commit(&mut doc, tx);
    let t = Instant::now();
    history.undo(&mut doc).expect("undo");
    let undo500_ms = ms(t);

    // Close: drop everything the document owned.
    drop((doc, index, caches, history, canvas, bytes));
    let ram_after_close_delta_mb = private_mb() - baseline_mb;

    Scenario {
        nodes,
        open_ms,
        ram_after_open_mb,
        pan_p50: percentile(&mut pan.clone(), 0.5),
        pan_p95: percentile(&mut pan, 0.95),
        drag50_p95: percentile(&mut drag, 0.95),
        undo500_ms,
        ram_after_close_delta_mb,
    }
}

/// §5 targets for a node count: (open s, RAM MB, pan p95 ms, drag p95 ms, undo ms).
fn targets(nodes: u32) -> (f64, f64, f64, f64, f64) {
    match nodes {
        0..=1_000 => (0.3, 150.0, 16.7, 16.7, 5.0),
        1_001..=10_000 => (1.0, 250.0, 16.7, 16.7, 5.0),
        _ => (5.0, 700.0, 33.0, 20.0, 10.0),
    }
}

fn mark(ok: bool) -> &'static str {
    if ok { "ok" } else { "MISS" }
}

pub fn main() {
    let baseline_mb = private_mb();
    println!(
        "\nM13 scenarios (headless CPU renderer, {W}x{H}; empty-process private {baseline_mb:.1} MB)"
    );
    println!(
        "{:>8} {:>13} {:>14} {:>16} {:>15} {:>12} {:>15}",
        "nodes",
        "open ms",
        "RAM open MB",
        "pan p50/p95 ms",
        "drag50 p95 ms",
        "undo500 ms",
        "close Δ MB"
    );
    let mut json_rows = Vec::new();
    for nodes in [1_000u32, 10_000, 100_000] {
        let s = run(nodes, baseline_mb);
        let (t_open, t_ram, t_pan, t_drag, t_undo) = targets(nodes);
        println!(
            "{:>8} {:>8.0} {:>4} {:>9.1} {:>4} {:>6.1}/{:>5.1} {:>4} {:>10.1} {:>4} {:>7.2} {:>4} {:>10.1} {:>4}",
            nodes,
            s.open_ms,
            mark(s.open_ms <= t_open * 1000.0),
            s.ram_after_open_mb,
            mark(s.ram_after_open_mb <= t_ram),
            s.pan_p50,
            s.pan_p95,
            mark(s.pan_p95 <= t_pan),
            s.drag50_p95,
            mark(s.drag50_p95 <= t_drag),
            s.undo500_ms,
            mark(s.undo500_ms <= t_undo),
            s.ram_after_close_delta_mb,
            mark(s.ram_after_close_delta_mb <= 15.0),
        );
        json_rows.push(format!(
            "{{\"nodes\":{},\"open_ms\":{:.2},\"ram_after_open_mb\":{:.1},\"pan_p50_ms\":{:.2},\"pan_p95_ms\":{:.2},\"drag50_p95_ms\":{:.2},\"undo500_ms\":{:.3},\"ram_after_close_delta_mb\":{:.1}}}",
            s.nodes, s.open_ms, s.ram_after_open_mb, s.pan_p50, s.pan_p95, s.drag50_p95, s.undo500_ms, s.ram_after_close_delta_mb
        ));
    }
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    let machine = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "machine".into());
    let dir = std::path::Path::new("target/bench").join(&sha);
    let json = format!(
        "{{\"sha\":\"{sha}\",\"machine\":\"{machine}\",\"os\":\"{}\",\"baseline_private_mb\":{baseline_mb:.1},\"scenarios\":[{}]}}\n",
        std::env::consts::OS,
        json_rows.join(",")
    );
    if std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(dir.join(format!("{machine}.json")), json).is_ok()
    {
        println!("wrote {}", dir.join(format!("{machine}.json")).display());
    }
}
