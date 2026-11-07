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
// Drawn under what's behind where that's see-through, like where xray has no wallpaper.
uniform vec4 bg_color;

// How far in from its edge the glass curves, in logical pixels.
uniform float bezel;
// How tall the glass is where it's flat, in logical pixels.
uniform float thickness;
uniform float refractive_index;
// How much more red bends than blue, as a fraction of the bend.
uniform float dispersion;
// How bright the light the bezel reflects is.
uniform float highlight;
// How shiny the surface is, from rough 0 to polished 1.
uniform float sheen;
// How big the light looks, from a point 0 to a broad window 1.
uniform float light_size;
// Towards the light, in screen coordinates.
uniform vec2 light_direction;

// Shapes closer than this melt into one piece.
const float MELT = 20.0;
// How high the light is over the screen, in radians.
const float LIGHT_ELEVATION = 0.7;
// How much the bezel darkens away from the light, and brightens towards it.
const float SHADING = 0.35;
// How much of the surroundings the bezel reflects where it's seen edge on.
const float REFLECTION = 0.5;

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

// How deep into a rounded rectangle the bezel's surface takes a point to be, as a signed
// distance like `rounded_box`'s, with its slope in `slope`.
//
// Deeper than a corner's radius, the distance to the nearest edge turns a sharp corner along the
// diagonal, which would fold the bezel there like a mitred frame. Instead, a point's depth is how
// far the outline has to shrink to pass through it, keeping its corners' radius as it shrinks:
// at the outline it's the distance to the edge, and deeper in its lines stay rounded, so the
// surface rolls around a corner as one curve, at any bezel.
float surface_box(vec2 p, vec4 rect, vec4 radii, out vec2 slope) {
    vec2 half_size = rect.zw * 0.5;
    vec2 q = p - rect.xy - half_size;
    float radius = corner_radius(q, half_size, radii);
    float outline = box_distance(q, half_size, radius);
    if (outline > 0.0) {
        slope = box_normal(q, half_size, radius);
        return outline;
    }

    vec2 side = vec2(q.x < 0.0 ? -1.0 : 1.0, q.y < 0.0 ? -1.0 : 1.0);
    vec2 d = abs(q) - half_size + radius;
    // In a corner, the shrunk outline's corner, centred `depth` further in on both axes, passes
    // through the point: |d + depth| = radius.
    float spread = 2.0 * radius * radius - (d.x - d.y) * (d.x - d.y);
    if (spread >= 0.0) {
        float depth = (sqrt(spread) - d.x - d.y) * 0.5;
        if (d.x + depth > 0.0 && d.y + depth > 0.0) {
            slope = side * normalize(d + depth);
            return -depth;
        }
    }
    // Along a side, it's the distance to the nearer side.
    slope = d.x > d.y ? vec2(side.x, 0.0) : vec2(0.0, side.y);
    return max(d.x, d.y) - radius;
}

// Polynomial smooth minimum: like min(), with a rounded seam where a and b are within k.
float smooth_min(float a, float b, float k) {
    float h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
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

// How deep the bezel's surface takes a point to be, as `glass` does the outline.
float surface(vec2 p) {
    vec2 slope;
    float d = surface_box(p, shape0, radii0, slope);
    if (shape_count > 1.5) d = smooth_min(d, surface_box(p, shape1, radii1, slope), MELT);
    if (shape_count > 2.5) d = smooth_min(d, surface_box(p, shape2, radii2, slope), MELT);
    if (shape_count > 3.5) d = smooth_min(d, surface_box(p, shape3, radii3, slope), MELT);
    if (shape_count > 4.5) d = smooth_min(d, surface_box(p, shape4, radii4, slope), MELT);
    if (shape_count > 5.5) d = smooth_min(d, surface_box(p, shape5, radii5, slope), MELT);
    if (shape_count > 6.5) d = smooth_min(d, surface_box(p, shape6, radii6, slope), MELT);
    if (shape_count > 7.5) d = smooth_min(d, surface_box(p, shape7, radii7, slope), MELT);
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

    // How deep the bezel's surface is, and which way it faces: exact for one shape, from the
    // distance's slope where shapes melt together.
    float depth;
    vec2 grad;
    if (shape_count < 1.5) {
        depth = surface_box(p, shape0, radii0, grad);
    } else {
        depth = surface(p);
        grad = vec2(
            surface(p + vec2(0.5, 0.0)) - surface(p - vec2(0.5, 0.0)),
            surface(p + vec2(0.0, 0.5)) - surface(p - vec2(0.0, 0.5))
        );
    }
    vec2 normal = dot(grad, grad) > 1e-8 ? normalize(grad) : vec2(0.0);

    // 1 at the edge, 0 where the bezel meets the flat middle. The bezel's surface is a squircle,
    // rising from nothing at the edge to the full thickness: unlike a circle, its curvature fades
    // to nothing at the flat middle, so the magnification eases out instead of stopping at a
    // crease.
    float rim = clamp(1.0 + depth / max(bezel, 1e-3), 0.0, 1.0);
    float rise4 = max(1.0 - rim * rim * rim * rim, 0.0);
    float height = thickness * pow(rise4, 0.25);
    float slope = thickness / max(bezel, 1e-3) * rim * rim * rim / max(pow(rise4, 0.75), 1e-4);
    // The surface's normal, up out of the screen.
    vec3 surface = normalize(vec3(normal * slope, 1.0));

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

    // The light, over the screen in its direction.
    vec3 light = vec3(light_direction * cos(LIGHT_ELEVATION), sin(LIGHT_ELEVATION));

    // The bezel's side facing the light is lit, the far side in its shade.
    float shade = clamp(dot(surface, light) - light.z, -1.0, 1.0);
    behind.rgb *= 1.0 + SHADING * shade;

    // Seen edge on, glass reflects more and lets less through (Schlick's Fresnel).
    float f0 = (refractive_index - 1.0) / (refractive_index + 1.0);
    f0 *= f0;
    float fresnel = f0 + (1.0 - f0) * pow(1.0 - surface.z, 5.0);
    behind.rgb = mix(behind.rgb, vec3(behind.a), fresnel * REFLECTION);

    // The light's reflection where the bezel tilts towards it, and a weaker one from the far
    // side, as light that went through the glass comes back off its inside. A rough surface and
    // a big light both spread it, broader and dimmer, the same light spread over more of the
    // bezel; a polished surface and a small light keep it tight and bright.
    float roughness = 1.0 - sheen;
    float spread = 0.06 + 0.6 * sqrt(roughness * roughness + light_size * light_size);
    float shininess = 2.0 / (spread * spread) - 2.0;
    // Bright as it is at a shininess of 60, the tight reflection the light is tuned for.
    float brightness = (shininess + 2.0) / 62.0;
    vec3 view = vec3(0.0, 0.0, 1.0);
    vec3 halfway = normalize(light + view);
    vec3 back = vec3(-light.xy, light.z);
    vec3 back_halfway = normalize(back + view);
    float specular = pow(max(dot(surface, halfway), 0.0), shininess)
        + 0.5 * pow(max(dot(surface, back_halfway), 0.0), shininess);
    // A broad reflection reaches the flat middle too; that even glow is left out, so the light
    // stays on the bezel.
    float flat_specular = pow(halfway.z, shininess) + 0.5 * pow(back_halfway.z, shininess);
    specular = clamp(max(specular - flat_specular, 0.0) * brightness * highlight, 0.0, 1.0);

    // The reflection goes over what's behind, both premultiplied.
    vec4 result = vec4(vec3(specular), specular) + behind * (1.0 - specular);
    result = result * coverage * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
