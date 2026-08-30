//! Wind particles advected on the GPU — the upgrade `wind_draw.rs` names in its header.
//!
//! Two textures ping-pong: a fragment shader reads the current particle positions and writes the
//! next ones. That is the webgl-wind trick, and it is deliberately *not* a compute shader — the
//! device is built with `downlevel_webgl2_defaults`, which zeroes the storage-buffer and
//! workgroup limits outright, so compute is unavailable no matter what the driver supports.
//!
//! Trails come from fading the previous frame's buffer rather than emitting a ribbon per
//! particle, which is what makes a trail cost one quad per particle per frame.
//!
//! The CPU path in [`crate::wind_draw`] stays: it is the fallback when this cannot be built, and
//! `HOOKECHO_CPU_WIND=1` forces it.

use wgpu::util::DeviceExt;

/// Particle texture edge. 64x64 = 4096 particles, at or above the CPU path's 3,000 ceiling, and
/// the whole buffer is 16 KB.
const SIDE: u32 = 64;
/// Particles actually drawn. The rest of the texture is advected anyway (it is one pass either
/// way) and simply not drawn.
pub const COUNT: u32 = SIDE * SIDE;

/// Wind grid resolution uploaded to the GPU. The HRRR grid is much finer than this, but the
/// particles are drawn at a few hundred per pane and read as flow, not as a field.
const GRID: u32 = 256;

/// What the app hands over each frame.
pub struct Frame {
    /// Mercator world bbox of the wind grid.
    pub bbox_min: [f32; 2],
    pub bbox_max: [f32; 2],
    pub center: [f32; 2],
    pub scale: [f32; 2],
    pub dt: f32,
    pub world_per_px: f32,
    pub opacity: f32,
    /// Pane size in physical pixels — the trail buffer matches it.
    pub viewport: (u32, u32),
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    bbox_min: [f32; 2],
    bbox_max: [f32; 2],
    center: [f32; 2],
    scale: [f32; 2],
    dt: f32,
    world_per_px: f32,
    px_per_sec_per_ms: f32,
    max_step_px: f32,
    respawn: f32,
    seed: f32,
    opacity: f32,
    point_px: f32,
}

/// Pipelines and the textures they run over. One instance for the whole app; the trail buffers
/// are per pane, since panes have their own cameras and sizes.
pub struct WindGpu {
    advect: wgpu::RenderPipeline,
    fade: wgpu::RenderPipeline,
    points: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    src_bgl: wgpu::BindGroupLayout,
    uni: wgpu::Buffer,
    wind: wgpu::Texture,
    /// Particle positions, ping-ponged: `pos[cur]` is read, `pos[1 - cur]` written. Reached
    /// through `pos_view` now — the textures are kept because the seeding test writes into them,
    /// and because owning them here says where they live.
    #[allow(dead_code)]
    pos: [wgpu::Texture; 2],
    /// Views and bind groups for both ping-pong slots, built once. Nothing they name is ever
    /// recreated — the grid and the LUT are written into, not rebuilt — so a frame indexes by
    /// `cur` instead of creating three bind groups and two views per pane.
    pos_view: [wgpu::TextureView; 2],
    main_bg: [wgpu::BindGroup; 2],
    pos_src_bg: [wgpu::BindGroup; 2],
    cur: usize,
    /// Per-pane trail buffers, ping-ponged the same way, keyed by pane id.
    trails: std::collections::HashMap<u32, Trails>,
    frames: u64,
}

struct Trails {
    /// The trail buffers themselves. Held rather than dropped in favour of the views below: a
    /// view outliving its texture is not a thing to find out about from a driver.
    #[allow(dead_code)]
    tex: [wgpu::Texture; 2],
    /// Same as the position slots: both views and both source bind groups, built with the
    /// textures they name.
    view: [wgpu::TextureView; 2],
    src_bg: [wgpu::BindGroup; 2],
    cur: usize,
    size: (u32, u32),
}

