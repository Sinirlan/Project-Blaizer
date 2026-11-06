//! REMASTER volumetric fog (17-fog-rework.md appendix) [design choice, not in the original]: a camera-aligned
//! froxel grid filled by a compute pass (`volfog.wgsl` `cs_inject`, jittered inside each froxel), blended with
//! the previous frame's froxels and integrated front to back (`cs_integrate`, 17-fog-rework.md "Temporal
//! filter"), and applied to the opaque scene by a depth-based full-screen pass (`fs_apply`).
//! Built on first use and kept across F9.

use std::cell::{OnceCell, RefCell};

use glam::{Mat4, Vec3};

/// Grid columns (x, y) and depth slices; the slices are exponential from `NEAR` to the view distance.
pub const COLUMNS: (u32, u32) = (160, 90);
pub const SLICES: u32 = 64;
/// First slice boundary (m): closer than this the fog fades to nothing.
pub const NEAR: f32 = 1.0;

// Headlight scatter relative to the street lamps' gain (`FogTune::lamps`) [design choice, tuned from shots].
motor_tune::tunable! {
    /// Headlight scatter relative to the street lamps' gain [design choice, tuned from shots].
    /// range: 0..1
    /// area: Graphics/Fog
    pub HEAD_SCATTER: f32 = 0.15;
}

// Weight of the reprojected history in the temporal blend [design choice, tuned from shots].
motor_tune::tunable! {
    /// Weight of the reprojected history in the volumetric fog's temporal blend [design choice, tuned from shots].
    /// range: 0..0.99
    /// area: Graphics/Fog
    pub HISTORY: f32 = 0.9;
}
/// Frames of the jitter sequence (Halton 2, 3, 5).
const JITTER_FRAMES: u32 = 16;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const WORKGROUP: u32 = 8;

/// `volfog.wgsl` `U`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolU {
    pub inv_vp: [[f32; 4]; 4],
    pub cam: [f32; 4],
    pub fwd: [f32; 4],
    pub fog: [f32; 4],
    pub sun: [f32; 4],
    pub sun_dir: [f32; 4],
    pub fog2: [f32; 4],
    pub fog3: [f32; 4],
    /// `Globals.fog4` / `fog5`: fog colour (linear) + sky haze; start, full, opacity, curve
    pub fog4: [f32; 4],
    pub fog5: [f32; 4],
    /// proj[0][0], proj[1][1], near, far
    pub proj: [f32; 4],
    /// volume near, volume far, slices
    pub vol: [f32; 4],
    /// output width, height
    pub scr: [f32; 4],
    /// the street lamps of `Globals` (position, range), (rgb, shadow layer); x: lamps in use,
    /// y, z: cosine of the cone's inner and outer half angle, w: scatter gain (`FogTune::lamps`)
    pub lamp_n: [f32; 4],
    pub lamps: [[f32; 4]; 2 * crate::render::MAX_LAMPS],
    /// the car headlights of `Globals` (kind 0 spots only): x: in use, w: scatter gain
    /// (`FogTune::lamps` x `HEAD_SCATTER`)
    pub spot_n: [f32; 4],
    /// (position, range), (rgb, 0), (beam axis, 0), (up, 0) per headlight
    pub spots: [[f32; 4]; 4 * crate::render::MAX_SPOTS],
    /// the previous frame's unjittered view-projection (world -> clip)
    pub prev_vp: [[f32; 4]; 4],
    /// jitter inside the froxel (x, y, slice), history weight (`VolFog::step`)
    pub jit: [f32; 4],
    /// the fog shadow map's world -> clip (the sun's view, `render.rs` `draw_shaft_map`)
    pub shaft_vp: [[f32; 4]; 4],
    /// x: the map holds this frame (sun shafts on), y: depth bias in map units, z: how much the haze darkens in shadow
    pub shaft: [f32; 4],
}

/// The last frame's camera, for the history.
struct Temporal {
    /// a history to blend with (cleared by `reset`, a camera cut or a change of the volume's depth)
    valid: bool,
    vp: Mat4,
    pos: Vec3,
    fwd: Vec3,
    far: f32,
    /// frames since the history was cleared (the jitter's index)
    age: u32,
    /// frames drawn: the history texture read this frame is `frame % 2`
    frame: u32,
}

fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

pub struct VolFog {
    uniform: wgpu::Buffer,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    compute: wgpu::ComputePipeline,
    compute_bind: wgpu::BindGroup,
    integrate: wgpu::ComputePipeline,
    /// [k]: reads history k, writes history 1 - k
    integrate_bind: [wgpu::BindGroup; 2],
    temporal: RefCell<Temporal>,
    /// one shader per depth kind (single-sampled, MSAA)
    shaders: [wgpu::ShaderModule; 2],
    apply_layouts: [wgpu::BindGroupLayout; 2],
    /// per sample count (log2); the scene is always the HDR target
    apply: [OnceCell<wgpu::RenderPipeline>; 5],
}

