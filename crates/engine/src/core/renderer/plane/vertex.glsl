#version 330 core

layout (location = 0) in vec3 in_position;
layout (location = 1) in vec4 in_color;
layout (location = 2) in vec4 in_dimensions;
layout (location = 3) in vec4 in_border_color;
layout (location = 4) in vec4 in_border_radius;
layout (location = 5) in float in_border_thickness;

out vec4 vertex_color;
out vec2 vertex_position;
out vec4 rect_size;
flat out vec4 borderColor;
flat out vec4 borderRadius;
flat out float borderThickness;

uniform mat4 projection;

void main()
{
    gl_Position = projection * vec4(in_position, 1.0);

    vertex_position = in_position.xy / in_dimensions.xy;
    vertex_color = in_color;
    rect_size = in_dimensions;
    borderColor = in_border_color;
    borderRadius = in_border_radius;
    borderThickness = in_border_thickness;
}
