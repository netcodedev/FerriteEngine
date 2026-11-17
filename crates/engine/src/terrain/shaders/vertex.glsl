#version 460 core

// The standard terrain vertex program (see `terrain::source::TerrainShaders`).
layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
// What the source made of the ground here (see
// `terrain::source::SurfaceSample`): a colour (display space) and twelve
// numbers only the source's fragment program knows the meaning of.
layout (location = 2) in vec3 groundColor;
layout (location = 3) in vec4 data0;
layout (location = 4) in vec4 data1;
layout (location = 5) in vec4 data2;

out vec3 Normal;
out vec3 WorldPos;
out vec3 GroundColor;
out vec4 Data0;
out vec4 Data1;
out vec4 Data2;

uniform mat4 model;
uniform mat4 viewProjection;
// The horizon (see `terrain::horizon`) lies `horizon::SINK` metres under
// the ground it samples, so the chunks' edges (their skirts) stand over it
// where they meet; where the chunks are loaded it is left out (see
// `hiddenHorizon`).
uniform int isHorizon;
uniform vec3 horizonSink;

vec3 sinkHorizon(vec3 p) {
    if (isHorizon == 1) {
        p.y -= 4.0;
    }
    return p;
}

// The same as in the depth-only program (`depth_vertex.glsl`): the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

void main()
{
    vec4 worldPosition = vec4(sinkHorizon((model * vec4(position, 1.0)).xyz), 1.0);
    gl_Position = viewProjection * worldPosition;
    Normal = normalize(transpose(inverse(mat3(model))) * normals);
    WorldPos = worldPosition.xyz;
    GroundColor = groundColor;
    Data0 = data0;
    Data1 = data1;
    Data2 = data2;
}
