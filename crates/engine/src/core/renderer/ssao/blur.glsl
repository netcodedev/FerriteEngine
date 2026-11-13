#version 330 core

// 4x4 blur of the occlusion over the dither pattern, leaving out pixels
// at a clearly different depth so the occlusion does not bleed over edges.

in vec2 uv;
out float occlusion;

uniform sampler2D occlusionMap;
uniform sampler2D depthMap;
uniform mat4 inverseProjection;

float viewDistance(ivec2 texel, ivec2 size) {
    float depth = texelFetch(depthMap, texel, 0).r;
    vec2 ndc = (vec2(texel) + 0.5) / vec2(size) * 2.0 - 1.0;
    vec4 view = inverseProjection * vec4(ndc, depth * 2.0 - 1.0, 1.0);
    return -view.z / view.w;
}

void main() {
    ivec2 size = textureSize(occlusionMap, 0);
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float center = viewDistance(texel, size);
    float sum = 0.0;
    float weights = 0.0;
    for (int y = -2; y < 2; ++y) {
        for (int x = -2; x < 2; ++x) {
            ivec2 at = clamp(texel + ivec2(x, y), ivec2(0), size - 1);
            float difference = abs(viewDistance(at, size) - center) / max(center, 1e-3);
            float weight = 1.0 - smoothstep(0.02, 0.1, difference);
            sum += texelFetch(occlusionMap, at, 0).r * weight;
            weights += weight;
        }
    }
    occlusion = weights > 0.0 ? sum / weights : texelFetch(occlusionMap, texel, 0).r;
}
