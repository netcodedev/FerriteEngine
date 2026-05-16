#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec3 color;

out vec3 toLightVector;
out vec2 worldXZ;      // stable world XZ for subtle wave tint

uniform vec3 lightPosition;
uniform vec3 chunkWorldOffset;
uniform mat4 model;
uniform mat4 viewProjection;

void main()
{
    gl_Position = viewProjection * model * vec4(position, 1.0);
    toLightVector = lightPosition;
    worldXZ = (chunkWorldOffset + position).xz;
}
