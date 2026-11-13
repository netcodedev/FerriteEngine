// Shared physically based lighting (metal/roughness workflow): the sun with
// its shadow map and cloud cover, the sky as ambient light, and the BRDF.
// Inserted after the `#version` line (after `hdr/tonemap.glsl`); colours
// are linear until `toDisplay`.

const float PI = 3.14159265;

// Sun irradiance at noon and the sky's ambient light, in linear units. Tuned
// so lit models sit at about the brightness of the terrain around them.
const float SUN_INTENSITY = 3.5;
const vec3 SKY_DAY = vec3(0.27, 0.29, 0.32);
const vec3 SKY_NIGHT = vec3(0.004, 0.005, 0.009);
const vec3 GROUND_TINT = vec3(0.45, 0.42, 0.38);
// Sky light that comes from around the sun (relative to the sun's own
// light), and the share of the even sky light it replaces.
const float SUN_GLOW = 0.06;
const float SUN_GLOW_SHARE = 0.25;

uniform vec3 sunDirection;     // normalized, points toward the sun
uniform vec3 cameraPosition;
uniform float cloudCoverage;   // 0..1
uniform int isShadowPass;
uniform int useShadowMap;

// Cascaded shadow map: one depth layer per cascade, nearest first. Must
// match `SHADOW_CASCADES` in `light/skylight.rs`.
const int SHADOW_CASCADES = 4;
uniform sampler2DArray shadowMap;
uniform mat4 lightProjections[SHADOW_CASCADES];
// World size of one texel of each cascade.
uniform float cascadeTexel[SHADOW_CASCADES];
// Fraction at a cascade's border over which it fades into the next.
const float CASCADE_BLEND = 0.1;

// Screen space ambient occlusion of the camera's view (`renderer/ssao`),
// one texel per pixel of the viewport, which starts at `ssaoOrigin`.
uniform int useSsao;
uniform sampler2D ssaoMap;
uniform ivec2 ssaoOrigin;

// Share of the sky this pixel sees past the geometry around it, 0..1.
float screenOcclusion() {
    if (useSsao == 0 || isShadowPass == 1) {
        return 1.0;
    }
    return texelFetch(ssaoMap, ivec2(gl_FragCoord.xy) - ssaoOrigin, 0).r;
}

vec3 cascadeCoord(int cascade, vec3 worldPos) {
    vec4 lightSpace = lightProjections[cascade] * vec4(worldPos, 1.0);
    return lightSpace.xyz / lightSpace.w * 0.5 + 0.5;
}

// Light reaching `worldPos` in one cascade, 0..1.
float cascadeShadow(int cascade, vec3 worldPos, vec3 geometryNormal, vec3 L) {
    // Offsetting along the normal by about a texel keeps surfaces from
    // shadowing themselves; texels grow with the cascade.
    vec3 p = cascadeCoord(cascade, worldPos + geometryNormal * cascadeTexel[cascade] * 1.5);
    float bias = 0.0002 + 0.0004 * (1.0 - max(dot(geometryNormal, L), 0.0));
    float reference = p.z - bias;
    // Bilinear 3x3 PCF: 4x4 texel tests weighted by where the point falls
    // between texels, so shadow edges are smooth instead of texel steps.
    ivec2 size = textureSize(shadowMap, 0).xy;
    vec2 texel = p.xy * vec2(size) - 0.5;
    ivec2 base = ivec2(floor(texel));
    vec2 f = texel - floor(texel);
    vec4 wx = vec4(1.0 - f.x, 1.0, 1.0, f.x);
    vec4 wy = vec4(1.0 - f.y, 1.0, 1.0, f.y);
    float lit = 0.0;
    for (int j = 0; j < 4; ++j) {
        for (int i = 0; i < 4; ++i) {
            ivec2 at = clamp(base + ivec2(i - 1, j - 1), ivec2(0), size - 1);
            float depth = texelFetch(shadowMap, ivec3(at, cascade), 0).r;
            lit += wx[i] * wy[j] * (reference > depth ? 0.0 : 1.0);
        }
    }
    return lit / 9.0;
}

