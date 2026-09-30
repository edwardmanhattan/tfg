//! The shader seam: one place where ARCONS stops being epaint.
//!
//! Everything else in this client is CPU-composed. `epaint` tessellates on
//! the CPU and egui hands one triangle mesh to `egui-wgpu`, which draws it
//! in a single pass with painter's-algorithm ordering. That is the right
//! tool for a console and it has no room for a shader.
//!
//! So this does not replace the renderer, it borrows a hook in it.
//! `egui_wgpu::CallbackTrait` gives a callback three phases — `prepare`
//! before egui's pass, `finish_prepare` after every `prepare`, and `paint`
//! *inside* egui's own render pass — and command buffers returned from
//! `prepare` are submitted in the same `queue.submit` as egui's.
//!
//! # Why one effect, and this one
//!
//! The design audit rejected most of what looked attractive. Bloom needs an
//! HDR target and something above 1.0 to bleed, and egui's attachment is a
//! plain swapchain — a real bloom chain is `prepare` plus an offscreen
//! `rgba16float`, which is a renderer rather than a widget. Glass fails the
//! function gate: refraction behind a roster someone is reading is
//! decoration on information-dense chrome.
//!
//! What survives is a **halo**: a state signal on the map, additive, one
//! quad per lit marker. Static at rest, never pulsing. Cyan means selected
//! or following, amber means stale — both already carrying that meaning in
//! the epaint rings, so the halo reinforces a channel rather than inventing
//! one. It is additive into a non-HDR target so it cannot clip badly, and it
//! is the smallest thing that proves the seam.
//!
//! # The constraint that decides everything added later
//!
//! Never blur at full resolution. A 41-tap separable gaussian at 1080p is
//! ~170M texture fetches; the same kernel at quarter resolution is 16x
//! cheaper. A blur is the first thing here that stops being affordable on a
//! software rasteriser or a weak integrated GPU, and it is why [`Quality`]
//! exists.
//!
//! # Verification state
//!
//! `proto/p5-epaint` compiles this file out of the tfg tree with `#[path]`
//! and runs [`build_pipeline`] against real hardware, so the WGSL is
//! compiled by a real driver rather than assumed valid, and the draw is
//! verified to put non-zero pixels into a target. What is *not* verified is
//! the pipeline format matching egui's pass inside the console — which is
//! why [`Quality::Low`] is the default and why the effect draws nothing
//! until a caller has handed it egui's own `target_format`.

use eframe::egui::epaint::{Pos2, Rect};
use eframe::egui::{Painter, Shape, Vec2};
use eframe::egui::PaintCallbackInfo;
use eframe::egui_wgpu::{
    Callback, CallbackResources, CallbackTrait, ScreenDescriptor, wgpu,
};

/// Which tier of effect the machine can afford.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quality {
    /// Software rasteriser, or not yet measured. No shaders. This is the
    /// default: an effect nobody has run on the operator's hardware should
    /// not be on by default.
    #[default]
    Low,
    /// Integrated GPU. Additive halos only.
    Medium,
    /// Discrete GPU. Room for a mip-chain blur later.
    High,
}

impl Quality {
    /// From the adapter's device type. Deliberately conservative — anything
    /// unrecognised gets `Low`, because guessing wrong in the expensive
    /// direction costs frames on the machine that can least afford them.
    pub fn from_device_type(dt: wgpu::DeviceType) -> Self {
        match dt {
            wgpu::DeviceType::DiscreteGpu => Quality::High,
            wgpu::DeviceType::IntegratedGpu => Quality::Medium,
            _ => Quality::Low,
        }
    }

    pub fn shaders(self) -> bool {
        self != Quality::Low
    }
}

/// What a halo is saying. Both variants reuse a meaning the epaint rings
/// already carry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Halo {
    /// Selected, or the camera is following this hull.
    Signal,
    /// Old data — backfilled rather than live.
    Stale,
}

impl Halo {
    /// Radar Cyan and the stale amber from DESIGN.md, as 0..1 floats.
    fn rgb(self) -> [f32; 3] {
        match self {
            Halo::Signal => [0.133, 0.827, 0.933],
            Halo::Stale => [0.965, 0.773, 0.420],
        }
    }
}

/// Radius in logical points. Sized to read as a halo rather than a glow
/// field: it clears the 8px marker and its 2px ring, and stops well before
/// it starts washing the trails underneath.
pub const HALO_R: f32 = 22.0;

