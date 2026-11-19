// Shared physically based lighting (metal/roughness workflow): the sun and
// the moon with the shadow map and cloud cover, the sky as ambient light,
// the scene's lights (`light/light.rs`) with the light they bounce around
// (`light/global_illumination.rs`), and the BRDF.
// Inserted after the `#version` line (after `hdr/tonemap.glsl`); colours
// are linear until `toDisplay`.

const float PI = 3.14159265;

// Rain or snow hiding the distance (in `weather/weather.glsl`, which comes
// after this).
vec3 weatherFog(vec3 color, vec3 worldPos);

// Sun irradiance at noon and the sky's ambient light, in linear units. Tuned
// so lit models sit at about the brightness of the terrain around them.
const float SUN_INTENSITY = 3.5;
const vec3 SKY_DAY = vec3(0.50, 0.55, 0.65);
const vec3 SKY_NIGHT = vec3(0.004, 0.005, 0.009);
// The night sky's own light (stars, airglow), times `nightSky`: what is left
// where the moon does not reach.
const vec3 NIGHT_SKY_GLOW = vec3(0.010, 0.013, 0.026);
// Moonlight at `moonlight` 1: a cool blue, about a tenth of the sun.
const vec3 MOON_COLOR = vec3(0.55, 0.68, 1.0);
const float MOON_INTENSITY = 0.5;
// Share of the moon's light the moonlit sky gives as ambient light.
const float MOON_SKY = 0.25;
// Must match `MOON_SHADOW_BELOW` in `light/global_illumination.rs`: the
// moon only shines once the sun is this far down.
const float MOON_SHADOW_BELOW = -0.06;
// How much of a lamp's light comes back from its surroundings, and how far
// past its range (`localBounce`).
const float BOUNCE_STRENGTH = 1.5;
const float BOUNCE_REACH = 2.0;
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

// The moon and the indirect light (`light::global_illumination`).
uniform vec3 moonDirection;    // normalized, points toward the moon
uniform float moonlight;       // 0 = moonless
uniform float nightSky;        // night sky light scale
uniform float lightBounce;     // lamp light bounce scale
uniform int moonShadows;       // 1: the shadow map is cast from the moon

// The scene's lights, uploaded once a frame (`light::LightBuffer`). Must
// match `MAX_LIGHTS` and `LIGHTS_BINDING` in `light/light.rs`.
const int MAX_LIGHTS = 64;
const int LIGHT_POINT = 0;
const int LIGHT_SPOT = 1;
const int LIGHT_DIRECTIONAL = 2;
struct Light {
    vec4 positionRange;  // xyz world position, w range
    vec4 radianceBounce; // rgb linear colour × intensity, w bounce
    vec4 directionKind;  // xyz direction it shines, w kind
    vec4 cone;           // x cos inner angle, y cos outer angle,
                         // z shadow layer (-1: no shadow)
};
layout(std140, binding = 1) uniform Lights {
    Light lights[MAX_LIGHTS];
};
uniform int lightCount;        // 0 while no light buffer is meant

// Clustered lighting (`light::clusters`): the lights that can reach each
// cluster of the camera's view. Must match the bindings and the grid there.
layout(std140, binding = 2) uniform LightClusterGrid {
    mat4 clusterView;          // world to the camera's view space
    vec4 clusterProjection;    // x, y: projection scale, z: near, w: ln(far/near)
    ivec4 clusterDims;         // tiles across, tiles up, slices, 1: in use
};
layout(std430, binding = 3) readonly buffer LightClusterRanges {
    uvec2 clusterRanges[];     // first index, count
};
layout(std430, binding = 4) readonly buffer LightClusterIndices {
    uint clusterLights[];
};

