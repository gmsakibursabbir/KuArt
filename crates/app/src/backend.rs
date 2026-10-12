//! Presentation backends: CPU (vello_cpu + softbuffer, the default) and GPU (vello on wgpu,
//! opt-in, falling back to the CPU on failure). The CPU backend caches the rendered document
//! so overlay-only changes (hover, selection, marquee) just recomposite.

use std::num::NonZeroU32;
use std::sync::Arc;

use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use ude_core::{Document, NodeId};
use ude_renderer::vello::peniko::Color;
use ude_renderer::vello::util::{RenderContext, RenderSurface};
use ude_renderer::vello::wgpu::{self, CurrentSurfaceTexture};
use ude_renderer::vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use ude_renderer::vello_cpu::{RasterizerSettings, TargetInit};
use ude_renderer::{
    CpuPainter, FrameStats, Overlay, RenderCaches, View, paint_overlay, paint_page, vello_cpu,
};
use winit::window::Window;

/// egui output to draw over the canvas (CPU backend only; see `Backend::render`).
pub struct UiFrame<'a> {
    pub jobs: &'a [ude_ui::egui::ClippedPrimitive],
    pub textures: &'a ude_ui::egui::TexturesDelta,
    pub pixels_per_point: f32,
}

pub enum Backend {
    Gpu(Box<Gpu>),
    Cpu(Box<Cpu>),
}

pub struct Gpu {
    context: RenderContext,
    surface: RenderSurface<'static>,
    renderer: Renderer,
    scene: Scene,
    adapter: String,
    /// The interface: drawn by egui's software renderer into `ui_pixels`, uploaded to
    /// `ui_texture` and alpha-blended over the canvas.
    egui: EguiSoftwareRender,
    ui_pixels: Vec<[u8; 4]>,
    ui_texture: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    ui_blitter: wgpu::util::TextureBlitter,
}

pub struct Cpu {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    cx: vello_cpu::RenderContext,
    /// Rendered document; reused while only overlays change (hover, selection, marquee).
    content: vello_cpu::Pixmap,
    content_key: Option<ContentKey>,
    content_stats: FrameStats,
    /// Content + overlays, the frame actually presented.
    frame: vello_cpu::Pixmap,
    overlay_cx: vello_cpu::RenderContext,
    /// Single-threaded context for pages with blur (filters need it); made on demand.
    filter_cx: Option<vello_cpu::RenderContext>,
    resources: vello_cpu::Resources,
    egui: EguiSoftwareRender,
}

/// Everything the rendered document depends on; any change re-renders the content.
#[derive(Clone, Copy, PartialEq)]
struct ContentKey {
    bg: Color,
    revision: u64,
    page: NodeId,
    transform: [f64; 6],
    size: (u16, u16),
}

pub enum FrameResult {
    Presented(FrameStats),
    /// Surface was out of date or occluded; try again on the next redraw.
    Skipped,
}

impl Backend {
    /// Tries the GPU first unless `force_cpu`; falls back to the CPU renderer on any failure.
    pub fn new(window: Arc<Window>, force_cpu: bool) -> Result<Self, String> {
        if !force_cpu {
            match Gpu::new(window.clone()) {
                Ok(gpu) => return Ok(Backend::Gpu(Box::new(gpu))),
                Err(e) => eprintln!("GPU renderer unavailable ({e}); using the CPU renderer"),
            }
        }
        Cpu::new(window).map(|c| Backend::Cpu(Box::new(c)))
    }

    pub fn name(&self) -> String {
        match self {
            Backend::Gpu(g) => format!("GPU renderer · {}", g.adapter),
            Backend::Cpu(_) => "CPU renderer".to_owned(),
        }
    }

