#version 330 core

layout (location = 0) in vec3 in_position;
layout (location = 1) in vec4 in_color;
layout (location = 2) in vec2 in_dimensions;

out vec4 vertex_color;
out vec2 vertex_position;
out vec2 rect_size;

uniform mat4 projection;

void main()
{
    // Convert to normalized device coordinates (-1 to 1)
    gl_Position = projection * vec4(in_position, 1.0);

    // Calculate texture coordinates (0 to 1)
    vertex_position = in_position.xy / in_dimensions;
    vertex_color = in_color;
    rect_size = in_dimensions;
}