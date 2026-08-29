//! wgpu rendering: a slippy-map tile layer plus a polar radar layer, both drawn
//! inside egui's render pass via [`egui_wgpu::CallbackTrait`].

pub mod field_ramps;
pub mod mercator;

use std::collections::HashMap;

use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// XYZ tile id.
pub type TileId = (u8, u32, u32);

/// A tile plus which basemap style it came from. Panes can show different basemaps at once, so
/// the same `(z, x, y)` may be resident twice with different imagery; the style key keeps them
/// apart in the GPU cache.
pub type TileKey = (u8, TileId);

const MAX_TILE_VERTS: u64 = 512 * 6; // up to 512 visible tiles per frame

/// A decoded RGBA tile the app wants uploaded this frame.
pub struct PendingTile {
    pub id: TileId,
    /// Basemap style this tile was fetched for ([`crate::tiles::BasemapStyle::key`]).
    pub style: u8,
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
    ///  smoothing, srv, motion_e, motion_n, tint, flag_nx, flag_ny, flag_west, flag_north,
    ///  flag_east, flag_south, _pad, _pad, _pad] (see `shaders/radar.wgsl`).
    pub uniform: [f32; 20],
    /// 256×3 RGBA color LUT indexed by the sweep's `u8`: row 0 rain (the user's own table),
    /// row 1 snow, row 2 mix. Rows 1 and 2 are copies of row 0 unless the precipitation-type
    /// tint is on (see `colormap::tint_lut`).
    pub lut: Vec<u8>,
    /// MRMS surface precipitation-type classes on their own lat/lon grid, one byte per cell
    /// (0 rain, 1 snow, 2 mix). Empty when the tint is off; a 1×1 dummy is bound instead,
    /// because every binding has to exist whether or not it is read.
    pub precip_flag: Vec<u8>,
    /// World-space quad corners covering the disk (min/max box).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
    /// Only the color table changed: `data` and `precip_flag` are empty and the retained
    /// textures keep their contents. A palette drag writes 3 KB instead of re-uploading the
    /// ~1.3 MB gate texture it was already showing.
    pub lut_only: bool,
}

/// A national gridded field layer (all share the MRMS warp pipeline; they differ only in data,
/// LUT, and draw order). `below_radar` layers paint under the single-site radar; the rest above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FieldLayer {
    Mrms,
    Hrrr,
    Rotation,
    Mesh,
    AzShear,
    Lightning,
    /// Instantaneous precipitation rate (mm/hr) — how hard it is coming down right now,
    /// as against the QPE layers' how much has fallen.
    PrecipRate,
    Qpe1h,
    Qpe24h,
    /// HRRR surface CAPE (environment suite).
    Cape,
    /// HRRR storm-relative helicity (environment suite).
    Srh,
    /// MRMS surface precipitation type (categorical context layer).
    PrecipType,
    /// MRMS FLASH flash-flood average recurrence interval.
    FlashFlood,
    /// Gridded Digital VIL (L3 DVL packet-16 product).
    Vil,
    /// Enhanced Echo Tops (L3 EET packet-16 product).
    EchoTops,
    /// 24-hour MESH max — hail swaths / damage tracks (MRMS).
    HailSwath,
    /// Hybrid Hydrometeor Classification (L3 HHC packet-16 product, categorical).
    Hca,
    /// HRRR max updraft helicity — forecast rotation tracks.
    UpdraftHelicity,
    /// HRRR near-surface smoke (mass density).
    Smoke,
    /// Multi-radar base-reflectivity composite (L3 N0B from every radar covering the view).
    Mosaic,
    /// Column-maximum reflectivity from this radar's own volume — the strongest echo anywhere
    /// above each point. Unlike the MRMS composite this is one site at its own resolution, and
    /// it works in archive replay.
    CompositeLocal,
    /// VIL integrated locally from the Level 2 volume (works in archive replay, unlike L3 DVL).
    VilLocal,
    /// VIL density — VIL over echo-top height, the classic large-hail discriminator.
    VilDensity,
    /// Echo tops integrated locally from the Level 2 volume, at a user-set dBZ threshold.
    EtopLocal,
    /// Maximum Expected Hail Size derived from the volume (Witt et al. 1998).
    HailMehs,
    /// Probability of Severe Hail derived from the volume (Witt et al. 1998).
    HailPosh,
    /// HRRR accumulated snowfall through the scrubbed forecast hour.
    Snowfall,
    /// NOHRSC observed snowfall analysis over the last 6/24/48/72 hours.
    SnowAnalysis,
    /// Global model mean sea-level pressure (GFS or ECMWF — see `settings.global_model`).
    GlobalMslp,
    /// Global model 500 hPa geopotential height.
    GlobalHeight500,
    /// Global model 2 m temperature.
    GlobalTemp2m,
    /// Global model 10 m wind speed.
    GlobalWind10m,
    /// Global model precipitable water / total precipitation.
    GlobalPrecip,
    /// Banded precipitation from the MRMS mosaic, narrowed to snow — the snow-squall layer.
    SnowBands,
    /// NBM calibrated probability of thunder over the hour ending at the scrubbed forecast hour.
    ThunderProb,
    /// GLM flash-extent density — the recent satellite flashes gridded into a density field.
    GlmFed,
    /// One model minus another — which field, and therefore which pair, is `app.diff_field`.
    ModelDiff,
}

