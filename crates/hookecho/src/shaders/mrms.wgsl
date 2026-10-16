// MRMS national mosaic layer. A world-space quad covers the CONUS grid's mercator bbox;
// each fragment inverts web-mercator to lon/lat, maps to the regular lat/lon grid (plate-carrée
// → mercator warp), samples the R8 index texture, and colors it through the shared LUT.

const PI: f32 = 3.14159265358979;

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    _pad: vec2<f32>,
    globe: vec4<f32>,
    planar: vec4<f32>,
};

// Grid bounds + dimensions (48 bytes, matching mrms_bgl's min_binding_size).
struct Mrms {
    lon_west: f32,
    lat_north: f32,
    lon_east: f32,
    lat_south: f32,
    nx: f32,
    ny: f32,
    /// Layer opacity 0..1, rewritten per frame from the Layer Manager slider.
    opacity: f32,
    /// 1 when grid_tex contains direct RGBA bytes instead of palette indices.
    direct_rgba: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
    _pad5: f32,
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
@group(1) @binding(0) var<uniform> mrms: Mrms;
@group(1) @binding(1) var grid_tex: texture_2d<u32>;
@group(1) @binding(2) var lut_tex: texture_2d<f32>;

struct VsIn { @location(0) world: vec2<f32> };
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) ndc: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn, @builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    var p = (in.world - camera.center) * camera.scale;
    if (camera.globe.z != 0.0) {
        let corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
            vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, -1.0));
        p = corners[index % 6u];
    }
    out.ndc = p;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    if (camera.globe.z == 0.0) { out.clip = planar_clip(in.world); }
    out.world = in.world;
    return out;
}

fn world_to_lonlat(w: vec2<f32>) -> vec2<f32> {
    let lon = w.x * 360.0 - 180.0;
    let n = PI * (1.0 - 2.0 * w.y);
    let lat = atan(sinh(n)) * 180.0 / PI;
    return vec2<f32>(lon, lat);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var world = in.world;
    if (camera.globe.z != 0.0) {
        let picked = globe_world(in.ndc);
        if (picked.z < 0.0) { discard; }
        world = picked.xy;
    }
    let ll = world_to_lonlat(world);
    let fu = (ll.x - mrms.lon_west) / (mrms.lon_east - mrms.lon_west);
    let fv = (mrms.lat_north - ll.y) / (mrms.lat_north - mrms.lat_south);
    if (fu < 0.0 || fu >= 1.0 || fv < 0.0 || fv >= 1.0) { discard; }

    let gx = i32(fu * mrms.nx);
    let gy = i32(fv * mrms.ny);
    let raw = textureLoad(grid_tex, vec2<i32>(gx, gy), 0);

    if (mrms.direct_rgba > 0.5) {
        if (raw.a == 0u) { discard; }
        let srgb = vec3<f32>(raw.rgb) / 255.0;
        let linear = pow(srgb, vec3<f32>(2.2));
        return vec4<f32>(linear, f32(raw.a) / 255.0 * mrms.opacity);
    }

    let color = textureLoad(lut_tex, vec2<i32>(i32(raw.r), 0), 0);
    if (color.a == 0.0) { discard; }
    return vec4<f32>(color.rgb, color.a * mrms.opacity);
}
