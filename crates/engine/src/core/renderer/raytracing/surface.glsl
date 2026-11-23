// Where the pixels of the camera's view are, from the depth prepass, and
// noise: shared by the ray traced passes (`raytracing/mod.rs` puts it
// after the `#version` line).

uniform sampler2D sceneDepth;
// NDC to positions relative to the camera (homogeneous).
uniform mat4 inverseRelative;
uniform vec3 eye;
uniform int frame;

vec3 relativeAt(ivec2 texel) {
    ivec2 size = textureSize(sceneDepth, 0);
    texel = clamp(texel, ivec2(0), size - 1);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    vec2 at = (vec2(texel) + 0.5) / vec2(size);
    vec4 p = inverseRelative * vec4(at * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    return p.xyz / p.w;
}

// Noise that spreads well over neighbouring pixels (Jimenez), moved on
// every frame.
float noise(vec2 pixel, int index) {
    pixel += float((frame * 2 + index) % 64) * vec2(47.0, 17.0) * 0.695;
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}


// The surface's normal at `texel` (whose position is `position`) from its
// neighbours, on the side where they lie closer (so edges do not take the
// slope to what lies behind); turned toward the camera.
vec3 normalAt(ivec2 texel, vec3 position) {
    vec3 left = relativeAt(texel - ivec2(1, 0));
    vec3 right = relativeAt(texel + ivec2(1, 0));
    vec3 down = relativeAt(texel - ivec2(0, 1));
    vec3 up = relativeAt(texel + ivec2(0, 1));
    vec3 dx = distance(left, position) < distance(right, position) ? position - left : right - position;
    vec3 dy = distance(down, position) < distance(up, position) ? position - down : up - position;
    vec3 normal = normalize(cross(dx, dy));
    // The camera sits at the origin here.
    return dot(normal, position) > 0.0 ? -normal : normal;
}


// Random numbers (PCG): a state per pixel and frame, then one number per
// call, 0..1.
uint randomState(ivec2 texel, int salt) {
    uint s = uint(texel.x) * 1973u + uint(texel.y) * 9277u + uint(frame) * 26699u + uint(salt) * 104729u;
    return s * 747796405u + 2891336453u;
}

float random(inout uint state) {
    state = state * 747796405u + 2891336453u;
    uint w = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    w = (w >> 22u) ^ w;
    return float(w) * (1.0 / 4294967296.0);
}

// The surface's distance from the camera at `texel` as clip w (as
// `accumulate.glsl` keeps it), from its NDC depth. `inverseW` is row w of
// the inverse view-projection.
float clipW(vec4 inverseW, ivec2 texel, float depth) {
    vec2 at = (vec2(texel) + 0.5) / vec2(textureSize(sceneDepth, 0));
    vec4 ndc = vec4(at * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    return 1.0 / max(dot(inverseW, ndc), 1e-6);
}
