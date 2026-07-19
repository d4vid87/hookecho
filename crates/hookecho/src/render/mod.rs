//! wgpu rendering: a slippy-map tile layer plus a polar radar layer, both drawn
//! inside egui's render pass via [`egui_wgpu::CallbackTrait`].

pub mod mercator;

use std::collections::HashMap;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// XYZ tile id.
pub type TileId = (u8, u32, u32);

const MAX_TILE_VERTS: u64 = 512 * 6; // up to 512 visible tiles per frame

/// A decoded RGBA tile the app wants uploaded this frame.
pub struct PendingTile {
    pub id: TileId,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A tile to draw this frame (world-space rect, top-left origin).
#[derive(Clone, Copy)]
pub struct VisibleTile {
    pub id: TileId,
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
}

/// A binned sweep plus the world-space quad covering its range disk.
pub struct RadarUpload {
    pub az_bins: u32,
    pub gate_count: u32,
    pub data: Vec<u8>,
    /// [radar_lat, radar_lon, first_gate_km, gate_interval_km, az_bins, gate_count,
    ///  smoothing, srv, motion_e, motion_n, _pad, _pad] (see `shaders/radar.wgsl`).
    pub uniform: [f32; 12],
    /// 256×1 RGBA color LUT indexed by the sweep's `u8` (see `colormap::bake_lut`).
    pub lut: Vec<u8>,
    /// World-space quad corners covering the disk (min/max box).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
}

/// A national gridded field layer (all share the MRMS warp pipeline; they differ only in data,
/// LUT, and draw order). `below_radar` layers paint under the single-site radar; the rest above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldLayer {
    Mrms,
    Hrrr,
    Rotation,
    Mesh,
    AzShear,
    Lightning,
    Qpe1h,
    Qpe24h,
}

impl FieldLayer {
    /// Painting under the single-site radar (national context) vs. over it (severe signals).
    pub fn below_radar(self) -> bool {
        matches!(self, FieldLayer::Mrms | FieldLayer::Hrrr)
    }

    /// Fixed bottom-to-top paint order within each band.
    pub const DRAW_ORDER: [FieldLayer; 8] = [
        FieldLayer::Mrms,
        FieldLayer::Hrrr,
        FieldLayer::Qpe1h,
        FieldLayer::Qpe24h,
        FieldLayer::Rotation,
        FieldLayer::Mesh,
        FieldLayer::AzShear,
        FieldLayer::Lightning,
    ];
}

/// A national MRMS mosaic to upload: an R8 index grid + LUT, warped plate-carrée→mercator.
pub struct MrmsUpload {
    pub data: Vec<u8>,
    pub nx: u32,
    pub ny: u32,
    /// World-space quad (mercator bbox of the grid).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
    /// [lon_west, lat_north, lon_east, lat_south, nx, ny, +6 pad] (see `shaders/mrms.wgsl`).
    pub uniform: [f32; 12],
    pub lut: Vec<u8>,
}

/// A tessellated vertex for the vector overlay layer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayVertex {
    pub world: [f32; 2],
    pub color: [f32; 4],
}

/// Pre-tessellated overlay geometry to upload this frame.
pub struct OverlayUpload {
    pub vertices: Vec<OverlayVertex>,
    pub indices: Vec<u32>,
}

/// A tessellated vector basemap tile to upload this frame (reuses the overlay pipeline).
pub struct PendingVectorTile {
    pub id: TileId,
    pub vertices: Vec<OverlayVertex>,
    pub indices: Vec<u32>,
}