impl FieldLayer {
    /// Painting under the single-site radar (national context) vs. over it (severe signals).
    pub fn below_radar(self) -> bool {
        matches!(
            self,
            FieldLayer::Mrms
                | FieldLayer::Mosaic
                | FieldLayer::Hrrr
                | FieldLayer::Cape
                | FieldLayer::Srh
                | FieldLayer::PrecipType
                | FieldLayer::ThunderProb
                | FieldLayer::Smoke
                | FieldLayer::Snowfall
                | FieldLayer::SnowAnalysis
                | FieldLayer::GlobalMslp
                | FieldLayer::GlobalHeight500
                | FieldLayer::GlobalTemp2m
                | FieldLayer::GlobalWind10m
                | FieldLayer::GlobalPrecip
                | FieldLayer::ModelDiff
        )
    }

    /// Fixed bottom-to-top paint order within each band.
    pub const DRAW_ORDER: [FieldLayer; 37] = [
        // Below-radar context band (bottom to top). The global models sit at the very bottom:
        // they are the synoptic backdrop everything else is drawn against.
        FieldLayer::GlobalMslp,
        FieldLayer::GlobalHeight500,
        FieldLayer::GlobalTemp2m,
        FieldLayer::GlobalWind10m,
        FieldLayer::GlobalPrecip,
        FieldLayer::ModelDiff,
        FieldLayer::Mrms,
        FieldLayer::Mosaic,
        FieldLayer::Hrrr,
        FieldLayer::Cape,
        FieldLayer::Srh,
        FieldLayer::Smoke,
        FieldLayer::Snowfall,
        FieldLayer::SnowAnalysis,
        FieldLayer::PrecipType,
        FieldLayer::ThunderProb,
        // Above-radar severe-signal band.
        FieldLayer::SnowBands,
        FieldLayer::PrecipRate,
        FieldLayer::Qpe1h,
        FieldLayer::Qpe24h,
        FieldLayer::FlashFlood,
        FieldLayer::HailSwath,
        FieldLayer::CompositeLocal,
        FieldLayer::Vil,
        FieldLayer::VilLocal,
        FieldLayer::EchoTops,
        FieldLayer::EtopLocal,
        FieldLayer::VilDensity,
        FieldLayer::HailMehs,
        FieldLayer::HailPosh,
        FieldLayer::Hca,
        FieldLayer::UpdraftHelicity,
        FieldLayer::Rotation,
        FieldLayer::Mesh,
        FieldLayer::AzShear,
        FieldLayer::Lightning,
        FieldLayer::GlmFed,
    ];

