#version 330 core

// Blurs the accumulated shadows as wide as their penumbra is: the light
// source's disc seen from the surface, as far behind the thing in the way
// as that is from the surface (from this frame's rays, `occluders` g).
// Contact shadows stay sharp, shadows of things far away soften. Only
// texels at about the same distance from the camera (the same surface)
// are blended, so shadows do not bleed across edges.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D accumulated;
uniform sampler2D occluders;
// Pixels a metre spans one metre in front of the camera.
uniform float pixelScale;
// Angular radius of the light's disc (radians).
uniform float lightRadius;

const int RADIUS = 3;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(accumulated, 0);
    vec2 center = texelFetch(accumulated, texel, 0).rg;
    // How far the things in the way are, on average, around this texel.
    float blockers = 0.0;
    float found = 0.0;
    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            float d = texelFetch(occluders, clamp(texel + ivec2(x, y) * 2, ivec2(0), size - 1), 0).g;
            if (d > 0.0) {
                blockers += d;
                found += 1.0;
            }
        }
    }
    // The penumbra's width on screen, in pixels; with nothing in the way
    // near here, no blur.
    float penumbra = found > 0.0 ? 2.0 * lightRadius * (blockers / found) * pixelScale / max(center.g, 0.01) : 0.0;
    float sigma = clamp(penumbra * 0.5, 0.5, float(RADIUS));
    float tolerance = 0.02 * center.g + 0.02;
    float sum = 0.0;
    float weights = 0.0;
    for (int y = -RADIUS; y <= RADIUS; y++) {
        for (int x = -RADIUS; x <= RADIUS; x++) {
            vec2 s = texelFetch(accumulated, clamp(texel + ivec2(x, y), ivec2(0), size - 1), 0).rg;
            float w = exp(-0.5 * float(x * x + y * y) / (sigma * sigma));
            w *= max(0.0, 1.0 - abs(s.g - center.g) / tolerance);
            sum += s.r * w;
            weights += w;
        }
    }
    float light = weights > 0.0 ? sum / weights : center.r;
    FragColor = vec4(light, light, light, 1.0);
}
