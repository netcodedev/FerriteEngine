#version 460 core

// The ground's depth alone (see `depth_pass`): the position as
// `vertex.glsl` computes it, nothing else.
layout (location = 0) in vec3 position;
// Humidity: above 1.5 on the horizon's open sea (`terrain::horizon`).
layout (location = 3) in vec2 climate;

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

invariant gl_Position;

void main()
{
    vec4 worldPosition = vec4(sinkHorizon((model * vec4(position, 1.0)).xyz, climate.y > 1.5), 1.0);
    gl_Position = viewProjection * worldPosition;
}
