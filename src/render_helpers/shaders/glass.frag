#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float niri_scale;

// The element's size, and the mappings between texture coordinates and the element's [0, 1].
uniform vec2 geo_size;
uniform mat3 input_to_geo;
uniform mat3 geo_to_input;

// The shapes: x, y, width and height in the element's logical pixels, with the radii of their
// top left, top right, bottom right and bottom left corners.
uniform float shape_count;
uniform vec4 shape0;
uniform vec4 shape1;
uniform vec4 shape2;
uniform vec4 shape3;
uniform vec4 shape4;
uniform vec4 shape5;
uniform vec4 shape6;
uniform vec4 shape7;
uniform vec4 radii0;
uniform vec4 radii1;
uniform vec4 radii2;
uniform vec4 radii3;
uniform vec4 radii4;
uniform vec4 radii5;
uniform vec4 radii6;
uniform vec4 radii7;

uniform float noise;
uniform float saturation;
// The window colour, and in alpha how far what's behind is taken towards it, so what's on the
// glass reads over anything.
uniform vec4 shade;
// Drawn under what's behind where that's see-through, like where xray has no wallpaper.
uniform vec4 bg_color;

// How far in from its edge the glass curves, in logical pixels.
uniform float bezel;
// How tall the glass is where it's flat, in logical pixels.
uniform float thickness;
uniform float refractive_index;
// How much more red bends than blue, as a fraction of the bend.
uniform float dispersion;
// How much of its surroundings the bezel reflects where it's seen edge on.
uniform float reflection;
// How shiny the surface is, from rough 0 to polished 1.
uniform float sheen;

// The lights that shine: towards each, over the screen in screen coordinates, with its intensity
// in w, and how big each looks, from a point 0 to a broad window 1.
uniform float light_count;
uniform vec4 light0;
uniform vec4 light1;
uniform vec4 light2;
uniform vec4 light3;
uniform vec4 light_sizes;

// How much of each colour glass as thick as it is takes out of the light coming through: minus
// the log of the colour it takes on, so 0 is clear.
uniform vec3 absorbance;
// The pattern pressed into the surface: 0 for smooth, 1 reeded, 2 hammered, 3 frosted, and how
// deep, from 0 for flat to 1.
uniform float pattern;
uniform float pattern_depth;
// The pointer, in the element's logical pixels, with the intensity of the light it carries in z,
// 0 for none.
uniform vec3 pointer;

// Shapes closer than this melt into one piece.
const float MELT = 20.0;
// How much a light of intensity 1 darkens the bezel away from it and brightens it towards it.
const float SHADING = 0.6;
// The steepest a pattern pressed in all the way slopes.
const float PATTERN_SLOPE = 0.6;
// How wide a reeded rib is, how far apart hammered dimples are, and how fine frost's grain is,
// in logical pixels.
const float REED = 14.0;
const float DIMPLE = 28.0;
const float GRAIN = 3.0;
// How high over the screen the pointer's light is, in logical pixels, which is also about how far
// from it it fades to half, and how big it looks.
const float POINTER_HEIGHT = 80.0;
const float POINTER_SIZE = 0.15;
// How far from the pointer its light reaches, in logical pixels, fading out on the way so it ends
// without an edge; bento redraws glass for the pointer only within it, as `POINTER_REACH` in
// `glass.rs` says.
const float POINTER_REACH = 400.0;

// The radius of the corner of a box nearest `q`, from the box's middle, fit to the box.
float corner_radius(vec2 q, vec2 half_size, vec4 radii) {
    float radius = q.x < 0.0 ? (q.y < 0.0 ? radii.x : radii.w) : (q.y < 0.0 ? radii.y : radii.z);
    return min(radius, min(half_size.x, half_size.y));
}

// Signed distance to a box with rounded corners, from its middle, negative inside.
float box_distance(vec2 q, vec2 half_size, float radius) {
    vec2 d = abs(q) - half_size + radius;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - radius;
}

