#version 460 core

// The ground's depth alone (see `depth_pass`): the position as
// `vertex.glsl` computes it, nothing else.
layout (location = 0) in vec3 position;

uniform mat4 model;
uniform mat4 viewProjection;
invariant gl_Position;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;
}
