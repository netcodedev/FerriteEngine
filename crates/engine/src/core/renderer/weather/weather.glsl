// Rain and snow on surfaces (`renderer/weather`): wet, darker and glossier
// surfaces, puddles with rain ripples, and snow cover, all only where the
// sky reaches. Comes after `lighting.glsl`.

uniform float rainAmount;    // how hard it rains now, 0..1
uniform float snowAmount;    // how hard it snows now, 0..1
uniform float wetness;       // how wet surfaces under open sky are, 0..1
uniform float puddleAmount;  // how full puddles are, 0..1
uniform float snowCover;     // how much of the open ground snow covers, 0..1
uniform float snowDepth;     // metres
uniform float weatherTime;   // seconds

// Depth of the scene seen from straight above: what is under a roof, a
// tree or an overhang stays dry.
uniform int useExposure;
uniform sampler2D exposureMap;
uniform mat4 exposureMatrix;
uniform float exposureRange; // metres the depth 0..1 spans

// Trails pressed into the snow: r how far pressed down (0..1), g the snow
// pushed up around a print. `trailArea` is (x, z) of its corner and size.
uniform int useSnowTrails;
uniform sampler2D snowTrailMap;
uniform vec3 trailArea;

const vec3 SNOW_ALBEDO = vec3(0.80, 0.83, 0.88);

