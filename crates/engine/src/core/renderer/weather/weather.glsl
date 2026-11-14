// Rain and snow on surfaces (`renderer/weather`): wet, darker and glossier
// surfaces, puddles with rain ripples, and snow cover, all only where the
// sky reaches. Comes after `lighting.glsl` and `weather_maps.glsl`.

uniform float rainAmount;    // how hard it rains now, 0..1
uniform float snowAmount;    // how hard it snows now, 0..1
uniform float wetness;       // how wet surfaces under open sky are, 0..1
uniform float puddleAmount;  // how full puddles are, 0..1
uniform float weatherTime;   // seconds

const vec3 SNOW_ALBEDO = vec3(0.80, 0.83, 0.88);

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

// How much of a model's surface at `worldPos` the snow on the ground
// buries, 0..1: what lies lower than the snow's surface around it (less
// what feet pressed down there) is under the snow. `exposure` is the open
// sky over it.
float snowBuried(vec3 worldPos, float exposure) {
    float cover = snowCover * exposure;
    if (useExposure == 0 || isShadowPass == 1 || cover <= 0.001 || snowDepth <= 0.005) {
        return 0.0;
    }
    float ground;
    if (!groundHeight(worldPos.xz, ground)) {
        return 0.0;
    }
    float above = worldPos.y - ground;
    vec2 trail = snowTrail(worldPos.xz);
    float top = snowDepth * (1.0 - 0.9 * trail.r + 0.35 * trail.g);
    // A soft line where the surface meets the snow, a little uneven.
    float soft = 0.01 + snowDepth * 0.08;
    top += (weatherNoise(worldPos.xz * 9.0) - 0.5) * soft;
    return (1.0 - smoothstep(top - soft, top, above)) * smoothstep(0.0, 0.2, cover);
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

// A model surface (`shadePbr`): wet in the rain, white under snow, buried
// where it is lower than the snow lying around it.
void applyWeather(vec3 worldPos, vec3 Ng,
                  inout vec3 albedo, inout vec3 N, inout float roughness, inout float metallic) {
    if (wetness + snowCover <= 0.0) {
        return;
    }
    float exposure = skyExposure(worldPos, Ng);
    applyRain(worldPos, Ng, exposure, albedo, N, roughness, metallic);
    applySnow(worldPos, Ng, snowShare(worldPos, N, Ng, exposure), albedo, N, roughness, metallic);
    // What lies on the ground sinks into the snow: its lower part looks
    // like the snow's surface.
    applySnow(worldPos, vec3(0.0, 1.0, 0.0), snowBuried(worldPos, exposure),
              albedo, N, roughness, metallic);
}
