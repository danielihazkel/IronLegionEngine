// Terrain pipeline (T1-053, TDD §10.1 terrain).
// A heightmap mesh at height_cell resolution, projected in the vertex shader
// with the same isometric formula as `Camera::world_to_screen`; the fragment
// shader looks the zone index up in an R8 raster and colours it from a
// palette, times the per-vertex slope shade, with faint contour lines so
// elevation reads under the fixed pitch.

struct Globals {
    screen: vec2<f32>,
    center: vec2<f32>,
    // Row-major 2x2 view rotation: view = (rot.xy . d, rot.zw . d).
    rot: vec4<f32>,
    zoom: f32,
    pitch: f32,
    elevation: f32,
    zone_cell: f32,
    zone_dims: vec2<u32>,
    contour: f32,
    _pad: f32,
};

struct Palette {
    colours: array<vec4<f32>, 256>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<uniform> palette: Palette;
@group(0) @binding(2) var zone_tex: texture_2d<u32>;

struct Vertex {
    @location(0) pos: vec2<f32>,
    @location(1) height: f32,
    @location(2) shade: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) height: f32,
    @location(2) shade: f32,
};

@vertex
fn vs_main(v: Vertex) -> VsOut {
    let d = v.pos - globals.center;
    let view = vec2<f32>(dot(globals.rot.xy, d), dot(globals.rot.zw, d));
    let sx = globals.screen.x * 0.5 + view.x * globals.zoom;
    let sy = globals.screen.y * 0.5 - view.y * globals.zoom * globals.pitch
        - v.height * globals.zoom * globals.elevation;
    let ndc = vec2<f32>(sx / globals.screen.x * 2.0 - 1.0, 1.0 - sy / globals.screen.y * 2.0);
    var out: VsOut;
    // Behind every sprite: the sprite pass tests Less against a cleared 1.0.
    out.clip = vec4<f32>(ndc, 1.0, 1.0);
    out.world = v.pos;
    out.height = v.height;
    out.shade = v.shade;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(globals.zone_dims);
    let cell = clamp(vec2<i32>(floor(in.world / globals.zone_cell)), vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    let idx = textureLoad(zone_tex, cell, 0).r;
    var colour = palette.colours[idx].rgb * in.shade;
    // Contour lines every `contour` metres, about 1.5 px wide.
    let level = in.height / globals.contour;
    let w = max(fwidth(level), 1e-4) * 1.5;
    let f = fract(level);
    let line = 1.0 - smoothstep(0.0, w, min(f, 1.0 - f));
    colour = colour * (1.0 - 0.22 * line);
    return vec4<f32>(colour, 1.0);
}