    /// Stable name for saved files — a workspace records which layers were on by slug, so a file
    /// written by a newer build names a layer this one skips rather than failing to load.
    pub fn slug(self) -> &'static str {
        match self {
            FieldLayer::Mrms => "mrms",
            FieldLayer::Hrrr => "hrrr",
            FieldLayer::Rotation => "rotation",
            FieldLayer::Mesh => "mesh",
            FieldLayer::AzShear => "azshear",
            FieldLayer::Lightning => "lightning",
            FieldLayer::PrecipRate => "preciprate",
            FieldLayer::Qpe1h => "qpe1h",
            FieldLayer::Qpe24h => "qpe24h",
            FieldLayer::Cape => "cape",
            FieldLayer::Srh => "srh",
            FieldLayer::PrecipType => "preciptype",
            FieldLayer::FlashFlood => "flashflood",
            FieldLayer::Vil => "vil",
            FieldLayer::EchoTops => "echotops",
            FieldLayer::HailSwath => "hailswath",
            FieldLayer::SnowBands => "snowbands",
            FieldLayer::ThunderProb => "thunderprob",
            FieldLayer::Hca => "hca",
            FieldLayer::UpdraftHelicity => "updrafthelicity",
            FieldLayer::Smoke => "smoke",
            FieldLayer::Mosaic => "mosaic",
            FieldLayer::CompositeLocal => "composite-local",
            FieldLayer::VilLocal => "vil-local",
            FieldLayer::VilDensity => "vil-density",
            FieldLayer::EtopLocal => "etop-local",
            FieldLayer::HailMehs => "hail-mehs",
            FieldLayer::HailPosh => "hail-posh",
            FieldLayer::Snowfall => "snowfall",
            FieldLayer::SnowAnalysis => "snow-analysis",
            FieldLayer::GlobalMslp => "global-mslp",
            FieldLayer::GlobalHeight500 => "global-height500",
            FieldLayer::GlobalTemp2m => "global-temp2m",
            FieldLayer::GlobalWind10m => "global-wind10m",
            FieldLayer::GlobalPrecip => "global-precip",
            FieldLayer::ModelDiff => "model-diff",
            FieldLayer::GlmFed => "glm-fed",
        }
    }

    /// The inverse of [`slug`](Self::slug), or `None` for a name this build doesn't have.
    pub fn from_slug(s: &str) -> Option<FieldLayer> {
        Self::DRAW_ORDER.into_iter().find(|f| f.slug() == s)
    }
}

#[cfg(test)]
mod field_slug_tests {
    use super::FieldLayer;

    #[test]
    fn every_layer_has_a_slug_that_parses_back() {
        for l in FieldLayer::DRAW_ORDER {
            assert_eq!(FieldLayer::from_slug(l.slug()), Some(l), "{}", l.slug());
        }
        let mut slugs: Vec<&str> = FieldLayer::DRAW_ORDER.iter().map(|l| l.slug()).collect();
        slugs.sort_unstable();
        let n = slugs.len();
        slugs.dedup();
        assert_eq!(slugs.len(), n, "two layers share a slug");
        assert_eq!(FieldLayer::from_slug("not-a-layer"), None);
    }
}

/// A national MRMS mosaic to upload: an R8 index grid + LUT, warped plate-carrée→mercator.
pub struct MrmsUpload {
    pub data: Vec<u8>,
    pub nx: u32,
    pub ny: u32,
    /// World-space quad (mercator bbox of the grid).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
    /// [lon_west, lat_north, lon_east, lat_south, nx, ny, opacity, +5 pad] (see
    /// `shaders/mrms.wgsl`). Opacity is rewritten each frame from `field_draws`, so the value
    /// here only matters until the first draw.
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
    /// Which basemap style this pane draws ([`crate::tiles::BasemapStyle::key`]).
    pub basemap_key: u8,
    /// Draw the vector basemap *after* the raster tiles instead of under them — the hybrid
    /// satellite style, where the vector geometry is roads over imagery.
    pub vector_over_raster: bool,
    pub radar_upload: Option<RadarUpload>,
    pub draw_radar: bool,
    /// `Some` only when the overlay geometry changed (else the last upload is reused).
    pub overlay_upload: Option<OverlayUpload>,
    pub draw_overlay: bool,
    /// Drop all cached GPU tiles before uploading (basemap style changed).
    pub clear_tiles: bool,
    /// Individual tiles the tile manager evicted; freed before this frame's uploads. Eviction is
    /// decided there, not here, so both sides agree on what is resident.
    pub drop_tiles: Vec<TileKey>,
    /// Field layers whose grid changed this frame (uploaded now); others reuse the last upload.
    pub field_uploads: Vec<(FieldLayer, MrmsUpload)>,
    /// Which field layers to paint this frame, with their opacity (0..1).
    pub field_draws: Vec<(FieldLayer, f32)>,
    /// Newly tessellated vector basemap tiles to upload this frame.
    pub new_vector_tiles: Vec<PendingVectorTile>,
    /// Vector tile ids to draw this frame (drawn first, under the raster/radar layers).
    pub visible_vector: Vec<TileId>,
    /// Drop all cached vector tiles before uploading (style or tess-zoom changed).
    pub clear_vector: bool,
    /// Individual vector tiles the manager evicted; freed before this frame's uploads.
    pub drop_vector_tiles: Vec<TileId>,
    /// A wind field to (re)upload for the GPU particle layer, with its mercator world bbox.
    pub wind_upload: Option<Box<crate::wind_gpu::WindGrid>>,
    /// Advance and draw the GPU wind particles this frame.
    pub wind: Option<crate::wind_gpu::Frame>,
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
    tex: wgpu::Texture,
    flag: wgpu::Texture,
    lut: wgpu::Texture,
    uni: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// (gate_count, az_bins) and the precipitation-flag grid size, so a later upload of the same
    /// shape writes into these textures instead of building new ones with a new bind group.
    dims: (u32, u32),
    flag_dims: (u32, u32),
}

