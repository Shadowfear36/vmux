//! GPU renderer for a terminal pane using wgpu.
//!
//! Architecture:
//!   - Each pane has a wgpu Surface on its Win32 HWND child window.
//!   - Per-frame: snapshot terminal grid → bg quads → glyph quads via texture atlas.
//!   - Atlas: 2048×2048 Rgba8Unorm, glyphs rasterized with cosmic-text at white fg
//!     (only coverage/alpha matters; actual colour is applied per-vertex in the shader).

use wgpu::{
    BindGroup, Device, Instance, Queue, RenderPipeline, Sampler,
    Surface, SurfaceConfiguration, Texture,
};
use wgpu::util::DeviceExt;
use raw_window_handle::{
    RawWindowHandle, RawDisplayHandle,
    Win32WindowHandle, WindowsDisplayHandle,
};
use std::num::NonZeroIsize;
use std::collections::HashMap;
use anyhow::Result;

use super::font::FontManager;
use super::grid::{GridSnapshot, CellColor, CellFlags};
use crate::theme::Theme;
use alacritty_terminal::vte::ansi::NamedColor;

// ─── Vertex types ─────────────────────────────────────────────────────────────

/// Vertex for solid-color background quads.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BgVertex {
    position: [f32; 2],
    color:    [f32; 4],
}

/// Vertex for atlas-sampled glyph quads.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphVertex {
    position: [f32; 2],
    uv:       [f32; 2],
    color:    [f32; 4],  // fg colour
}

// ─── Glyph atlas ──────────────────────────────────────────────────────────────

#[derive(Hash, Eq, PartialEq, Clone)]
struct GlyphKey {
    ch:     char,
    bold:   bool,
    italic: bool,
}

#[derive(Clone)]
struct AtlasEntry {
    px: u32,   // pixel x in atlas
    py: u32,   // pixel y in atlas
    pw: u32,   // pixel width
    ph: u32,   // pixel height
}

// ─── GpuRenderer ──────────────────────────────────────────────────────────────

pub struct GpuRenderer {
    device:         Device,
    queue:          Queue,
    surface:        Surface<'static>,
    surface_config: SurfaceConfiguration,
    pub width:  u32,
    pub height: u32,

    bg_pipeline:    RenderPipeline,
    glyph_pipeline: RenderPipeline,
    glyph_bg:       BindGroup,
    atlas:          Texture,
    _atlas_sampler: Sampler,   // kept alive; referenced by glyph_bg
    atlas_size:     u32,
    atlas_x:        u32,       // shelf-packer cursor x
    atlas_y:        u32,       // shelf-packer cursor y (top of current row)
    atlas_row_h:    u32,       // tallest glyph seen in current row
    atlas_glyphs:   HashMap<GlyphKey, AtlasEntry>,

    pub font:  FontManager,
    pub theme: Theme,
    pub cursor_blink_on: bool,
}

impl GpuRenderer {
    // ── Construction ──────────────────────────────────────────────────────────

