#version 460

uniform sampler2D texture0;

in vec2 v_tex_coords;
in vec4 v_color;

out vec4 f_color;

void main() {
    f_color = v_color * vec4(1.0, 1.0, 1.0, texture(texture0, v_tex_coords).r);
}