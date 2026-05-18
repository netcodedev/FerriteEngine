#version 460 core

layout (location = 0) in vec2 position;

out vec2 ndcPos;

void main() {
    ndcPos = position;
    // Render at the far plane so any pixel without geometry shows sky.
    gl_Position = vec4(position, 1.0, 1.0);
}
