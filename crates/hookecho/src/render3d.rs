//! 3D reflectivity raymarch: a fullscreen-triangle MIP raymarch of a volume texture, rendered
//! into an egui rect via an `egui_wgpu` paint callback (mirrors the map callback pattern).

use glam::{Mat4, Vec3};

/// Raymarch uniform block (matches `shaders/raymarch.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    inv_view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    box_min: [f32; 4],
    box_max: [f32; 4],
    dims: [f32; 4], // nx, ny, nz, step_count
}

/// A new volume grid to upload: `data` is `n×n×nz` R8 indices, `lut` a 256-entry RGBA table.
pub struct Volume3dUpload {
    pub data: Vec<u8>,
    pub n: u32,
    pub nz: u32,
    pub lut: Vec<u8>,
}

/// Box extents of the rendered volume (z exaggerated for legibility).
const BOX_MIN: Vec3 = Vec3::new(-1.0, -1.0, 0.0);
const BOX_MAX: Vec3 = Vec3::new(1.0, 1.0, 0.5);

/// Orbit-camera uniforms: azimuth/elevation in degrees, `dist` from the box center, view `aspect`.
pub fn orbit_uniform(az_deg: f32, el_deg: f32, dist: f32, aspect: f32, n: u32, nz: u32, steps: u32) -> Uniforms {
    let center = (BOX_MIN + BOX_MAX) * 0.5;
    let (az, el) = (az_deg.to_radians(), el_deg.to_radians());
    let dir = Vec3::new(el.cos() * az.sin(), el.cos() * az.cos(), el.sin());
    let eye = center + dir * dist;
    let view = Mat4::look_at_rh(eye, center, Vec3::Z);
    let proj = Mat4::perspective_rh(45f32.to_radians(), aspect.max(0.1), 0.01, 100.0);
    let inv = (proj * view).inverse();
    Uniforms {
        inv_view_proj: inv.to_cols_array_2d(),
        cam_pos: [eye.x, eye.y, eye.z, 1.0],
        box_min: [BOX_MIN.x, BOX_MIN.y, BOX_MIN.z, 0.0],
        box_max: [BOX_MAX.x, BOX_MAX.y, BOX_MAX.z, 0.0],
        dims: [n as f32, n as f32, nz as f32, steps as f32],
    }
}

struct Gpu {
    _tex: wgpu::Texture,
    _lut: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// Long-lived raymarch resources (pipeline + latest volume), stored in egui's callback map.
pub struct Volume3dResources {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
    gpu: Option<Gpu>,
}

impl Volume3dResources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/raymarch.wgsl"));
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raymarch_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raymarch_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raymarch_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Premultiplied alpha (shader outputs rgb*a, a).
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch_uniform"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, bgl, uniform_buf, gpu: None }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, up: &Volume3dUpload) {
        let size = wgpu::Extent3d { width: up.n, height: up.n, depth_or_array_layers: up.nz };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volume3d_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &up.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(up.n), rows_per_image: Some(up.n) },
            size,
        );
        let lut_size = wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 };
        let lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volume3d_lut"),
            size: lut_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &lut, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &up.lut,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
            lut_size,
        );
        let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&tex_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lut_view) },
            ],
        });
        self.gpu = Some(Gpu { _tex: tex, _lut: lut, bind_group });
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(gpu) = &self.gpu {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Upload a volume + camera and raymarch it once into `view` (headless verify harness).
    pub fn render_once(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        upload: &Volume3dUpload,
        uniform: Uniforms,
        clear: wgpu::Color,
    ) {
        self.upload(device, queue, upload);
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("raymarch_headless") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raymarch_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.record(&mut pass);
        }
        queue.submit(Some(enc.finish()));
    }
}

/// Per-frame raymarch draw: an optional new volume upload + the current camera uniforms.
pub struct Volume3dCallback {
    pub upload: Option<Volume3dUpload>,
    pub uniform: Uniforms,
}

impl egui_wgpu::CallbackTrait for Volume3dCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get_mut::<Volume3dResources>() {
            if let Some(up) = &self.upload {
                res.upload(device, queue, up);
            }
            queue.write_buffer(&res.uniform_buf, 0, bytemuck::bytes_of(&self.uniform));
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(res) = resources.get::<Volume3dResources>() {
            res.record(pass);
        }
    }
}
