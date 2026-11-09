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
    // Kind (0 shape, 1 glyph), border width, opacity, blur in logical pixels.
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
    // A pixel past the edge, for its smoothing, and as far as it's blurred. A glyph's quad has
    // room around it already.
    var grow = 1.0 / frame.scale + item.params.w;
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
    // WGSL's clip space has y up, the scene's y goes down.
    let clip = p / frame.size * 2.0 - 1.0;
    out.position = vec4<f32>(clip.x, -clip.y, 0.0, 1.0);
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
    let blur = item.params.w * frame.scale;
    if item.params.x > 0.5 {
        let uv = mix(item.uv.xy, item.uv.zw, (in.p - item.rect.xy) / item.rect.zw);
        var alpha = textureSampleLevel(atlas, atlas_sampler, uv, 0.0).r;
        if blur > 0.05 {
            // Taps on a spiral out to the blur's reach, nearer ones counting more.
            let texel = 1.0 / vec2<f32>(textureDimensions(atlas));
            let reach = min(blur, 7.0);
            var sum = alpha;
            var weight = 1.0;
            for (var i = 0; i < 16; i++) {
                let t = (f32(i) + 0.5) / 16.0;
                let angle = f32(i) * 2.39996;
                let offset = vec2<f32>(cos(angle), sin(angle)) * sqrt(t) * reach;
                let w = 1.0 - 0.6 * t;
                sum += textureSampleLevel(atlas, atlas_sampler, uv + offset * texel, 0.0).r * w;
                weight += w;
            }
            alpha = sum / weight;
        }
        color = item.fill * alpha;
    } else {
        let d = distance_to(in.p, item.rect, item.radii);
        // Blurred, the edge spreads over the blur's width.
        let width = max(1.0, blur * 2.0);
        let coverage = clamp(0.5 - d * frame.scale / width, 0.0, 1.0);
        let border = item.params.y;
        var inside = coverage;
        if border > 0.0 {
            inside = clamp(0.5 - (d + border) * frame.scale / width, 0.0, 1.0);
        }
        color = item.fill * inside + item.border_color * (coverage - inside);
    }
    if item.clip_rect.z >= 0.0 {
        let d = distance_to(in.p, item.clip_rect, item.clip_radii);
        color = color * clamp(0.5 - d * frame.scale, 0.0, 1.0);
    }
    return color * item.params.z;
}