struct OverlayGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
}

struct MrmsGpu {
    _tex: wgpu::Texture,
    _lut: wgpu::Texture,
    /// Kept (not `_`-prefixed) so `prepare` can rewrite the opacity word each frame.
    uni: wgpu::Buffer,
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
    /// Ids of the tiles this frame's quads draw, in quad order. Not the same as the visible list:
    /// a missing tile is stood in for by resident children or an ancestor.
    frame_visible: Vec<TileKey>,
    /// What the tile quads in `tile_vbuf` were built from: tile-cache generation, basemap, camera
    /// and visible count. Unchanged means the quads are still the right ones, so a still map
    /// re-uploads nothing.
    // ponytail: the visible *count* rather than the list — camera plus generation already decide
    // which tiles are asked for; compare the ids if a case ever shows a stale quad.
    quads_key: Option<(u64, u8, u32, u32, u32, u32, usize)>,
    frame_visible_vector: Vec<TileId>,
    vector_over_raster: bool,
    frame_draw_radar: bool,
    frame_draw_overlay: bool,
    /// Field layers this pane draws this frame. Per-pane, not shared: two panes are how you look
    /// at two fields at once.
    field_draws: Vec<FieldLayer>,
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
    mrms_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    // Shared across panes: tile image cache, vector tile geometry, and the world-space overlay
    // (severe weather + placefiles) — the overlay is camera-independent, drawn per-pane camera.
    //
    // Plain maps, bounded from the CPU side: `TileManager` owns the eviction policy and hands us
    // the ids to free (`drop_tiles`). Evicting here instead was a bug — the manager went on
    // believing an evicted tile was still uploaded and never re-sent it, so zooming back to an
    // area left black squares where those tiles used to be.
    tiles: HashMap<TileKey, TileGpu>,
    vector_tiles: HashMap<TileId, OverlayGpu>,
    overlay: Option<OverlayGpu>,
    fields: HashMap<FieldLayer, MrmsGpu>,
    // One entry per live pane.
    panes: HashMap<u32, PaneGpu>,
    /// Bumped whenever the shared tile cache gains or loses a texture, so a pane can tell that
    /// its quad list is still current.
    tiles_gen: u64,
    /// GPU wind particles, built on first use. `None` when the CPU path is in charge.
    wind: Option<crate::wind_gpu::WindGpu>,
    target_format: wgpu::TextureFormat,
}