    pub async fn new(hwnd: isize, width: u32, height: u32, theme: Theme) -> Result<Self> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
            ..Default::default()
        });

        let surface = unsafe {
            let win32 = Win32WindowHandle::new(
                NonZeroIsize::new(hwnd).ok_or_else(|| anyhow::anyhow!("null HWND"))?,
            );
            let display = WindowsDisplayHandle::new();
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_window_handle:  RawWindowHandle::Win32(win32),
                raw_display_handle: RawDisplayHandle::Windows(display),
            })?
        };

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("no wgpu adapter found"))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label:              Some("vmux"),
                required_features:  wgpu::Features::empty(),
                required_limits:    wgpu::Limits::default(),
                ..Default::default()
            }, None)
            .await?;

        // Prefer non-sRGB to avoid implicit gamma conversion of theme colours.
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let surface_config = SurfaceConfiguration {
            usage:                           wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width:  width.max(1),
            height: height.max(1),
            present_mode:                    wgpu::PresentMode::Fifo,
            alpha_mode:                      wgpu::CompositeAlphaMode::Opaque,
            view_formats:                    vec![],
            desired_maximum_frame_latency:   2,
        };
        surface.configure(&device, &surface_config);

        // ── Atlas ─────────────────────────────────────────────────────────────

        let atlas_size = 2048u32;
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label:  Some("glyph_atlas"),
            size:   wgpu::Extent3d { width: atlas_size, height: atlas_size, depth_or_array_layers: 1 },
            mip_level_count:  1,
            sample_count:     1,
            dimension:        wgpu::TextureDimension::D2,
            format:           wgpu::TextureFormat::Rgba8Unorm,
            usage:            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:     &[],
        });
        let atlas_view    = atlas.create_view(&Default::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:            Some("atlas"),
            address_mode_u:   wgpu::AddressMode::ClampToEdge,
            address_mode_v:   wgpu::AddressMode::ClampToEdge,
            mag_filter:       wgpu::FilterMode::Nearest,
            min_filter:       wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // ── Bind group for glyph pipeline (atlas texture + sampler) ───────────

        let glyph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("glyph_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count:      None,
                },
            ],
        });

        let glyph_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("glyph_bg"),
            layout:  &glyph_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&atlas_sampler) },
            ],
        });

        // ── Background pipeline ───────────────────────────────────────────────

        let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("bg"),
            source: wgpu::ShaderSource::Wgsl(BG_SHADER.into()),
        });
        let bg_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[], push_constant_ranges: &[],
        });
        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("bg"),
            layout: Some(&bg_layout),
            vertex: wgpu::VertexState {
                module:     &bg_shader,
                entry_point: "vs_main",
                buffers:    &[wgpu::VertexBufferLayout {
                    array_stride:  std::mem::size_of::<BgVertex>() as u64,
                    step_mode:     wgpu::VertexStepMode::Vertex,
                    attributes:    &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &bg_shader,
                entry_point: "fs_main",
                targets:     &[Some(wgpu::ColorTargetState {
                    format, blend: None, write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:    wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:  wgpu::MultisampleState::default(),
            multiview:    None,
            cache:        None,
        });

        // ── Glyph pipeline ────────────────────────────────────────────────────

        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("glyph"),
            source: wgpu::ShaderSource::Wgsl(GLYPH_SHADER.into()),
        });
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[&glyph_bgl], push_constant_ranges: &[],
        });
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("glyph"),
            layout: Some(&glyph_layout),
            vertex: wgpu::VertexState {
                module:      &glyph_shader,
                entry_point: "vs_main",
                buffers:     &[wgpu::VertexBufferLayout {
                    array_stride:  std::mem::size_of::<GlyphVertex>() as u64,
                    step_mode:     wgpu::VertexStepMode::Vertex,
                    attributes:    &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &glyph_shader,
                entry_point: "fs_main",
                targets:     &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:    wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:  wgpu::MultisampleState::default(),
            multiview:    None,
            cache:        None,
        });

        let font = FontManager::new(14.0);

        Ok(GpuRenderer {
            device, queue, surface, surface_config, width, height,
            bg_pipeline, glyph_pipeline, glyph_bg,
            atlas, _atlas_sampler: atlas_sampler, atlas_size,
            atlas_x: 0, atlas_y: 0, atlas_row_h: 0,
            atlas_glyphs: HashMap::new(),
            font, theme,
            cursor_blink_on: true,
        })
    }

    pub fn toggle_cursor_blink(&mut self) {
        self.cursor_blink_on = !self.cursor_blink_on;
    }

    // ── Resize ────────────────────────────────────────────────────────────────

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.width  = width;
        self.height = height;
        self.surface_config.width  = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    // ── Glyph atlas ───────────────────────────────────────────────────────────

    /// Ensure a glyph is in the atlas. Returns the entry (cloned) on success.
    fn ensure_glyph(&mut self, ch: char, bold: bool, italic: bool) -> Option<AtlasEntry> {
        let key = GlyphKey { ch, bold, italic };
        if let Some(e) = self.atlas_glyphs.get(&key) {
            return Some(e.clone());
        }

        let cw = self.font.cell_width.ceil() as u32;
        let ch_h = self.font.cell_height.ceil() as u32;

        // Shelf packing: wrap to next row when needed.
        if self.atlas_x + cw > self.atlas_size {
            self.atlas_y    += self.atlas_row_h;
            self.atlas_x     = 0;
            self.atlas_row_h = 0;
        }
        if self.atlas_y + ch_h > self.atlas_size { return None; } // atlas full

        let ax = self.atlas_x;
        let ay = self.atlas_y;

        // Rasterize with white fg — coverage baked into the alpha channel.
        let pixels = self.font.rasterize_cell(&ch.to_string(), bold, italic, [255, 255, 255, 255]);

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture:    &self.atlas,
                mip_level:  0,
                origin:     wgpu::Origin3d { x: ax, y: ay, z: 0 },
                aspect:     wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::ImageDataLayout {
                offset:         0,
                bytes_per_row:  Some(cw * 4),
                rows_per_image: Some(ch_h),
            },
            wgpu::Extent3d { width: cw, height: ch_h, depth_or_array_layers: 1 },
        );

        self.atlas_x += cw;
        if ch_h > self.atlas_row_h { self.atlas_row_h = ch_h; }

        let entry = AtlasEntry { px: ax, py: ay, pw: cw, ph: ch_h };
        self.atlas_glyphs.insert(key, entry.clone());
        Some(entry)
    }

    // ── Render ────────────────────────────────────────────────────────────────

    pub fn render(&mut self, snapshot: &GridSnapshot) -> Result<()> {
        let cw = self.font.cell_width;
        let ch = self.font.cell_height;
        let sw = self.width  as f32;
        let sh = self.height as f32;

        // ── Collect resolved cell data (avoids re-borrowing self later) ───────
        struct Cell {
            col: usize, row: usize,
            ch:  char,
            fg:  [f32; 4],
            bg:  [f32; 4],
            bold:   bool,
            italic: bool,
        }

        let cells: Vec<Cell> = snapshot.cells.iter().map(|c| {
            let bold   = c.flags.contains(CellFlags::BOLD);
            let italic = c.flags.contains(CellFlags::ITALIC);
            let dim    = c.flags.contains(CellFlags::DIM);
            let mut fg = resolve_color(&c.fg, true, &self.theme);
            if dim { fg = [fg[0] * 0.6, fg[1] * 0.6, fg[2] * 0.6, fg[3]]; }
            Cell {
                col: c.col, row: c.row, ch: c.ch,
                fg, bg: resolve_color(&c.bg, false, &self.theme),
                bold, italic,
            }
        }).collect();

        // ── Upload any new glyphs to the atlas ────────────────────────────────
        for c in &cells {
            if c.ch != ' ' && c.ch != '\0' {
                self.ensure_glyph(c.ch, c.bold, c.italic);
            }
        }

        // ── Build vertex buffers ──────────────────────────────────────────────
        let mut bg_verts:    Vec<BgVertex>    = Vec::with_capacity(cells.len() * 6);
        let mut glyph_verts: Vec<GlyphVertex> = Vec::with_capacity(cells.len() * 6);
        let atlas_sz = self.atlas_size as f32;

        for c in &cells {
            let x0 = c.col as f32 * cw;
            let y0 = c.row as f32 * ch;

            // Background cell quad
            push_bg_quad(&mut bg_verts, x0, y0, x0 + cw, y0 + ch, c.bg, sw, sh);

            // Glyph quad
            if c.ch != ' ' && c.ch != '\0' {
                let key = GlyphKey { ch: c.ch, bold: c.bold, italic: c.italic };
                if let Some(e) = self.atlas_glyphs.get(&key) {
                    let u0 = e.px as f32 / atlas_sz;
                    let v0 = e.py as f32 / atlas_sz;
                    let u1 = (e.px + e.pw) as f32 / atlas_sz;
                    let v1 = (e.py + e.ph) as f32 / atlas_sz;
                    push_glyph_quad(
                        &mut glyph_verts,
                        x0, y0, x0 + e.pw as f32, y0 + e.ph as f32,
                        u0, v0, u1, v1,
                        c.fg, sw, sh,
                    );
                }
            }
        }

        // Cursor — blinking rect overlay. Skipped on the "off" half of the blink cycle.
        if self.cursor_blink_on {
            let cc = self.theme.cursor;
            let cursor_col = [cc[0] as f32 / 255.0, cc[1] as f32 / 255.0, cc[2] as f32 / 255.0, 0.75];
            let cx = snapshot.cursor_col as f32 * cw;
            let cy = snapshot.cursor_row as f32 * ch;
            push_bg_quad(&mut bg_verts, cx, cy, cx + cw, cy + ch, cursor_col, sw, sh);
        }

        // ── GPU draw ──────────────────────────────────────────────────────────
        let output = self.surface.get_current_texture()?;
        let view   = output.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("vmux") }
        );

        let bg_col = self.theme.background;
        let clear = wgpu::Color {
            r: bg_col[0] as f64 / 255.0,
            g: bg_col[1] as f64 / 255.0,
            b: bg_col[2] as f64 / 255.0,
            a: 1.0,
        };

        let bg_buf = (!bg_verts.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:    Some("bg_verts"),
                contents: bytemuck::cast_slice(&bg_verts),
                usage:    wgpu::BufferUsages::VERTEX,
            })
        });
        let glyph_buf = (!glyph_verts.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:    Some("glyph_verts"),
                contents: bytemuck::cast_slice(&glyph_verts),
                usage:    wgpu::BufferUsages::VERTEX,
            })
        });

        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vmux"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            if let Some(ref buf) = bg_buf {
                pass.set_pipeline(&self.bg_pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..bg_verts.len() as u32, 0..1);
            }

            if let Some(ref buf) = glyph_buf {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(0, &self.glyph_bg, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..glyph_verts.len() as u32, 0..1);
            }
        }

        self.queue.submit(std::iter::once(enc.finish()));
        output.present();
        Ok(())
    }
}

