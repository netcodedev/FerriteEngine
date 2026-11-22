#version 330 core

// A small blur of the accumulated shadows, only between texels at about
// the same distance from the camera (the same surface), so the noise
// left over fades without shadows bleeding across edges.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D accumulated;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(accumulated, 0);
    vec2 center = texelFetch(accumulated, texel, 0).rg;
    float tolerance = 0.02 * center.g + 0.02;
    float sum = 0.0;
    float weights = 0.0;
    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            vec2 s = texelFetch(accumulated, clamp(texel + ivec2(x, y), ivec2(0), size - 1), 0).rg;
            float w = exp(-0.5 * float(x * x + y * y) / 2.0);
            w *= max(0.0, 1.0 - abs(s.g - center.g) / tolerance);
            sum += s.r * w;
            weights += w;
        }
    }
    float light = weights > 0.0 ? sum / weights : center.r;
    FragColor = vec4(light, light, light, 1.0);
}
