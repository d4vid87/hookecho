// Textured slippy-map tile quad. Vertices carry normalized-mercator world coords;
// the camera uniform maps them to clip space.

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    _pad: vec2<f32>,
    globe: vec4<f32>,
};


// Orthographic globe shared with CPU picking. globe.z == 0 retains Mercator.
const G_PI: f32 = 3.14159265358979;
fn project_world(w: vec2<f32>) -> vec3<f32> {
    if (camera.globe.z == 0.0) { return vec3<f32>((w - camera.center) * camera.scale, 1.0); }
    let lat = atan(sinh(G_PI * (1.0 - 2.0 * w.y)));
    let dl = (w.x - camera.center.x) * 2.0 * G_PI;
    let sl = sin(lat); let cl = cos(lat);
    return vec3<f32>(camera.globe.z * cl * sin(dl),
        camera.globe.w * (camera.globe.y * sl - camera.globe.x * cl * cos(dl)),
        camera.globe.x * sl + camera.globe.y * cl * cos(dl));
}
fn globe_world(ndc: vec2<f32>) -> vec3<f32> {
    let xy = ndc / camera.globe.zw;
    let r2 = dot(xy, xy);
    if (r2 > 1.0) { return vec3<f32>(0.0, 0.0, -1.0); }
    let z = sqrt(max(0.0, 1.0 - r2));
    let lat = asin(clamp(z * camera.globe.x + xy.y * camera.globe.y, -1.0, 1.0));
    if (abs(lat) > 1.48442223) { return vec3<f32>(0.0, 0.0, -1.0); }
    let dl = atan2(xy.x, z * camera.globe.y - xy.y * camera.globe.x);
    return vec3<f32>(fract(camera.center.x + dl / (2.0 * G_PI)),
        0.5 - log((1.0 + sin(lat)) / (1.0 - sin(lat))) / (4.0 * G_PI), 1.0);
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var tile_tex: texture_2d<f32>;
@group(1) @binding(1) var tile_smp: sampler;

struct VsIn {
    @location(0) world: vec2<f32>,
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
    let projected = project_world(in.world);
    let p = projected.xy;
    out.horizon = projected.z;
    out.ndc = p;
    out.transform = in.transform;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (camera.globe.z != 0.0) {
        if (in.horizon < 0.0) { discard; }
        let w = globe_world(in.ndc);
        if (w.z < 0.0) { discard; }
        // Wrap the inverse longitude to this tile's copy at the antimeridian.
        let wx = w.x + round(in.transform.x - w.x);
        uv = (vec2<f32>(wx, w.y) - in.transform.xy) * in.transform.zw;
    }
    return textureSample(tile_tex, tile_smp, uv);
}
