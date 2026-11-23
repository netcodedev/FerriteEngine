#version 330 core

// One channel of a cut-out material's texture (its opacity) into a layer
// of the atlas the ray traced shadows test leaves against.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D source;
uniform int channel;

void main() {
    vec4 texel = texture(source, uv);
    float opacity = channel == 0 ? texel.r : channel == 1 ? texel.g : channel == 2 ? texel.b : texel.a;
    FragColor = vec4(opacity);
}
