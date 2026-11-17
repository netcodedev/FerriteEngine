#version 460 core

// The ground's depth alone (see `depth_pass`): the position as
// `vertex.glsl` computes it, nothing else.
layout (location = 0) in vec3 position;

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

invariant gl_Position;

// For the horizon's depth program (see `shaders::horizon_depth_fragment`).
out vec3 WorldPos;

void main()
{
    vec4 worldPosition = vec4(sinkHorizon((model * vec4(position, 1.0)).xyz), 1.0);
    gl_Position = viewProjection * worldPosition;
    WorldPos = worldPosition.xyz;
}
