// Vector overlay layer: pre-tessellated colored triangles (alert/outlook/MD polygons and
// their outlines) in world space, transformed by the shared camera uniform.

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    mode_3d: f32,
    _pad: f32,
    view_proj: mat4x4<f32>,
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
    let width = mix(1.0, camera.road_scale, in.offset.z);
    let world = in.world + in.offset.xy * width * camera.world_per_pixel;
    let p = (world - camera.center) * camera.scale;
    if (camera.mode_3d > 0.5) {
        var delta = world - camera.center;
        delta.x = delta.x - floor(delta.x + 0.5);
        let local = vec3<f32>(delta.x, -delta.y, 0.0) / camera.world_per_pixel;
        out.clip = camera.view_proj * vec4<f32>(local, 1.0);
    } else {
        out.clip = vec4<f32>(p, 0.0, 1.0);
    }
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
