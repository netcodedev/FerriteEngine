#version 330 core

// Copies a colour texture into a layer of the albedo atlas (sRGB): what
// reflection rays look up where they meet a mesh (`rays.glsl`).

in vec2 uv;
out vec4 FragColor;

uniform sampler2D source;
uniform int channel;

void main() {
    // Several texels per atlas texel: the mip the size asks for.
    vec2 size = vec2(textureSize(source, 0));
    float lod = log2(max(size.x, size.y) / 256.0);
    FragColor = textureLod(source, uv, max(lod, 0.0));
    FragColor.a = 1.0;
}
