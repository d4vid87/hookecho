// Instanced observed Level II gates in geographic 3D. Each instance is one real gate; the six
// generated vertices make its azimuth/range footprint. Camera motion changes only group 0.

const PI: f32 = 3.14159265358979;
const EARTH_RADIUS_M: f32 = 6371000.0;
const EFFECTIVE_RADIUS_M: f32 = EARTH_RADIUS_M * 4.0 / 3.0;

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    mode_3d: f32,
    _pad: f32,
    view_proj: mat4x4<f32>,
};

struct Radar3d {
    radar_lat: f32,
    radar_lon: f32,
    antenna_altitude_m: f32,
    vertical_exaggeration: f32,
    opacity: f32,
    threshold_idx: f32,
    world_units_per_metre: f32,
    srv: f32,
    motion_e: f32,
    motion_n: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> radar: Radar3d;
@group(1) @binding(1) var lut_tex: texture_2d<f32>;

struct Gate {
    @location(0) polar: vec4<f32>, // azimuth, beam width, slant start, slant span (km)
    @location(1) data: vec4<f32>,  // elevation, palette index, original gate number, spare
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) value_idx: f32,
    @location(1) @interpolate(flat) azimuth: f32,
};

fn beam_world(azimuth_deg: f32, slant_km: f32, elevation_deg: f32) -> vec3<f32> {
    let r = max(slant_km, 0.0) * 1000.0;
    let e = elevation_deg * PI / 180.0;
    let height = sqrt(r * r + EFFECTIVE_RADIUS_M * EFFECTIVE_RADIUS_M
        + 2.0 * r * EFFECTIVE_RADIUS_M * sin(e)) - EFFECTIVE_RADIUS_M;
    let central = atan2(r * cos(e), EFFECTIVE_RADIUS_M + r * sin(e));
    let ground = EFFECTIVE_RADIUS_M * central;

    let phi1 = radar.radar_lat * PI / 180.0;
    let lambda1 = radar.radar_lon * PI / 180.0;
    let bearing = azimuth_deg * PI / 180.0;
    let delta = ground / EARTH_RADIUS_M;
    let phi2 = asin(clamp(sin(phi1) * cos(delta)
        + cos(phi1) * sin(delta) * cos(bearing), -1.0, 1.0));
    let lambda2 = lambda1 + atan2(
        sin(bearing) * sin(delta) * cos(phi1),
        cos(delta) - sin(phi1) * sin(phi2),
    );
    let world = vec2<f32>(
        lambda2 / (2.0 * PI) + 0.5,
        0.5 - log((1.0 + sin(phi2)) / (1.0 - sin(phi2))) / (4.0 * PI),
    );
    var d = world - camera.center;
    d.x = d.x - floor(d.x + 0.5);
    let altitude = (radar.antenna_altitude_m + height) * radar.vertical_exaggeration;
    return vec3<f32>(
        d.x / camera.world_per_pixel,
        -d.y / camera.world_per_pixel,
        altitude * radar.world_units_per_metre / camera.world_per_pixel,
    );
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, gate: Gate) -> VsOut {
    let corner = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, 0.0), vec2<f32>(0.5, 0.0), vec2<f32>(0.5, 1.0),
        vec2<f32>(-0.5, 0.0), vec2<f32>(0.5, 1.0), vec2<f32>(-0.5, 1.0),
    )[vertex];
    let az = gate.polar.x + corner.x * gate.polar.y;
    let range = gate.polar.z + corner.y * gate.polar.w;
    var out: VsOut;
    out.clip = camera.view_proj * vec4<f32>(beam_world(az, range, gate.data.x), 1.0);
    out.value_idx = gate.data.y;
    out.azimuth = gate.polar.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var idx = in.value_idx;
    if (radar.srv > 0.5) {
        let az = in.azimuth * PI / 180.0;
        idx = clamp(idx - radar.motion_e * sin(az) - radar.motion_n * cos(az), 2.0, 255.0);
    }
    if (idx < radar.threshold_idx) { discard; }
    let color = textureLoad(lut_tex, vec2<i32>(i32(round(idx)), 0), 0);
    let alpha = color.a * radar.opacity;
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(color.rgb, alpha);
}
