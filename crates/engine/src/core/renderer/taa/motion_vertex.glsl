#version 330 core

// A moving model's mesh into the motion vectors: placed exactly as the
// shading pass placed it (same expressions as `model/vertex.glsl`, so the
// depth test against the view's depth passes where it is seen), and once
// more as it stood a frame before.

const int MAX_BONES = 100;
const int MAX_WEIGHTS = 4;

layout (location = 0) in vec3 position;
layout (location = 3) in ivec4 boneIDs;
layout (location = 4) in vec4 weights;

uniform mat4 model;
uniform mat4 previousModel;
// What the view was drawn with (jittered), and the cameras without jitter.
uniform mat4 viewProjection;
uniform mat4 currentCamera;
uniform mat4 previousCamera;
uniform mat4 boneTransforms[MAX_BONES];
uniform mat4 previousBones[MAX_BONES];

invariant gl_Position;

out vec4 currentClip;
out vec4 previousClip;

void main() {
    mat4 BoneTransform = boneTransforms[boneIDs[0]] * weights[0];
    mat4 previousBone = previousBones[boneIDs[0]] * weights[0];
    for (int i = 1; i < MAX_WEIGHTS; i++) {
        if (weights[i] == 0.0)
            break;
        BoneTransform += boneTransforms[boneIDs[i]] * weights[i];
        previousBone += previousBones[boneIDs[i]] * weights[i];
    }
    vec4 objectPosition = BoneTransform * vec4(position, 1.0);
    vec4 worldPosition = model * objectPosition;
    gl_Position = viewProjection * worldPosition;
    currentClip = currentCamera * worldPosition;
    previousClip = previousCamera * (previousModel * (previousBone * vec4(position, 1.0)));
}
