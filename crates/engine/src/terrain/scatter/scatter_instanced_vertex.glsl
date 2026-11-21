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
// The depth-only program (`depth_pass`) runs this shader too; the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

out vec3 Normal;
out vec3 WorldPos;
out vec2 TexCoords;

void main() {
    mat4 instanceMat = mat4(instanceCol0, instanceCol1, instanceCol2, instanceCol3);
    mat4 modelTransform = parentTransform * instanceMat;
    float shrink = 1.0;
    if (detailFade.z > 0.0) {
        vec2 base = (modelTransform * vec4(0.0, 0.0, 0.0, 1.0)).xz;
        shrink = 1.0 - smoothstep(0.8 * detailFade.z, detailFade.z, distance(base, detailFade.xy));
    }
    vec4 worldPos = modelTransform * vec4(position * shrink, 1.0);
    gl_Position = viewProjection * worldPos;
    WorldPos = worldPos.xyz;
    // Inverse transpose: the parent entity may be scaled non-uniformly.
    Normal = transpose(inverse(mat3(modelTransform))) * normal;
    TexCoords = texCoords;
}
