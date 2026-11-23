#version 330 core

// Blends this frame's share of the lamps' light (`light_trace.glsl`) into
// the frames before, found by moving each texel with the camera, as
// `accumulate.glsl` does for the sun: rgb the share, a the surface's
// distance (clip w). Where no shadow casting lamp reaches, 1.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D current;
uniform sampler2D history;
uniform sampler2D sceneDepth;
uniform mat4 reprojection;
uniform vec4 inverseW;
uniform int historyValid;
uniform float currentWeight;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(current, 0);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    vec4 ndc = vec4(uv * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    float now = 1.0 / max(dot(inverseW, ndc), 1e-6);
    vec4 here = texelFetch(current, texel, 0);
    if (depth >= 1.0 || here.a == 0.0) {
        FragColor = vec4(1.0, 1.0, 1.0, now);
        return;
    }
    vec3 light = here.rgb;

    // What this frame's neighbours allow, a little widened: one ray per
    // pixel among several lamps is noisy.
    vec3 low = light;
    vec3 high = light;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec4 c = texelFetch(current, clamp(texel + ivec2(x, y), ivec2(0), size - 1), 0);
            if (c.a == 0.0) {
                continue;
            }
            low = min(low, c.rgb);
            high = max(high, c.rgb);
        }
    }
    low = max(low - 0.1, vec3(0.0));
    high = high + 0.1;

    vec3 result = light;
    vec4 previous = reprojection * ndc;
    if (historyValid == 1 && previous.w > 1e-6) {
        vec2 previousUv = previous.xy / previous.w * 0.5 + 0.5;
        float expected = previous.w * now;
        vec2 at = previousUv * vec2(size) - 0.5;
        ivec2 base = ivec2(floor(at));
        vec2 f = at - vec2(base);
        vec3 sum = vec3(0.0);
        float weights = 0.0;
        for (int i = 0; i < 4; i++) {
            ivec2 offset = ivec2(i & 1, i >> 1);
            ivec2 t = base + offset;
            if (any(lessThan(t, ivec2(0))) || any(greaterThanEqual(t, size))) {
                continue;
            }
            vec4 h = texelFetch(history, t, 0);
            if (abs(h.a - expected) > 0.03 * expected + 0.05) {
                continue;
            }
            vec2 b = mix(1.0 - f, f, vec2(offset));
            float w = b.x * b.y;
            sum += h.rgb * w;
            weights += w;
        }
        if (weights > 0.01) {
            vec3 before = clamp(sum / weights, low, high);
            float weight = mix(1.0, currentWeight, clamp(weights, 0.0, 1.0));
            result = mix(before, light, weight);
        }
    }
    FragColor = vec4(result, now);
}
