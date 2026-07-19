// Maximum-intensity-projection raymarch of a 3D reflectivity volume.
//
// A fullscreen triangle casts one ray per pixel from an orbit camera, intersects the volume's
// axis-aligned box, marches it taking the max reflectivity index, and colors that via the
// reflectivity LUT. Empty rays are transparent so the egui window background shows through.

struct Uniforms {
    inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    box_min: vec4<f32>,
    box_max: vec4<f32>,
    dims: vec4<f32>, // nx, ny, nz, step_count
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var vol: texture_3d<u32>;
@group(0) @binding(2) var lut: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VsOut;
    o.pos = vec4<f32>(p[vid], 0.0, 1.0);
    o.ndc = p[vid];
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Reconstruct the world-space ray from the inverse view-projection.
    let far4 = u.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let far = far4.xyz / far4.w;
    let ro = u.cam_pos.xyz;
    let rd = normalize(far - ro);

    // Slab intersection with the volume box.
    let inv = 1.0 / rd;
    let t0s = (u.box_min.xyz - ro) * inv;
    let t1s = (u.box_max.xyz - ro) * inv;
    let tsmall = min(t0s, t1s);
    let tbig = max(t0s, t1s);
    let tmin = max(max(tsmall.x, tsmall.y), max(tsmall.z, 0.0));
    let tmax = min(min(tbig.x, tbig.y), tbig.z);
    if (tmax <= tmin) {
        discard;
    }

    let steps = i32(u.dims.w);
    let span = u.box_max.xyz - u.box_min.xyz;
    let dims = vec3<f32>(u.dims.x, u.dims.y, u.dims.z);
    var max_idx: u32 = 0u;
    for (var s = 0; s < steps; s = s + 1) {
        let t = tmin + (tmax - tmin) * (f32(s) + 0.5) / f32(steps);
        let pos = ro + rd * t;
        let uvw = (pos - u.box_min.xyz) / span;
        let voxel = vec3<i32>(clamp(uvw * dims, vec3<f32>(0.0), dims - 1.0));
        let idx = textureLoad(vol, voxel, 0).r;
        if (idx > max_idx) {
            max_idx = idx;
        }
    }

    if (max_idx < 2u) {
        discard;
    }
    let color = textureLoad(lut, vec2<i32>(i32(max_idx), 0), 0);
    // Opacity ramps with intensity so weak echo is see-through and cores read solid.
    let alpha = clamp((f32(max_idx) - 2.0) / 253.0 * 1.6 + 0.15, 0.0, 1.0);
    return vec4<f32>(color.rgb * alpha, alpha);
}
