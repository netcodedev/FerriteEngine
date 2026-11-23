#version 330 core

// Blends this frame's ray traced shadows into the frames before, found by
// moving each texel with the camera. Writes the light (r) and the
// surface's distance from the camera (g, clip w), which the next frame
// compares with to drop history that showed another surface (what was
// revealed), and the filter afterwards to blur only within a surface.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D current;
uniform sampler2D history;
uniform sampler2D sceneDepth;
// This frame's NDC to the last frame's clip space.
uniform mat4 reprojection;
// Row w of this frame's inverse view-projection: 1 / clip w from NDC.
uniform vec4 inverseW;
uniform int historyValid;
uniform float currentWeight;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(current, 0);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    vec4 ndc = vec4(uv * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    float now = 1.0 / max(dot(inverseW, ndc), 1e-6);
    float light = texelFetch(current, texel, 0).r;
    if (depth >= 1.0) {
        FragColor = vec4(1.0, now, 0.0, 1.0);
        return;
    }

    // What this frame's neighbours allow: the history may not lie outside
    // (a shadow that moved does not linger).
    float low = light;
    float high = light;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            float c = texelFetch(current, clamp(texel + ivec2(x, y), ivec2(0), size - 1), 0).r;
            low = min(low, c);
            high = max(high, c);
        }
    }

    float result = light;
    vec4 previous = reprojection * ndc;
    if (historyValid == 1 && previous.w > 1e-6) {
        vec2 previousUv = previous.xy / previous.w * 0.5 + 0.5;
        // The surface's distance a frame before.
        float expected = previous.w * now;
        // Bilinear from the four texels around, each only where it showed
        // this surface.
        vec2 at = previousUv * vec2(size) - 0.5;
        ivec2 base = ivec2(floor(at));
        vec2 f = at - vec2(base);
        float sum = 0.0;
        float weights = 0.0;
        for (int i = 0; i < 4; i++) {
            ivec2 offset = ivec2(i & 1, i >> 1);
            ivec2 t = base + offset;
            if (any(lessThan(t, ivec2(0))) || any(greaterThanEqual(t, size))) {
                continue;
            }
            vec2 h = texelFetch(history, t, 0).rg;
            if (abs(h.g - expected) > 0.03 * expected + 0.05) {
                continue;
            }
            vec2 b = mix(1.0 - f, f, vec2(offset));
            float w = b.x * b.y;
            sum += h.r * w;
            weights += w;
        }
        if (weights > 0.01) {
            float before = clamp(sum / weights, low, high);
            // Where little history was found, lean on this frame.
            float weight = mix(1.0, currentWeight, clamp(weights, 0.0, 1.0));
            result = mix(before, light, weight);
        }
    }
    FragColor = vec4(result, now, 0.0, 1.0);
}