// Light reaching `worldPos` from the sun, 0..1, from the nearest cascade
// that covers it, faded into the next one at its border. Beyond the last
// cascade everything is lit.
float sunShadow(vec3 worldPos, vec3 geometryNormal, vec3 L) {
    if (useShadowMap == 0) {
        return 1.0;
    }
    for (int i = 0; i < SHADOW_CASCADES; ++i) {
        vec3 p = cascadeCoord(i, worldPos);
        // How far toward the border, 0 at the centre, 1 at the edge.
        float edge = max(abs(p.x * 2.0 - 1.0), abs(p.y * 2.0 - 1.0));
        if (edge >= 0.98 || p.z <= 0.0 || p.z >= 1.0) {
            continue;
        }
        float shadow = cascadeShadow(i, worldPos, geometryNormal, L);
        float fade = smoothstep(0.98 - CASCADE_BLEND, 0.98, edge);
        if (fade > 0.0) {
            float next = i + 1 < SHADOW_CASCADES
                ? cascadeShadow(i + 1, worldPos, geometryNormal, L)
                : 1.0;
            shadow = mix(shadow, next, fade);
        }
        return shadow;
    }
    return 1.0;
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
// ground bounce below, and a brighter glow toward the sun. The glow also
// reaches surfaces the sun itself is blocked from, so bumps in shadow still
// have a lit and a dark side.
vec3 skyLight(vec3 dir) {
    float day = smoothstep(-0.10, 0.20, sunDirection.y);
    float cover = clamp(cloudCoverage, 0.0, 1.0);
    vec3 sky = mix(SKY_NIGHT, SKY_DAY, day) * (1.0 - 0.3 * cover);
    vec3 hemisphere = mix(sky * GROUND_TINT, sky, clamp(dir.y * 0.5 + 0.5, 0.0, 1.0));
    float towardSun = max(dot(dir, normalize(sunDirection)), 0.0);
    vec3 glow = sunRadiance() * SUN_GLOW * towardSun * towardSun * (1.0 - 0.5 * cover);
    return hemisphere * (1.0 - SUN_GLOW_SHARE) + glow;
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

// Light leaving a surface toward the camera, in linear space. `N` is the
// shading normal, `Ng` the geometric one (for shadow lookups); `sunFactor`
// dims the direct sun, e.g. for cloud shadows. The ambient light is
// darkened by `ao` and by the screen space occlusion.
vec3 shadeSurface(vec3 worldPos, vec3 N, vec3 Ng, vec3 albedo, float roughness,
                  float metallic, float ao, float sunFactor) {
    vec3 V = normalize(cameraPosition - worldPos);
    vec3 L = normalize(sunDirection);
    roughness = clamp(roughness, 0.04, 1.0);
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
    vec3 direct = (kd * albedo / PI + specular) * sunRadiance() * NdotL * shadow * sunFactor;

    vec3 ambient = skyLight(N) * albedo * (1.0 - metallic)
        + skyLight(reflect(-V, N)) * envBrdf(F0, roughness, NdotV);
    return direct + ambient * ao * screenOcclusion();
}

// 1 while drawing into the HDR target (`renderer/hdr`), which wants
// linear light; tone mapping and the sRGB transfer happen after the scene.
uniform int hdrOutput;

// Linear light to what the framebuffer expects: unchanged for the HDR
// target, otherwise (previews drawn straight to the window) tone mapped
// with the default curve. `tonemap.glsl` comes before this file.
vec3 toDisplay(vec3 color) {
    if (hdrOutput == 1) {
        return max(color, vec3(0.0));
    }
    return linearToSrgb(tonemap(color, TONEMAP_ACES));
}
