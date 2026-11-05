// Gridded weather layer. Each screen fragment picks geographic coordinates, maps to the
// source lat/lon grid, and samples either direct RGBA or indices in the shared color LUT.

const PI: f32 = 3.14159265358979;

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
    origin: vec4<f32>,
};


// Subtract split positions before multiplying by the high-zoom camera scale.
fn world_delta(world: vec2<f32>, low: vec2<f32>) -> vec2<f32> {
    let d = world - camera.center;
    let b = world - d;
    let error = (world - (d + b)) + (b - camera.center);
    return d + ((low - camera.center_low) + error);
}

// Perspective Mercator plane; CPU mirror is Camera::planar_point.
// Keep homogeneous W here so world/UV varyings interpolate on the ground plane.
fn planar_clip(world: vec2<f32>, low: vec2<f32>) -> vec4<f32> {
    if (all(camera.planar == vec4<f32>(1.0, 0.0, 1.0, 0.0))) {
        return vec4<f32>(world_delta(world, low) * camera.scale, 0.0, 1.0);
    }
    let q = world_delta(world, low) / camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    let r = vec2<f32>(camera.planar.x * q.x - camera.planar.y * q.y,
        camera.planar.y * q.x + camera.planar.x * q.y);
    let clip_scale = camera.scale * camera.world_per_pixel * vec2<f32>(1.0, -1.0);
    return vec4<f32>(r * vec2<f32>(1.0, camera.planar.z) * clip_scale, 0.0,
        1.0 + r.y * camera.planar.w);
}

// Reconstruct a small camera-relative ground offset in the fragment. Interpolating
// world positions across a continent-sized quad loses street-level pixel precision.
fn planar_delta(ndc: vec2<f32>) -> vec3<f32> {
    let screen = ndc / (camera.scale * camera.world_per_pixel) * vec2<f32>(1.0, -1.0);
    let denom = camera.planar.z - screen.y * camera.planar.w;
    if (denom <= 0.0) { return vec3<f32>(0.0, 0.0, -1.0); }
    let ry = screen.y / denom;
    let rx = screen.x * (1.0 + ry * camera.planar.w);
    return vec3<f32>(vec2<f32>(camera.planar.x * rx + camera.planar.y * ry,
        camera.planar.y * rx - camera.planar.x * ry) * camera.world_per_pixel, 1.0);
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

fn project_world(w: vec2<f32>, low: vec2<f32>) -> vec3<f32> {
    let delta = world_delta(w, low);
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
@group(1) @binding(0) var<uniform> mrms: Mrms;
@group(1) @binding(1) var grid_tex: texture_2d<u32>;
@group(1) @binding(2) var lut_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // Reconstruct ground coordinates from a screen quad. At street zoom, interpolating
    // continent-sized clip coordinates loses whole pixels on native GPU interpolators.
    let corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, -1.0));
    let p = corners[index % 6u];
    var out: VsOut;
    out.ndc = p;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    return out;
}


@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var delta: vec2<f32>;
    if (camera.globe.z != 0.0) {
        let picked = globe_delta(in.ndc);
        if (picked.z < 0.0) { discard; }
        delta = picked.xy;
    } else {
        let picked = planar_delta(in.ndc);
        if (picked.z < 0.0) { discard; }
        delta = picked.xy;
    }
    var relative = delta - world_delta(mrms.origin.xy, mrms.origin.zw);
    if (camera.globe.z != 0.0) {
        // Pick the periodic copy centered on this source, including grids spanning the seam.
        relative.x -= round(relative.x - (mrms.lon_east - mrms.lon_west) / 720.0);
    }
    let fu = relative.x * 360.0 / (mrms.lon_east - mrms.lon_west);
    // geographic_origin uses Mercator's latitude limit, while global grids can reach ±90°.
    let reference_lat = clamp(mrms.lat_north, -85.05112878, 85.05112878);
    let fv = (mrms.lat_north - reference_lat - degrees(latitude_delta(radians(reference_lat), relative.y)))
        / (mrms.lat_north - mrms.lat_south);
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
