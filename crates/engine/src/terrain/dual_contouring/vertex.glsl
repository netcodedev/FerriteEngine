#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;

out vec3 Normal;
out vec3 WorldPos;

uniform mat4 model;
uniform mat4 viewProjection;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;
    Normal = normalize(transpose(inverse(mat3(model))) * normals);
    WorldPos = worldPosition.xyz;
}
