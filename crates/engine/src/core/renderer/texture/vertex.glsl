#version 460 core
// A unit square (0..1), placed at `rect` (x, y, width, height in clip
// space); its corner is the texture coordinate.
layout(location = 0) in vec2 corner;

uniform vec4 rect;

out vec2 texCoord;

void main() {
    gl_Position = vec4(rect.xy + corner * rect.zw, 0.0, 1.0);
    texCoord = corner;
}
