// Shared physically based shading for model materials (metal/roughness
// workflow). Inserted after the `#version` line of a fragment shader; the
// shader calls `shadePbr`. Lighting is computed in linear space and returned
// in display (gamma) space, like the rest of the renderer writes it.

const float PI = 3.14159265;

// Sun irradiance at noon and the sky's ambient light, in linear units. Tuned
// so lit models sit at about the brightness of the terrain around them.
const float SUN_INTENSITY = 3.5;
const vec3 SKY_DAY = vec3(0.27, 0.29, 0.32);
const vec3 SKY_NIGHT = vec3(0.004, 0.005, 0.009);
const vec3 GROUND_TINT = vec3(0.45, 0.42, 0.38);
const float DEFAULT_ROUGHNESS = 0.8;

uniform vec3 sunDirection;     // normalized, points toward the sun
uniform vec3 cameraPosition;
uniform float cloudCoverage;   // 0..1
uniform int isShadowPass;
uniform int useShadowMap;
uniform sampler2D shadowMap;
uniform mat4 lightProjection;

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

float sunShadow(vec3 worldPos, vec3 geometryNormal, vec3 L) {
    if (useShadowMap == 0) {
        return 1.0;
    }
    // Offsetting along the normal keeps a model from shadowing itself.
    vec4 lightSpace = lightProjection * vec4(worldPos + geometryNormal * 0.03, 1.0);
    vec3 p = lightSpace.xyz / lightSpace.w * 0.5 + 0.5;
    if (p.z > 1.0) {
        return 1.0;
    }
    float bias = max(0.0004 * (1.0 - dot(geometryNormal, L)), 0.0002);
    vec2 texel = 1.0 / vec2(textureSize(shadowMap, 0));
    float shadow = 0.0;
    for (int x = -1; x <= 1; ++x) {
        for (int y = -1; y <= 1; ++y) {
            float depth = texture(shadowMap, p.xy + vec2(x, y) * texel).r;
            shadow += p.z - bias > depth ? 1.0 : 0.0;
        }
    }
    return 1.0 - shadow / 9.0;
}

vec3 sunRadiance() {
    float above = smoothstep(-0.05, 0.15, sunDirection.y);
    vec3 low = vec3(1.0, 0.55, 0.30);
    vec3 high = vec3(1.0, 0.95, 0.88);
    vec3 color = mix(low, high, smoothstep(0.05, 0.45, sunDirection.y));
    float clouds = 1.0 - 0.85 * clamp(cloudCoverage, 0.0, 1.0);
    return color * SUN_INTENSITY * above * clouds;
}

// Light arriving from the sky in direction `dir`: sky above, a darker
// ground bounce below.
vec3 skyLight(vec3 dir) {
    float day = smoothstep(-0.10, 0.20, sunDirection.y);
    vec3 sky = mix(SKY_NIGHT, SKY_DAY, day) * (1.0 - 0.3 * clamp(cloudCoverage, 0.0, 1.0));
    return mix(sky * GROUND_TINT, sky, clamp(dir.y * 0.5 + 0.5, 0.0, 1.0));
}

float distributionGGX(float NdotH, float a) {
    float a2 = a * a;
    float d = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

float geometrySmith(float NdotV, float NdotL, float roughness) {
    float k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    float gv = NdotV / (NdotV * (1.0 - k) + k);
    float gl = NdotL / (NdotL * (1.0 - k) + k);
    return gv * gl;
}

vec3 fresnelSchlick(float cosTheta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(1.0 - cosTheta, 5.0);
}

// Analytic fit of the split-sum environment BRDF (Karis, "Mobile PBR").
vec3 envBrdf(vec3 F0, float roughness, float NdotV) {
    const vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    const vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    vec4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
    vec2 ab = vec2(-1.04, 1.04) * a004 + r.zw;
    return F0 * ab.x + ab.y;
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
    vec3 V = normalize(cameraPosition - worldPos);
    vec3 L = normalize(sunDirection);

    float roughness = hasRoughnessMap == 1 ? texture(roughnessMap, uv).r : DEFAULT_ROUGHNESS;
    roughness = clamp(roughness, 0.04, 1.0);
    float metallic = hasMetallicMap == 1 ? texture(metallicMap, uv).r : 0.0;
    float ao = hasOcclusionMap == 1 ? texture(occlusionMap, uv).r : 1.0;
    vec3 albedo = base.rgb;
    vec3 F0 = mix(vec3(0.04), albedo, metallic);

    float NdotV = max(dot(N, V), 1e-4);
    float NdotL = max(dot(N, L), 0.0);
    vec3 H = normalize(V + L);
    float NdotH = max(dot(N, H), 0.0);
    float HdotV = max(dot(H, V), 0.0);

    vec3 F = fresnelSchlick(HdotV, F0);
    vec3 specular = distributionGGX(NdotH, roughness * roughness)
        * geometrySmith(NdotV, NdotL, roughness) * F
        / max(4.0 * NdotV * NdotL, 1e-4);
    vec3 kd = (1.0 - F) * (1.0 - metallic);
    float shadow = NdotL > 0.0 ? sunShadow(worldPos, Ng, L) : 0.0;
    vec3 direct = (kd * albedo / PI + specular) * sunRadiance() * NdotL * shadow;

    vec3 ambient = skyLight(N) * albedo * (1.0 - metallic)
        + skyLight(reflect(-V, N)) * envBrdf(F0, roughness, NdotV);

    vec3 color = direct + ambient * ao;
    return vec4(pow(clamp(color, 0.0, 1.0), vec3(1.0 / 2.2)), 1.0);
}
