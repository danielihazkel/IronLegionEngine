// Instanced sprite pipeline (T1-051, TDD §10.1).
// One quad per instance; position is already projected to screen pixels on
// the CPU, depth comes from the projected y so the depth buffer gives
// painter's order without a CPU sort. T3-031: with flag bit 2 the instance
// is a block quad (always frame 0 of its sheet) whose two screen-space axes
// arrive packed as i16 quarter-pixels in the frame and reserved words, so a
// regiment's rank block draws as the parallelogram its rectangle projects to.

struct Globals {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

struct AtlasInfo {
    inv_size: vec2<f32>,
    frame: vec2<f32>,
    origin: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
@group(1) @binding(2) var<uniform> atlas: AtlasInfo;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) depth: f32,
    // Sprite: the atlas column and facing row. Block: the x axis (two i16).
    @location(2) frame_facing: u32,
    @location(3) tint: vec4<f32>,
    @location(4) scale: f32,
    @location(5) flags: u32,
    // Block: the y axis (two i16); unused for sprites.
    @location(6) axis_y: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) @interpolate(flat) flags: u32,
};

// Two triangles: (0,0) (1,0) (0,1) / (1,0) (1,1) (0,1)
fn corner(i: u32) -> vec2<f32> {
    switch i {
        case 0u: { return vec2<f32>(0.0, 0.0); }
        case 1u: { return vec2<f32>(1.0, 0.0); }
        case 2u: { return vec2<f32>(0.0, 1.0); }
        case 3u: { return vec2<f32>(1.0, 0.0); }
        case 4u: { return vec2<f32>(1.0, 1.0); }
        default: { return vec2<f32>(0.0, 1.0); }
    }
}

// An axis packed by `SpriteInstance::pack_axis`: quarter pixels as i16.
fn unpack_axis(bits: u32) -> vec2<f32> {
    return unpack2x16snorm(bits) * (32767.0 / 4.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    let c = corner(vi);
    let block = (inst.flags & 4u) != 0u;

    var screen: vec2<f32>;
    var cell: vec2<f32>;
    if (block) {
        // The quad centre plus the axes spanning the whole quad; frame 0.
        let ax = unpack_axis(inst.frame_facing);
        let ay = unpack_axis(inst.axis_y);
        screen = inst.pos + (c.x - 0.5) * ax + (c.y - 0.5) * ay;
        cell = vec2<f32>(0.0, 0.0);
    } else {
        screen = inst.pos + (c * atlas.frame - atlas.origin) * inst.scale;
        cell = vec2<f32>(f32(inst.frame_facing & 0xffffu),
                         f32((inst.frame_facing >> 16u) & 0xffu));
    }
    let ndc = vec2<f32>(screen.x / globals.screen.x * 2.0 - 1.0,
                        1.0 - screen.y / globals.screen.y * 2.0);

    var out: VsOut;
    out.clip = vec4<f32>(ndc, inst.depth, 1.0);
    out.uv = (cell + c) * atlas.frame * atlas.inv_size;
    out.tint = inst.tint;
    out.flags = inst.flags;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var colour = textureSample(atlas_tex, atlas_sampler, in.uv) * in.tint;
    // Bit 0: selected -> brighten.
    if ((in.flags & 1u) != 0u) {
        colour = vec4<f32>(min(colour.rgb * 1.35 + 0.08, vec3<f32>(1.0)), colour.a);
    }
    return colour;
}
