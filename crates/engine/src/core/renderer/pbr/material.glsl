// A model material's maps, shaded with `lighting.glsl` (which comes first).

const float DEFAULT_ROUGHNESS = 0.8;

uniform sampler2D baseColorMap;
uniform sampler2D normalMap;
uniform sampler2D roughnessMap;
uniform sampler2D metallicMap;
uniform sampler2D occlusionMap;
uniform int hasBaseColorMap;
uniform int hasNormalMap;
uniform int hasRoughnessMap;
uniform int hasMetallicMap;
uniform int hasOcclusionMap;

// Tangent frame from screen-space derivatives ("normal mapping without
// precomputed tangents"), so meshes need no tangent attribute.
vec3 perturbNormal(vec3 N, vec3 worldPos, vec2 uv) {
    if (hasNormalMap == 0) {
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
    vec3 t = texture(normalMap, uv).xyz * 2.0 - 1.0;
    // OpenGL-style maps: green points up in the image. Images are uploaded
    // top row first, so v grows downwards and "up" is -B.
    t.y = -t.y;
    return normalize(mat3(T * invmax, B * invmax, N) * t);
}

// Shade one fragment. Discards texels the colour map's alpha cuts out
// (foliage cards); in the shadow pass only that cut-out matters.
vec4 shadePbr(vec3 worldPos, vec3 vertexNormal, vec2 uv) {
    vec4 base = hasBaseColorMap == 1 ? texture(baseColorMap, uv) : vec4(1.0);
    if (base.a < 0.5) {
        discard;
    }
    if (isShadowPass == 1) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    vec3 Ng = normalize(vertexNormal);
    if (!gl_FrontFacing) {
        Ng = -Ng; // two-sided geometry such as leaves
    }
    vec3 N = perturbNormal(Ng, worldPos, uv);
    float roughness = hasRoughnessMap == 1 ? texture(roughnessMap, uv).r : DEFAULT_ROUGHNESS;
    float metallic = hasMetallicMap == 1 ? texture(metallicMap, uv).r : 0.0;
    float ao = hasOcclusionMap == 1 ? texture(occlusionMap, uv).r : 1.0;
    vec3 color = shadeSurface(worldPos, N, Ng, base.rgb, roughness, metallic, ao, 1.0);
    return vec4(toDisplay(color), 1.0);
}