const POS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn rgba8_target(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: POS_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

impl WindGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/wind.wgsl"));

        let main_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wind_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(4),
            ],
        });
        let src_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wind_src_bgl"),
            entries: &[texture_entry(0)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wind_layout"),
            bind_group_layouts: &[Some(&main_bgl), Some(&src_bgl)],
            immediate_size: 0,
        });

        let pipeline = |label: &str, vs: &str, fs: &str, fmt, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: fmt,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);

        let uni = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wind_uni"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The shared WIND ramp, baked once. Same table the CPU path and the legend use, so the
        // particles and the legend cannot disagree about what a color means.
        let lut_rgba = match &crate::render::field_ramps::WIND.scale {
            crate::render::field_ramps::FieldScale::Ramp { stops, .. } => {
                crate::render::field_ramps::bake_ramp_lut(stops, 255)
            }
            crate::render::field_ramps::FieldScale::Categorical(_) => vec![255; 256 * 4],
        };
        let lut = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("wind_lut"),
                size: wgpu::Extent3d {
                    width: 256,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &lut_rgba,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wind_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let wind = rgba8_target(device, "wind_grid", GRID, GRID);
        let pos = [
            rgba8_target(device, "wind_pos_a", SIDE, SIDE),
            rgba8_target(device, "wind_pos_b", SIDE, SIDE),
        ];
        let pos_view = [
            pos[0].create_view(&Default::default()),
            pos[1].create_view(&Default::default()),
        ];
        let wind_view = wind.create_view(&Default::default());
        let lut_view = lut.create_view(&Default::default());
        let main_bg = std::array::from_fn(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("wind_bg"),
                layout: &main_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uni.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&pos_view[i]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&wind_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&lut_view),
                    },
                ],
            })
        });
        let pos_src_bg = std::array::from_fn(|i| src_bind_group(device, &src_bgl, &pos_view[i]));

        Self {
            advect: pipeline(
                "wind_advect",
                "vs_fullscreen",
                "fs_advect",
                POS_FORMAT,
                None,
            ),
            fade: pipeline("wind_fade", "vs_fullscreen", "fs_fade", POS_FORMAT, None),
            points: pipeline("wind_points", "vs_points", "fs_points", POS_FORMAT, alpha),
            composite: pipeline(
                "wind_composite",
                "vs_fullscreen",
                "fs_composite",
                target_format,
                alpha,
            ),
            src_bgl,
            uni,
            wind,
            pos,
            pos_view,
            main_bg,
            pos_src_bg,
            cur: 0,
            trails: std::collections::HashMap::new(),
            frames: 0,
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl WindGpu {
    /// Upload a warped wind grid. Called when the field changes, not per frame.
    pub fn upload_grid(&mut self, queue: &wgpu::Queue, grid: &WindGrid) {
        let rgba = &grid.rgba;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.wind,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GRID * 4),
                rows_per_image: Some(GRID),
            },
            wgpu::Extent3d {
                width: GRID,
                height: GRID,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Advance the particles and their trails for one pane. Runs in the egui `prepare` phase,
    /// which is where a callback is allowed to record passes of its own.
    pub fn step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pane: u32,
        f: &Frame,
    ) {
        self.frames += 1;
        queue.write_buffer(
            &self.uni,
            0,
            bytemuck::bytes_of(&Uniform {
                bbox_min: f.bbox_min,
                bbox_max: f.bbox_max,
                center: f.center,
                scale: f.scale,
                dt: f.dt.clamp(0.0, 0.25),
                world_per_px: f.world_per_px,
                px_per_sec_per_ms: crate::wind_draw::PX_PER_SEC_PER_MS as f32,
                max_step_px: crate::wind_draw::MAX_STEP_PX as f32,
                // ~2% a step: long enough to trace a streamline, short enough that the field
                // stays evenly covered.
                respawn: 0.02,
                seed: (self.frames % 4096) as f32 * 0.7913,
                opacity: f.opacity,
                point_px: 2.0,
            }),
        );

        // Advect: read the current positions, write the next ones.
        {
            let mut pass = color_pass(encoder, "wind_advect", &self.pos_view[1 - self.cur], None);
            pass.set_pipeline(&self.advect);
            pass.set_bind_group(0, &self.main_bg[self.cur], &[]);
            pass.set_bind_group(1, &self.pos_src_bg[self.cur], &[]);
            pass.draw(0..3, 0..1);
        }
        let read = self.cur;
        self.cur = 1 - self.cur;

        // Trails: fade what was there, then stamp this frame's particles on top.
        self.ensure_trails(device, pane, f.viewport);
        let trails = &self.trails[&pane];
        let (a, b) = (trails.cur, 1 - trails.cur);
        {
            let mut pass = color_pass(
                encoder,
                "wind_trails",
                &trails.view[b],
                Some(wgpu::Color::TRANSPARENT),
            );
            pass.set_pipeline(&self.fade);
            pass.set_bind_group(0, &self.main_bg[read], &[]);
            pass.set_bind_group(1, &trails.src_bg[a], &[]);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&self.points);
            pass.set_bind_group(1, &self.pos_src_bg[self.cur], &[]);
            pass.draw(0..6, 0..COUNT);
        }
        if let Some(t) = self.trails.get_mut(&pane) {
            t.cur = b;
        }
    }

    /// Draw a pane's trail buffer over it. Called from the paint phase, inside egui's pass.
    pub fn draw(&self, pane: u32, pass: &mut wgpu::RenderPass<'_>) {
        let Some(t) = self.trails.get(&pane) else {
            return;
        };
        pass.set_pipeline(&self.composite);
        // Group 0 is here for the uniform; `fs_composite` reads the trail texture and the opacity
        // word, never the position texture, so which ping-pong slot it names does not reach a pixel.
        pass.set_bind_group(0, &self.main_bg[self.cur], &[]);
        pass.set_bind_group(1, &t.src_bg[t.cur], &[]);
        pass.draw(0..3, 0..1);
    }

    /// Forget a pane's trail buffers (the pane closed, or the layout changed).
    pub fn retain_panes(&mut self, keep: impl Fn(u32) -> bool) {
        self.trails.retain(|k, _| keep(*k));
    }

    fn ensure_trails(&mut self, device: &wgpu::Device, pane: u32, size: (u32, u32)) {
        let stale = self
            .trails
            .get(&pane)
            .is_none_or(|t| t.size != (size.0.max(1), size.1.max(1)));
        if stale {
            self.trails.insert(pane, {
                let tex = [
                    rgba8_target(device, "wind_trail_a", size.0, size.1),
                    rgba8_target(device, "wind_trail_b", size.0, size.1),
                ];
                let view: [wgpu::TextureView; 2] =
                    std::array::from_fn(|i| tex[i].create_view(&Default::default()));
                let src_bg =
                    std::array::from_fn(|i| src_bind_group(device, &self.src_bgl, &view[i]));
                Trails {
                    tex,
                    view,
                    src_bg,
                    cur: 0,
                    size: (size.0.max(1), size.1.max(1)),
                }
            });
        }
    }
}