// The slope of `box_distance`: its outward normal, exactly.
vec2 box_normal(vec2 q, vec2 half_size, float radius) {
    vec2 d = abs(q) - half_size + radius;
    vec2 side = vec2(q.x < 0.0 ? -1.0 : 1.0, q.y < 0.0 ? -1.0 : 1.0);
    if (d.x > 0.0 && d.y > 0.0) {
        return side * normalize(d);
    }
    return d.x > d.y ? vec2(side.x, 0.0) : vec2(0.0, side.y);
}

// Signed distance to a rounded rectangle, negative inside.
float rounded_box(vec2 p, vec4 rect, vec4 radii) {
    vec2 half_size = rect.zw * 0.5;
    vec2 q = p - rect.xy - half_size;
    return box_distance(q, half_size, corner_radius(q, half_size, radii));
}

// Polynomial smooth minimum: like min(), with a rounded seam where a and b are within k.
float smooth_min(float a, float b, float k) {
    float h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

// The radius of a corner of the outline once it has shrunk by `depth`: it grows as the outline
// shrinks, so the surface rolls around the corner over about the bezel's width, until it has to
// fit what's left of the shape.
float shrunk_radius(float radius, float depth, float inradius) {
    return max(smooth_min(radius + depth, max(inradius - depth, 0.0), 0.5 * inradius), 0.0);
}

// How far `q` is outside the outline shrunk by `depth`, negative inside it.
float outside_shrunk(vec2 q, vec2 half_size, float radius, float depth, float inradius) {
    return box_distance(q, half_size - depth, shrunk_radius(radius, depth, inradius));
}

// How deep into a rounded rectangle the bezel's surface takes a point to be, as a signed
// distance like `rounded_box`'s, with its slope in `slope`.
//
// A point's depth is how far the outline has to shrink to pass through it. At the edge that's the
// distance to the edge, and as the outline shrinks its corners grow rounder, so the surface rolls
// around a corner as one smooth curve instead of folding along the diagonal, at any bezel.
float surface_box(vec2 p, vec4 rect, vec4 radii, out vec2 slope) {
    vec2 half_size = rect.zw * 0.5;
    vec2 q = p - rect.xy - half_size;
    float radius = corner_radius(q, half_size, radii);
    float outline = box_distance(q, half_size, radius);
    if (outline > 0.0) {
        slope = box_normal(q, half_size, radius);
        return outline;
    }

    // Newton's method from the distance to the edge, where it's exact along the sides; three
    // steps settle it to within a hundredth of a pixel in the corners.
    float inradius = min(half_size.x, half_size.y);
    float depth = -outline;
    for (int i = 0; i < 3; i++) {
        float f = outside_shrunk(q, half_size, radius, depth, inradius);
        float g = (outside_shrunk(q, half_size, radius, depth + 0.25, inradius) - f) / 0.25;
        depth = clamp(depth - f / max(g, 0.5), 0.0, inradius);
    }

    slope = box_normal(q, half_size - depth, shrunk_radius(radius, depth, inradius));
    return -depth;
}

// Signed distance to the glass's edge, negative inside.
float glass(vec2 p) {
    float d = rounded_box(p, shape0, radii0);
    if (shape_count > 1.5) d = smooth_min(d, rounded_box(p, shape1, radii1), MELT);
    if (shape_count > 2.5) d = smooth_min(d, rounded_box(p, shape2, radii2), MELT);
    if (shape_count > 3.5) d = smooth_min(d, rounded_box(p, shape3, radii3), MELT);
    if (shape_count > 4.5) d = smooth_min(d, rounded_box(p, shape4, radii4), MELT);
    if (shape_count > 5.5) d = smooth_min(d, rounded_box(p, shape5, radii5), MELT);
    if (shape_count > 6.5) d = smooth_min(d, rounded_box(p, shape6, radii6), MELT);
    if (shape_count > 7.5) d = smooth_min(d, rounded_box(p, shape7, radii7), MELT);
    return d;
}

// Half a shape's narrowest side: the widest its bezel gets, as two sides' bezels would meet in a
// ridge instead of rising to one rounded top, and the furthest its refraction reaches, past
// which it would show the far side's background.
float inradius_of(vec4 rect) {
    return 0.5 * min(rect.z, rect.w);
}

// Melts the surface of a shape, `d2` deep with inradius `r2`, into `d` and `r`: the depths melt
// as the outlines do, and the inradius follows the nearer shape, blended across the seam, so a
// thin shape keeps its own beside a wide one.
void melt(inout float d, inout float r, float d2, float r2) {
    float nearer = clamp(0.5 + 0.5 * (d2 - d) / MELT, 0.0, 1.0);
    r = mix(r2, r, nearer);
    d = smooth_min(d, d2, MELT);
}

// How deep the bezel's surface takes a point to be, as `glass` does the outline, with the
// inradius of the glass there.
float surface(vec2 p, out float inradius) {
    vec2 slope;
    float d = surface_box(p, shape0, radii0, slope);
    inradius = inradius_of(shape0);
    if (shape_count > 1.5) melt(d, inradius, surface_box(p, shape1, radii1, slope), inradius_of(shape1));
    if (shape_count > 2.5) melt(d, inradius, surface_box(p, shape2, radii2, slope), inradius_of(shape2));
    if (shape_count > 3.5) melt(d, inradius, surface_box(p, shape3, radii3, slope), inradius_of(shape3));
    if (shape_count > 4.5) melt(d, inradius, surface_box(p, shape4, radii4, slope), inradius_of(shape4));
    if (shape_count > 5.5) melt(d, inradius, surface_box(p, shape5, radii5, slope), inradius_of(shape5));
    if (shape_count > 6.5) melt(d, inradius, surface_box(p, shape6, radii6, slope), inradius_of(shape6));
    if (shape_count > 7.5) melt(d, inradius, surface_box(p, shape7, radii7, slope), inradius_of(shape7));
    return d;
}

// A light's reflection on the surface: where the bezel tilts towards it, and weaker from the far
// side, as light that went through the glass comes back off its inside. A rough surface and a
// big light both spread it, broader and dimmer, the same light spread over more of the bezel; a
// polished surface and a small light keep it tight and bright. With `on_bezel` 1, the even glow
// a broad reflection gives the flat middle is left out, so a far light stays on the bezel; a near
// light keeps it, as its spot on the flat middle is under it.
float specular(vec3 surface, vec3 light, float size, float on_bezel) {
    float roughness = 1.0 - sheen;
    float spread = 0.06 + 0.6 * sqrt(roughness * roughness + size * size);
    float shininess = 2.0 / (spread * spread) - 2.0;
    // Bright as it is at a shininess of 60, the tight reflection lights are tuned for.
    float brightness = (shininess + 2.0) / 62.0;
    vec3 view = vec3(0.0, 0.0, 1.0);
    vec3 halfway = normalize(light + view);
    vec3 back_halfway = normalize(vec3(-light.xy, light.z) + view);
    float reflected = pow(max(dot(surface, halfway), 0.0), shininess)
        + 0.5 * pow(max(dot(surface, back_halfway), 0.0), shininess);
    float on_flat = pow(halfway.z, shininess) + 0.5 * pow(back_halfway.z, shininess);
    return max(reflected - on_flat * on_bezel, 0.0) * brightness;
}

// Sin-less white noise by David Hoskins (MIT License).
// https://www.shadertoy.com/view/4djSRW
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

vec2 hash22(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// Smooth noise from 0 to 1, varying over about one unit.
float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float bottom = mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x);
    float top = mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x);
    return mix(bottom, top, u.y);
}

