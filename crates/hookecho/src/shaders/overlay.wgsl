// Vector overlay layer: pre-tessellated colored triangles (alert/outlook/MD polygons and
// their outlines) in world space, transformed by the shared camera uniform.

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

struct VsIn {
    @location(0) world: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) offset: vec3<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) horizon: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let projected = project_world(in.world);
    out.horizon = projected.z;
    let p = projected.xy
        + in.offset.xy * mix(1.0, camera.road_scale, in.offset.z)
        * camera.world_per_pixel * camera.scale;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (camera.globe.z != 0.0 && in.horizon < 0.0) { discard; }
    return in.color;
}

fn gamma_from_linear(c: vec3<f32>) -> vec3<f32> {
    return select(12.92 * c, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}

@fragment
fn fs_main_gamma(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(gamma_from_linear(in.color.rgb), in.color.a);
}
