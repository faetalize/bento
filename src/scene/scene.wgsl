// The scene's shapes and text: one quad per item, edges from signed distances, so they're sharp
// at any size and every number in them can move smoothly.

struct Item {
    // x, y, width, height, in logical pixels.
    rect: vec4<f32>,
    // Top left, top right, bottom right, bottom left.
    radii: vec4<f32>,
    // Premultiplied.
    fill: vec4<f32>,
    border_color: vec4<f32>,
    // A width below zero for no clip.
    clip_rect: vec4<f32>,
    clip_radii: vec4<f32>,
    // The glyph's place in the atlas, from 0 to 1.
    uv: vec4<f32>,
    // Kind (0 shape, 1 glyph), border width, opacity.
    params: vec4<f32>,
}

struct Frame {
    // The output's logical size, and its scale.
    size: vec2<f32>,
    scale: f32,
    pad: f32,
}

@group(0) @binding(0) var<storage, read> items: array<Item>;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

var<immediate> frame: Frame;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) @interpolate(flat) index: u32,
}

@vertex
fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> Out {
    let item = items[index];
    // A pixel past the edge, for its smoothing.
    var grow = 1.0 / frame.scale;
    if item.params.x > 0.5 {
        grow = 0.0;
    }
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[vertex];
    let p = item.rect.xy - vec2<f32>(grow) + c * (item.rect.zw + vec2<f32>(2.0 * grow));
    var out: Out;
    out.position = vec4<f32>(p / frame.size * 2.0 - 1.0, 0.0, 1.0);
    out.p = p;
    out.index = index;
    return out;
}

// The signed distance from `p` to a box of half size `half` around the origin, with corners
// `radii`: top left, top right, bottom right, bottom left.
fn rounded_box(p: vec2<f32>, half: vec2<f32>, radii: vec4<f32>) -> f32 {
    var r: f32;
    if p.x < 0.0 {
        if p.y < 0.0 { r = radii.x; } else { r = radii.w; }
    } else {
        if p.y < 0.0 { r = radii.y; } else { r = radii.z; }
    }
    let q = abs(p) - half + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

fn distance_to(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half = rect.zw * 0.5;
    return rounded_box(p - rect.xy - half, half, radii);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let item = items[in.index];
    var color: vec4<f32>;
    if item.params.x > 0.5 {
        let uv = mix(item.uv.xy, item.uv.zw, (in.p - item.rect.xy) / item.rect.zw);
        color = item.fill * textureSampleLevel(atlas, atlas_sampler, uv, 0.0).r;
    } else {
        let d = distance_to(in.p, item.rect, item.radii);
        let coverage = clamp(0.5 - d * frame.scale, 0.0, 1.0);
        let border = item.params.y;
        var inside = coverage;
        if border > 0.0 {
            inside = clamp(0.5 - (d + border) * frame.scale, 0.0, 1.0);
        }
        color = item.fill * inside + item.border_color * (coverage - inside);
    }
    if item.clip_rect.z >= 0.0 {
        let d = distance_to(in.p, item.clip_rect, item.clip_radii);
        color = color * clamp(0.5 - d * frame.scale, 0.0, 1.0);
    }
    return color * item.params.z;
}
