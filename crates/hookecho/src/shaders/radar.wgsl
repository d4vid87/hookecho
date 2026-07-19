// Polar radar layer. A single quad covers the radar's range disk in world space;
// each fragment inverts web-mercator to lon/lat, computes azimuth+range from the
// radar, samples the binned R8 sweep texture, and colors it.

const PI: f32 = 3.14159265358979;
const R_EARTH_KM: f32 = 6371.0;

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
};

// Radar + gate geometry. `az_bins`/`gate_count` are texture dims.
struct Radar {
    radar_lat: f32,
    radar_lon: f32,
    first_gate_km: f32,
    gate_interval_km: f32,
    az_bins: f32,
    gate_count: f32,
    smoothing: f32, // 0 = nearest, 1 = bilinear over valid gates
    srv: f32,       // 0 = ground-relative, 1 = subtract storm motion (velocity only)
    // Storm motion east/north, premultiplied into raw-index units (253 / value_span).
    motion_e: f32,
    motion_n: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> radar: Radar;
@group(1) @binding(1) var sweep_tex: texture_2d<u32>;
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
    let lat = ll.y * PI / 180.0;
    let lon = ll.x * PI / 180.0;
    let lat0 = radar.radar_lat * PI / 180.0;
    let lon0 = radar.radar_lon * PI / 180.0;
    let dlon = lon - lon0;

    // Great-circle range (haversine) in km.
    let dlat = lat - lat0;
    let a = sin(dlat * 0.5) * sin(dlat * 0.5)
          + cos(lat0) * cos(lat) * sin(dlon * 0.5) * sin(dlon * 0.5);
    let range_km = 2.0 * R_EARTH_KM * asin(sqrt(clamp(a, 0.0, 1.0)));

    // Initial bearing (azimuth) from radar, degrees clockwise from north.
    let y = sin(dlon) * cos(lat);
    let x = cos(lat0) * sin(lat) - sin(lat0) * cos(lat) * cos(dlon);
    var az = atan2(y, x) * 180.0 / PI;
    az = az - floor(az / 360.0) * 360.0;

    // Map to texture coords.
    let gate_f = (range_km - radar.first_gate_km) / radar.gate_interval_km;
    if (gate_f < 0.0 || gate_f >= radar.gate_count) { discard; }
    let az_bin = az / 360.0 * radar.az_bins;

    let nbins = i32(radar.az_bins);
    var raw: u32;
    if (radar.smoothing > 0.5) {
        // Bilinear over the four surrounding gates, blending only valid data cells
        // (raw >= 2). Interpolating raw indices is meaningful because each colormap is
        // monotonic in value across 2..=255.
        let gf = gate_f - 0.5;
        let af = az_bin - 0.5;
        let g0 = i32(floor(gf));
        let a0 = i32(floor(af));
        let tg = gf - floor(gf);
        let ta = af - floor(af);
        var acc = 0.0;
        var wsum = 0.0;
        for (var i = 0; i < 2; i++) {
            for (var j = 0; j < 2; j++) {
                let gx = g0 + i;
                if (gx < 0 || gx >= i32(radar.gate_count)) { continue; }
                let gy = ((a0 + j) % nbins + nbins) % nbins;
                let r = textureLoad(sweep_tex, vec2<i32>(gx, gy), 0).r;
                if (r < 2u) { continue; } // skip below-threshold / range-folded
                let wg = select(1.0 - tg, tg, i == 1);
                let wa = select(1.0 - ta, ta, j == 1);
                let w = wg * wa;
                acc += f32(r) * w;
                wsum += w;
            }
        }
        if (wsum <= 0.0) { discard; }
        raw = u32(round(acc / wsum));
    } else {
        let gx = i32(gate_f);
        let gy = i32(az_bin) % nbins;
        raw = textureLoad(sweep_tex, vec2<i32>(gx, gy), 0).r;
    }

    // Storm-relative velocity: shift the value-band index by the storm-motion component
    // along this radial. SRV = v_r - (mu*sin az + mv*cos az); motion_* are already in
    // raw-index units, so we shift the index directly. 0/1 (no-data / range-fold) pass through.
    if (radar.srv > 0.5 && raw >= 2u) {
        let az_rad = az * PI / 180.0;
        let delta = -(radar.motion_e * sin(az_rad) + radar.motion_n * cos(az_rad));
        raw = u32(clamp(round(f32(raw) + delta), 2.0, 255.0));
    }

    // The LUT encodes everything: 0 -> transparent, 1 -> range-fold, 2..255 -> value band,
    // threshold baked into the alpha. Colormap selection is the choice of LUT, so the shader
    // is moment-agnostic.
    let color = textureLoad(lut_tex, vec2<i32>(i32(raw), 0), 0);
    if (color.a == 0.0) { discard; }
    return color;
}
