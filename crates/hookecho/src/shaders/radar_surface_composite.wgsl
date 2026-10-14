// Depth-resolved surface in pane clip space. Premultiplied color preserves
// transparent edges when a high-DPI pane samples the bounded offscreen target.
@group(0) @binding(0) var surface: texture_2d<f32>;
@group(0) @binding(1) var sampling: sampler;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Vertex {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0,-1.0), vec2<f32>(3.0,-1.0), vec2<f32>(-1.0,3.0))[i];
    var v: Vertex;
    v.position = vec4<f32>(p,0.0,1.0);
    v.uv = p * vec2<f32>(0.5,-0.5) + vec2<f32>(0.5);
    return v;
}
@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    return textureSample(surface, sampling, v.uv);
}