/// The single-texture bind group both the fade and the composite passes read through.
fn src_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wind_src_bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}

/// A wind field warped onto the mercator-uniform texture the shader samples.
///
/// The HRRR grids are regular in lon/lat and the particles live in mercator world space, so the
/// warp has to happen somewhere. Here, on the CPU, once per new field.
pub struct WindGrid {
    pub rgba: Vec<u8>,
    pub bbox_min: [f32; 2],
    pub bbox_max: [f32; 2],
}

/// Resample a wind field onto that grid.
pub fn warp_field(
    field: &crate::wind_draw::WindField,
    bbox_min: [f32; 2],
    bbox_max: [f32; 2],
) -> Vec<u8> {
    let mut rgba = vec![0u8; (GRID * GRID * 4) as usize];
    for gy in 0..GRID {
        for gx in 0..GRID {
            let wx = bbox_min[0] + (bbox_max[0] - bbox_min[0]) * (gx as f32 + 0.5) / GRID as f32;
            let wy = bbox_min[1] + (bbox_max[1] - bbox_min[1]) * (gy as f32 + 0.5) / GRID as f32;
            let (lon, lat) = crate::render::mercator::world_to_lonlat(wx as f64, wy as f64);
            let i = ((gy * GRID + gx) * 4) as usize;
            // Alpha marks "there is wind here": the model domain is not the shape of its bbox,
            // and a particle that wanders outside it should respawn rather than freeze.
            match field.sample(lon, lat) {
                Some((u, v)) => {
                    rgba[i] = encode_component(u);
                    rgba[i + 1] = encode_component(v);
                    rgba[i + 3] = 255;
                }
                None => rgba[i..i + 4].copy_from_slice(&[128, 128, 0, 0]),
            }
        }
    }
    rgba
}