impl VolFog {
    pub fn new(device: &wgpu::Device, shaft: &wgpu::TextureView, shaft_sampler: &wgpu::Sampler) -> Self {
        let volume = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: COLUMNS.0, height: COLUMNS.1, depth_or_array_layers: SLICES },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        // the integrated volume (applied), this frame's samples, and the two history images
        let view = volume("fog volume");
        let scatter = volume("fog volume samples");
        let history = [volume("fog volume history 0"), volume("fog volume history 1")];
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog volume globals"),
            size: std::mem::size_of::<VolU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog volume"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shaders = [false, true].map(|msaa| {
            let ty = if msaa { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fog volume shader"),
                source: wgpu::ShaderSource::Wgsl(crate::shaders::src("volfog.wgsl").replace("DEPTH_TEX", ty).into()),
            })
        });
        let buffer = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: FORMAT, view_dimension: wgpu::TextureViewDimension::D3 },
            count: None,
        };
        let sampled = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
            count: None,
        };
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog volume inject"),
            entries: &[
                buffer(wgpu::ShaderStages::COMPUTE),
                storage(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 10, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            ],
        });
        let integrate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog volume integrate"),
            entries: &[
                buffer(wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                storage(4),
                sampled(6, false),
                sampled(7, true),
                storage(8),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fog volume inject"), bind_group_layouts: &[Some(&compute_layout)], immediate_size: 0 });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fog volume inject"),
            layout: Some(&pl),
            module: &shaders[0],
            entry_point: Some("cs_inject"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compute_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog volume inject"),
            layout: &compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&scatter) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(shaft) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(shaft_sampler) },
            ],
        });
        let ipl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fog volume integrate"), bind_group_layouts: &[Some(&integrate_layout)], immediate_size: 0 });
        let integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fog volume integrate"),
            layout: Some(&ipl),
            module: &shaders[0],
            entry_point: Some("cs_integrate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let integrate_bind = [0, 1].map(|k| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog volume integrate"),
                layout: &integrate_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&scatter) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&history[k]) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&history[1 - k]) },
                ],
            })
        });
        let apply_layouts = [false, true].map(|msaa| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fog volume apply"),
                entries: &[
                    buffer(wgpu::ShaderStages::FRAGMENT),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                ],
            })
        });
        let temporal = Temporal { valid: false, vp: Mat4::IDENTITY, pos: Vec3::ZERO, fwd: Vec3::Z, far: 0.0, age: 0, frame: 0 };
        VolFog { uniform, view, sampler, compute, compute_bind, integrate, integrate_bind, temporal: RefCell::new(temporal), shaders, apply_layouts, apply: Default::default() }
    }

    pub fn write(&self, queue: &wgpu::Queue, u: &VolU) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    /// Drop the history: the next frame is drawn from its own samples (a camera cut, the volume was
    /// not drawn, a track load, another view).
    pub fn reset(&self) {
        self.temporal.borrow_mut().valid = false;
    }

    /// Advance the history one frame: the previous unjittered view-projection and this frame's jitter and
    /// history weight (`VolU::prev_vp`, `jit`). The first frame after a reset samples the froxel centres
    /// and has no history, so a single shot is the plain unjittered volume.
    pub fn step(&self, vp: Mat4, pos: Vec3, fwd: Vec3, far: f32) -> ([[f32; 4]; 4], [f32; 4]) {
        let mut t = self.temporal.borrow_mut();
        // CLASSIC-like cut test of the velocity blur: a jump or a sharp turn has nothing to blend with
        if t.valid && (pos.distance(t.pos) > crate::render::CUT_DISTANCE.get() || fwd.dot(t.fwd) < crate::render::CUT_COS.get() || far != t.far) {
            t.valid = false;
        }
        if !t.valid {
            t.age = 0;
        }
        let out = if t.age == 0 {
            (Mat4::IDENTITY, [0.5, 0.5, 0.5, 0.0])
        } else {
            let i = t.age % JITTER_FRAMES + 1;
            (t.vp, [halton(i, 2), halton(i, 3), halton(i, 5), HISTORY.get()])
        };
        t.valid = true;
        t.vp = vp;
        t.pos = pos;
        t.fwd = fwd;
        t.far = far;
        t.age += 1;
        t.frame = t.frame.wrapping_add(1);
        (out.0.to_cols_array_2d(), out.1)
    }

    /// Fill the grid, blend it with the history and integrate it (after `write`, which follows `step`).
    pub fn inject(&self, enc: &mut wgpu::CommandEncoder) {
        // `step` counted this frame already: it reads the history the frame before wrote
        let read = (self.temporal.borrow().frame.wrapping_sub(1) % 2) as usize;
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fog volume inject"), timestamp_writes: None });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &self.compute_bind, &[]);
            pass.dispatch_workgroups(COLUMNS.0.div_ceil(WORKGROUP), COLUMNS.1.div_ceil(WORKGROUP), SLICES);
        }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fog volume integrate"), timestamp_writes: None });
        pass.set_pipeline(&self.integrate);
        pass.set_bind_group(0, &self.integrate_bind[read], &[]);
        pass.dispatch_workgroups(COLUMNS.0.div_ceil(WORKGROUP), COLUMNS.1.div_ceil(WORKGROUP), 1);
    }

    /// The integrated volume (in-scatter, transmittance up to each slice's far side): the water surface,
    /// drawn after the apply pass, looks it up at its own depth (water.wgsl `fs_water`, #189).
    pub fn volume(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The apply pass's bind group over the scene's depth.
    pub fn apply_bind(&self, device: &wgpu::Device, depth: &wgpu::TextureView, samples: u32) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog volume apply"),
            layout: &self.apply_layouts[(samples > 1) as usize],
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// scene * transmittance + in-scatter, in a pass of its own: the depth is read as a texture, so it cannot be attached.
    pub fn apply_pipeline(&self, device: &wgpu::Device, samples: u32, hdr_format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.apply[samples.ilog2() as usize].get_or_init(|| {
            let k = (samples > 1) as usize;
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fog volume apply"), bind_group_layouts: &[Some(&self.apply_layouts[k])], immediate_size: 0 });
            let blend = wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fog volume apply"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &self.shaders[k], entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shaders[k],
                    entry_point: Some("fs_apply"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}
