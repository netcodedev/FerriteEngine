#version 330 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;

out vec4 outColor;
out vec3 Normal;
out vec3 toLightVector;

uniform mat4 model;
uniform mat4 view;
uniform mat4 projection;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = projection * view * worldPosition;
    outColor = vec4(0.3, 0.6, 0.4, 1.0);
    Normal = normals;
    toLightVector = vec3(0.0, 2000.0, 0.0) - worldPosition.xyz;
}