    /// Drops cached frames (after opening another document).
    pub fn invalidate(&mut self) {
        if let Backend::Cpu(c) = self {
            c.content_key = None;
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        match self {
            Backend::Gpu(g) => g.context.resize_surface(&mut g.surface, w, h),
            Backend::Cpu(c) => {
                if let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) {
                    let _ = c.surface.resize(nw, nh);
                }
                let (cw, ch) = (clamp_u16(w), clamp_u16(h));
                c.cx = vello_cpu::RenderContext::new(cw, ch);
                c.overlay_cx = vello_cpu::RenderContext::new(cw, ch);
                c.filter_cx = None;
                c.content = vello_cpu::Pixmap::new(cw, ch);
                c.frame = vello_cpu::Pixmap::new(cw, ch);
                c.content_key = None;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        doc: &Document,
        page: NodeId,
        view: &View,
        overlay: &Overlay,
        bg: Color,
        ui: Option<&UiFrame<'_>>,
        caches: &mut RenderCaches,
    ) -> Result<FrameResult, String> {
        match self {
            // The GPU path has no egui renderer yet (egui-wgpu targets a different wgpu than vello).
            Backend::Gpu(g) => g.render(doc, page, view, overlay, bg, ui, caches),
            Backend::Cpu(c) => c.render(doc, page, view, overlay, bg, ui, caches),
        }
    }
}

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let mut context = RenderContext::new();
        let surface = pollster::block_on(context.create_surface(
            window,
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .map_err(|e| e.to_string())?;
        let device = &context.devices[surface.dev_id];
        let info = device.adapter().get_info();
        let renderer = Renderer::new(
            &device.device,
            RendererOptions {
                // Area AA only: fewer pipelines to compile and keep in memory.
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        // Software adapters (WARP, llvmpipe) are slower than our CPU renderer.
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(format!("{} is a software adapter", info.name));
        }
        // egui colours are premultiplied.
        let ui_blitter =
            wgpu::util::TextureBlitterBuilder::new(&device.device, surface.config.format)
                .blend_state(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                .build();
        Ok(Self {
            adapter: info.name.clone(),
            context,
            surface,
            renderer,
            scene: Scene::new(),
            egui: EguiSoftwareRender::new(ColorFieldOrder::Rgba),
            ui_pixels: Vec::new(),
            ui_texture: None,
            ui_blitter,
        })
    }

    /// Draws the interface into the UI texture (recreated when the window size changes).
    fn upload_ui(&mut self, ui: &UiFrame<'_>, width: u32, height: u32) {
        let device = &self.context.devices[self.surface.dev_id];
        let (w, h) = (width as usize, height as usize);
        self.ui_pixels.clear();
        self.ui_pixels.resize(w * h, [0, 0, 0, 0]);
        self.egui.render(
            &mut BufferMutRef::new(&mut self.ui_pixels, w, h),
            ui.jobs,
            ui.textures,
            ui.pixels_per_point,
        );
        if !matches!(&self.ui_texture, Some((_, _, tw, th)) if *tw == width && *th == height) {
            let texture = device.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ui"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.ui_texture = Some((texture, view, width, height));
        }
        if let Some((texture, ..)) = &self.ui_texture {
            device.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&self.ui_pixels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        doc: &Document,
        page: NodeId,
        view: &View,
        overlay: &Overlay,
        bg: Color,
        ui: Option<&UiFrame<'_>>,
        caches: &mut RenderCaches,
    ) -> Result<FrameResult, String> {
        let (width, height) = (self.surface.config.width, self.surface.config.height);
        if let Some(ui) = ui {
            self.upload_ui(ui, width, height);
        }
        self.scene.reset();
        let stats = paint_page(doc, page, view, &mut self.scene, caches);
        paint_overlay(overlay, &mut self.scene);
        let device = &self.context.devices[self.surface.dev_id];
        self.renderer
            .render_to_texture(
                &device.device,
                &device.queue,
                &self.scene,
                &self.surface.target_view,
                &RenderParams {
                    base_color: bg,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|e| e.to_string())?;
        let frame = match self.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) | CurrentSurfaceTexture::Suboptimal(t) => t,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.context.configure_surface(&self.surface);
                return Ok(FrameResult::Skipped);
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(FrameResult::Skipped);
            }
            CurrentSurfaceTexture::Validation => return Err("surface validation error".into()),
        };
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("blit"),
            });
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.surface.blitter.copy(
            &device.device,
            &mut encoder,
            &self.surface.target_view,
            &target,
        );
        if ui.is_some()
            && let Some((_, view, ..)) = &self.ui_texture
        {
            self.ui_blitter
                .copy(&device.device, &mut encoder, view, &target);
        }
        device.queue.submit([encoder.finish()]);
        device.queue.present(frame);
        Ok(FrameResult::Presented(stats))
    }
}

impl Cpu {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let mut surface = softbuffer::Surface::new(&context, window).map_err(|e| e.to_string())?;
        let (w, h) = (size.width.max(1), size.height.max(1));
        if let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) {
            surface.resize(nw, nh).map_err(|e| e.to_string())?;
        }
        let (cw, ch) = (clamp_u16(w), clamp_u16(h));
        Ok(Self {
            surface,
            cx: vello_cpu::RenderContext::new(cw, ch),
            content: vello_cpu::Pixmap::new(cw, ch),
            content_key: None,
            content_stats: FrameStats::default(),
            frame: vello_cpu::Pixmap::new(cw, ch),
            overlay_cx: vello_cpu::RenderContext::new(cw, ch),
            filter_cx: None,
            resources: vello_cpu::Resources::new(),
            egui: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        doc: &Document,
        page: NodeId,
        view: &View,
        overlay: &Overlay,
        bg: Color,
        ui: Option<&UiFrame<'_>>,
        caches: &mut RenderCaches,
    ) -> Result<FrameResult, String> {
        let key = ContentKey {
            bg,
            revision: doc.revision(),
            page,
            transform: view.transform.as_coeffs(),
            size: (self.cx.width(), self.cx.height()),
        };
        if self.content_key != Some(key) {
            let filters = caches.page_has_blur(doc, page);
            let (w, h) = (self.cx.width(), self.cx.height());
            let cx = if filters {
                self.filter_cx
                    .get_or_insert_with(|| ude_renderer::filter_context(w, h))
            } else {
                &mut self.cx
            };
            cx.reset();
            self.content_stats = paint_page(
                doc,
                page,
                view,
                &mut CpuPainter {
                    cx,
                    resources: &mut self.resources,
                    filters,
                },
                caches,
            );
            cx.flush();
            let settings = RasterizerSettings {
                target_init: TargetInit::Clear(bg),
                ..Default::default()
            };
            cx.render_with(&mut self.content, &mut self.resources, settings);
            self.content_key = Some(key);
        }

        // Overlays only: copy the cached content and composite the overlay over it.
        let shown = if overlay.is_empty() {
            &self.content
        } else {
            self.frame
                .data_as_u8_slice_mut()
                .copy_from_slice(self.content.data_as_u8_slice());
            self.overlay_cx.reset();
            paint_overlay(
                overlay,
                &mut CpuPainter {
                    cx: &mut self.overlay_cx,
                    resources: &mut self.resources,
                    filters: false,
                },
            );
            self.overlay_cx.flush();
            let settings = RasterizerSettings {
                target_init: TargetInit::SrcOver,
                ..Default::default()
            };
            self.overlay_cx
                .render_with(&mut self.frame, &mut self.resources, settings);
            &self.frame
        };

        let mut buffer = self.surface.buffer_mut().map_err(|e| e.to_string())?;
        // Opaque background, so premultiplied RGBA equals straight RGBA here.
        for (dst, px) in buffer
            .iter_mut()
            .zip(shown.data_as_u8_slice().chunks_exact(4))
        {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        if let Some(ui) = ui {
            let (w, h) = (usize::from(self.cx.width()), usize::from(self.cx.height()));
            let pixels: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buffer[..]);
            if pixels.len() == w * h {
                self.egui.render(
                    &mut BufferMutRef::new(pixels, w, h),
                    ui.jobs,
                    ui.textures,
                    ui.pixels_per_point,
                );
            }
        }
        buffer.present().map_err(|e| e.to_string())?;
        Ok(FrameResult::Presented(self.content_stats))
    }
}

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX).max(1)
}
