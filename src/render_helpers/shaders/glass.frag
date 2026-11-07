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

// The shapes: x, y, width and height in the element's logical pixels, with a radius each.
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

uniform float noise;
uniform float saturation;
// Drawn under what's behind where that's see-through, like where xray has no wallpaper.
uniform vec4 bg_color;

// How far in from its edge the glass curves, in logical pixels.
uniform float bezel;
// How tall the glass is where it's flat, in logical pixels.
uniform float thickness;
uniform float refractive_index;
// How much more red bends than blue, as a fraction of the bend.
uniform float dispersion;
// How bright the lit edge is.
uniform float highlight;
// Towards the light, in screen coordinates.
uniform vec2 light_direction;

// Shapes closer than this melt into one piece.
const float MELT = 20.0;
// The width of the lit edge.
const float EDGE = 1.5;

float rounded_box(vec2 p, vec4 rect, float radius) {
    vec2 half_size = rect.zw * 0.5;
    radius = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p - rect.xy - half_size) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

// Polynomial smooth minimum: like min(), with a rounded seam where a and b are within k.
float smooth_min(float a, float b, float k) {
    float h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

// Signed distance to the glass's edge, negative inside.
float glass(vec2 p) {
    float d = rounded_box(p, shape0, radii0.x);
    if (shape_count > 1.5) d = smooth_min(d, rounded_box(p, shape1, radii0.y), MELT);
    if (shape_count > 2.5) d = smooth_min(d, rounded_box(p, shape2, radii0.z), MELT);
    if (shape_count > 3.5) d = smooth_min(d, rounded_box(p, shape3, radii0.w), MELT);
    if (shape_count > 4.5) d = smooth_min(d, rounded_box(p, shape4, radii1.x), MELT);
    if (shape_count > 5.5) d = smooth_min(d, rounded_box(p, shape5, radii1.y), MELT);
    if (shape_count > 6.5) d = smooth_min(d, rounded_box(p, shape6, radii1.z), MELT);
    if (shape_count > 7.5) d = smooth_min(d, rounded_box(p, shape7, radii1.w), MELT);
    return d;
}

// Sin-less white noise by David Hoskins (MIT License).
// https://www.shadertoy.com/view/4djSRW
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    vec2 p = (input_to_geo * vec3(v_coords, 1.0)).xy * geo_size;
    float d = glass(p);

    float coverage = clamp(0.5 - d * niri_scale, 0.0, 1.0);
    if (coverage <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    // The edge's outward normal.
    vec2 grad = vec2(
        glass(p + vec2(0.5, 0.0)) - glass(p - vec2(0.5, 0.0)),
        glass(p + vec2(0.0, 0.5)) - glass(p - vec2(0.0, 0.5))
    );
    vec2 normal = dot(grad, grad) > 1e-8 ? normalize(grad) : vec2(0.0);

    // 1 at the edge, 0 where the bezel meets the flat middle. The bezel's surface is a squircle,
    // rising from nothing at the edge to the full thickness: unlike a circle, its curvature fades
    // to nothing at the flat middle, so the magnification eases out instead of stopping at a
    // crease.
    float rim = clamp(1.0 + d / max(bezel, 1e-3), 0.0, 1.0);
    float rise4 = max(1.0 - rim * rim * rim * rim, 0.0);
    float height = thickness * pow(rise4, 0.25);
    float slope = thickness / max(bezel, 1e-3) * rim * rim * rim / max(pow(rise4, 0.75), 1e-4);

    // Looking straight down, the ray meets the surface tilted by `tilt` and bends towards its
    // normal, so it lands on what's behind further in: the bezel magnifies.
    float tilt = atan(slope);
    float bent = asin(sin(tilt) / max(refractive_index, 1.0));
    float bend = height * tan(tilt - bent);

    // What's behind, seen through the bezel, red bent more than blue.
    vec2 shift = (geo_to_input * vec3(-normal * bend / geo_size, 0.0)).xy;
    vec4 red = texture2D(tex, v_coords + shift * (1.0 + dispersion));
    vec4 green = texture2D(tex, v_coords + shift);
    vec4 blue = texture2D(tex, v_coords + shift * (1.0 - dispersion));
    vec4 behind = vec4(red.r, green.g, blue.b, green.a);
    behind = behind + bg_color * (1.0 - behind.a);

    const vec3 luma = vec3(0.2126, 0.7152, 0.0722);
    behind.rgb = mix(vec3(dot(behind.rgb, luma)), behind.rgb, saturation);
    behind.rgb += (hash12(gl_FragCoord.xy) - 0.5) * noise * behind.a;

    // The edge catches the light where it faces it, and a little on the far side too.
    float facing = dot(normal, light_direction);
    float edge = 1.0 - smoothstep(0.0, EDGE, -d);
    float lit = highlight * (0.27 + pow(max(facing, 0.0), 2.0) + 0.5 * pow(max(-facing, 0.0), 2.0));
    float sheen = pow(rim, 4.0) * 0.2 * highlight * max(facing, 0.0);
    float light = clamp(edge * lit + sheen, 0.0, 1.0);

    // The light goes over what's behind, both premultiplied.
    vec4 result = vec4(vec3(light), light) + behind * (1.0 - light);
    result = result * coverage * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