/// Per-frame draw instructions handed to the render callback.
pub struct MapCallback {
    /// Which pane this callback draws (indexes into `RenderResources.panes`).
    pub pane: u32,
    pub camera_center: [f32; 2],
    pub camera_scale: [f32; 2],
    pub new_tiles: Vec<PendingTile>,
    pub visible: Vec<VisibleTile>,
    pub radar_upload: Option<RadarUpload>,
    pub draw_radar: bool,
    /// `Some` only when the overlay geometry changed (else the last upload is reused).
    pub overlay_upload: Option<OverlayUpload>,
    pub draw_overlay: bool,
    /// Drop all cached GPU tiles before uploading (basemap style changed).
    pub clear_tiles: bool,
    /// Field layers whose grid changed this frame (uploaded now); others reuse the last upload.
    pub field_uploads: Vec<(FieldLayer, MrmsUpload)>,
    /// Which field layers to paint this frame.
    pub field_draws: Vec<FieldLayer>,
    /// Newly tessellated vector basemap tiles to upload this frame.
    pub new_vector_tiles: Vec<PendingVectorTile>,
    /// Vector tile ids to draw this frame (drawn first, under the raster/radar layers).
    pub visible_vector: Vec<TileId>,
    /// Drop all cached vector tiles before uploading (style or tess-zoom changed).
    pub clear_vector: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileVertex {
    world: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadarVertex {
    world: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    scale: [f32; 2],
}

struct TileGpu {
    _tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct RadarGpu {
    _tex: wgpu::Texture,
    _lut: wgpu::Texture,
    _uni: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct OverlayGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
}

struct MrmsGpu {
    _tex: wgpu::Texture,
    _lut: wgpu::Texture,
    _uni: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
}

/// Per-pane GPU state: its own camera uniform, radar sweep, tile/radar quad buffers, and the
/// draw lists staged during `prepare` and consumed during `paint`. (All `prepare`s run before
/// any `paint`, so shared per-frame buffers would clobber across panes — hence per-pane.)
struct PaneGpu {
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    tile_vbuf: wgpu::Buffer,
    radar_vbuf: wgpu::Buffer,
    radar: Option<RadarGpu>,
    frame_visible: Vec<VisibleTile>,
    frame_visible_vector: Vec<TileId>,
    frame_draw_radar: bool,
    frame_draw_overlay: bool,
}

/// Long-lived GPU resources, stored in egui's `CallbackResources` type-map.
pub struct RenderResources {
    tile_pipeline: wgpu::RenderPipeline,
    radar_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    mrms_pipeline: wgpu::RenderPipeline,
    camera_bgl: wgpu::BindGroupLayout,
    tile_bgl: wgpu::BindGroupLayout,
    radar_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    // Shared across panes: tile image cache, vector tile geometry, and the world-space overlay
    // (severe weather + placefiles) — the overlay is camera-independent, drawn per-pane camera.
    tiles: HashMap<TileId, TileGpu>,
    vector_tiles: HashMap<TileId, OverlayGpu>,
    overlay: Option<OverlayGpu>,
    fields: HashMap<FieldLayer, MrmsGpu>,
    field_draws: Vec<FieldLayer>,
    // One entry per live pane.
    panes: HashMap<u32, PaneGpu>,
}

impl RenderResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let tile_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/tiles.wgsl"));
        let radar_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/radar.wgsl"));
        let overlay_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/overlay.wgsl"));
        let mrms_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/mrms.wgsl"));

        // group 0: camera uniform (shared).
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            }],
        });
        // group 1 (tiles): texture + sampler.
        let tile_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile_bgl"),
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
        // group 1 (radar): uniform + u32 texture.
        let radar_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("radar_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(48),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
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

        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let color_target = wgpu::ColorTargetState {
            format: target_format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        };

        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&tile_bgl)],
            immediate_size: 0,
        });
        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile_pipeline"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &tile_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TileVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target.clone())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let radar_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("radar_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&radar_bgl)],
            immediate_size: 0,
        });
        let radar_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radar_pipeline"),
            layout: Some(&radar_layout),
            vertex: wgpu::VertexState {
                module: &radar_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RadarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &radar_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target)],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // MRMS pipeline: same layout as radar (camera + radar_bgl), warps a world quad.
        let mrms_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mrms_pipeline"),
            layout: Some(&radar_layout),
            vertex: wgpu::VertexState {
                module: &mrms_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RadarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mrms_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Overlay pipeline: camera-transformed colored triangles (group 0 only).
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay_layout"),
            bind_group_layouts: &[Some(&camera_bgl)],
            immediate_size: 0,
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay_pipeline"),
            layout: Some(&overlay_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<OverlayVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            tile_pipeline,
            radar_pipeline,
            overlay_pipeline,
            mrms_pipeline,
            camera_bgl,
            tile_bgl,
            radar_bgl,
            sampler,
            tiles: HashMap::new(),
            vector_tiles: HashMap::new(),
            overlay: None,
            fields: HashMap::new(),
            field_draws: Vec::new(),
            panes: HashMap::new(),
        }
    }

    /// Get or create the per-pane GPU state for `id`.
    fn pane_mut(&mut self, device: &wgpu::Device, id: u32) -> &mut PaneGpu {
        let camera_bgl = &self.camera_bgl;
        self.panes.entry(id).or_insert_with(|| {
            let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("camera_buf"),
                size: std::mem::size_of::<CameraUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera_bg"),
                layout: camera_bgl,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buf.as_entire_binding() }],
            });
            let tile_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tile_vbuf"),
                size: MAX_TILE_VERTS * std::mem::size_of::<TileVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let radar_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("radar_vbuf"),
                size: 6 * std::mem::size_of::<RadarVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            PaneGpu {
                camera_buf,
                camera_bg,
                tile_vbuf,
                radar_vbuf,
                radar: None,
                frame_visible: Vec::new(),
                frame_visible_vector: Vec::new(),
                frame_draw_radar: false,
                frame_draw_overlay: false,
            }
        })
    }


    fn upload_tile(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, t: &PendingTile) {
        let size = wgpu::Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &t.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * t.width),
                rows_per_image: Some(t.height),
            },
            size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile_bg"),
            layout: &self.tile_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.tiles.insert(t.id, TileGpu { _tex: tex, bind_group });
    }

    fn build_radar(&self, device: &wgpu::Device, queue: &wgpu::Queue, r: &RadarUpload) -> RadarGpu {
        let size = wgpu::Extent3d {
            width: r.gate_count,
            height: r.az_bins,
            depth_or_array_layers: 1,
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sweep_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &r.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(r.gate_count),
                rows_per_image: Some(r.az_bins),
            },
            size,
        );
        let uni = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar_uniform"),
            contents: bytemuck::cast_slice(&r.uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // 256×1 color LUT, indexed by the sweep u8 in the fragment shader.
        let lut_size = wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 };
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar_lut"),
            size: lut_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &r.lut,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(1),
            },
            lut_size,
        );

        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar_bg"),
            layout: &self.radar_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uni.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lut_view) },
            ],
        });
        RadarGpu { _tex: tex, _lut: lut_tex, _uni: uni, bind_group }
    }

    /// Upload camera/tiles/radar for `cb` and stage its pane's draw list. Shared caches (tiles,
    /// vector tiles, overlay) update once; per-pane state (camera, radar, tile quads) is keyed
    /// by `cb.pane`. Shared by the egui callback and the headless renderer.
    fn upload_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, cb: &MapCallback) {
        // --- Shared caches ---
        if cb.clear_tiles {
            self.tiles.clear();
        }
        if cb.clear_vector {
            self.vector_tiles.clear();
        }
        for t in &cb.new_vector_tiles {
            self.upload_vector_tile(device, t);
        }
        for t in &cb.new_tiles {
            self.upload_tile(device, queue, t);
        }
        if let Some(o) = &cb.overlay_upload {
            self.upload_overlay(device, o);
        }
        for (layer, up) in &cb.field_uploads {
            let gpu = self.build_field_layer(device, queue, up);
            self.fields.insert(*layer, gpu);
        }
        // Draw only the requested layers that actually have GPU data.
        self.field_draws = cb.field_draws.iter().copied().filter(|l| self.fields.contains_key(l)).collect();

        // --- Per-pane state ---
        let new_radar = cb.radar_upload.as_ref().map(|r| self.build_radar(device, queue, r));
        // Build the tile quad list against the shared tile cache before mutably borrowing the pane.
        let mut tverts: Vec<TileVertex> = Vec::new();
        let mut visible: Vec<VisibleTile> = Vec::new();
        for v in &cb.visible {
            if tverts.len() as u64 + 6 > MAX_TILE_VERTS {
                break;
            }
            if !self.tiles.contains_key(&v.id) {
                continue;
            }
            let [x0, y0] = v.world_min;
            let [x1, y1] = v.world_max;
            tverts.extend_from_slice(&[
                TileVertex { world: [x0, y0], uv: [0.0, 0.0] },
                TileVertex { world: [x1, y0], uv: [1.0, 0.0] },
                TileVertex { world: [x1, y1], uv: [1.0, 1.0] },
                TileVertex { world: [x0, y0], uv: [0.0, 0.0] },
                TileVertex { world: [x1, y1], uv: [1.0, 1.0] },
                TileVertex { world: [x0, y1], uv: [0.0, 1.0] },
            ]);
            visible.push(*v);
        }
        let overlay_present = self.overlay.is_some();

        let pane = self.pane_mut(device, cb.pane);
        queue.write_buffer(
            &pane.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform { center: cb.camera_center, scale: cb.camera_scale }),
        );
        if let Some(r) = &cb.radar_upload {
            let [x0, y0] = r.world_min;
            let [x1, y1] = r.world_max;
            let verts = [
                RadarVertex { world: [x0, y0] },
                RadarVertex { world: [x1, y0] },
                RadarVertex { world: [x1, y1] },
                RadarVertex { world: [x0, y0] },
                RadarVertex { world: [x1, y1] },
                RadarVertex { world: [x0, y1] },
            ];
            queue.write_buffer(&pane.radar_vbuf, 0, bytemuck::cast_slice(&verts));
        }
        if let Some(radar) = new_radar {
            pane.radar = Some(radar);
        }
        if !tverts.is_empty() {
            queue.write_buffer(&pane.tile_vbuf, 0, bytemuck::cast_slice(&tverts));
        }
        pane.frame_visible = visible;
        pane.frame_visible_vector = cb.visible_vector.clone();
        pane.frame_draw_radar = cb.draw_radar && pane.radar.is_some();
        pane.frame_draw_overlay = cb.draw_overlay && overlay_present;
    }

    fn upload_overlay(&mut self, device: &wgpu::Device, o: &OverlayUpload) {
        if o.indices.is_empty() {
            self.overlay = None;
            return;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay_vbuf"),
            contents: bytemuck::cast_slice(&o.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay_ibuf"),
            contents: bytemuck::cast_slice(&o.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.overlay = Some(OverlayGpu { vbuf, ibuf, index_count: o.indices.len() as u32 });
    }

    /// Build a national field-layer (MRMS mosaic or lightning): R8 index texture + LUT texture +
    /// grid uniform + full-grid quad. Shared by both layers; they differ only in data + LUT.
    fn build_field_layer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, m: &MrmsUpload) -> MrmsGpu {
        let size = wgpu::Extent3d { width: m.nx, height: m.ny, depth_or_array_layers: 1 };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mrms_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &m.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(m.nx), rows_per_image: Some(m.ny) },
            size,
        );
        let uni = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mrms_uniform"),
            contents: bytemuck::cast_slice(&m.uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let lut_size = wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 };
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mrms_lut"),
            size: lut_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &lut_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &m.lut,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
            lut_size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mrms_bg"),
            layout: &self.radar_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uni.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lut_view) },
            ],
        });
        let [x0, y0] = m.world_min;
        let [x1, y1] = m.world_max;
        let verts = [
            RadarVertex { world: [x0, y0] },
            RadarVertex { world: [x1, y0] },
            RadarVertex { world: [x1, y1] },
            RadarVertex { world: [x0, y0] },
            RadarVertex { world: [x1, y1] },
            RadarVertex { world: [x0, y1] },
        ];
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mrms_vbuf"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        MrmsGpu { _tex: tex, _lut: lut_tex, _uni: uni, bind_group, vbuf }
    }

    fn upload_vector_tile(&mut self, device: &wgpu::Device, t: &PendingVectorTile) {
        if t.indices.is_empty() {
            self.vector_tiles.remove(&t.id);
            return;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vector_vbuf"),
            contents: bytemuck::cast_slice(&t.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vector_ibuf"),
            contents: bytemuck::cast_slice(&t.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.vector_tiles
            .insert(t.id, OverlayGpu { vbuf, ibuf, index_count: t.indices.len() as u32 });
    }

    /// Paint the active field layers in the requested band (below/above the radar), in the fixed
    /// bottom-to-top order, using this pane's camera.
    fn draw_fields(&self, cam: &wgpu::BindGroup, pass: &mut wgpu::RenderPass<'_>, below: bool) {
        for layer in FieldLayer::DRAW_ORDER {
            if layer.below_radar() != below || !self.field_draws.contains(&layer) {
                continue;
            }
            if let Some(f) = self.fields.get(&layer) {
                pass.set_pipeline(&self.mrms_pipeline);
                pass.set_bind_group(0, cam, &[]);
                pass.set_bind_group(1, &f.bind_group, &[]);
                pass.set_vertex_buffer(0, f.vbuf.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
    }

    /// Record one pane's draws (vector basemap → raster tiles → radar → overlay), all using
    /// that pane's camera bind group.
    fn record_pane(&self, id: u32, pass: &mut wgpu::RenderPass<'_>) {
        let Some(pane) = self.panes.get(&id) else { return };
        let cam = &pane.camera_bg;
        // Vector basemap first (opaque, under everything).
        if !pane.frame_visible_vector.is_empty() {
            pass.set_pipeline(&self.overlay_pipeline);
            pass.set_bind_group(0, cam, &[]);
            for tid in &pane.frame_visible_vector {
                if let Some(t) = self.vector_tiles.get(tid) {
                    pass.set_vertex_buffer(0, t.vbuf.slice(..));
                    pass.set_index_buffer(t.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..t.index_count, 0, 0..1);
                }
            }
        }
        pass.set_pipeline(&self.tile_pipeline);
        pass.set_bind_group(0, cam, &[]);
        pass.set_vertex_buffer(0, pane.tile_vbuf.slice(..));
        for (i, v) in pane.frame_visible.iter().enumerate() {
            if let Some(tile) = self.tiles.get(&v.id) {
                pass.set_bind_group(1, &tile.bind_group, &[]);
                let base = (i * 6) as u32;
                pass.draw(base..base + 6, 0..1);
            }
        }
        // Field layers under the radar (national mosaic context).
        self.draw_fields(cam, pass, true);
        if pane.frame_draw_radar {
            if let Some(radar) = &pane.radar {
                pass.set_pipeline(&self.radar_pipeline);
                pass.set_bind_group(0, cam, &[]);
                pass.set_bind_group(1, &radar.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.radar_vbuf.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        // Field layers over the radar (rotation/hail/shear/lightning signals).
        self.draw_fields(cam, pass, false);
        if pane.frame_draw_overlay {
            if let Some(overlay) = &self.overlay {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_bind_group(0, cam, &[]);
                pass.set_vertex_buffer(0, overlay.vbuf.slice(..));
                pass.set_index_buffer(overlay.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..overlay.index_count, 0, 0..1);
            }
        }
    }

    /// Stage a pane's uploads (mirrors the egui `prepare` phase). Public for the headless harness.
    pub fn prepare_pane(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, cb: &MapCallback) {
        self.upload_frame(device, queue, cb);
    }

    /// Draw a previously-prepared pane to `view` (mirrors the egui `paint` phase).
    pub fn draw_pane(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        pane: u32,
        clear: wgpu::Color,
    ) {
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("headless") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_pass"),
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
            self.record_pane(pane, &mut pass);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Render one frame to `view` without a window (used by the headless verify harness).
    pub fn render_once(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        cb: &MapCallback,
        clear: wgpu::Color,
    ) {
        self.prepare_pane(device, queue, cb);
        self.draw_pane(device, queue, view, cb.pane, clear);
    }
}

impl egui_wgpu::CallbackTrait for MapCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res: &mut RenderResources = resources.get_mut().unwrap();
        res.upload_frame(device, queue, self);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let res: &RenderResources = resources.get().unwrap();
        res.record_pane(self.pane, pass);
    }
}