// The slope of the pattern's surface at `p`, uphill, at its steepest about 1.
vec2 pattern_gradient(vec2 p) {
    if (pattern < 0.5) {
        return vec2(0.0);
    }
    if (pattern < 1.5) {
        // Each rib is a rounded ridge, rising from a crease to its top and falling to the next.
        float across = fract(p.x / REED) * 2.0 - 1.0;
        return vec2(-across, 0.0);
    }
    if (pattern < 2.5) {
        // Each dimple is a shallow bowl around the nearest of points scattered one to a cell,
        // rising away from its middle to the creases where dimples meet.
        vec2 cell = floor(p / DIMPLE);
        vec2 nearest = vec2(0.0);
        float best = 1e9;
        for (int j = -1; j <= 1; j++) {
            for (int i = -1; i <= 1; i++) {
                vec2 c = cell + vec2(float(i), float(j));
                vec2 away = p - (c + 0.15 + 0.7 * hash22(c)) * DIMPLE;
                float d = dot(away, away);
                if (d < best) {
                    best = d;
                    nearest = away;
                }
            }
        }
        return nearest / (0.6 * DIMPLE);
    }
    // Frost's grain: smooth noise, at two sizes so it doesn't look like a grid.
    vec2 q = p / GRAIN;
    vec2 dx = vec2(0.5, 0.0);
    vec2 dy = vec2(0.0, 0.5);
    vec2 fine = vec2(
        value_noise(q + dx) - value_noise(q - dx),
        value_noise(q + dy) - value_noise(q - dy)
    );
    q = q * 0.37 + 17.0;
    vec2 coarse = vec2(
        value_noise(q + dx) - value_noise(q - dx),
        value_noise(q + dy) - value_noise(q - dy)
    );
    return fine + 0.6 * coarse;
}