float weatherHash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float weatherNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(weatherHash(i), weatherHash(i + vec2(1.0, 0.0)), u.x),
               mix(weatherHash(i + vec2(0.0, 1.0)), weatherHash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// Share of the sky above `worldPos` that is open, 0..1. The point is moved
// off the surface along its normal first, so walls and cliffs (which see
// their own top from above) count as open.
float skyExposure(vec3 worldPos, vec3 Ng) {
    if (useExposure == 0 || isShadowPass == 1) {
        return 1.0;
    }
    vec3 p = worldPos + Ng * 0.3 + vec3(0.0, 0.05, 0.0);
    vec4 clip = exposureMatrix * vec4(p, 1.0);
    vec3 q = clip.xyz / clip.w * 0.5 + 0.5;
    vec2 edge = abs(q.xy * 2.0 - 1.0);
    float outside = smoothstep(0.85, 0.98, max(edge.x, edge.y));
    if (outside >= 1.0 || q.z <= 0.0) {
        return 1.0;
    }
    vec2 size = vec2(textureSize(exposureMap, 0));
    // Ground on a slope is higher a texel uphill: allow for it.
    float slope = sqrt(max(1.0 - Ng.y * Ng.y, 0.0)) / max(abs(Ng.y), 0.2);
    float texelMetres = 1.0 / (size.x * exposureMatrix[0][0] * 0.5);
    float bias = (0.15 + texelMetres * 1.5 * slope) / exposureRange;
    float open = 0.0;
    for (int j = -1; j <= 1; ++j) {
        for (int i = -1; i <= 1; ++i) {
            float depth = texture(exposureMap, q.xy + vec2(i, j) * 1.5 / size).r;
            open += q.z - bias <= depth ? 1.0 : 0.0;
        }
    }
    return mix(open / 9.0, 1.0, outside);
}

// Height (dh/dx, dh/dz) of rings spreading where drops hit standing water,
// for rain of `amount` 0..1 at `p` (metres); `footprint` is the metres one
// pixel covers there (`length(fwidth(p))`, taken where all pixels run).
vec2 rainRipples(vec2 p, float t, float amount, float footprint) {
    vec2 slope = vec2(0.0);
    // Rings far finer than a pixel would only glitter.
    float detail = 1.0 - smoothstep(0.015, 0.05, footprint);
    if (amount <= 0.0 || detail <= 0.0) {
        return slope;
    }
    for (int layer = 0; layer < 3; ++layer) {
        vec2 q = p * 2.2 + vec2(float(layer) * 0.37, float(layer) * 0.71);
        vec2 cell = floor(q);
        for (int j = -1; j <= 1; ++j) {
            for (int i = -1; i <= 1; ++i) {
                vec2 c = cell + vec2(i, j);
                float h = weatherHash(c + float(layer) * 17.0);
                // Fewer drops in light rain.
                if (h > amount) {
                    continue;
                }
                vec2 centre = c + vec2(weatherHash(c + 3.1), weatherHash(c + 7.7));
                float phase = fract(t * 0.9 + h * 7.0 + float(layer) * 0.33);
                vec2 d = q - centre;
                float r = length(d);
                float ring = r - phase * 0.9;
                float wave = sin(ring * 40.0) * exp(-ring * ring * 90.0) * (1.0 - phase) * (1.0 - phase);
                slope += d / max(r, 1e-3) * wave;
            }
        }
    }
    return slope * 0.6 * detail;
}

// Where puddles gather on flat ground, 0..1, as they fill up. Their edges
// soften with distance (`footprint`: metres per pixel), so far away they
// blend instead of glittering.
float puddleMask(vec3 worldPos, vec3 Ng, float footprint) {
    float level = smoothstep(0.975, 0.995, Ng.y);
    float n = weatherNoise(worldPos.xz * 0.18) * 0.7 + weatherNoise(worldPos.xz * 0.7) * 0.3;
    float soft = 0.04 + footprint * 0.6;
    float edge = 1.0 - puddleAmount * 0.5;
    return level * smoothstep(edge, edge + soft, n);
}

// Wet the surface: porous (rough) surfaces darken as they soak up water,
// all of them get a glossy film, and puddles become mirror-like water with
// ripples. `exposure` is the open sky over it. Returns the puddle share.
float applyRain(vec3 worldPos, vec3 Ng, float exposure,
                inout vec3 albedo, inout vec3 N, inout float roughness, inout float metallic) {
    // Before any branch: derivatives need every pixel of a quad.
    float footprint = length(fwidth(worldPos.xz));
    // Upward faces catch more; walls still get rain blown at them.
    float wet = wetness * exposure * mix(0.55, 1.0, clamp(Ng.y, 0.0, 1.0));
    if (wet <= 0.001) {
        return 0.0;
    }
    float porosity = smoothstep(0.35, 0.9, roughness) * (1.0 - metallic);
    albedo *= mix(1.0, mix(0.85, 0.55, porosity), wet);
    roughness = mix(roughness, mix(0.12, 0.32, porosity), wet);

    float puddle = puddleMask(worldPos, Ng, footprint) * exposure * smoothstep(0.4, 0.8, wetness);
    if (puddle > 0.0) {
        albedo *= mix(1.0, 0.55, puddle);
        roughness = mix(roughness, 0.03, puddle);
        metallic = mix(metallic, 0.0, puddle);
        vec2 slope = rainRipples(worldPos.xz, weatherTime, rainAmount, footprint);
        vec3 still = normalize(Ng + vec3(-slope.x, 0.0, -slope.y));
        N = normalize(mix(N, still, puddle));
    }
    return puddle;
}

// How much snow lies on a surface facing `N` (geometry `Ng`), 0..1: first
// on what faces up, more and steeper the more snow there is.
float snowShare(vec3 worldPos, vec3 N, vec3 Ng, float exposure) {
    float cover = snowCover * exposure;
    if (cover <= 0.001) {
        return 0.0;
    }
    float n = weatherNoise(worldPos.xz * 1.7) * 0.6 + weatherNoise(worldPos.xz * 6.0) * 0.4;
    // Deep snow buries the small bumps: it follows the smooth surface.
    float up = mix(N.y, Ng.y, smoothstep(0.02, 0.15, snowDepth));
    // Light snow stays on what is nearly level; full cover reaches slopes
    // of about 60 degrees. Walls stay bare.
    float threshold = mix(0.92, 0.45, cover) + (n - 0.5) * 0.2;
    return smoothstep(threshold - 0.08, threshold + 0.08, up) * smoothstep(0.0, 0.2, cover);
}

// Cover the surface with `share` of snow: bright, rough, with a fine
// grain whose facets glint.
void applySnow(vec3 worldPos, vec3 Ng, float share,
               inout vec3 albedo, inout vec3 N, inout float roughness, inout float metallic) {
    if (share <= 0.0) {
        return;
    }
    vec2 grain = vec2(weatherNoise(worldPos.xz * 40.0), weatherNoise(worldPos.zx * 40.0 + 5.0)) - 0.5;
    // Soft drifts the wind piled up, a metre or two across.
    vec2 q = worldPos.xz * 0.7;
    float d0 = weatherNoise(q);
    vec2 drift = vec2(weatherNoise(q + vec2(0.15, 0.0)) - d0, weatherNoise(q + vec2(0.0, 0.15)) - d0);
    vec3 snowN = normalize(Ng + vec3(grain.x, 0.0, grain.y) * 0.25 - vec3(drift.x, 0.0, drift.y) * 1.2);
    albedo = mix(albedo, SNOW_ALBEDO, share);
    N = normalize(mix(N, snowN, share));
    roughness = mix(roughness, 0.55, share);
    metallic = mix(metallic, 0.0, share);
}

// The trail map at `p`: (pressed 0..1, rim 0..1). Zero outside it.
vec2 snowTrail(vec2 p) {
    if (useSnowTrails == 0) {
        return vec2(0.0);
    }
    vec2 uv = (p - trailArea.xy) / trailArea.z;
    vec2 edge = abs(uv * 2.0 - 1.0);
    float fade = 1.0 - smoothstep(0.9, 1.0, max(edge.x, edge.y));
    if (fade <= 0.0) {
        return vec2(0.0);
    }
    return texture(snowTrailMap, uv).rg * fade;
}

// Height of the snow's surface above the ground at `p`, in metres: its
// depth, less what feet pressed down, plus what they pushed up around.
float snowHeight(vec2 p) {
    vec2 trail = snowTrail(p);
    return snowDepth * (1.0 - 0.9 * trail.r + 0.35 * trail.g);
}

// Murk of falling rain or snow that hides the distance (display colour;
// `sky/fragment.glsl` turns its horizon the same grey).
const vec3 MURK = vec3(0.50, 0.53, 0.57);
// Per metre: how fast the distance fades into the murk in the heaviest rain
// or snow (about 60 % at 300 m).
const float MURK_DENSITY = 0.003;

// `color` (linear) seen from the camera through the falling rain or snow.
vec3 weatherFog(vec3 color, vec3 worldPos) {
    float falling = max(rainAmount, snowAmount * 1.4);
    if (falling <= 0.0 || isShadowPass == 1) {
        return color;
    }
    float day = smoothstep(-0.10, 0.20, sunDirection.y);
    vec3 murk = pow(MURK * mix(0.04, 1.0 - 0.3 * falling, day), vec3(2.2));
    float fog = 1.0 - exp(-length(worldPos - cameraPosition) * MURK_DENSITY * falling);
    return mix(color, murk, fog);
}

// A model surface (`shadePbr`): wet in the rain, white under snow.
void applyWeather(vec3 worldPos, vec3 Ng,
                  inout vec3 albedo, inout vec3 N, inout float roughness, inout float metallic) {
    if (wetness + snowCover <= 0.0) {
        return;
    }
    float exposure = skyExposure(worldPos, Ng);
    applyRain(worldPos, Ng, exposure, albedo, N, roughness, metallic);
    applySnow(worldPos, Ng, snowShare(worldPos, N, Ng, exposure), albedo, N, roughness, metallic);
}
