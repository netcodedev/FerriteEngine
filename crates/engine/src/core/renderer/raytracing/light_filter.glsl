#version 330 core

// Blurs the accumulated share of the lamps' light a little, only between
// texels at about the same distance from the camera (the same surface),
// and limits it to 0..1 for the lit shaders (`lampShadowRatio`).

in vec2 uv;
out vec4 FragColor;

uniform sampler2D accumulated;

const int RADIUS = 2;
const float SIGMA = 1.5;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(accumulated, 0);
    vec4 center = texelFetch(accumulated, texel, 0);
    float tolerance = 0.02 * center.a + 0.02;
    vec3 sum = vec3(0.0);
    float weights = 0.0;
    for (int y = -RADIUS; y <= RADIUS; y++) {
        for (int x = -RADIUS; x <= RADIUS; x++) {
            vec4 s = texelFetch(accumulated, clamp(texel + ivec2(x, y), ivec2(0), size - 1), 0);
            float w = exp(-0.5 * float(x * x + y * y) / (SIGMA * SIGMA));
            w *= max(0.0, 1.0 - abs(s.a - center.a) / tolerance);
            sum += s.rgb * w;
            weights += w;
        }
    }
    vec3 light = weights > 0.0 ? sum / weights : center.rgb;
    FragColor = vec4(clamp(light, vec3(0.0), vec3(1.0)), 1.0);
}
