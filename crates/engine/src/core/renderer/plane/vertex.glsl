#version 330 core

layout (location = 0) in vec3 in_position;
layout (location = 1) in vec4 in_color;
layout (location = 2) in vec4 in_dimensions;

out vec4 vertex_color;
out vec2 vertex_position;
out vec4 rect_size;

uniform mat4 projection;

void main()
{
    gl_Position = projection * vec4(in_position, 1.0);

    vertex_position = in_position.xy / in_dimensions.xy;
    vertex_color = in_color;
    rect_size = in_dimensions;
}