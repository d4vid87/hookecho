//! On-map reflectivity value extrusion. The height is a visualization of the
//! current sweep's intensity, distinct from measured beam height or a volume.
//! A bounded polar mesh reads the resident R8/R16 sweep; no network or CPU mesh
//! rebuilding occurs when the camera moves. Depth resolves folds before the
//! premultiplied image is composited into egui's depthless map pass.
use super::{MapCallback, PaneGpu};

pub fn radar_surface_supported(moment: wxdata::level2::Moment) -> bool {
    moment == wxdata::level2::Moment::Reflectivity
}

fn cells(dims: (u32, u32)) -> u32 {
    dims.0.saturating_sub(1).min(256) * dims.1.min(360)
}

fn target_size(cb: &MapCallback) -> [u32; 2] {
    cb.camera_scale.map(|scale| {
        let size = 2.0 / (scale * cb.world_per_pixel).abs();
        if size.is_finite() {
            size.round().clamp(1.0, 2048.0) as u32
        } else {
            1
        }
    })
}

pub(super) struct Target {
    color: wgpu::Texture,
    depth: wgpu::Texture,
    group: wgpu::BindGroup,
    camera_key: Vec<u32>,
}
impl Target {
    pub fn memory_bytes(&self) -> u64 {
        u64::from(self.color.width()) * u64::from(self.color.height()) * 8
    }
}

pub(super) struct Renderer {
    surface: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
        radar: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("radar value surface"),
            bind_group_layouts: &[Some(camera), Some(radar)],
            immediate_size: 0,
        });
        let surface = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radar value surface"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_surface"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_surface"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_shader = device.create_shader_module(wgpu::include_wgsl!(
            "../shaders/radar_surface_composite.wgsl"
        ));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface composite"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("surface composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("surface composite"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            surface,
            composite,
            composite_layout,
            sampler,
        }
    }

    pub fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pane: &mut PaneGpu,
        cb: &MapCallback,
    ) {
        if !cb.radar_surface
            || !pane.frame_draw_radar
            || pane.radar.as_ref().is_none_or(|r| cells(r.dims) == 0)
        {
            pane.surface = None;
            return;
        }
        let size = target_size(cb);
        let key: Vec<u32> = cb
            .camera_center
            .iter()
            .chain(&cb.camera_scale)
            .chain(&cb.globe)
            .chain(&cb.planar)
            .chain(std::iter::once(&cb.world_per_pixel))
            .map(|v| v.to_bits())
            .collect();
        if pane
            .surface
            .as_ref()
            .is_none_or(|s| [s.color.width(), s.color.height()] != size)
        {
            let make = |format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("radar value surface"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = make(
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let depth = make(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let view = color.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface composite"),
                layout: &self.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            pane.surface = Some(Target {
                color,
                depth,
                group,
                camera_key: Vec::new(),
            });
        }
        let target = pane.surface.as_mut().unwrap();
        if target.camera_key == key && cb.radar_upload.is_none() {
            return;
        }
        target.camera_key = key;
        let color = target.color.create_view(&Default::default());
        let depth = target.depth.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("radar surface depth pass"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("radar surface depth pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let radar = pane.radar.as_ref().unwrap();
            pass.set_pipeline(&self.surface);
            pass.set_bind_group(0, &pane.camera_bg, &[]);
            pass.set_bind_group(1, &radar.bind_group, &[]);
            pass.draw(0..cells(radar.dims) * 6, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }

    pub fn draw(&self, target: &Target, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &target.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composite_shader_validates_for_webgl2() {
        use wgpu::naga;
        let source = include_str!("../shaders/radar_surface_composite.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let _info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap();
        #[cfg(target_os = "linux")]
        for entry in &module.entry_points {
            let options = naga::back::glsl::Options { version: naga::back::glsl::Version::Embedded { version: 300, is_webgl: true }, ..Default::default() };
            let pipeline = naga::back::glsl::PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone(), multiview: None };
            let mut output = String::new();
            naga::back::glsl::Writer::new(&mut output, &module, &_info, &options, &pipeline, Default::default()).unwrap().write().unwrap();
        }
    }

    #[test]
    fn surface_is_bounded_and_only_continuous_reflectivity_is_eligible() {
        assert_eq!(cells((0, 720)), 0);
        assert_eq!(cells((1, 720)), 0);
        assert_eq!(cells((2, 1)), 1);
        assert_eq!(cells((1840, 720)), 256 * 360);
        for moment in wxdata::level2::Moment::ALL {
            assert_eq!(
                radar_surface_supported(moment),
                moment == wxdata::level2::Moment::Reflectivity
            );
        }
    }
}
