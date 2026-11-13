#version 330 core

const int MAX_BONES = 100;
const int MAX_WEIGHTS = 4;

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec2 texCoords;
layout (location = 3) in ivec4 boneIDs;
layout (location = 4) in vec4 weights;

out vec3 Normal;
out vec3 WorldPos;
out vec2 TexCoords;

uniform mat4 model;
uniform mat4 viewProjection;
uniform mat4 boneTransforms[MAX_BONES];

void main()
{
    mat4 BoneTransform = boneTransforms[boneIDs[0]] * weights[0];

    for (int i = 1; i < MAX_WEIGHTS; i++)
    {
        if (weights[i] == 0.0)
            break;
        BoneTransform += boneTransforms[boneIDs[i]] * weights[i];
    }

    vec4 worldPosition = model * (BoneTransform * vec4(position, 1.0));
    gl_Position = viewProjection * worldPosition;
    WorldPos = worldPosition.xyz;
    // Inverse transpose: keeps normals perpendicular under non-uniform scale.
    Normal = transpose(inverse(mat3(model))) * mat3(BoneTransform) * normals;
    TexCoords = texCoords;
}
