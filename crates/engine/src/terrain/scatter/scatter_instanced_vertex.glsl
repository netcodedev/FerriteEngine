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
// RGBA, 8 bits each (white where the file has none).
layout (location = 7) in uint vertexColor;

uniform mat4 viewProjection;
uniform mat4 parentTransform;
// Detail instances (grass): the camera's x and z, the range they are
// planted to, and the share of it at its end over which they fade away. A
// range of 0: not a detail instance. They shrink about their base, or with
// `fadeThin` each vanishes whole at its own point of the fade (a number of
// its own, from where it stands), so they thin out.
uniform vec4 detailFade;
uniform int fadeThin;
// The depth-only program (`depth_pass`) runs this shader too; the shading
// pass tests LEQUAL against the prepass' depth.
invariant gl_Position;

// After the material's vertex code (`materialWorldOffset`), which moves
// the vertices (the wind among others).
void main() {
    // The bottom row carries the tint less one; put the row back.
    Tint = vec3(1.0) + vec3(instanceCol0.w, instanceCol1.w, instanceCol2.w);
    mat4 instanceMat = mat4(vec4(instanceCol0.xyz, 0.0), vec4(instanceCol1.xyz, 0.0), vec4(instanceCol2.xyz, 0.0), instanceCol3);
    mat4 modelTransform = parentTransform * instanceMat;
    vec3 pivot = (modelTransform * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
    float shrink = 1.0;
    if (detailFade.z > 0.0) {
        vec2 base = pivot.xz;
        float start = detailFade.z * (1.0 - clamp(detailFade.w, 0.0, 1.0));
        float d = distance(base, detailFade.xy);
        // No fade share: a hard edge (smoothstep is undefined for equal
        // edges).
        float left = start < detailFade.z ? 1.0 - smoothstep(start, detailFade.z, d) : (d < detailFade.z ? 1.0 : 0.0);
        if (fadeThin == 1) {
            float own = fract(sin(dot(base, vec2(12.9898, 78.233))) * 43758.5453);
            shrink = own < left ? 1.0 : 0.0;
        } else {
            shrink = left;
        }
    }
    vec4 worldPos = modelTransform * vec4(position * shrink, 1.0);
    // Inverse transpose: the parent entity may be scaled non-uniformly.
    Normal = transpose(inverse(mat3(modelTransform))) * normal;
    MaterialInputs i;
    i.worldPos = worldPos.xyz;
    i.normal = normalize(Normal);
    i.objectNormal = normal;
    i.uv = texCoords;
    i.color = unpackUnorm4x8(vertexColor);
    i.objectPos = position;
    i.pivot = pivot;
    i.tint = Tint;
    worldPos.xyz += materialWorldOffset(i);
    gl_Position = viewProjection * worldPos;
    WorldPos = worldPos.xyz;
    TexCoords = texCoords;
    VertexColor = i.color;
    ObjectPos = position;
    PivotPos = pivot;
}