// The lights to go through at `P`: `count` of them, from `first` in
// `clusterLights`, or (first < 0) the first `count` of `lights`.
//
// Hardened against a non-finite `P` (a NaN from a broken transform): every
// test below is written so that NaN takes the safe branch, the cell is
// clamped into the grid and the count to `MAX_LIGHTS`. Without that, an
// undefined cell index read a count from outside the buffer and the light
// loops ran for billions of steps, long enough for Windows to reset the
// GPU driver (TDR).
ivec2 lightsAt(vec3 P) {
    ivec2 all = ivec2(-1, clamp(lightCount, 0, MAX_LIGHTS));
    if (lightCount <= 0 || clusterDims.w != 1) {
        return all;
    }
    vec3 v = (clusterView * vec4(P, 1.0)).xyz;
    float depth = -v.z;
    if (!(depth > 0.0)) {
        return all;
    }
    vec2 ndc = clusterProjection.xy * v.xy / depth;
    float slice = log(max(depth, clusterProjection.z) / clusterProjection.z) / clusterProjection.w;
    if (!(abs(ndc.x) <= 1.0 && abs(ndc.y) <= 1.0 && slice <= 1.0)) {
        return all;
    }
    ivec3 dims = max(clusterDims.xyz, ivec3(1));
    ivec3 cell = clamp(ivec3(vec3(ndc * 0.5 + 0.5, slice) * vec3(dims)), ivec3(0), dims - 1);
    int index = (cell.z * dims.y + cell.y) * dims.x + cell.x;
    if (index < 0 || index >= clusterRanges.length()) {
        return all;
    }
    uvec2 range = clusterRanges[index];
    int first = int(min(range.x, uint(clusterLights.length())));
    int count = int(min(range.y, uint(MAX_LIGHTS)));
    return ivec2(first, min(count, clusterLights.length() - first));
}

// The `k`th light of what `lightsAt` gave, as an index into `lights`
// (callers skip one at or past `lightCount`).
int lightAt(ivec2 list, int k) {
    return list.x < 0 ? k : int(min(clusterLights[list.x + k], uint(MAX_LIGHTS)));
}

// Shadow cube maps of point and spot lights (`light::shadow`), one cube
// per layer, with hardware depth comparison. Must match `SHADOW_NEAR`.
const float LIGHT_SHADOW_NEAR = 0.05;
uniform samplerCubeArrayShadow lightShadowMaps;

