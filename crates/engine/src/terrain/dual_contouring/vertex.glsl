#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
// The biomes' ground colour (display space) and the climate: temperature
// (°C at this height) and humidity (see `generator::biome`).
layout (location = 2) in vec3 groundColor;
layout (location = 3) in vec2 climate;

out vec3 Normal;
out vec3 WorldPos;
out vec3 GroundColor;
out vec2 Climate;

uniform mat4 model;
uniform mat4 viewProjection;
// The same as in the depth-only program (`depth_vertex.glsl`): the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;
    Normal = normalize(transpose(inverse(mat3(model))) * normals);
    WorldPos = worldPosition.xyz;
    GroundColor = groundColor;
    Climate = climate;
}
