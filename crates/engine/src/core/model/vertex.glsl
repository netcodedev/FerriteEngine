const int MAX_BONES = 100;
const int MAX_WEIGHTS = 4;

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec2 texCoords;
layout (location = 3) in ivec4 boneIDs;
layout (location = 4) in vec4 weights;
// RGBA, 8 bits each (white where the file has none).
layout (location = 5) in uint vertexColor;

uniform mat4 model;
uniform mat4 viewProjection;
uniform mat4 boneTransforms[MAX_BONES];
// The depth-only program (`depth_pass`) runs this shader too; the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

// After the material's vertex code (`materialWorldOffset`, see
// `material/programs.rs`), which goes in front of this.
void main()
{
    mat4 BoneTransform = boneTransforms[boneIDs[0]] * weights[0];

    for (int i = 1; i < MAX_WEIGHTS; i++)
    {
        if (weights[i] == 0.0)
            break;
        BoneTransform += boneTransforms[boneIDs[i]] * weights[i];
    }

    vec4 objectPosition = BoneTransform * vec4(position, 1.0);
    vec4 worldPosition = model * objectPosition;
    // Inverse transpose: keeps normals perpendicular under non-uniform scale.
    Normal = transpose(inverse(mat3(model))) * mat3(BoneTransform) * normals;
    MaterialInputs i;
    i.worldPos = worldPosition.xyz;
    i.normal = normalize(Normal);
    i.objectNormal = normals;
    i.uv = texCoords;
    i.color = unpackUnorm4x8(vertexColor);
    i.objectPos = objectPosition.xyz;
    i.pivot = (model * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
    i.tint = vec3(1.0);
    worldPosition.xyz += materialWorldOffset(i);
    gl_Position = viewProjection * worldPosition;
    WorldPos = worldPosition.xyz;
    TexCoords = texCoords;
    VertexColor = i.color;
    ObjectPos = i.objectPos;
    PivotPos = i.pivot;
    Tint = vec3(1.0);
}
