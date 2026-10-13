// Polar radar layer. A single quad covers the radar's range disk in world space;
// each fragment inverts web-mercator to lon/lat, computes azimuth+range from the
// radar, samples the binned R8 sweep texture, and colors it.

const PI: f32 = 3.14159265358979;
const R_EARTH_KM: f32 = 6371.0;

struct Camera {
    center: vec2<f32>,
    scale: vec2<f32>,
    world_per_pixel: f32,
    road_scale: f32,
    _pad: vec2<f32>,
    globe: vec4<f32>,
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
    // Precipitation-type tint: 0 = off (always LUT row 0), 1 = pick the row from the flag grid.
    tint: f32,
    flag_nx: f32,
    flag_ny: f32,
    flag_west: f32,
    flag_north: f32,
    flag_east: f32,
    flag_south: f32,
    blend_width: f32,
    code_max: f32,
    multipass: f32,
    sector_opacity: array<vec4<f32>, 180>,
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
@group(1) @binding(0) var<uniform> radar: Radar;
@group(1) @binding(1) var sweep_tex: texture_2d<u32>;
@group(1) @binding(2) var lut_tex: texture_2d<f32>;
@group(1) @binding(3) var precip_flag_tex: texture_2d<u32>;

fn code_max() -> f32 { return select(255.0, radar.code_max, radar.code_max >= 255.0); }

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
    out.world = in.world;
    return out;
}

// Width of the blend band as a fraction of a cell. 1.0 is plain bilinear; 0.0 is nearest.
// Configurable per pane; a zero word from older callers retains the original default.
fn blend_width() -> f32 {
    return select(0.55, clamp(radar.blend_width, 0.05, 1.0), radar.blend_width > 0.0);
}

fn sharpen(t: f32) -> f32 {
    return smoothstep(0.5 - blend_width() * 0.5, 0.5 + blend_width() * 0.5, t);
}

fn world_to_lonlat(w: vec2<f32>) -> vec2<f32> {
    let lon = w.x * 360.0 - 180.0;
    let n = PI * (1.0 - 2.0 * w.y);
    let lat = atan(sinh(n)) * 180.0 / PI;
    return vec2<f32>(lon, lat);
}

