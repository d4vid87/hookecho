// GPU wind particles: the webgl-wind technique the note in `wind_draw.rs` names as the upgrade
// path. Positions live in a small RGBA8 texture and are advected by a fragment shader that reads
// one texture and writes the other; nothing here needs a compute shader or a storage buffer,
// which the device (built with `downlevel_webgl2_defaults`) does not have.
//
// Everything is 8-bit-encodable on purpose: RGBA8 is the one color format guaranteed renderable
// on WebGL2 without an extension. A position is two bytes per axis — 1/65536 of the wind domain,
// far finer than a pixel at any zoom — and a wind component is one byte over ±40 m/s, about
// 0.3 m/s, which is finer than the color ramp resolves.

// Native GPU sine can round street-scale angles to zero (for example on NVIDIA).
// The cubic retains their sign and relative precision; its error below .001 is below f32 ULP.
fn precise_sin(angle: f32) -> f32 {
    if (abs(angle) < 0.001) { return angle * (1.0 - angle * angle / 6.0); }
    return sin(angle);
}

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
    globe: vec4<f32>,
    planar: vec4<f32>,
    trail: vec4<f32>,
    ramp: vec4<f32>,
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

/// Wind in m/s and validity. Interpolate valid neighbors without inventing wind in data gaps.
fn wind_at(p: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(wind_tex));
    let cell = clamp(vec2<i32>(floor(p * vec2<f32>(size))), vec2<i32>(0), size - vec2<i32>(1));
    if (textureLoad(wind_tex, cell, 0).a < 0.5) {
        return vec3<f32>(0.0);
    }
    let c = textureSampleLevel(wind_tex, smp, p, 0.0);
    return vec3<f32>((c.rg / max(c.a, 1e-6) * 255.0 - 128.0) * (40.0 / 127.0), 1.0);
}

fn wind_color(speed: f32) -> vec4<f32> {
    let value = speed * w.ramp.x;
    if (value < w.ramp.y) { return vec4<f32>(0.0); }
    let t = clamp((value - w.ramp.y) / (w.ramp.z - w.ramp.y), 0.0, 1.0);
    return textureLoad(lut_tex, vec2<i32>(i32(2.0 + t * 253.0), 0), 0);
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
    let wind = wind_at(p);
    let uv = wind.xy;
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
    if (next.x < 0.0 || next.x > 1.0 || next.y < 0.0 || next.y > 1.0 || wind.z == 0.0 || speed < 0.2 || r < w.respawn) {
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
    return textureSampleLevel(src_tex, smp, in.uv, 0.0) * w.trail.x;
}

/// Draw the trail buffer over the pane.
@fragment
fn fs_composite(in: FsIn) -> @location(0) vec4<f32> {
    if (w.globe.z != 0.0) {
        let ndc = vec2<f32>(2.0 * in.uv.x - 1.0, 1.0 - 2.0 * in.uv.y) / w.globe.zw;
        if (dot(ndc, ndc) > 1.0) { discard; }
    }
    let c = textureSampleLevel(src_tex, smp, in.uv, 0.0);
    // Trail RGB is already premultiplied by the particle pass; scale RGBA exactly once.
    return c * w.opacity;
}

// ----------------------------------------------------------------- particles

struct PointOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) horizon: f32,
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
    let delta = (world - w.center) - w.trail.yz;
    var base = delta * w.scale;
    out.horizon = 1.0;
    if (w.globe.z != 0.0) {
        let pi = 3.14159265358979;
        let dn = -2.0 * pi * delta.y;
        let sl0 = w.globe.x;
        let cl0 = w.globe.y;
        var dlat = dn * cl0 * (1.0 - 0.5 * dn * sl0 + dn * dn * (sl0 * sl0 - cl0 * cl0) / 6.0);
        if (abs(dn) >= 0.001) {
            let n = 0.5 * log((1.0 + sl0) / (1.0 - sl0));
            dlat = atan2(2.0 * cosh(n + dn * 0.5) * sinh(dn * 0.5),
                1.0 + sinh(n) * sinh(n + dn));
        }
        let dl = delta.x * 2.0 * pi;
        let cl = cl0 * cos(dlat) - sl0 * precise_sin(dlat);
        let drop = 2.0 * precise_sin(dl * 0.5) * precise_sin(dl * 0.5);
        base = vec2<f32>(w.globe.z * cl * precise_sin(dl), w.globe.w * (precise_sin(dlat) + sl0 * cl * drop));
        out.horizon = cos(dlat) - cl0 * cl * drop;
    }
    var corner = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let px = w.point_px * w.world_per_px * w.scale * 0.5;
    out.clip = vec4<f32>(base + corner[vi] * px, 0.0, 1.0);
    if (w.globe.z == 0.0) {
        let q = delta / w.world_per_px * vec2<f32>(1.0, -1.0);
        let r = vec2<f32>(w.planar.x * q.x - w.planar.y * q.y,
            w.planar.y * q.x + w.planar.x * q.y);
        let cw = 1.0 + r.y * w.planar.w;
        let clip_scale = w.scale * w.world_per_px * vec2<f32>(1.0, -1.0);
        out.clip = vec4<f32>(r * vec2<f32>(1.0, w.planar.z) * clip_scale + corner[vi] * px * cw, 0.0, cw);
    }

    let wind = wind_at(p);
    out.color = wind_color(length(wind.xy));
    if (wind.z == 0.0 || length(wind.xy) < 0.2) { out.color = vec4<f32>(0.0); }
    return out;
}

@fragment
fn fs_points(in: PointOut) -> @location(0) vec4<f32> {
    if (in.horizon < 0.0) { discard; }
    return in.color;
}
