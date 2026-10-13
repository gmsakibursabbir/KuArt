//! `snapshot <file.krt> <out.png> [--light] [--select] [--home] [--dev] [WIDTHxHEIGHT]`: renders the
//! editor window (panels, canvas fitted to the page) offscreen to a PNG, the same way the app
//! does on the CPU backend. For checking the interface without driving a real window.

use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use ude_format_native::{Limits, load_from_path};
use ude_renderer::vello::kurbo::Affine;
use ude_renderer::vello_cpu::{self, RasterizerSettings, TargetInit};
use ude_renderer::{CpuPainter, RenderCaches, View, export_bounds, paint_page};
use ude_ui::{Panels, Status, egui};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(out)) = (args.first(), args.get(1)) else {
        return Err(
            "usage: snapshot <file.krt> <out.png> [--light] [--select] [--home] [--dev] [WxH]"
                .into(),
        );
    };
    let flag = |f: &str| args.iter().any(|a| a == f);
    let (w, h) = args
        .iter()
        .find_map(|a| {
            a.split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        })
        .unwrap_or((1600u16, 1000u16));
    let doc = load_from_path(std::path::Path::new(input), &Limits::default())
        .map_err(|e| e.to_string())?;
    let page = doc.children(doc.root())[0];
    let tokens = if flag("--light") {
        &ude_ui::theme::LIGHT
    } else {
        &ude_ui::theme::DARK
    };

    let ctx = egui::Context::default();
    ude_ui::theme::install_fonts(&ctx);
    ude_ui::theme::apply(&ctx, tokens);
    let mut caches = RenderCaches::default();
    let fonts = caches.text.family_names();
    let mut panels = Panels::default();
    panels.dev_mode = flag("--dev");
    let mut selection = ude_editor::Selection::default();
    if flag("--select")
        && let Some(&first) = doc.children(page).first()
    {
        selection.set([first]);
    }
    let status = Status {
        title: "snapshot",
        zoom: 1.0,
        renderer: "CPU renderer",
        can_undo: true,
        can_redo: false,
        tool: ude_editor::Tool::Move,
        missing_fonts: &[],
        missing_families: &[],
        font_status: None,
        diagnostics: None,
        dirty: false,
        recents: &[],
        home: flag("--home"),
        tabs: &[],
        active_tab: Some(0),
        hint: Some("Enter or double-click to edit points"),
        update: None,
        update_status: None,
        loading: None,
        fonts: &fonts,
    };
    let mut egui_sw = EguiSoftwareRender::new(ColorFieldOrder::Rgba);
    let mut buffer = vec![[0u8; 4]; usize::from(w) * usize::from(h)];
    let mut canvas = egui::Rect::NOTHING;
    // Two frames: the first builds fonts and layout, the second is the picture.
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(f32::from(w), f32::from(h)),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            canvas = panels
                .ui(ui, &doc, page, &selection, None, &status, tokens)
                .1;
        });
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        // Canvas: the page fitted into the canvas rect, drawn under the panels.
        let mut cx = vello_cpu::RenderContext::new(w, h);
        let mut resources = vello_cpu::Resources::new();
        if let Some(b) = export_bounds(&doc, page).filter(|_| canvas.is_positive()) {
            let scale = (f64::from(canvas.width()) / b.width())
                .min(f64::from(canvas.height()) / b.height())
                * 0.9;
            let view = View {
                transform: Affine::translate((
                    f64::from(canvas.center().x),
                    f64::from(canvas.center().y),
                )) * Affine::scale(scale)
                    * Affine::translate(-b.center().to_vec2()),
                width: f64::from(w),
                height: f64::from(h),
            };
            caches.images.set_blocking(true);
            paint_page(
                &doc,
                page,
                &view,
                &mut CpuPainter {
                    cx: &mut cx,
                    resources: &mut resources,
                    filters: false,
                },
                &mut caches,
            );
        }
        cx.flush();
        let mut pixmap = vello_cpu::Pixmap::new(w, h);
        let [r, g, b, _] = tokens.bg_canvas.to_array();
        let settings = RasterizerSettings {
            target_init: TargetInit::Clear(ude_renderer::vello::peniko::Color::from_rgba8(
                r, g, b, 255,
            )),
            ..Default::default()
        };
        cx.render_with(&mut pixmap, &mut resources, settings);
        for (dst, src) in buffer
            .iter_mut()
            .zip(pixmap.data_as_u8_slice().chunks_exact(4))
        {
            dst.copy_from_slice(src);
        }
        egui_sw.render(
            &mut BufferMutRef::new(&mut buffer, usize::from(w), usize::from(h)),
            &jobs,
            &output.textures_delta,
            output.pixels_per_point,
        );
    }
    let rgba: Vec<u8> = buffer.into_iter().flatten().collect();
    let png = ude_renderer::encode_rgba(
        u32::from(w),
        u32::from(h),
        rgba,
        ude_renderer::RasterFormat::Png,
        ude_renderer::Compression::default(),
    )?;
    std::fs::write(out, png).map_err(|e| e.to_string())
}