// ─── Vertex helpers ───────────────────────────────────────────────────────────

/// Push two triangles for a solid-colour rect (pixel → NDC).
fn push_bg_quad(
    out: &mut Vec<BgVertex>,
    x0: f32, y0: f32, x1: f32, y1: f32,
    color: [f32; 4],
    sw: f32, sh: f32,
) {
    let (nx0, ny0) = pix_to_ndc(x0, y0, sw, sh);
    let (nx1, ny1) = pix_to_ndc(x1, y1, sw, sh);
    out.extend_from_slice(&[
        BgVertex { position: [nx0, ny0], color },
        BgVertex { position: [nx1, ny0], color },
        BgVertex { position: [nx1, ny1], color },
        BgVertex { position: [nx0, ny0], color },
        BgVertex { position: [nx1, ny1], color },
        BgVertex { position: [nx0, ny1], color },
    ]);
}

/// Push two triangles for a textured glyph quad.
#[allow(clippy::too_many_arguments)]
fn push_glyph_quad(
    out: &mut Vec<GlyphVertex>,
    x0: f32, y0: f32, x1: f32, y1: f32,
    u0: f32, v0: f32, u1: f32, v1: f32,
    color: [f32; 4],
    sw: f32, sh: f32,
) {
    let (nx0, ny0) = pix_to_ndc(x0, y0, sw, sh);
    let (nx1, ny1) = pix_to_ndc(x1, y1, sw, sh);
    out.extend_from_slice(&[
        GlyphVertex { position: [nx0, ny0], uv: [u0, v0], color },
        GlyphVertex { position: [nx1, ny0], uv: [u1, v0], color },
        GlyphVertex { position: [nx1, ny1], uv: [u1, v1], color },
        GlyphVertex { position: [nx0, ny0], uv: [u0, v0], color },
        GlyphVertex { position: [nx1, ny1], uv: [u1, v1], color },
        GlyphVertex { position: [nx0, ny1], uv: [u0, v1], color },
    ]);
}

