#version 330 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec2 tex_coords;

uniform mat4 projection;
uniform vec3 color;

out vec2 v_tex_coords;
out vec4 v_color;

void main() {
    gl_Position = projection * vec4(position, 1.0);
    v_tex_coords = tex_coords;
    v_color = vec4(color, 1.0);
}
