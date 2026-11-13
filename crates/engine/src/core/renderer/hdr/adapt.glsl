#version 330 core

// Eye adaptation: moves the adapted luminance (log2, 1×1) toward this
// frame's average, the way eyes get used to bright and dark scenes.

out float FragColor;

uniform sampler2D luminance;       // log2 luminance with mipmaps
uniform int averageLevel;          // mip level holding the average
uniform sampler2D previous;        // last frame's adapted value
uniform float blend;               // 0: keep the previous, 1: jump
uniform vec2 range;                // log2 luminance the adaptation stays in

void main() {
    float average = clamp(textureLod(luminance, vec2(0.5), float(averageLevel)).r, range.x, range.y);
    float last = texelFetch(previous, ivec2(0), 0).r;
    FragColor = mix(last, average, blend);
}
