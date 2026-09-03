// Instanced sprite pipeline (T1-051, TDD §10.1).
// One quad per instance; position is already projected to screen pixels on
// the CPU, depth comes from the projected y so the depth buffer gives
// painter's order without a CPU sort.

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
    @location(2) frame_facing: u32,
    @location(3) tint: vec4<f32>,
    @location(4) scale: f32,
    @location(5) flags: u32,
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

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    let c = corner(vi);
    let frame = f32(inst.frame_facing & 0xffffu);
    let facing = f32((inst.frame_facing >> 16u) & 0xffu);

    let offset = (c * atlas.frame - atlas.origin) * inst.scale;
    let screen = inst.pos + offset;
    let ndc = vec2<f32>(screen.x / globals.screen.x * 2.0 - 1.0,
                        1.0 - screen.y / globals.screen.y * 2.0);

    var out: VsOut;
    out.clip = vec4<f32>(ndc, inst.depth, 1.0);
    out.uv = (vec2<f32>(frame, facing) + c) * atlas.frame * atlas.inv_size;
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