/// ±40 m/s over one byte — about 0.3 m/s, finer than the color ramp resolves.
fn encode_component(ms: f32) -> u8 {
    (((ms / 40.0).clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8
}

fn color_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    view: &wgpu::TextureView,
    clear: Option<wgpu::Color>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: match clear {
                    Some(c) => wgpu::LoadOp::Clear(c),
                    None => wgpu::LoadOp::Load,
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wxdata::mrms::MrmsField;

    fn constant_field(u_ms: f32, v_ms: f32) -> crate::wind_draw::WindField {
        let grid = |v: f32| MrmsField {
            values: vec![v; 4],
            nx: 2,
            ny: 2,
            lon_west: -110.0,
            lon_east: -80.0,
            lat_north: 45.0,
            lat_south: 25.0,
            time: chrono::Utc::now(),
        };
        crate::wind_draw::WindField {
            u: grid(u_ms),
            v: grid(v_ms),
            level: wxdata::hrrr::WindLevel::Surface,
            run: chrono::Utc::now(),
            fcst_hour: 0,
        }
    }

    /// One advection step against a known constant wind, read back off the GPU. No golden image:
    /// what is being checked is the arithmetic (does a particle move the way the wind blows, by
    /// about the right amount), which a picture of trails would only obscure.
    ///
    /// Run with `cargo test -p hookecho -- --ignored gpu`.
    #[test]
    #[ignore = "gpu"]
    fn gpu_wind_advects_downwind() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let instance = wgpu::Instance::default();
        let adapter = rt
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: std::env::var("HOOKECHO_GPU_FALLBACK").is_ok(),
                compatible_surface: None,
            }))
            .expect("no wgpu adapter");
        let (device, queue) = rt
            .block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .unwrap();

        // A box well inside the synthetic field's lon/lat extent, in mercator world units — the
        // shader samples the wind by world position, so a box outside the domain would be a test
        // of the respawn branch instead.
        let nw = crate::render::mercator::lonlat_to_world(-100.0, 40.0);
        let se = crate::render::mercator::lonlat_to_world(-90.0, 35.0);
        let bbox_min = [nw.0 as f32, nw.1 as f32];
        let bbox_max = [se.0 as f32, se.1 as f32];
        let mut gpu = WindGpu::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        // Due east at 10 m/s, everywhere.
        let field = constant_field(10.0, 0.0);
        gpu.upload_grid(
            &queue,
            &WindGrid {
                rgba: warp_field(&field, bbox_min, bbox_max),
                bbox_min,
                bbox_max,
            },
        );

        // Seed every particle at the middle of the domain so the step is unambiguous.
        let seed: Vec<u8> = std::iter::repeat_n([128u8, 0, 128, 0], (SIDE * SIDE) as usize)
            .flatten()
            .collect();
        for tex in &gpu.pos {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &seed,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIDE * 4),
                    rows_per_image: Some(SIDE),
                },
                wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
            );
        }

        // A tenth of a second at 10 m/s: 16 px/s/(m/s) x 10 x 0.1 = 16 px, under the step clamp.
        let world_per_px = 0.000_001;
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        gpu.step(
            &device,
            &queue,
            &mut encoder,
            0,
            &Frame {
                bbox_min,
                bbox_max,
                center: [0.3, 0.4],
                scale: [100.0, 100.0],
                dt: 0.1,
                world_per_px,
                opacity: 1.0,
                viewport: (64, 64),
            },
        );
        queue.submit(Some(encoder.finish()));

        let back = read_positions(&device, &queue, &gpu.pos[gpu.cur]);
        // Most particles stepped east by 16 px worth of world units; a couple of percent were
        // respawned this step and landed anywhere, so this is a majority test, not a universal one.
        // The seed decodes to 128/255, not 0.5 — the texture is where the truth is.
        let start = 128.0 / 255.0;
        let expected = start + 16.0 * world_per_px / (bbox_max[0] - bbox_min[0]);
        let tol = 2.0 / 65535.0; // one step of the two-byte encoding, either way
        let moved = back
            .iter()
            .filter(|(x, y)| (x - expected).abs() < tol && (y - start).abs() < tol)
            .count();
        if moved == 0 {
            eprintln!("first few positions: {:?}", &back[..4.min(back.len())]);
        }
        assert!(
            moved as f32 > back.len() as f32 * 0.9,
            "{moved} of {} particles stepped {expected} east, adapter {}",
            back.len(),
            adapter.get_info().name
        );
    }

    /// Read the position texture back and decode it the way the shader encodes it.
    fn read_positions(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex: &wgpu::Texture,
    ) -> Vec<(f32, f32)> {
        let padded = (SIDE * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wind_readback"),
            size: (padded * SIDE) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(SIDE),
                },
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let mut out = Vec::with_capacity((SIDE * SIDE) as usize);
        for row in 0..SIDE {
            let start = (row * padded) as usize;
            for px in mapped[start..start + (SIDE * 4) as usize].as_chunks::<4>().0 {
                out.push((
                    px[0] as f32 / 255.0 + px[1] as f32 / 255.0 / 255.0,
                    px[2] as f32 / 255.0 + px[3] as f32 / 255.0 / 255.0,
                ));
            }
        }
        drop(mapped);
        buffer.unmap();
        out
    }
}
