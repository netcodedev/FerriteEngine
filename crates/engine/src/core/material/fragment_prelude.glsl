// Fragment side of a material: the varyings and helpers of its nodes.

in vec3 Normal;
in vec3 WorldPos;
in vec2 TexCoords;
in vec4 VertexColor;
in vec3 ObjectPos;
flat in vec3 PivotPos;
flat in vec3 Tint;

// Cut-out materials discard below this opacity.
uniform float materialCutoff;

MaterialInputs materialInputs() {
    MaterialInputs i;
    i.worldPos = WorldPos;
    i.normal = Normal;
    i.objectNormal = Normal;
    i.uv = TexCoords;
    i.color = VertexColor;
    i.objectPos = ObjectPos;
    i.pivot = PivotPos;
    i.tint = Tint;
    return i;
}

// `N` bent by a tangent-space normal map, with a tangent frame from
// screen-space derivatives ("normal mapping without precomputed
// tangents"), so meshes need no tangent attribute. `N` unchanged without
// a map or usable UVs.
vec3 materialNormalMap(sampler2D map, int set, vec3 N, vec3 worldPos, vec2 uv, float strength) {
    if (set == 0) {
        return N;
    }
    vec3 dp1 = dFdx(worldPos);
    vec3 dp2 = dFdy(worldPos);
    vec2 duv1 = dFdx(uv);
    vec2 duv2 = dFdy(uv);
    vec3 dp2perp = cross(dp2, N);
    vec3 dp1perp = cross(N, dp1);
    vec3 T = dp2perp * duv1.x + dp1perp * duv2.x;
    vec3 B = dp2perp * duv1.y + dp1perp * duv2.y;
    float len = max(dot(T, T), dot(B, B));
    if (len < 1e-20) {
        return N; // no usable UVs
    }
    float invmax = inversesqrt(len);
    vec3 t = texture(map, uv).xyz * 2.0 - 1.0;
    // OpenGL-style maps: green points up in the image. Images are uploaded
    // top row first, so v grows downwards and "up" is -B.
    t.y = -t.y;
    t.xy *= strength;
    return normalize(mat3(T * invmax, B * invmax, N) * t);
}
