#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normal;
layout (location = 2) in vec2 texCoords;
// Per-instance attributes — a column-major mat4 split across four vec4 slots.
// glVertexAttribDivisor(loc, 1) is set on each so all four advance once per
// instance.
layout (location = 3) in vec4 instanceCol0;
layout (location = 4) in vec4 instanceCol1;
layout (location = 5) in vec4 instanceCol2;
layout (location = 6) in vec4 instanceCol3;

uniform mat4 viewProjection;
uniform mat4 parentTransform;
// Detail instances (grass): the camera's x and z, and the range they are
// planted to; they shrink about their base from 80 % of it to nothing at
// it. A range of 0: not a detail instance.
uniform vec3 detailFade;
// The wind: direction (x, z), strength, and the time in seconds; and how
// much these models sway in it (0: not at all).
uniform vec4 wind;
uniform float windAmount;
// The depth-only program (`depth_pass`) runs this shader too; the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

out vec3 Normal;
out vec3 WorldPos;
out vec2 TexCoords;
// The instance's tint (see `Placement::matrix`).
flat out vec3 Tint;

void main() {
    // The bottom row carries the tint less one; put the row back.
    Tint = vec3(1.0) + vec3(instanceCol0.w, instanceCol1.w, instanceCol2.w);
    mat4 instanceMat = mat4(vec4(instanceCol0.xyz, 0.0), vec4(instanceCol1.xyz, 0.0), vec4(instanceCol2.xyz, 0.0), instanceCol3);
    mat4 modelTransform = parentTransform * instanceMat;
    float shrink = 1.0;
    if (detailFade.z > 0.0) {
        vec2 base = (modelTransform * vec4(0.0, 0.0, 0.0, 1.0)).xz;
        shrink = 1.0 - smoothstep(0.8 * detailFade.z, detailFade.z, distance(base, detailFade.xy));
    }
    vec4 worldPos = modelTransform * vec4(position * shrink, 1.0);
    if (windAmount > 0.0) {
        // Along the wind by the square of the height over the base: the
        // tips sway, the roots stay. Two sines and gusts rolling across.
        vec3 base = (modelTransform * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
        float h = max(worldPos.y - base.y, 0.0);
        float t = wind.w;
        float phase = dot(base.xz, vec2(0.71, 0.37));
        float gust = 0.5 + 0.5 * sin(dot(base.xz, wind.xy) * 0.08 - t * 0.9);
        float sway = sin(t * 1.9 + phase) * 0.5 + sin(t * 3.7 + phase * 1.7) * 0.2 + gust * 0.7;
        worldPos.xz += wind.xy * (windAmount * wind.z * 0.35 * h * h * sway);
    }
    gl_Position = viewProjection * worldPos;
    WorldPos = worldPos.xyz;
    // Inverse transpose: the parent entity may be scaled non-uniformly.
    Normal = transpose(inverse(mat3(modelTransform))) * normal;
    TexCoords = texCoords;
}
