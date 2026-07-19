// MRMS national mosaic layer. A world-space quad covers the CONUS grid's mercator bbox;
// each fragment inverts web-mercator to lon/lat, maps to the regular lat/lon grid (plate-carrée
// → mercator warp), samples the R8 index texture, and colors it through the shared LUT.

const PI: f32 = 3.14159265358979;

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
};

// Grid bounds + dimensions (padded to 48 bytes to share the radar bind-group layout).
struct Mrms {
    lon_west: f32,
    lat_north: f32,
    lon_east: f32,
    lat_south: f32,
    nx: f32,
    ny: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
    _pad5: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> mrms: Mrms;
@group(1) @binding(1) var grid_tex: texture_2d<u32>;
@group(1) @binding(2) var lut_tex: texture_2d<f32>;

struct VsIn { @location(0) world: vec2<f32> };
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let p = (in.world - camera.center) * camera.scale;
    out.clip = vec4<f32>(p, 0.0, 1.0);
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
    let ll = world_to_lonlat(in.world);
    let fu = (ll.x - mrms.lon_west) / (mrms.lon_east - mrms.lon_west);
    let fv = (mrms.lat_north - ll.y) / (mrms.lat_north - mrms.lat_south);
    if (fu < 0.0 || fu >= 1.0 || fv < 0.0 || fv >= 1.0) { discard; }

    let gx = i32(fu * mrms.nx);
    let gy = i32(fv * mrms.ny);
    let raw = textureLoad(grid_tex, vec2<i32>(gx, gy), 0).r;

    let color = textureLoad(lut_tex, vec2<i32>(i32(raw), 0), 0);
    if (color.a == 0.0) { discard; }
    return color;
}
