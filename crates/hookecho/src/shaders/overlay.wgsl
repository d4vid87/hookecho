// Vector overlay layer: pre-tessellated colored triangles (alert/outlook/MD polygons and
// their outlines) in world space, transformed by the shared camera uniform.

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    _pad: vec2<f32>,
    globe: vec4<f32>,
    planar: vec4<f32>,
};


// Perspective Mercator plane; CPU mirror is Camera::planar_point.
// Keep homogeneous W here so world/UV varyings interpolate on the ground plane.
fn planar_clip(world: vec2<f32>) -> vec4<f32> {
    if (all(camera.planar == vec4<f32>(1.0, 0.0, 1.0, 0.0))) {
        return vec4<f32>((world - camera.center) * camera.scale, 0.0, 1.0);
    }
    let q = (world - camera.center) / camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    let r = vec2<f32>(camera.planar.x * q.x - camera.planar.y * q.y,
        camera.planar.y * q.x + camera.planar.x * q.y);
    let clip_scale = camera.scale * camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    return vec4<f32>(r * vec2<f32>(1.0, camera.planar.z) * clip_scale, 0.0,
        1.0 + r.y * camera.planar.w);
}

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
    // Homogeneous pixel extrusion and its width. Clipping a triangulated stroke across
    // W=0 can otherwise extend its diagonal outside the intended screen-space strip.
    @location(2) stroke: vec4<f32>,
    @location(3) @interpolate(flat) building: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let projected = project_world(in.world);
    out.horizon = projected.z;
    out.stroke = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    out.building = select(0.0, 1.0, in.offset.z <= -1.0);
    let p = projected.xy
        + in.offset.xy * mix(1.0, camera.road_scale, in.offset.z)
        * camera.world_per_pixel * camera.scale;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    if (camera.globe.z == 0.0) {
        out.clip = planar_clip(in.world);
        if (out.building > 0.5 && camera.planar.w != 0.0) {
            // The map plane uses homogeneous W. A height in metres must move both its clip Y
            // and W; moving only Y would make roofs drift relative to the tilted footprint.
            let lat = atan(sinh(G_PI * (1.0 - 2.0 * in.world.y)));
            let metres_per_world = 40075017.0 * max(cos(lat), 0.08);
            let height_px = max(0.0, -in.offset.z - 1.0)
                / (metres_per_world * camera.world_per_pixel);
            let sin_pitch = sqrt(max(0.0, 1.0 - camera.planar.z * camera.planar.z));
            let clip_px = camera.scale * camera.world_per_pixel * vec2<f32>(1.0, -1.0);
            out.clip.y += height_px * sin_pitch * clip_px.y;
            out.clip.w -= height_px * camera.planar.z * camera.planar.w / sin_pitch;
        }
        let width = length(in.offset.xy);
        if (width > 0.0) {
            // Transform the stroke normal with inverse-transpose of the local projection
            // Jacobian, then restore its pixel width. Geometry remains cached while rotating.
            let n = in.offset.xy * vec2<f32>(1.0, -1.0);
            let nr = vec2<f32>(camera.planar.x * n.x - camera.planar.y * n.y,
                camera.planar.y * n.x + camera.planar.x * n.y);
            let q = (in.world - camera.center) / camera.world_per_pixel * vec2<f32>(1.0, -1.0);
            let rx = camera.planar.x * q.x - camera.planar.y * q.y;
            let cw = out.clip.w;
            let normal = vec2<f32>(nr.x, (rx * camera.planar.w * nr.x + cw * nr.y) / camera.planar.z);
            let offset = normal / max(length(normal), 1e-8) * width
                * mix(1.0, camera.road_scale, in.offset.z);
            let clip_scale = camera.scale * camera.world_per_pixel * vec2<f32>(1.0, -1.0);
            out.clip = out.clip + vec4<f32>(offset * clip_scale * cw, 0.0, 0.0);
            out.stroke = vec4<f32>(offset * cw, cw, width * mix(1.0, camera.road_scale, in.offset.z));
        }
    }
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.building > 0.5 && (camera.globe.z != 0.0 || camera.planar.w == 0.0)) { discard; }
    if (camera.globe.z != 0.0 && in.horizon < 0.0) { discard; }
    if (camera.globe.z == 0.0 && camera.planar.w != 0.0 && in.stroke.w > 0.0
        && length(in.stroke.xy / max(in.stroke.z, 1e-8)) > in.stroke.w + 0.01) { discard; }
    return in.color;
}

fn gamma_from_linear(c: vec3<f32>) -> vec3<f32> {
    return select(12.92 * c, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}

@fragment
fn fs_main_gamma(in: VsOut) -> @location(0) vec4<f32> {
    if (in.building > 0.5 && (camera.globe.z != 0.0 || camera.planar.w == 0.0)) { discard; }
    if (camera.globe.z == 0.0 && camera.planar.w != 0.0 && in.stroke.w > 0.0
        && length(in.stroke.xy / max(in.stroke.z, 1e-8)) > in.stroke.w + 0.01) { discard; }
    return vec4<f32>(gamma_from_linear(in.color.rgb), in.color.a);
}