impl RenderResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let tile_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/tiles.wgsl"));
        let radar_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/radar.wgsl"));
        let overlay_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/overlay.wgsl"));
        let mrms_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/mrms.wgsl"));

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
                        min_binding_size: NonZeroU64::new(80),
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
                // The precipitation-type grid. Always bound — a binding cannot be conditional —
                // and 1×1 when the tint is off.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
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

        // The mosaic shares radar.wgsl's shape — uniform, index grid, LUT — but not its layout:
        // radar_bgl has since grown a precipitation-type texture and an 80-byte uniform, and
        // borrowing it meant the mosaic had to carry a dummy texture and dead padding that
        // nobody remembered to keep in step. It got out of step, and every MRMS draw died in
        // bind-group validation. Its own three-binding layout cannot drift.
        let mrms_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mrms_bgl"),
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
        let mrms_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mrms_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&mrms_bgl)],
            immediate_size: 0,
        });
        let mrms_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mrms_pipeline"),
            layout: Some(&mrms_layout),
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
            mrms_bgl,
            sampler,
            tiles: HashMap::new(),
            vector_tiles: HashMap::new(),
            overlay: None,
            fields: HashMap::new(),
            panes: HashMap::new(),
            tiles_gen: 0,
            wind: None,
            target_format,
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
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                }],
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
                quads_key: None,
                frame_visible_vector: Vec::new(),
                vector_over_raster: false,
                frame_draw_radar: false,
                frame_draw_overlay: false,
                field_draws: Vec::new(),
            }
        })
    }

    /// Quads to draw for one visible tile: normally itself, but if its texture hasn't arrived,
    /// whatever resident neighbours in the pyramid cover the same ground.
    ///
    /// Children first (pinch-out: the level we came from is still resident and sharper), each
    /// covering its quarter with the full texture. Otherwise the nearest resident ancestor, up to
    /// three levels up, drawn once with its UVs cropped to the part this tile occupies. Without
    /// this the whole basemap blanks every time the integer zoom level changes, even though the
    /// pixels to show are sitting on the GPU.
    #[allow(clippy::type_complexity)] // one call site; a struct here would be ceremony
    fn tile_quads(
        &self,
        style: u8,
        v: &VisibleTile,
    ) -> Vec<(TileKey, [f32; 2], [f32; 2], [f32; 2], [f32; 2])> {
        const FULL: ([f32; 2], [f32; 2]) = ([0.0, 0.0], [1.0, 1.0]);
        if self.tiles.contains_key(&(style, v.id)) {
            return vec![((style, v.id), v.world_min, v.world_max, FULL.0, FULL.1)];
        }
        let (z, x, y) = v.id;
        let [x0, y0] = v.world_min;
        let [x1, y1] = v.world_max;
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let kids: Vec<_> = [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
            .into_iter()
            .filter_map(|(dx, dy)| {
                let id = (style, (z + 1, x * 2 + dx, y * 2 + dy));
                self.tiles.contains_key(&id).then(|| {
                    let (qx0, qx1) = if dx == 0 { (x0, mx) } else { (mx, x1) };
                    let (qy0, qy1) = if dy == 0 { (y0, my) } else { (my, y1) };
                    (id, [qx0, qy0], [qx1, qy1], FULL.0, FULL.1)
                })
            })
            .collect();
        if !kids.is_empty() {
            return kids;
        }
        for up in 1..=3u8 {
            if up > z {
                break;
            }
            let id = (style, (z - up, x >> up, y >> up));
            if self.tiles.contains_key(&id) {
                let (uv_min, uv_max) = ancestor_uv(x, y, up);
                return vec![(id, v.world_min, v.world_max, uv_min, uv_max)];
            }
        }
        Vec::new()
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
        self.tiles.insert(
            (t.style, t.id),
            TileGpu {
                _tex: tex,
                bind_group,
            },
        );
    }

    /// Upload one sweep for a pane, reusing `existing` when it is the same shape.
    ///
    /// Returns `None` only for a LUT-only upload with nothing retained to write into — which
    /// the app does not ask for (it only sends one when the same key is already shown), but is
    /// answered by leaving the pane's radar alone rather than by a panic.
    fn build_radar(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        r: &RadarUpload,
        existing: Option<RadarGpu>,
    ) -> Option<RadarGpu> {
        let flag_dims = flag_dims(r);
        // Same shape: write into the retained textures and keep the bind group. Dimensions are
        // the only thing a bind group depends on here, so nothing else can go stale.
        if let Some(g) = existing {
            if g.dims == (r.gate_count, r.az_bins) && g.flag_dims == flag_dims {
                if !r.lut_only {
                    write_r8(queue, &g.tex, g.dims, &r.data);
                    write_r8(queue, &g.flag, g.flag_dims, &flag_bytes(r, g.flag_dims));
                }
                queue.write_buffer(&g.uni, 0, bytemuck::cast_slice(&r.uniform));
                write_lut(queue, &g.lut, &r.lut);
                return Some(g);
            }
        }
        if r.lut_only {
            return None;
        }
        wxdata::stats::bump(wxdata::stats::Counter::RadarTexturesBuilt);
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
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // 256×3 color LUT, indexed by the sweep u8 across and the precipitation type down.
        let lut_size = wgpu::Extent3d {
            width: 256,
            height: 3,
            depth_or_array_layers: 1,
        };
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
                rows_per_image: Some(3),
            },
            lut_size,
        );

        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Precipitation-type grid. One byte per cell; 1×1 of zeroes when the tint is off, which
        // the shader never reads because the tint flag in the uniform is zero too.
        let (fnx, fny) = flag_dims;
        let flag_data = flag_bytes(r, flag_dims);
        let flag_size = wgpu::Extent3d {
            width: fnx,
            height: fny,
            depth_or_array_layers: 1,
        };
        let flag_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar_precip_flag"),
            size: flag_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &flag_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &flag_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(fnx),
                rows_per_image: Some(fny),
            },
            flag_size,
        );
        let flag_view = flag_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar_bg"),
            layout: &self.radar_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uni.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&flag_view),
                },
            ],
        });
        Some(RadarGpu {
            tex,
            flag: flag_tex,
            lut: lut_tex,
            uni,
            bind_group,
            dims: (r.gate_count, r.az_bins),
            flag_dims,
        })
    }

    /// Upload camera/tiles/radar for `cb` and stage its pane's draw list. Shared caches (tiles,
    /// vector tiles, overlay) update once; per-pane state (camera, radar, tile quads) is keyed
    /// by `cb.pane`. Shared by the egui callback and the headless renderer.
    /// Advance the GPU wind particles for this pane, building the layer on first use. Separate
    /// from `upload_frame` because it needs an encoder of its own — the advection is a render
    /// pass, and egui's `prepare` is the one place a callback may record one.
    fn step_wind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cb: &MapCallback,
    ) {
        if cb.wind.is_none() && cb.wind_upload.is_none() {
            return;
        }
        let format = self.target_format;
        let wind = self
            .wind
            .get_or_insert_with(|| crate::wind_gpu::WindGpu::new(device, queue, format));
        if let Some(up) = &cb.wind_upload {
            wind.upload_grid(queue, up);
        }
        if let Some(frame) = &cb.wind {
            wind.step(device, queue, encoder, cb.pane, frame);
        }
    }

    /// The tile quads for this frame, and the tile ids they draw in quad order.
    fn tile_verts(&self, cb: &MapCallback) -> (Vec<TileVertex>, Vec<TileKey>) {
        wxdata::stats::bump(wxdata::stats::Counter::TileQuadsBuilt);
        let mut tverts: Vec<TileVertex> = Vec::new();
        let mut visible: Vec<TileKey> = Vec::new();
        for v in &cb.visible {
            for (id, wmin, wmax, uvmin, uvmax) in self.tile_quads(cb.basemap_key, v) {
                if tverts.len() as u64 + 6 > MAX_TILE_VERTS {
                    break;
                }
                let ([x0, y0], [x1, y1]) = (wmin, wmax);
                let ([u0, t0], [u1, t1]) = (uvmin, uvmax);
                tverts.extend_from_slice(&[
                    TileVertex {
                        world: [x0, y0],
                        uv: [u0, t0],
                    },
                    TileVertex {
                        world: [x1, y0],
                        uv: [u1, t0],
                    },
                    TileVertex {
                        world: [x1, y1],
                        uv: [u1, t1],
                    },
                    TileVertex {
                        world: [x0, y0],
                        uv: [u0, t0],
                    },
                    TileVertex {
                        world: [x1, y1],
                        uv: [u1, t1],
                    },
                    TileVertex {
                        world: [x0, y1],
                        uv: [u0, t1],
                    },
                ]);
                visible.push(id);
            }
        }
        (tverts, visible)
    }

    fn upload_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, cb: &MapCallback) {
        // --- Shared caches ---
        if cb.clear_tiles {
            self.tiles.clear();
        }
        if cb.clear_vector {
            self.vector_tiles.clear();
        }
        for id in &cb.drop_vector_tiles {
            self.vector_tiles.remove(id);
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
        // Grow to fit the frame before touching anything. A zoomed-out view can need more tiles
        // than the resting cap, and promoting a visible tile into a full cache evicts another
        // visible one — which showed up as a band of basemap with the rest of the map bare.
        // The cap is a resting size, not a limit on what one frame may hold.
        for id in &cb.drop_tiles {
            self.tiles.remove(id);
        }
        if cb.clear_tiles || !cb.new_tiles.is_empty() || !cb.drop_tiles.is_empty() {
            self.tiles_gen = self.tiles_gen.wrapping_add(1);
        }
        // Touch everything this frame draws so the LRU evicts what is off screen, not what is in
        // front of the user. Visible entries can't be evicted mid-frame: the caches only shrink
        // here, before any drawing.
        for (layer, up) in &cb.field_uploads {
            let gpu = self.build_field_layer(device, queue, up);
            self.fields.insert(*layer, gpu);
        }
        // Draw only the requested layers that actually have GPU data. Opacity rides in the grid
        // uniform's first pad word, so it costs one 4-byte write per drawn layer — no LUT re-bake.
        let mut field_draws = Vec::new();
        for (layer, opacity) in &cb.field_draws {
            if let Some(f) = self.fields.get(layer) {
                queue.write_buffer(&f.uni, 24, &opacity.to_le_bytes());
                field_draws.push(*layer);
            }
        }

        // --- Per-pane state ---
        let new_radar = match cb.radar_upload.as_ref() {
            Some(r) => {
                let existing = self.panes.get_mut(&cb.pane).and_then(|p| p.radar.take());
                self.build_radar(device, queue, r, existing)
            }
            None => None,
        };
        // Build the tile quad list against the shared tile cache before mutably borrowing the pane
        // — a tile with no texture yet borrows one from the tiles around it (see `tile_quads`).
        // Skipped outright when nothing it depends on moved: a still map rebuilt up to 512 tiles
        // worth of vertices and re-uploaded them every heartbeat frame.
        let quads_key = (
            self.tiles_gen,
            cb.basemap_key,
            cb.camera_center[0].to_bits(),
            cb.camera_center[1].to_bits(),
            cb.camera_scale[0].to_bits(),
            cb.camera_scale[1].to_bits(),
            cb.visible.len(),
        );
        let quads = (self.panes.get(&cb.pane).map(|p| p.quads_key) != Some(Some(quads_key)))
            .then(|| self.tile_verts(cb));
        let overlay_present = self.overlay.is_some();

        let pane = self.pane_mut(device, cb.pane);
        queue.write_buffer(
            &pane.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform {
                center: cb.camera_center,
                scale: cb.camera_scale,
            }),
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
        if let Some((tverts, visible)) = quads {
            if !tverts.is_empty() {
                queue.write_buffer(&pane.tile_vbuf, 0, bytemuck::cast_slice(&tverts));
            }
            pane.frame_visible = visible;
            pane.quads_key = Some(quads_key);
        }
        pane.frame_visible_vector = cb.visible_vector.clone();
        pane.vector_over_raster = cb.vector_over_raster;
        pane.frame_draw_radar = cb.draw_radar && pane.radar.is_some();
        pane.frame_draw_overlay = cb.draw_overlay && overlay_present;
        pane.field_draws = field_draws;
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
        self.overlay = Some(OverlayGpu {
            vbuf,
            ibuf,
            index_count: o.indices.len() as u32,
        });
    }

    /// Build a national field-layer (MRMS mosaic or lightning): R8 index texture + LUT texture +
    /// grid uniform + full-grid quad. Shared by both layers; they differ only in data + LUT.
    fn build_field_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        m: &MrmsUpload,
    ) -> MrmsGpu {
        let size = wgpu::Extent3d {
            width: m.nx,
            height: m.ny,
            depth_or_array_layers: 1,
        };
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
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &m.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(m.nx),
                rows_per_image: Some(m.ny),
            },
            size,
        );
        let uni = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mrms_uniform"),
            contents: bytemuck::cast_slice(&m.uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lut_size = wgpu::Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        };
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
            wgpu::TexelCopyTextureInfo {
                texture: &lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &m.lut,
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
            label: Some("mrms_bg"),
            layout: &self.mrms_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uni.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
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
        MrmsGpu {
            _tex: tex,
            _lut: lut_tex,
            uni,
            bind_group,
            vbuf,
        }
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
        self.vector_tiles.insert(
            t.id,
            OverlayGpu {
                vbuf,
                ibuf,
                index_count: t.indices.len() as u32,
            },
        );
    }

    /// Paint the active field layers in the requested band (below/above the radar), in the fixed
    /// bottom-to-top order, using this pane's camera.
    fn draw_fields(&self, pane: &PaneGpu, pass: &mut wgpu::RenderPass<'_>, below: bool) {
        let cam = &pane.camera_bg;
        for layer in FieldLayer::DRAW_ORDER {
            if layer.below_radar() != below || !pane.field_draws.contains(&layer) {
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
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        let cam = &pane.camera_bg;
        // Vector basemap first (opaque, under everything) — unless it is the hybrid overlay, in
        // which case it is roads drawn *onto* the raster imagery and has to come after it.
        if !pane.vector_over_raster {
            self.draw_vector_basemap(pane, cam, pass);
        }
        pass.set_pipeline(&self.tile_pipeline);
        pass.set_bind_group(0, cam, &[]);
        pass.set_vertex_buffer(0, pane.tile_vbuf.slice(..));
        for (i, id) in pane.frame_visible.iter().enumerate() {
            if let Some(tile) = self.tiles.get(id) {
                pass.set_bind_group(1, &tile.bind_group, &[]);
                let base = (i * 6) as u32;
                pass.draw(base..base + 6, 0..1);
            }
        }
        if pane.vector_over_raster {
            self.draw_vector_basemap(pane, cam, pass);
        }
        // Field layers under the radar (national mosaic context).
        self.draw_fields(pane, pass, true);
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
        self.draw_fields(pane, pass, false);
        // Wind particles under the overlay, not over it: the CPU path paints in egui's own layer,
        // which is above everything, and a warning polygon disappearing under a particle trail
        // was the one complaint that layer ever attracted.
        if let Some(wind) = &self.wind {
            wind.draw(id, pass);
        }
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
    /// Draw one pane's resident vector-basemap tiles. Extracted so the caller can put it either
    /// side of the raster tiles (see [`MapCallback::vector_over_raster`]).
    fn draw_vector_basemap(
        &self,
        pane: &PaneGpu,
        cam: &wgpu::BindGroup,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        if pane.frame_visible_vector.is_empty() {
            return;
        }
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
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("headless"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
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
        crate::prof_scope!("render prepare");
        let res: &mut RenderResources = resources.get_mut().unwrap();
        res.upload_frame(device, queue, self);
        res.step_wind(device, queue, _encoder, self);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        crate::prof_scope!("render paint");
        let res: &RenderResources = resources.get().unwrap();
        res.record_pane(self.pane, pass);
    }
}

/// Where tile `(x, y)` sits inside the ancestor `up` levels above it, in that ancestor's UV space.
/// `up = 1` gives one of four quadrants, `up = 2` one of sixteen, and so on.
fn ancestor_uv(x: u32, y: u32, up: u8) -> ([f32; 2], [f32; 2]) {
    let n = (1u32 << up) as f32;
    let fx = (x & ((1 << up) - 1)) as f32 / n;
    let fy = (y & ((1 << up) - 1)) as f32 / n;
    ([fx, fy], [fx + 1.0 / n, fy + 1.0 / n])
}

#[cfg(test)]
mod tests {
    use super::ancestor_uv;

    #[test]
    fn ancestor_uv_picks_the_right_quadrant() {
        // Direct parent: (x, y) = (3, 2) is the odd column, even row -> right/top quadrant.
        assert_eq!(ancestor_uv(3, 2, 1), ([0.5, 0.0], [1.0, 0.5]));
        assert_eq!(ancestor_uv(2, 3, 1), ([0.0, 0.5], [0.5, 1.0]));
        // Two levels up: sixteenths, and only the low bits matter.
        assert_eq!(ancestor_uv(0, 0, 2), ([0.0, 0.0], [0.25, 0.25]));
        assert_eq!(ancestor_uv(7, 5, 2), ([0.75, 0.25], [1.0, 0.5]));
        // Three levels up: the whole ancestor is still covered exactly.
        let (min, max) = ancestor_uv(7, 7, 3);
        assert_eq!(min, [0.875, 0.875]);
        assert_eq!(max, [1.0, 1.0]);
    }
}

/// The precipitation-flag grid size for an upload: the grid the uniform declares when the tint
/// is on and the bytes match it, else the 1×1 dummy every binding needs whether or not it is read.
fn flag_dims(r: &RadarUpload) -> (u32, u32) {
    let (nx, ny) = (r.uniform[11] as u32, r.uniform[12] as u32);
    match r.precip_flag.len() == (nx * ny) as usize && nx > 0 {
        true => (nx, ny),
        false => (1, 1),
    }
}

/// The precipitation-flag bytes to write for `dims` (the dummy grid is a single zero).
fn flag_bytes(r: &RadarUpload, dims: (u32, u32)) -> Vec<u8> {
    match dims == (1, 1) && r.precip_flag.len() != 1 {
        true => vec![0u8],
        false => r.precip_flag.clone(),
    }
}

/// Write a full R8 texture of `dims`.
fn write_r8(queue: &wgpu::Queue, tex: &wgpu::Texture, dims: (u32, u32), data: &[u8]) {
    let (width, height) = dims;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Write the 256×3 RGBA color LUT.
fn write_lut(queue: &wgpu::Queue, tex: &wgpu::Texture, lut: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        lut,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256 * 4),
            rows_per_image: Some(3),
        },
        wgpu::Extent3d {
            width: 256,
            height: 3,
            depth_or_array_layers: 1,
        },
    );
}
