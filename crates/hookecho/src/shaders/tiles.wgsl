// Textured slippy-map tile quad. Vertices and tile origins carry camera-relative
// Mercator positions, subtracted in f64 on the CPU before conversion to f32.

// Native GPU sine can round street-scale angles to zero (for example on NVIDIA).
// The cubic retains their sign and relative precision; its error below .001 is below f32 ULP.
fn precise_sin(angle: f32) -> f32 {
    if (abs(angle) < 0.001) { return angle * (1.0 - angle * angle / 6.0); }
    return sin(angle);
}

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    center_low: vec2<f32>,
    globe: vec4<f32>,
    planar: vec4<f32>,
};


// Perspective Mercator plane; CPU mirror is Camera::planar_point.
// Keep homogeneous W here so world/UV varyings interpolate on the ground plane.
fn planar_clip(delta: vec2<f32>) -> vec4<f32> {
    if (all(camera.planar == vec4<f32>(1.0, 0.0, 1.0, 0.0))) {
        return vec4<f32>(delta * camera.scale, 0.0, 1.0);
    }
    let q = delta / camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    let r = vec2<f32>(camera.planar.x * q.x - camera.planar.y * q.y,
        camera.planar.y * q.x + camera.planar.x * q.y);
    let clip_scale = camera.scale * camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    return vec4<f32>(r * vec2<f32>(1.0, camera.planar.z) * clip_scale, 0.0,
        1.0 + r.y * camera.planar.w);
}

// Orthographic globe shared with CPU picking. globe.z == 0 retains Mercator.
const G_PI: f32 = 3.14159265358979;
fn latitude_delta(base_lat: f32, world_dy: f32) -> f32 {
    let s = precise_sin(base_lat);
    let n = 0.5 * log((1.0 + s) / (1.0 - s));
    let dn = -2.0 * G_PI * world_dy;
    // GLES sinh may subtract two rounded exponentials near zero. The local series
    // retains subpixel latitude deltas at street zoom while the exact path covers a globe.
    if (abs(dn) < 0.001) {
        let c = cos(base_lat);
        return dn * c * (1.0 - 0.5 * dn * s + dn * dn * (s * s - c * c) / 6.0);
    }
    return atan2(2.0 * cosh(n + dn * 0.5) * sinh(dn * 0.5),
        1.0 + sinh(n) * sinh(n + dn));
}

fn project_delta(delta: vec2<f32>) -> vec3<f32> {
    if (camera.globe.z == 0.0) { return vec3<f32>(delta * camera.scale, 1.0); }
    let base_lat = atan2(camera.globe.x, camera.globe.y);
    let dlat = latitude_delta(base_lat, delta.y);
    let dl = delta.x * 2.0 * G_PI;
    let cl = camera.globe.y * cos(dlat) - camera.globe.x * precise_sin(dlat);
    let longitude_drop = 2.0 * precise_sin(dl * 0.5) * precise_sin(dl * 0.5);
    return vec3<f32>(camera.globe.z * cl * precise_sin(dl),
        camera.globe.w * (precise_sin(dlat) + camera.globe.x * cl * longitude_drop),
        cos(dlat) - camera.globe.y * cl * longitude_drop);
}
fn globe_delta(ndc: vec2<f32>) -> vec3<f32> {
    let xy = ndc / camera.globe.zw;
    let r2 = dot(xy, xy);
    if (r2 > 1.0) { return vec3<f32>(0.0, 0.0, -1.0); }
    let z = sqrt(max(0.0, 1.0 - r2));
    let ds = -camera.globe.x * r2 / (1.0 + z) + xy.y * camera.globe.y;
    let s = camera.globe.x + ds;
    if (abs(s) > 0.99627208) { return vec3<f32>(0.0, 0.0, -1.0); }
    let q = ds / (1.0 - s * camera.globe.x);
    var dn = q + q * q * q / 3.0;
    if (abs(q) >= 0.001) { dn = 0.5 * log((1.0 + q) / (1.0 - q)); }
    let dl = atan2(xy.x, z * camera.globe.y - xy.y * camera.globe.x);
    return vec3<f32>(dl / (2.0 * G_PI), -dn / (2.0 * G_PI), 1.0);
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var tile_tex: texture_2d<f32>;
@group(1) @binding(1) var tile_smp: sampler;

struct VsIn {
    @location(0) delta: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) transform: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) ndc: vec2<f32>,
    @location(2) @interpolate(flat) transform: vec4<f32>,
    @location(3) horizon: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let projected = project_delta(in.delta);
    let p = projected.xy;
    out.horizon = projected.z;
    out.ndc = p;
    out.transform = in.transform;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    if (camera.globe.z == 0.0) { out.clip = planar_clip(in.delta); }
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (camera.globe.z != 0.0) {
        if (in.horizon < 0.0) { discard; }
        let delta = globe_delta(in.ndc);
        if (delta.z < 0.0) { discard; }
        var relative = delta.xy - in.transform.xy;
        // Polar views can draw both copies of a boundary tile. Center the period on the
        // source texture, preserving ancestor subrects and clamping just outside its edges.
        relative.x -= round(relative.x - 0.5 / in.transform.z);
        uv = relative * in.transform.zw;
    }
    return textureSample(tile_tex, tile_smp, uv);
}
