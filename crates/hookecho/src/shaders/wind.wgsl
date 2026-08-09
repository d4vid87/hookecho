// GPU wind particles: the webgl-wind technique the note in `wind_draw.rs` names as the upgrade
// path. Positions live in a small RGBA8 texture and are advected by a fragment shader that reads
// one texture and writes the other; nothing here needs a compute shader or a storage buffer,
// which the device (built with `downlevel_webgl2_defaults`) does not have.
//
// Everything is 8-bit-encodable on purpose: RGBA8 is the one color format guaranteed renderable
// on WebGL2 without an extension. A position is two bytes per axis — 1/65536 of the wind domain,
// far finer than a pixel at any zoom — and a wind component is one byte over ±40 m/s, about
// 0.3 m/s, which is finer than the color ramp resolves.

struct Wind {
    /// Mercator world bbox of the wind grid: min, then max.
    bbox_min: vec2<f32>,
    bbox_max: vec2<f32>,
    /// Camera, as everywhere else: clip = (world - center) * scale.
    center: vec2<f32>,
    scale: vec2<f32>,
    /// Seconds since the last advection step, and world units per screen pixel. Speed is
    /// calibrated in pixels (see `wind_draw`), so the conversion needs this frame's camera.
    dt: f32,
    world_per_px: f32,
    /// Screen pixels per second per m/s of wind, and the per-step clamp in pixels.
    px_per_sec_per_ms: f32,
    max_step_px: f32,
    /// Share of the particles respawned each step, and this frame's seed.
    respawn: f32,
    seed: f32,
    /// Layer opacity, and one particle's size in pixels.
    opacity: f32,
    point_px: f32,
};

@group(0) @binding(0) var<uniform> w: Wind;
@group(0) @binding(1) var pos_tex: texture_2d<f32>;
@group(0) @binding(2) var wind_tex: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
@group(0) @binding(4) var lut_tex: texture_2d<f32>;

/// The texture a full-screen pass reads: the previous trail buffer.
@group(1) @binding(0) var src_tex: texture_2d<f32>;

/// Position in [0,1)² of the wind bbox, packed two bytes per axis.
fn decode_pos(c: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(c.r + c.g / 255.0, c.b + c.a / 255.0);
}

fn encode_pos(p: vec2<f32>) -> vec4<f32> {
    // High byte then low byte, so `decode_pos` is exactly the inverse. Splitting with `fract`
    // alone loses the carry and drifts a couple of thousandths per step, which over a few hundred
    // steps is a particle sliding sideways through the wind.
    let c = clamp(p, vec2<f32>(0.0), vec2<f32>(0.999984));
    let hi = floor(c * 255.0) / 255.0;
    let lo = fract(c * 255.0);
    return vec4<f32>(hi.x, lo.x, hi.y, lo.y);
}

/// Wind at a normalized position, in m/s, bilinear across grid cells.
fn wind_at(p: vec2<f32>) -> vec2<f32> {
    let c = textureSampleLevel(wind_tex, smp, p, 0.0);
    return (c.rg * 2.0 - 1.0) * 40.0;
}

/// A cheap hash in [0,1), for respawn positions. Not random enough for anything that matters, and
/// nothing here matters — the same argument the CPU path's xorshift makes.
fn hash(v: vec2<f32>) -> f32 {
    return fract(sin(dot(v, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

struct FsIn {
    @builtin(position) frag: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

/// Fullscreen triangle; no vertex buffer.
@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    var out: FsIn;
    let xy = vec2<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0);
    out.frag = vec4<f32>(xy, 0.0, 1.0);
    // Framebuffer v runs the other way from clip-space y.
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

// ---------------------------------------------------------------- advection

@fragment
fn fs_advect(in: FsIn) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.frag.xy);
    let p = decode_pos(textureLoad(pos_tex, texel, 0));
    let uv = wind_at(p);
    let speed = length(uv);

    // Screen-space calibration converted to world units through this frame's camera: the same
    // constants the CPU path uses, so switching paths does not change how fast anything looks.
    var step_px = uv * (w.px_per_sec_per_ms * w.dt);
    let len_px = length(step_px);
    if (len_px > w.max_step_px) {
        step_px = step_px * (w.max_step_px / max(len_px, 1e-6));
    }
    let span = w.bbox_max - w.bbox_min;
    // +v is north and world y grows southward, so the vertical step is negated.
    let d = vec2<f32>(step_px.x, -step_px.y) * w.world_per_px / span;

    var next = p + d;
    // Respawn particles that leave the domain, particles in dead air, and a slice of the rest
    // each step — without that last one every particle ends up on the same convergence line.
    let r = hash(vec2<f32>(f32(texel.x) + w.seed, f32(texel.y) - w.seed));
    if (next.x < 0.0 || next.x > 1.0 || next.y < 0.0 || next.y > 1.0 || speed < 0.2 || r < w.respawn) {
        next = vec2<f32>(
            hash(vec2<f32>(f32(texel.x) * 1.7 + w.seed, f32(texel.y) * 0.3)),
            hash(vec2<f32>(f32(texel.y) * 2.3 - w.seed, f32(texel.x) * 0.9))
        );
    }
    return encode_pos(next);
}

// -------------------------------------------------------------------- trails

/// Redraw the previous trail buffer, dimmed. Fading in screen space is what makes a trail cost
/// one point per particle per frame instead of a twelve-vertex ribbon.
@fragment
fn fs_fade(in: FsIn) -> @location(0) vec4<f32> {
    return textureSampleLevel(src_tex, smp, in.uv, 0.0) * 0.94;
}

/// Draw the trail buffer over the pane.
@fragment
fn fs_composite(in: FsIn) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src_tex, smp, in.uv, 0.0);
    return vec4<f32>(c.rgb, c.a * w.opacity);
}

// ----------------------------------------------------------------- particles

struct PointOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

/// One quad per particle, sized in pixels. The vertex shader reads the position texture directly
/// (vertex texture fetch, which GLES 3.0 guarantees), so no position ever returns to the CPU.
@vertex
fn vs_points(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> PointOut {
    var out: PointOut;
    let side = i32(textureDimensions(pos_tex).x);
    let texel = vec2<i32>(i32(ii) % side, i32(ii) / side);
    let p = decode_pos(textureLoad(pos_tex, texel, 0));

    let world = w.bbox_min + p * (w.bbox_max - w.bbox_min);
    let base = (world - w.center) * w.scale;
    var corner = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let px = w.point_px * w.world_per_px * w.scale * 0.5;
    out.clip = vec4<f32>(base + corner[vi] * px, 0.0, 1.0);

    // The shared WIND ramp, baked into a 256x1 LUT: 0..60 m/s across the table.
    let t = clamp(length(wind_at(p)) / 60.0, 0.0, 0.999);
    let c = textureLoad(lut_tex, vec2<i32>(i32(t * 256.0), 0), 0);
    out.color = vec4<f32>(c.rgb, c.a * w.opacity);
    return out;
}

@fragment
fn fs_points(in: PointOut) -> @location(0) vec4<f32> {
    return in.color;
}
