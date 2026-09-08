// Vector overlay layer: pre-tessellated colored triangles (alert/outlook/MD polygons and
// their outlines) in world space, transformed by the shared camera uniform.

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) world: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) offset: vec3<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let p = (in.world - camera.center) * camera.scale
        + in.offset.xy * mix(1.0, camera.road_scale, in.offset.z)
        * camera.world_per_pixel * camera.scale;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}

fn gamma_from_linear(c: vec3<f32>) -> vec3<f32> {
    return select(12.92 * c, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}

@fragment
fn fs_main_gamma(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(gamma_from_linear(in.color.rgb), in.color.a);
}
