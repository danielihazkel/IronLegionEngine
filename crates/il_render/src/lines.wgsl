// Line-list pipeline (T1-053 outlines, T1-054 debug overlays).
// Vertices arrive already projected to screen pixels with a per-vertex
// colour; drawn over the sprites with alpha blending and no depth test.

struct Globals {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Vertex {
    @location(0) pos: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

@vertex
fn vs_main(v: Vertex) -> VsOut {
    let ndc = vec2<f32>(v.pos.x / globals.screen.x * 2.0 - 1.0,
                        1.0 - v.pos.y / globals.screen.y * 2.0);
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.colour = v.colour;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.colour;
}