/// The one shader in the client.
///
/// No vertex buffers: the quad's six corners come from
/// `@builtin(vertex_index)`, and everything else arrives in a 32-byte
/// uniform. For a single quad that is smaller than the machinery a vertex
/// buffer would need, and it means nothing to re-upload but eight floats.
pub const HALO_WGSL: &str = r#"
struct Uniforms {
    /// xy = halo centre in NDC, zw = half-extent in NDC.
    centre_half: vec4<f32>,
    /// rgb = tint, a = strength.
    tint: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    // Two triangles, wound consistently.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    let corner = corners[vi];
    var out: VsOut;
    out.clip = vec4<f32>(u.centre_half.xy + corner * u.centre_half.zw, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // 0 at the centre, 1 at the quad's inscribed circle.
    let d = length(in.uv);
    // smoothstep so the rim has no visible ring, squared so the falloff is
    // flat near the marker and steep at the edge.
    let a = pow(1.0 - smoothstep(0.35, 1.0, d), 2.0);
    // The blend is One/One, so alpha is already carried by the colour and
    // must not be added twice.
    return vec4<f32>(u.tint.rgb * a * u.tint.a, 0.0);
}
"#;

/// Bytes for a `[f32; 8]`, little-endian, without pulling in `bytemuck`.
fn pack8(v: [f32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, f) in v.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// Build the halo pipeline against a target format.
///
/// `format` must be the format of the pass it will be set into, or wgpu
/// raises a validation error at draw time. Take it from egui's own
/// `RenderState::target_format` rather than guessing.
pub fn build_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> Option<wgpu::RenderPipeline> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tfx halo"),
        source: wgpu::ShaderSource::Wgsl(HALO_WGSL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tfx halo"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tfx halo"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tfx halo"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // Additive: dst + src. That is what makes this a halo rather
                // than a disc, and it cannot darken anything behind it.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // egui does not cull; matching that means a mis-wound quad
            // degrades to a visible shape rather than vanishing.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    }))
}

/// GPU state, cached in egui's `CallbackResources` so it is built once per
/// device rather than once per frame. A pipeline creation per frame would
/// cost more than the effect.
struct HaloGpu {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

fn ensure_gpu(device: &wgpu::Device, format: wgpu::TextureFormat, res: &mut CallbackResources) {
    if res.get::<HaloGpu>().is_some() {
        return;
    }
    let Some(pipeline) = build_pipeline(device, format) else {
        return;
    };
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tfx halo uniform"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tfx halo"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    res.insert(HaloGpu {
        pipeline,
        uniform,
        bind,
    });
}

/// The seam's public face. One instance lives on `ShipApp`.
pub struct Fx {
    quality: Quality,
    format: wgpu::TextureFormat,
}

impl Fx {
    /// `format` must come from egui (`RenderState::target_format`). A wrong
    /// one produces a wgpu validation error at draw time rather than a
    /// wrong pixel, so the failure is loud rather than subtle.
    pub fn new(quality: Quality, format: wgpu::TextureFormat) -> Self {
        Self { quality, format }
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// A shape drawing one halo, or `None` when the machine cannot afford a
    /// shader.
    ///
    /// Returning `None` rather than drawing nothing is deliberate: the
    /// caller's layout is identical across tiers, which is what makes the
    /// low tier safe — it is the current code path, not a second one.
    pub fn halo(&self, center: Pos2, kind: Halo, alpha: f32) -> Option<Shape> {
        if !self.quality.shaders() || alpha <= 0.0 {
            return None;
        }
        let cb = Callback::new_paint_callback(
            Rect::from_center_size(center, Vec2::splat(HALO_R * 2.0)),
            HaloPass {
                center,
                kind,
                alpha: alpha.clamp(0.0, 1.0),
                format: self.format,
            },
        );
        Some(Shape::Callback(cb))
    }

    /// Add every halo in `halos` to `painter`.
    pub fn paint_halos(&self, painter: &Painter, halos: &[(Pos2, Halo, f32)]) {
        for (center, kind, alpha) in halos {
            if let Some(shape) = self.halo(*center, *kind, *alpha) {
                painter.add(shape);
            }
        }
    }
}

/// The callback: a centre, a colour and a strength, all of which the CPU
/// already knows. Carrying them means `prepare` never needs anything from
/// `paint`, which has no device and no queue.
struct HaloPass {
    center: Pos2,
    kind: Halo,
    alpha: f32,
    format: wgpu::TextureFormat,
}

impl CallbackTrait for HaloPass {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        ensure_gpu(device, self.format, resources);
        let Some(gpu) = resources.get::<HaloGpu>() else {
            return Vec::new();
        };
        let [w_px, h_px] = screen.size_in_pixels;
        if w_px == 0 || h_px == 0 {
            return Vec::new();
        }

        // egui's root viewport is at point-origin, so a point position maps
        // to NDC by dividing by the point-size of the window. `ppp` cancels
        // in the ratio, but the half-extent needs it in pixels first.
        let w_pts = w_px as f32 / screen.pixels_per_point;
        let h_pts = h_px as f32 / screen.pixels_per_point;
        let ndc_x = (self.center.x / w_pts) * 2.0 - 1.0;
        let ndc_y = 1.0 - (self.center.y / h_pts) * 2.0;
        let half = HALO_R * screen.pixels_per_point;
        let half_x = half / w_px as f32 * 2.0;
        let half_y = half / h_px as f32 * 2.0;
        let rgb = self.kind.rgb();

        queue.write_buffer(
            &gpu.uniform,
            0,
            &pack8([
                ndc_x, ndc_y, half_x, half_y, rgb[0], rgb[1], rgb[2], self.alpha,
            ]),
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        // `prepare` had the device and the queue and did all the work;
        // `paint` only draws. Six vertices, no vertex buffer.
        let Some(gpu) = resources.get::<HaloGpu>() else {
            return;
        };
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.bind, &[]);
        pass.draw(0..6, 0..1);
    }
}