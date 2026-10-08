//! Presentation backends: GPU (vello on wgpu) with automatic CPU fallback
//! (vello_cpu + softbuffer) for adapters without compute shaders or broken drivers.

use std::num::NonZeroU32;
use std::sync::Arc;

use ude_core::{Document, NodeId};
use ude_renderer::vello::kurbo::{Affine, Rect};
use ude_renderer::vello::peniko::{Color, Fill};
use ude_renderer::vello::util::{RenderContext, RenderSurface};
use ude_renderer::vello::wgpu::{self, CurrentSurfaceTexture};
use ude_renderer::vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use ude_renderer::{BrushSpec, FrameStats, Geom, Painter, View, paint_page, vello_cpu};
use winit::window::Window;

/// `bg.canvas` from docs/ui-design-system.md.
pub const CANVAS_BG: Color = Color::from_rgba8(0x07, 0x09, 0x11, 0xFF);

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
}

pub struct Cpu {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    cx: vello_cpu::RenderContext,
    pixmap: vello_cpu::Pixmap,
    resources: vello_cpu::Resources,
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
            Backend::Gpu(g) => format!("GPU · {}", g.adapter),
            Backend::Cpu(_) => "CPU renderer".to_owned(),
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
                c.pixmap = vello_cpu::Pixmap::new(cw, ch);
            }
        }
    }

    pub fn render(
        &mut self,
        doc: &Document,
        page: NodeId,
        view: &View,
    ) -> Result<FrameResult, String> {
        match self {
            Backend::Gpu(g) => g.render(doc, page, view),
            Backend::Cpu(c) => c.render(doc, page, view),
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
        Ok(Self {
            adapter: format!("{} ({:?})", info.name, info.backend),
            context,
            surface,
            renderer,
            scene: Scene::new(),
        })
    }

    fn render(&mut self, doc: &Document, page: NodeId, view: &View) -> Result<FrameResult, String> {
        self.scene.reset();
        let stats = paint_page(doc, page, view, &mut self.scene);
        let device = &self.context.devices[self.surface.dev_id];
        let (width, height) = (self.surface.config.width, self.surface.config.height);
        self.renderer
            .render_to_texture(
                &device.device,
                &device.queue,
                &self.scene,
                &self.surface.target_view,
                &RenderParams {
                    base_color: CANVAS_BG,
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
            pixmap: vello_cpu::Pixmap::new(cw, ch),
            resources: vello_cpu::Resources::new(),
        })
    }

    fn render(&mut self, doc: &Document, page: NodeId, view: &View) -> Result<FrameResult, String> {
        let (w, h) = (self.cx.width(), self.cx.height());
        self.cx.reset();
        let bg = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
        self.cx.fill(
            Affine::IDENTITY,
            &Geom::Rect(bg),
            Fill::NonZero,
            &BrushSpec::Solid(CANVAS_BG),
            None,
        );
        let stats = paint_page(doc, page, view, &mut self.cx);
        self.cx.flush();
        self.cx.render(&mut self.pixmap, &mut self.resources);

        let mut buffer = self.surface.buffer_mut().map_err(|e| e.to_string())?;
        // Opaque background, so premultiplied RGBA equals straight RGBA here.
        for (dst, px) in buffer
            .iter_mut()
            .zip(self.pixmap.data_as_u8_slice().chunks_exact(4))
        {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        buffer.present().map_err(|e| e.to_string())?;
        Ok(FrameResult::Presented(stats))
    }
}

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX).max(1)
}