// Cascaded shadow map: one depth layer per cascade, nearest first. Must
// match `SHADOW_CASCADES` in `light/skylight.rs`.
const int SHADOW_CASCADES = 4;
uniform sampler2DArrayShadow shadowMap;
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
    // Bilinear 3x3 PCF: 3x3 taps a texel apart, each comparing the 2x2
    // texels around it in hardware and weighting them by where the point
    // falls between them, so shadow edges are smooth instead of texel
    // steps. (The same as 4x4 texel tests weighted 1-f, 1, 1, f per axis.)
    vec2 texel = 1.0 / vec2(textureSize(shadowMap, 0).xy);
    float lit = 0.0;
    for (int j = -1; j <= 1; ++j) {
        for (int i = -1; i <= 1; ++i) {
            vec2 at = p.xy + vec2(i, j) * texel;
            lit += texture(shadowMap, vec4(at, float(cascade), reference));
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

// Moonlight from above the horizon, before it fades for the dawn.
vec3 moonGlow() {
    float above = smoothstep(-0.05, 0.15, moonDirection.y);
    float clouds = 1.0 - 0.85 * clamp(cloudCoverage, 0.0, 1.0);
    return MOON_COLOR * MOON_INTENSITY * moonlight * above * clouds;
}

// Direct moonlight. It fades out just before the sun comes up (and in just
// after it has set), in a short window so dawn does not go darker than the
// night before it.
vec3 moonRadiance() {
    float night = 1.0 - smoothstep(MOON_SHADOW_BELOW - 0.06, MOON_SHADOW_BELOW, sunDirection.y);
    return moonGlow() * night;
}

// Light arriving from the sky in direction `dir`: sky above, a darker
// ground bounce below, and a brighter glow toward the sun. The glow also
// reaches surfaces the sun itself is blocked from, so bumps in shadow still
// have a lit and a dark side.
vec3 skyLight(vec3 dir) {
    float day = smoothstep(-0.10, 0.20, sunDirection.y);
    float cover = clamp(cloudCoverage, 0.0, 1.0);
    // The moonlit sky fades into the day sky with the mix below.
    vec3 night = SKY_NIGHT + NIGHT_SKY_GLOW * nightSky + moonGlow() * MOON_SKY;
    vec3 sky = mix(night, SKY_DAY, day) * (1.0 - 0.3 * cover);
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

// Light a surface reflects toward `V` from a light from `L` with
// irradiance 1, the cosine included.
vec3 brdf(vec3 N, vec3 V, vec3 L, vec3 albedo, float roughness, float metallic, vec3 F0) {
    float NdotL = max(dot(N, L), 0.0);
    if (NdotL <= 0.0) {
        return vec3(0.0);
    }
    float NdotV = max(dot(N, V), 1e-4);
    vec3 H = normalize(V + L);
    float NdotH = max(dot(N, H), 0.0);
    float HdotV = max(dot(H, V), 0.0);
    vec3 F = fresnelSchlick(HdotV, F0);
    vec3 specular = distributionGGX(NdotH, roughness * roughness)
        * geometrySmith(NdotV, NdotL, roughness) * F
        / max(4.0 * NdotV * NdotL, 1e-4);
    vec3 kd = (1.0 - F) * (1.0 - metallic);
    return (kd * albedo / PI + specular) * NdotL;
}

// Light reaching `P` from light `l` past what stands in between, 0..1.
// `Ng` is the geometric normal (zero for particles), `L` toward the light.
float lightShadow(Light l, vec3 P, vec3 Ng, vec3 L) {
    float layer = l.cone.z;
    if (layer < 0.0 || isShadowPass == 1) {
        return 1.0;
    }
    vec3 lightPos = l.positionRange.xyz;
    float range = l.positionRange.w;
    vec3 v = P - lightPos;
    vec3 a = abs(v);
    float major = max(a.x, max(a.y, a.z));
    // World size of one texel here (a face spans 90°).
    float texel = 2.0 * major / float(textureSize(lightShadowMaps, 0).x);
    // Offset along the normal and toward the light, more at grazing
    // angles, so surfaces do not shadow themselves.
    float NdotL = dot(Ng, Ng) > 0.0 ? max(dot(Ng, L), 0.0) : 1.0;
    v += Ng * texel * 1.5;
    a = abs(v);
    major = max(a.x, max(a.y, a.z)) - texel * (1.0 + 2.0 * (1.0 - NdotL));
    major = max(major, LIGHT_SHADOW_NEAR);
    // The depth a surface at this distance along the face's axis has in
    // the map (the faces' perspective projection).
    float n = LIGHT_SHADOW_NEAR;
    float f = range;
    float depth = ((f + n) / (f - n) - 2.0 * f * n / ((f - n) * major)) * 0.5 + 0.5;
    // Four taps around the direction, each a 2×2 hardware comparison.
    vec3 dir = normalize(v);
    vec3 helper = abs(dir.y) < 0.9 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 u = normalize(cross(dir, helper));
    vec3 w = cross(dir, u);
    float spread = texel * 1.25 / max(length(v), 1e-4);
    float lit = 0.0;
    lit += texture(lightShadowMaps, vec4(dir + (u + w) * spread, layer), depth);
    lit += texture(lightShadowMaps, vec4(dir + (u - w) * spread, layer), depth);
    lit += texture(lightShadowMaps, vec4(dir + (-u + w) * spread, layer), depth);
    lit += texture(lightShadowMaps, vec4(dir + (-u - w) * spread, layer), depth);
    return lit * 0.25;
}

// Smooth fade to nothing at `range` (the inverse square law alone never
// reaches 0).
float rangeFade(float d, float range) {
    float x = d / range;
    float w = clamp(1.0 - x * x * x * x, 0.0, 1.0);
    return w * w;
}

// 0 outside a spot light's cone, 1 inside its core, smooth in between.
float spotCone(Light l, vec3 L) {
    float c = dot(-L, l.directionKind.xyz);
    float t = clamp((c - l.cone.y) / max(l.cone.x - l.cone.y, 1e-4), 0.0, 1.0);
    return t * t;
}

// Direct light of the scene's lights reflected toward `V`, with their
// shadows (`Ng`: the geometric normal).
vec3 localLights(vec3 P, vec3 N, vec3 Ng, vec3 V, vec3 albedo, float roughness, float metallic,
                 vec3 F0) {
    vec3 sum = vec3(0.0);
    ivec2 list = lightsAt(P);
    for (int k = 0; k < list.y; ++k) {
        int i = lightAt(list, k);
        if (i >= lightCount) {
            continue;
        }
        Light l = lights[i];
        int kind = int(l.directionKind.w + 0.5);
        vec3 L;
        float attenuation = 1.0;
        if (kind == LIGHT_DIRECTIONAL) {
            L = -l.directionKind.xyz;
        } else {
            vec3 toLight = l.positionRange.xyz - P;
            float d2 = dot(toLight, toLight);
            float range = l.positionRange.w;
            if (d2 >= range * range) {
                continue;
            }
            float d = sqrt(d2);
            L = toLight / max(d, 1e-4);
            attenuation = rangeFade(d, range) / max(d2, 0.01);
            if (kind == LIGHT_SPOT) {
                attenuation *= spotCone(l, L);
            }
        }
        if (attenuation <= 0.0 || dot(N, L) <= 0.0) {
            continue;
        }
        attenuation *= lightShadow(l, P, Ng, L);
        sum += brdf(N, V, L, albedo, roughness, metallic, F0) * l.radianceBounce.rgb * attenuation;
    }
    return sum;
}

// Light of the scene's lights arriving at `P` from every side, for things
// lit all round such as falling snow (no surface, no shadow).
vec3 localLightAt(vec3 P) {
    vec3 sum = vec3(0.0);
    ivec2 list = lightsAt(P);
    for (int k = 0; k < list.y; ++k) {
        int i = lightAt(list, k);
        if (i >= lightCount) {
            continue;
        }
        Light l = lights[i];
        int kind = int(l.directionKind.w + 0.5);
        if (kind == LIGHT_DIRECTIONAL) {
            sum += l.radianceBounce.rgb;
            continue;
        }
        vec3 toLight = l.positionRange.xyz - P;
        float d2 = dot(toLight, toLight);
        float range = l.positionRange.w;
        if (d2 >= range * range) {
            continue;
        }
        float d = sqrt(d2);
        float attenuation = rangeFade(d, range) / max(d2, 0.01);
        if (kind == LIGHT_SPOT) {
            attenuation *= spotCone(l, toLight / max(d, 1e-4));
        }
        if (attenuation > 0.0) {
            attenuation *= lightShadow(l, P, vec3(0.0), toLight / max(d, 1e-4));
        }
        sum += l.radianceBounce.rgb * attenuation;
    }
    return sum;
}

// Light of the scene's lights that reaches `P` after bouncing off what is
// around them, as radiance like `skyLight`: a soft, wide glow that also
// gets around corners and onto the sides facing away. A point light's
// bounce comes from around the light, a spot light's from where its cone
// points and in proportion to the share of light it sends out; a
// directional light's comes up from the ground everywhere.
vec3 localBounce(vec3 P, vec3 N) {
    vec3 sum = vec3(0.0);
    if (lightBounce <= 0.0) {
        return sum;
    }
    ivec2 list = lightsAt(P);
    for (int k = 0; k < list.y; ++k) {
        int i = lightAt(list, k);
        if (i >= lightCount) {
            continue;
        }
        Light l = lights[i];
        float bounce = l.radianceBounce.w * lightBounce * BOUNCE_STRENGTH;
        if (bounce <= 0.0) {
            continue;
        }
        int kind = int(l.directionKind.w + 0.5);
        vec3 dir = l.directionKind.xyz;
        if (kind == LIGHT_DIRECTIONAL) {
            // Up from the lit ground: strongest on what faces down.
            float facing = 0.5 + 0.5 * dot(N, dir);
            sum += l.radianceBounce.rgb * bounce * 0.15 * facing;
            continue;
        }
        float range = l.positionRange.w;
        vec3 source = l.positionRange.xyz;
        float share = 1.0;
        if (kind == LIGHT_SPOT) {
            source += dir * range * 0.3;
            share = (1.0 - l.cone.y) * 0.5;
        }
        vec3 toSource = source - P;
        float d2 = dot(toSource, toSource);
        float reach = range * BOUNCE_REACH;
        if (d2 >= reach * reach) {
            continue;
        }
        float d = sqrt(d2);
        float soft = range * 0.35;
        float attenuation = rangeFade(d, reach) / (d2 + soft * soft);
        float facing = 0.6 + 0.4 * dot(N, toSource / max(d, 1e-4));
        sum += l.radianceBounce.rgb * bounce * share * attenuation * facing;
    }
    return sum / PI;
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

    // The sun, and the moon at night; the shadow map belongs to whichever
    // of them is up (the other gives no light then).
    vec3 direct = vec3(0.0);
    vec3 sun = sunRadiance();
    if (moonShadows == 0 && dot(sun, sun) > 0.0 && dot(N, L) > 0.0) {
        direct += brdf(N, V, L, albedo, roughness, metallic, F0) * sun
            * sunShadow(worldPos, Ng, L) * sunFactor;
    }
    vec3 moon = moonRadiance();
    vec3 M = normalize(moonDirection);
    if (dot(moon, moon) > 0.0 && dot(N, M) > 0.0) {
        float shadow = moonShadows == 1 ? sunShadow(worldPos, Ng, M) : 1.0;
        direct += brdf(N, V, M, albedo, roughness, metallic, F0) * moon * shadow;
    }
    direct += localLights(worldPos, N, Ng, V, albedo, roughness, metallic, F0);

    vec3 ambient = (skyLight(N) + localBounce(worldPos, N)) * albedo * (1.0 - metallic)
        + skyLight(reflect(-V, N)) * envBrdf(F0, roughness, NdotV);
    return weatherFog(direct + ambient * ao * screenOcclusion(), worldPos);
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
