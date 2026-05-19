#version 330 core

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
uniform vec3 lightPosition;

out vec3 Normal;
out vec3 toLightVector;
out vec2 TexCoords;

void main() {
    mat4 instanceMat = mat4(instanceCol0, instanceCol1, instanceCol2, instanceCol3);
    mat4 modelTransform = parentTransform * instanceMat;
    vec4 worldPos = modelTransform * vec4(position, 1.0);
    gl_Position = viewProjection * worldPos;
    // Transforms in this pipeline are rigid (rotation + translation only), so
    // we can use the upper-left 3x3 directly for the normal.
    Normal = mat3(modelTransform) * normal;
    TexCoords = texCoords;
    toLightVector = lightPosition;
}