/// Pixel (top-left origin) → NDC (wgpu clip space: x right, y up).
#[inline]
fn pix_to_ndc(px: f32, py: f32, sw: f32, sh: f32) -> (f32, f32) {
    ((px / sw) * 2.0 - 1.0, 1.0 - (py / sh) * 2.0)
}

// ─── Colour resolution ────────────────────────────────────────────────────────

fn resolve_color(c: &CellColor, is_fg: bool, theme: &Theme) -> [f32; 4] {
    let rgba: [u8; 4] = match c {
        CellColor::Named(n) => named_color(*n, is_fg, theme),
        CellColor::Indexed(i) => indexed_color(*i, theme),
        CellColor::Rgb(r, g, b) => [*r, *g, *b, 255],
    };
    [rgba[0] as f32 / 255.0, rgba[1] as f32 / 255.0, rgba[2] as f32 / 255.0, rgba[3] as f32 / 255.0]
}

fn named_color(n: NamedColor, _is_fg: bool, theme: &Theme) -> [u8; 4] {
    match n {
        NamedColor::Black         => theme.ansi[0],
        NamedColor::Red           => theme.ansi[1],
        NamedColor::Green         => theme.ansi[2],
        NamedColor::Yellow        => theme.ansi[3],
        NamedColor::Blue          => theme.ansi[4],
        NamedColor::Magenta       => theme.ansi[5],
        NamedColor::Cyan          => theme.ansi[6],
        NamedColor::White         => theme.ansi[7],
        NamedColor::BrightBlack   => theme.ansi[8],
        NamedColor::BrightRed     => theme.ansi[9],
        NamedColor::BrightGreen   => theme.ansi[10],
        NamedColor::BrightYellow  => theme.ansi[11],
        NamedColor::BrightBlue    => theme.ansi[12],
        NamedColor::BrightMagenta => theme.ansi[13],
        NamedColor::BrightCyan    => theme.ansi[14],
        NamedColor::BrightWhite   => theme.ansi[15],
        // Dim variants: approximate by reducing brightness of the normal colour.
        NamedColor::DimBlack   => dim(theme.ansi[0]),
        NamedColor::DimRed     => dim(theme.ansi[1]),
        NamedColor::DimGreen   => dim(theme.ansi[2]),
        NamedColor::DimYellow  => dim(theme.ansi[3]),
        NamedColor::DimBlue    => dim(theme.ansi[4]),
        NamedColor::DimMagenta => dim(theme.ansi[5]),
        NamedColor::DimCyan    => dim(theme.ansi[6]),
        NamedColor::DimWhite   => dim(theme.ansi[7]),
        NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground
            => theme.foreground,
        NamedColor::Background
            => theme.background,
        NamedColor::Cursor
            => theme.cursor,
    }
}

