#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
// The biomes' ground colour (display space) and the climate: temperature
// (°C at this height) and humidity (see `generator::biome`).
layout (location = 2) in vec3 groundColor;
layout (location = 3) in vec2 climate;
// Forest floor, dry grass, jungle floor and moss (see
// `generator::biome::BiomeBlend`); road, settlement, gravel (see
// `generator::Column::paint`) and mud. 0..1 each.
layout (location = 4) in vec4 cover;
layout (location = 5) in vec4 paint;
// How far under the surface, 0..1.
layout (location = 6) in float underground;

out vec3 Normal;
out vec3 WorldPos;
out vec3 GroundColor;
out vec2 Climate;
out vec4 Cover;
out vec4 Paint;
out float Underground;

uniform mat4 model;
uniform mat4 viewProjection;
// The horizon (see `terrain::horizon`) sinks under the loaded chunks:
// within `horizonSink.z` metres of the camera at `horizonSink.xy` (x, z).
uniform int isHorizon;
uniform vec3 horizonSink;

// `sea`: open sea, which lies flat at the chunks' water level and only
// needs to stay a little under their water.
vec3 sinkHorizon(vec3 p, bool sea) {
    if (isHorizon == 1) {
        float d = length(p.xz - horizonSink.xy);
        p.y -= (sea ? 1.0 : 4.0) + 60.0 * (1.0 - smoothstep(horizonSink.z - 160.0, horizonSink.z - 20.0, d));
    }
    return p;
}

// The same as in the depth-only program (`depth_vertex.glsl`): the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

void main()
{
    vec4 worldPosition = vec4(sinkHorizon((model * vec4(position, 1.0)).xyz, climate.y > 1.5), 1.0);
    gl_Position = viewProjection * worldPosition;
    Normal = normalize(transpose(inverse(mat3(model))) * normals);
    WorldPos = worldPosition.xyz;
    GroundColor = groundColor;
    Climate = climate;
    Cover = cover;
    Paint = paint;
    Underground = underground;
}