void main() {
    vec2 p = (input_to_geo * vec3(v_coords, 1.0)).xy * geo_size;
    float d = glass(p);

    float coverage = clamp(0.5 - d * niri_scale, 0.0, 1.0);
    if (coverage <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    // How deep the bezel's surface is, which way it faces, and the glass's inradius there: exact
    // for one shape, from the distance's slope where shapes melt together.
    float depth;
    vec2 grad;
    float inradius;
    if (shape_count < 1.5) {
        depth = surface_box(p, shape0, radii0, grad);
        inradius = inradius_of(shape0);
    } else {
        depth = surface(p, inradius);
        float r;
        grad = vec2(
            surface(p + vec2(0.5, 0.0), r) - surface(p - vec2(0.5, 0.0), r),
            surface(p + vec2(0.0, 0.5), r) - surface(p - vec2(0.0, 0.5), r)
        );
    }
    vec2 normal = dot(grad, grad) > 1e-8 ? normalize(grad) : vec2(0.0);

    // 1 at the edge, 0 where the bezel meets the flat middle. The bezel's surface is a squircle,
    // rising from nothing at the edge to the full thickness: unlike a circle, its curvature fades
    // to nothing at the flat middle, so the magnification eases out instead of stopping at a
    // crease. It's never wider than the glass's inradius where it is.
    float width = max(min(bezel, inradius), 1e-3);

    float rim = clamp(1.0 + depth / width, 0.0, 1.0);
    float rise4 = max(1.0 - rim * rim * rim * rim, 0.0);
    float height = thickness * pow(rise4, 0.25);
    float slope = thickness / width * rim * rim * rim / max(pow(rise4, 0.75), 1e-4);
    // Which way and how steeply the surface leans out: the bezel's fall, and the pattern pressed
    // into the top, which the bezel's curve takes over towards the edge.
    vec2 lean = normal * slope
        - pattern_gradient(p) * pattern_depth * PATTERN_SLOPE * (1.0 - rim);
    float steepness = length(lean);
    vec2 facing = steepness > 1e-6 ? lean / steepness : vec2(0.0);
    // The surface's normal, up out of the screen.
    vec3 surface = normalize(vec3(lean, 1.0));

    // Looking straight down, the ray meets the surface tilted by `tilt` and bends towards its
    // normal, so it lands on what's behind further in: the bezel magnifies.
    float tilt = atan(steepness);
    float bent = asin(sin(tilt) / max(refractive_index, 1.0));
    // Never past the middle of the glass where it is, where it would show the far side's
    // background.
    float bend = min(height * tan(tilt - bent), max(inradius + depth, 0.0));

    // What's behind, seen through the bezel, red bent more than blue.
    vec2 shift = (geo_to_input * vec3(-facing * bend / geo_size, 0.0)).xy;
    vec4 red = texture2D(tex, v_coords + shift * (1.0 + dispersion));
    vec4 green = texture2D(tex, v_coords + shift);
    vec4 blue = texture2D(tex, v_coords + shift * (1.0 - dispersion));
    vec4 behind = vec4(red.r, green.g, blue.b, green.a);
    behind = behind + bg_color * (1.0 - behind.a);

    const vec3 luma = vec3(0.2126, 0.7152, 0.0722);
    behind.rgb = mix(vec3(dot(behind.rgb, luma)), behind.rgb, saturation);
    behind.rgb += (hash12(gl_FragCoord.xy) - 0.5) * noise * behind.a;
    behind.rgb = mix(behind.rgb, shade.rgb * behind.a, shade.a);

    // Light takes on the glass's colour the further it comes through it: down through it, along
    // the bent ray, and, the more the surface faces sideways, along inside the bezel from the
    // edge, so the colour deepens towards the edge.
    float through = height / max(cos(tilt - bent), 0.2) + 2.0 * thickness * (1.0 - surface.z);
    behind.rgb *= exp(-absorbance * through / max(thickness, 1.0));

    // Each light lights the bezel's side facing it, and shades the far side, against how it
    // lights the flat middle.
    float shade = 0.0;
    float shine = 0.0;
    for (int i = 0; i < 4; i++) {
        if (float(i) >= light_count) break;
        vec4 light = i == 0 ? light0 : i == 1 ? light1 : i == 2 ? light2 : light3;
        float size = i == 0 ? light_sizes.x : i == 1 ? light_sizes.y
            : i == 2 ? light_sizes.z : light_sizes.w;
        shade += light.w * clamp(dot(surface, light.xyz) - light.z, -1.0, 1.0);
        shine += light.w * specular(surface, light.xyz, size, 1.0);
    }
    behind.rgb *= max(1.0 + SHADING * shade, 0.0);

    // The pointer's light, over it: it glints on the parts of the surface facing it, the flat
    // middle under it too, fading as they get further from it.
    if (pointer.z > 0.0) {
        vec3 to_pointer = vec3(pointer.xy - p, POINTER_HEIGHT);
        float h2 = POINTER_HEIGHT * POINTER_HEIGHT;
        float at_reach = h2 / (POINTER_REACH * POINTER_REACH + h2);
        float falloff = max(h2 / dot(to_pointer, to_pointer) - at_reach, 0.0) / (1.0 - at_reach);
        shine += pointer.z * falloff * specular(surface, normalize(to_pointer), POINTER_SIZE, 0.0);
    }

    // Seen edge on, glass reflects more and lets less through (Schlick's Fresnel).
    float f0 = (refractive_index - 1.0) / (refractive_index + 1.0);
    f0 *= f0;
    float fresnel = f0 + (1.0 - f0) * pow(1.0 - surface.z, 5.0);
    behind.rgb = mix(behind.rgb, vec3(behind.a), fresnel * reflection);

    // The lights' reflections go over what's behind, both premultiplied.
    shine = clamp(shine, 0.0, 1.0);
    vec4 result = vec4(vec3(shine), shine) + behind * (1.0 - shine);
    result = result * coverage * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