fn indexed_color(i: u8, theme: &Theme) -> [u8; 4] {
    if (i as usize) < 16 {
        return theme.ansi[i as usize];
    }
    match i {
        16..=231 => {
            let v = i as u32 - 16;
            let r = (v / 36) * 51;
            let g = ((v % 36) / 6) * 51;
            let b = (v % 6) * 51;
            [r as u8, g as u8, b as u8, 255]
        }
        _ => {
            // 232-255: grayscale ramp
            let v = (8 + (i as u32 - 232) * 10) as u8;
            [v, v, v, 255]
        }
    }
}

#[inline]
fn dim(c: [u8; 4]) -> [u8; 4] {
    [(c[0] as f32 * 0.6) as u8, (c[1] as f32 * 0.6) as u8, (c[2] as f32 * 0.6) as u8, c[3]]
}

// ─── WGSL Shaders ─────────────────────────────────────────────────────────────

const BG_SHADER: &str = r#"
struct Vert {
    @location(0) pos:   vec2<f32>,
    @location(1) color: vec4<f32>,
}
struct Frag {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}
@vertex
fn vs_main(v: Vert) -> Frag {
    return Frag(vec4<f32>(v.pos, 0.0, 1.0), v.color);
}
@fragment
fn fs_main(f: Frag) -> @location(0) vec4<f32> {
    return f.color;
}
"#;

const GLYPH_SHADER: &str = r#"
@group(0) @binding(0) var t_atlas: texture_2d<f32>;
@group(0) @binding(1) var s_atlas: sampler;

struct Vert {
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) color: vec4<f32>,
}
struct Frag {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv:    vec2<f32>,
    @location(1) color: vec4<f32>,
}
@vertex
fn vs_main(v: Vert) -> Frag {
    return Frag(vec4<f32>(v.pos, 0.0, 1.0), v.uv, v.color);
}
@fragment
fn fs_main(f: Frag) -> @location(0) vec4<f32> {
    // Atlas stores coverage in all channels (rasterized at white fg).
    // Apply the per-vertex foreground colour here.
    let coverage = textureSample(t_atlas, s_atlas, f.uv).r;
    return vec4<f32>(f.color.rgb, coverage * f.color.a);
}
"#;
