#version 460 core

// The ground's depth alone (see `depth_pass`): the position as
// `vertex.glsl` computes it, nothing else.
layout (location = 0) in vec3 position;

uniform mat4 model;
uniform mat4 viewProjection;
// The horizon (see `terrain::horizon`) sinks under the loaded chunks:
// within `horizonSink.z` metres of the camera at `horizonSink.xy` (x, z).
uniform int isHorizon;
uniform vec3 horizonSink;

vec3 sinkHorizon(vec3 p) {
    if (isHorizon == 1) {
        float d = length(p.xz - horizonSink.xy);
        p.y -= 4.0 + 60.0 * (1.0 - smoothstep(horizonSink.z - 160.0, horizonSink.z - 20.0, d));
    }
    return p;
}

invariant gl_Position;

void main()
{
    vec4 worldPosition = vec4(sinkHorizon((model * vec4(position, 1.0)).xyz), 1.0);
    gl_Position = viewProjection * worldPosition;
}