// Two separable normalized stages, evaluated on demand rather than storing a
// full intermediate texture. The center classification always wins; invalid
// neighbours never become numeric values. This works with R8 and R16 textures.
fn display_gate(gx: i32, gy: i32, nbins: i32) -> f32 {
    let center = textureLoad(sweep_tex, vec2<i32>(gx, gy), 0).r;
    if (radar.multipass < 0.5 || center < 2u) { return f32(center); }
    var radial_sum = 0.0;
    var radial_weight = 0.0;
    for (var j = -1; j <= 1; j++) {
        let y = ((gy + j) % nbins + nbins) % nbins;
        if (textureLoad(sweep_tex, vec2<i32>(gx, y), 0).r < 2u) { continue; }
        var gate_sum = 0.0;
        var gate_weight = 0.0;
        for (var i = -1; i <= 1; i++) {
            let x = gx + i;
            if (x < 0 || x >= i32(radar.gate_count)) { continue; }
            let value = textureLoad(sweep_tex, vec2<i32>(x, y), 0).r;
            if (value < 2u) { continue; }
            let weight = select(1.0, 2.0, i == 0);
            gate_sum += f32(value) * weight;
            gate_weight += weight;
        }
        let weight = select(1.0, 2.0, j == 0);
        radial_sum += gate_sum / gate_weight * weight;
        radial_weight += weight;
    }
    return radial_sum / radial_weight;
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
    // Carried as a float so the smoothed path can also interpolate the LUT lookup: a nearest
    // lookup quantizes back to the table's bands, which is what made smoothed data still show
    // hard color edges. The nearest path rounds at the end and stays bit-identical.
    var rawf: f32;
    if (radar.smoothing > 0.5) {
        // Bilinear over the four surrounding gates, blending only valid data cells
        // (raw >= 2). Interpolating raw indices is meaningful because each colormap is
        // monotonic in value across 2..=255.
        let gf = gate_f - 0.5;
        let af = az_bin - 0.5;
        let g0 = i32(floor(gf));
        let a0 = i32(floor(af));
        // Sharpened bilinear. Plain bilinear ramps linearly all the way across a gate, which
        // smears every edge in the field by a full gate — a hail core's boundary and the far side
        // of a hook both go soft, and the smoothing setting stops being usable for interrogation.
        // Squeezing the ramp into a band around the cell boundary keeps the interior of each gate
        // at its own value (crisp edges) while removing the staircase between them.
        let tg = sharpen(gf - floor(gf));
        let ta = sharpen(af - floor(af));
        var acc = 0.0;
        var wsum = 0.0;
        for (var i = 0; i < 2; i++) {
            for (var j = 0; j < 2; j++) {
                let gx = g0 + i;
                if (gx < 0 || gx >= i32(radar.gate_count)) { continue; }
                let gy = ((a0 + j) % nbins + nbins) % nbins;
                let r = display_gate(gx, gy, nbins);
                if (r < 2.0) { continue; } // skip below-threshold / range-folded
                let wg = select(1.0 - tg, tg, i == 1);
                let wa = select(1.0 - ta, ta, j == 1);
                let w = wg * wa;
                acc += r * w;
                wsum += w;
            }
        }
        if (wsum <= 0.0) { discard; }
        rawf = acc / wsum;
    } else {
        let gx = i32(gate_f);
        let gy = i32(az_bin) % nbins;
        rawf = f32(textureLoad(sweep_tex, vec2<i32>(gx, gy), 0).r);
    }

    // Storm-relative velocity: shift the value-band index by the storm-motion component
    // along this radial. SRV = v_r - (mu*sin az + mv*cos az); motion_* are already in
    // raw-index units, so we shift the index directly. 0/1 (no-data / range-fold) pass through.
    if (radar.srv > 0.5 && rawf >= 2.0) {
        let az_rad = az * PI / 180.0;
        let delta = -(radar.motion_e * sin(az_rad) + radar.motion_n * cos(az_rad));
        rawf = clamp(rawf + delta, 2.0, code_max());
    }

    // Which LUT row: 0 rain, 1 snow, 2 mix. Reflectivity cannot tell them apart — the same
    // 30 dBZ is a downpour or a heavy snow band — so the class comes from the MRMS surface
    // precipitation-type grid, sampled at this fragment's own position. Off the grid, or with
    // the tint off, everything falls back to row 0, which is the user's own table.
    var row: i32 = 0;
    if (radar.tint > 0.5) {
        // `ll` is degrees; `lon`/`lat` above were shadowed into radians for the range maths.
        let u = (ll.x - radar.flag_west) / (radar.flag_east - radar.flag_west);
        let v = (radar.flag_north - ll.y) / (radar.flag_north - radar.flag_south);
        if (u >= 0.0 && u < 1.0 && v >= 0.0 && v < 1.0) {
            let fx = i32(u * radar.flag_nx);
            let fy = i32(v * radar.flag_ny);
            let cls = textureLoad(precip_flag_tex, vec2<i32>(fx, fy), 0).r;
            row = min(i32(cls), 2);
        }
    }

    // The LUT encodes everything: 0 -> transparent, 1 -> range-fold, 2..255 -> value band,
    // threshold baked into the alpha. Colormap selection is the choice of LUT, so the shader
    // is moment-agnostic.
    let last = f32(textureDimensions(lut_tex).x - 1u);
    let palette_index = select(rawf, 2.0 + (rawf - 2.0) / (code_max() - 2.0) * (last - 2.0), rawf >= 2.0 && (code_max() > 255.0 || last != 255.0));
    var color: vec4<f32>;
    if (radar.smoothing > 0.5 || code_max() > 255.0) {
        // Blend adjacent LUT entries. Only ever adjacent, so this stays correct for the diverging
        // velocity maps too — the zero crossing spans two neighbouring bands.
        let i0 = i32(floor(palette_index));
        let a = textureLoad(lut_tex, vec2<i32>(i0, row), 0);
        let b = textureLoad(lut_tex, vec2<i32>(min(i0 + 1, i32(last)), row), 0);
        color = mix(a, b, fract(palette_index));
    } else {
        color = textureLoad(lut_tex, vec2<i32>(i32(round(palette_index)), row), 0);
    }
    let sector = u32(az * 2.0) % 720u;
    color.a *= radar.sector_opacity[sector / 4u][sector % 4u];
    if (color.a == 0.0) { discard; }
    return color;
}
