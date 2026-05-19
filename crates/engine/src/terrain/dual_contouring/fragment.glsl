#version 460 core

in vec3 Color;
in vec3 Normal;
in vec3 toLightVector;
in vec4 fragPosLightSpace;
in vec3 WorldPos;

out vec4 FragColor;

uniform sampler2D shadowMap;
uniform int isShadowPass;
uniform float cloudCoverage; // 0..1 — fraction of sky obscured by clouds
uniform vec3  sunDirection;  // normalized, points from origin toward sun
uniform float uTime;         // monotonic seconds (matches sky shader)

// ── Cloud shadow ─────────────────────────────────────────────────────
// Ports just enough of the sky shader's volumetric cloud field to sample
// density along the sun ray. Doing it this way (instead of a separate 2D
// approximation) means the shadow on the ground actually matches whatever
// cloud is sitting between this terrain point and the sun.
const float CLOUD_BOTTOM_T = 350.0;
const float CLOUD_TOP_T    = 850.0;

float hash13T(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return fract((p.x + p.y) * p.z);
}

float vnoise3T(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    vec3 u = f * f * (3.0 - 2.0 * f);
    float n000 = hash13T(i);
    float n100 = hash13T(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash13T(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash13T(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash13T(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash13T(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash13T(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash13T(i + vec3(1.0, 1.0, 1.0));
    float nx00 = mix(n000, n100, u.x);
    float nx10 = mix(n010, n110, u.x);
    float nx01 = mix(n001, n101, u.x);
    float nx11 = mix(n011, n111, u.x);
    float nxy0 = mix(nx00, nx10, u.y);
    float nxy1 = mix(nx01, nx11, u.y);
    return mix(nxy0, nxy1, u.z);
}

// Cheap 3-octave 3D fbm — matches the sky's `cloudFbm(p, 3)` exactly.
float cloudFbm3T(vec3 p) {
    float s = 0.0;
    float a = 0.5;
    float n = 0.0;
    mat2 R = mat2(0.80, 0.60, -0.60, 0.80);
    for (int i = 0; i < 3; i++) {
        s += a * vnoise3T(p);
        n += a;
        p.xz = R * p.xz * 2.03;
        p.y  = p.y * 2.03 + 11.0;
        a *= 0.5;
    }
    return s / max(n, 1e-4);
}

// Mirrors the sky shader's `cloudDensity(p, cheap=true, lod=0)`. Same
// constants, same wind, same coverage threshold, so the value here equals
// what the sky raymarch sees at the same world point.
float cloudDensityT(vec3 p) {
    float h = (p.y - CLOUD_BOTTOM_T) / (CLOUD_TOP_T - CLOUD_BOTTOM_T);
    if (h < 0.0 || h > 1.0) return 0.0;

    vec3 wind = vec3(0.9, 0.0, 0.3) * uTime * 0.7;
    vec3 q = (p - wind) * 0.0018;

    float weather    = cloudFbm3T(q * 0.30 + vec3(3.1));
    float weatherMod = mix(0.35, 1.0, smoothstep(0.30, 0.85, weather));

    float bottomNoise = cloudFbm3T(q * 1.6 + vec3(31.0));
    float bottomEdge  = smoothstep(0.0, mix(0.08, 0.32, bottomNoise), h);
    float topEdge     = smoothstep(1.0, 0.55, h);
    float vshape      = bottomEdge * topEdge;

    float base = cloudFbm3T(q);

    float cov    = clamp(cloudCoverage, 0.0, 1.0);
    float cutoff = mix(1.05, -0.10, cov);
    float d = max(base - cutoff, 0.0);

    float erode = cloudFbm3T(q * 4.5 + vec3(17.0));
    d *= mix(0.30, 1.0, erode);

    return d * vshape * weatherMod * 5.0;
}

float cloudShadow() {
    // Sun below the horizon — no point sampling.
    if (sunDirection.y < 0.05) return 1.0;

    // March from cloud base to cloud top along the sun direction starting
    // above this terrain point. Few steps, large jumps — we just need an
    // optical-depth estimate, not detailed scattering.
    float tEnter = (CLOUD_BOTTOM_T - WorldPos.y) / sunDirection.y;
    float tExit  = (CLOUD_TOP_T    - WorldPos.y) / sunDirection.y;
    if (tExit <= 0.0) return 1.0;
    tEnter = max(tEnter, 0.0);

    const int   STEPS = 4;
    float stepLen = (tExit - tEnter) / float(STEPS);
    vec3  pos = WorldPos + sunDirection * (tEnter + 0.5 * stepLen);

    float optical = 0.0;
    for (int i = 0; i < STEPS; i++) {
        optical += cloudDensityT(pos) * stepLen;
        pos += sunDirection * stepLen;
    }

    // Beer–Lambert through the cloud column toward the sun. Coefficient
    // is gentler than the sky's self-shadow (0.85) so terrain stays
    // readable under thick cover.
    float sunT = exp(-optical * 0.35);
    return mix(0.20, 1.0, sunT);
}

float ShadowCalculation(vec4 fragPosLightSpace, vec3 toLightVector, vec3 normal) {
    vec3 projCoords = fragPosLightSpace.xyz / fragPosLightSpace.w;
    projCoords = projCoords * 0.5 + 0.5;
    if (projCoords.z > 1.0) {
        return 0.0;
    }
    float closestDepth = texture(shadowMap, projCoords.xy).r;
    float currentDepth = projCoords.z;
    float bias = max(0.0002 * (1.0 - dot(normal, toLightVector)), 0.0003);
    float shadow = 0.0;
    vec2 texelSize = 1.0 / textureSize(shadowMap, 0);
    for(int x = -2; x <= 2; ++x) {
        for(int y = -2; y <= 2; ++y) {
            float pcfDepth = texture(shadowMap, projCoords.xy + vec2(x, y) * texelSize).r; 
            shadow += currentDepth - bias > pcfDepth ? 1.0 : 0.0;        
        }    
    }
    shadow /= 25.0;
    return shadow;
}

void main() {
    if (isShadowPass == 1) {
        FragColor = vec4(Color, 1.0);
        return;
    }

    vec3 unitNormal = normalize(Normal);
    vec3 normal = unitNormal;

    normal = normalize(normal);

    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(normal, unitToLightVector);

    // Sun altitude drives the whole day/night response.
    float sunAlt = unitToLightVector.y;
    float dayFactor = smoothstep(-0.10, 0.20, sunAlt);

    // Wrap lighting slightly so north-facing slopes aren't pitch-black
    float diffuseFactor = max(intensity * 0.9 + 0.1, 0.0) * dayFactor;
    // Ambient floor drops to a faint moonlight at night.
    float ambient = mix(0.05, 0.35, dayFactor);
    float shadow = ShadowCalculation(fragPosLightSpace, unitToLightVector, normal);

    // Cloud cover attenuates direct sun (blocked) and slightly dims the
    // sky ambient (overcast skies still scatter light, just less of it).
    float cov = clamp(cloudCoverage, 0.0, 1.0);
    float sunBlock   = 1.0 - 0.85 * cov;
    float ambientDim = 1.0 - 0.30 * cov;

    // Procedural cloud shadow — patches darken when a cloud is overhead.
    float cloudS = cloudShadow();

    float lit = ambient * ambientDim
              + (1.0 - shadow) * diffuseFactor * sunBlock * cloudS;
    FragColor = vec4(lit * Color, 1.0);
}