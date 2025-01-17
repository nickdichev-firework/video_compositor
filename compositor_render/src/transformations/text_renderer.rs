use std::{
    cmp::max,
    sync::{Arc, Mutex},
};

use compositor_common::scene::{
    text_spec::{self, TextDimensions},
    Resolution,
};
use glyphon::{
    AttrsOwned, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache, TextArea, TextAtlas,
    TextBounds,
};
use log::info;
use text_spec::TextSpec;
use wgpu::{
    CommandEncoderDescriptor, LoadOp, MultisampleState, Operations, RenderPassColorAttachment,
    RenderPassDescriptor, TextureFormat,
};

use crate::{renderer::RenderCtx, utils::rgba_to_wgpu_color, wgpu::texture::NodeTexture};

#[allow(dead_code)]
pub struct TextParams {
    content: Arc<str>,
    attributes: AttrsOwned,
    font_size: f32,
    line_height: f32,
    align: glyphon::cosmic_text::Align,
    wrap: glyphon::cosmic_text::Wrap,
}

impl From<TextSpec> for TextParams {
    fn from(text_params: TextSpec) -> Self {
        Self {
            attributes: Into::into(&text_params),
            content: text_params.content,
            font_size: text_params.font_size,
            line_height: text_params.line_height.unwrap_or(text_params.font_size),
            align: text_params.align.into(),
            wrap: text_params.wrap.into(),
        }
    }
}

#[allow(dead_code)]
pub struct TextRendererCtx {
    font_system: Mutex<FontSystem>,
    swash_cache: Mutex<SwashCache>,
}

impl TextRendererCtx {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            font_system: Mutex::new(FontSystem::new()),
            swash_cache: Mutex::new(SwashCache::new()),
        }
    }
}

impl Default for TextRendererCtx {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
pub struct TextRendererNode {
    buffer: Buffer,
    resolution: Resolution,
    background_color: wgpu::Color,
    was_rendered: Mutex<bool>,
}

impl TextRendererNode {
    #[allow(dead_code)]
    pub fn new(renderer_ctx: &RenderCtx, text_spec: TextSpec) -> Self {
        let text_renderer_ctx = &renderer_ctx.text_renderer_ctx;
        let text_dimensions = text_spec.dimensions;
        let background_color = rgba_to_wgpu_color(&text_spec.background_color_rgba);
        let (buffer, resolution) =
            Self::layout_text(text_renderer_ctx, text_spec.into(), text_dimensions);

        Self {
            buffer,
            resolution,
            background_color,
            was_rendered: Mutex::new(false),
        }
    }

    pub fn resolution(&self) -> Resolution {
        self.resolution
    }

    pub fn render(&self, renderer_ctx: &mut RenderCtx, target: &mut NodeTexture) {
        let mut was_rendered = self.was_rendered.lock().unwrap();
        if *was_rendered {
            return;
        }

        info!("Text render");
        let text_renderer = renderer_ctx.text_renderer_ctx;
        let font_system = &mut text_renderer.font_system.lock().unwrap();
        let cache = &mut text_renderer.swash_cache.lock().unwrap();

        let swapchain_format = TextureFormat::Rgba8Unorm;
        let mut atlas = TextAtlas::new(
            &renderer_ctx.wgpu_ctx.device,
            &renderer_ctx.wgpu_ctx.queue,
            swapchain_format,
        );
        let mut text_renderer = glyphon::TextRenderer::new(
            &mut atlas,
            &renderer_ctx.wgpu_ctx.device,
            MultisampleState::default(),
            None,
        );

        text_renderer
            .prepare(
                &renderer_ctx.wgpu_ctx.device,
                &renderer_ctx.wgpu_ctx.queue,
                font_system,
                &mut atlas,
                glyphon::Resolution {
                    width: self.resolution.width as u32,
                    height: self.resolution.height as u32,
                },
                [TextArea {
                    buffer: &self.buffer,
                    left: 0 as f32,
                    top: 0 as f32,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.resolution.width as i32,
                        bottom: self.resolution.height as i32,
                    },
                    default_color: Color::rgb(255, 255, 255),
                }],
                cache,
            )
            .unwrap();

        let mut encoder =
            renderer_ctx
                .wgpu_ctx
                .device
                .create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("Text renderer encoder"),
                });

        let target_state = target.ensure_size(renderer_ctx.wgpu_ctx, self.resolution);
        let view = &target_state.rgba_texture().texture().view;
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(self.background_color),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            text_renderer.render(&atlas, &mut pass).unwrap();
        }

        renderer_ctx.wgpu_ctx.queue.submit(Some(encoder.finish()));
        *was_rendered = true;
    }

    fn layout_text(
        text_renderer_ctx: &TextRendererCtx,
        text_params: TextParams,
        text_resolution: TextDimensions,
    ) -> (Buffer, Resolution) {
        let font_system = &mut text_renderer_ctx.font_system.lock().unwrap();
        let mut buffer = Buffer::new(
            font_system,
            Metrics::new(text_params.font_size, text_params.line_height),
        );

        buffer.set_text(
            font_system,
            &text_params.content,
            text_params.attributes.as_attrs(),
            Shaping::Advanced,
        );
        buffer.set_wrap(font_system, text_params.wrap);

        let texture_size = match text_resolution {
            TextDimensions::Fixed { width, height } => Resolution {
                width: width as usize,
                height: height as usize,
            },
            TextDimensions::Fitted {
                max_width,
                max_height,
            } => {
                buffer.set_size(font_system, max_width as f32, max_height as f32);
                buffer.shape_until_scroll(font_system);
                Self::get_text_resolution(
                    buffer.lines.iter(),
                    text_params.line_height,
                    text_params.font_size,
                )
            }
            TextDimensions::FittedColumn { width, max_height } => {
                buffer.set_size(font_system, width as f32, max_height as f32);
                buffer.shape_until_scroll(font_system);
                let text_size = Self::get_text_resolution(
                    buffer.lines.iter(),
                    text_params.line_height,
                    text_params.font_size,
                );

                Resolution {
                    width: width as usize,
                    height: text_size.height,
                }
            }
        };

        buffer.set_size(
            font_system,
            texture_size.width as f32,
            texture_size.height as f32,
        );
        for line in &mut buffer.lines {
            line.set_align(Some(text_params.align));
        }
        buffer.shape_until_scroll(font_system);

        (buffer, texture_size)
    }

    fn get_text_resolution<'a, I: Iterator<Item = &'a glyphon::BufferLine>>(
        lines: I,
        line_height: f32,
        font_size: f32,
    ) -> Resolution {
        let mut width = 0;
        let mut lines_count = 0u32;

        for line in lines {
            if let Some(layout) = line.layout_opt() {
                for layout_line in layout {
                    lines_count += 1;
                    width = max(width, layout_line.w.ceil() as usize);
                }
            }
        }

        let last_line_padding = font_size / 5.0;
        let height = (lines_count as f32 * line_height.ceil() + last_line_padding) as usize;
        Resolution { width, height }
    }
}
