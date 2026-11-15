// Terrain surface: grass, sand and rock textures blended by height, slope
// and the textures' own height maps, projected triplanar so cliffs do not
// stretch, and lit like models (`lighting.glsl` comes before this). Without
// textures the terrain falls back to plain colours.

in vec3 Normal;
in vec3 WorldPos;

out vec4 FragColor;

layout(binding = 0) uniform sampler2DArray terrainAlbedo;
layout(binding = 1) uniform sampler2DArray terrainNormal;
// r: roughness, g: ambient occlusion, b: height
layout(binding = 2) uniform sampler2DArray terrainSurface;
uniform int hasTerrainTextures;
// Metres one texture tile covers, per layer.
uniform float tileSize[3];
// Metres between the lowest and highest point of a layer's height map.
uniform float heightDepth[3];

// Layers, in the order of `terrain::materials::LAYERS`.
const int GRASS = 0;
const int SAND = 1;
const int ROCK = 2;

const vec3 SNOW = vec3(0.86, 0.88, 0.92);

// The second, coarser sampling of each layer: this many tiles per tile.
// Its bumps stay visible from afar, where the fine normals are mipmapped
// flat, and it breaks up the tiling.
const float MACRO_SCALE = 6.0;
// Parallax depth relative to the scan's real height, and the distance
// over which it fades out (it is invisible further away).
const float PARALLAX_STRENGTH = 2.0;
const float PARALLAX_FADE_START = 6.0;
const float PARALLAX_FADE_END = 20.0;

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float valueNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x), u.y);
}

// How much of each layer a spot gets before the height maps refine it:
// sand on the shore and under water, rock on cliffs and high up, grass
// elsewhere. `n` (-1..1) wobbles the borders.
vec3 layerWeights(float h, float slope, float n) {
    float sand = 1.0 - smoothstep(49.0, 52.5, h + n * 1.5);
    float cliff = smoothstep(0.50, 0.72, slope + n * 0.05) * smoothstep(44.0, 52.0, h);
    float rock = max(cliff, smoothstep(80.0, 92.0, h + n * 4.0));
    sand *= 1.0 - rock;
    return vec3(max(1.0 - sand - rock, 0.0), sand, rock);
}

// Height-based blend: where layers meet, the one whose height map is higher
// shows, so grass grows into the cracks of rock instead of fading over it.
vec3 heightBlend(vec3 weights, vec3 heights) {
    const float depth = 0.2;
    vec3 h = heights + weights;
    float top = max(max(h.x, h.y), h.z) - depth;
    vec3 b = max(h - top, 0.0) * step(0.001, weights);
    return b / max(b.x + b.y + b.z, 1e-4);
}

vec3 unpackNormal(vec4 texel) {
    vec3 n = texel.xyz * 2.0 - 1.0;
    // OpenGL-style maps, uploaded top row first: green points along -v.
    n.y = -n.y;
    return n;
}

struct LayerSample {
    vec3 albedo;
    vec3 normal;  // world space
    vec3 surface; // roughness, occlusion, height
};

// Triplanar sample of one layer with whiteout-blended normals
// (Ben Golus, "Normal Mapping for a Triplanar Shader"), `s` tiles per metre.
// `parallax` (0..1) shifts the top projection by the height map toward the
// viewer `V`, so the ground looks deeper close up.
LayerSample sampleLayer(int layer, vec3 p, vec3 Ng, vec3 blend, float s, vec3 V, float parallax) {
    vec3 axisSign = vec3(Ng.x < 0.0 ? -1.0 : 1.0, Ng.y < 0.0 ? -1.0 : 1.0, Ng.z < 0.0 ? -1.0 : 1.0);
    vec3 albedo = vec3(0.0);
    vec3 surface = vec3(0.0);
    vec3 nX = vec3(0.0), nY = vec3(0.0), nZ = vec3(0.0);
    if (blend.x > 0.01) {
        vec3 uv = vec3(p.z * s * axisSign.x, p.y * s, layer);
        albedo += texture(terrainAlbedo, uv).rgb * blend.x;
        surface += texture(terrainSurface, uv).rgb * blend.x;
        vec3 t = unpackNormal(texture(terrainNormal, uv));
        t.x *= axisSign.x;
        t.z *= axisSign.x;
        nX = vec3(t.xy + Ng.zy, abs(t.z) * Ng.x);
    }
    if (blend.y > 0.01) {
        vec3 uv = vec3(p.x * s * axisSign.y, p.z * s, layer);
        if (parallax > 0.0 && Ng.y > 0.0) {
            float height = texture(terrainSurface, uv).b;
            vec2 toEye = vec2(V.x * axisSign.y, V.z) / max(V.y, 0.3);
            uv.xy += toEye * (height - 0.5) * heightDepth[layer] * PARALLAX_STRENGTH * s * parallax;
        }
        albedo += texture(terrainAlbedo, uv).rgb * blend.y;
        surface += texture(terrainSurface, uv).rgb * blend.y;
        vec3 t = unpackNormal(texture(terrainNormal, uv));
        t.x *= axisSign.y;
        t.z *= axisSign.y;
        nY = vec3(t.xy + Ng.xz, abs(t.z) * Ng.y);
    }
    if (blend.z > 0.01) {
        vec3 uv = vec3(-p.x * s * axisSign.z, p.y * s, layer);
        albedo += texture(terrainAlbedo, uv).rgb * blend.z;
        surface += texture(terrainSurface, uv).rgb * blend.z;
        vec3 t = unpackNormal(texture(terrainNormal, uv));
        t.x *= -axisSign.z;
        t.z *= axisSign.z;
        nZ = vec3(t.xy + Ng.xy, abs(t.z) * Ng.z);
    }
    float total = max(blend.x * step(0.01, blend.x) + blend.y * step(0.01, blend.y)
                      + blend.z * step(0.01, blend.z), 1e-4);
    vec3 n = normalize(nX.zyx * blend.x + nY.xzy * blend.y + nZ.xyz * blend.z);
    return LayerSample(albedo / total, n, surface / total);
}

// The colours the terrain had before it was textured (display space).
vec3 paletteColor(float h, float slope) {
    vec3 seabedDeep = vec3(0.32, 0.22, 0.12);
    vec3 sand       = vec3(0.76, 0.70, 0.50);
    vec3 grassLow   = vec3(0.22, 0.52, 0.12);
    vec3 grassHigh  = vec3(0.17, 0.40, 0.09);
    vec3 rock       = vec3(0.46, 0.43, 0.39);
    vec3 darkRock   = vec3(0.36, 0.34, 0.32);
    vec3 alpine     = vec3(0.50, 0.48, 0.46);
    vec3 color;
    if (h < 44.0) {
        color = mix(seabedDeep, sand, smoothstep(15.0, 44.0, h));
    } else if (h < 52.0) {
        color = mix(sand, grassLow, smoothstep(48.0, 52.0, h));
    } else if (h < 78.0) {
        color = mix(grassLow, grassHigh, smoothstep(52.0, 78.0, h));
    } else {
        color = mix(grassHigh, alpine, smoothstep(78.0, 90.0, h));
    }
    float rockBlend = smoothstep(0.57, 0.80, slope) * smoothstep(44.0, 52.0, h);
    return mix(color, mix(rock, darkRock, smoothstep(55.0, 90.0, h)), rockBlend);
}

// Height of the water's surface (`dual_contouring::WATER_LEVEL`).
const float WATER_LINE = 44.0;
// Footprints show their depth this close; further away the flat cover.
const float TRAIL_DISTANCE = 30.0;

// Snow on the ground: a layer `snowDepth` deep whose untouched top is
// the drawn surface (where everything stands), with the trails pressed
// into it. The view ray is followed on down into the snow (parallax), so
// prints look as deep as the snow.
void terrainSnow(float exposure, inout vec3 albedo, inout vec3 N, vec3 Ng,
                 inout float roughness, inout float ao) {
    float share = snowShare(WorldPos, N, Ng, exposure);
    if (share <= 0.0) {
        return;
    }
    vec2 at = WorldPos.xz;
    vec3 toEye = cameraPosition - WorldPos;
    float near = 1.0 - smoothstep(TRAIL_DISTANCE * 0.6, TRAIL_DISTANCE, length(toEye));
    // Where the snow layer lies, it shows the trails: here the snow stays
    // untouched, so nothing of the ground shows through the layer.
    vec2 fromEye = abs(toEye.xz);
    if (max(fromEye.x, fromEye.y) < snowLayerReach) {
        near = 0.0;
    }
    vec2 trail = vec2(0.0);
    vec3 snowN = Ng;
    if (useSnowTrails == 1 && near > 0.0 && snowDepth > 0.005) {
        // Down into the snow is away from the eye. March down to where the
        // ray meets the pressed surface, then narrow that step down, so
        // the walls of a print are smooth instead of terraced.
        vec3 V = normalize(toEye);
        vec2 drift = -V.xz / max(V.y, 0.2);
        float deepest = snowDepth * 0.9;
        int steps = int(mix(8.0, 32.0, 1.0 - clamp(V.y, 0.0, 1.0)));
        float above = 0.0;
        float below = 0.0;
        bool hit = snowDent(WorldPos.xz) <= 0.0;
        for (int i = 1; i <= steps && !hit; ++i) {
            float t = deepest * float(i) / float(steps);
            if (snowDent(WorldPos.xz + drift * t) <= t) {
                below = t;
                hit = true;
            } else {
                above = t;
            }
        }
        float t = 0.0;
        if (hit && below > 0.0) {
            for (int i = 0; i < 5; ++i) {
                float mid = 0.5 * (above + below);
                if (snowDent(WorldPos.xz + drift * mid) <= mid) {
                    below = mid;
                } else {
                    above = mid;
                }
            }
            // Between the last two: where the ray crosses the surface.
            float da = snowDent(WorldPos.xz + drift * above) - above;
            float db = snowDent(WorldPos.xz + drift * below) - below;
            t = mix(above, below, clamp(da / max(da - db, 1e-6), 0.0, 1.0));
        } else if (!hit) {
            t = deepest;
        }
        at = WorldPos.xz + drift * t * near;
        trail = snowTrail(at) * near;
        // Slope of the snow's surface where the ray meets it.
        const float e = 0.05;
        float dx = snowDent(at + vec2(e, 0.0)) - snowDent(at - vec2(e, 0.0));
        float dz = snowDent(at + vec2(0.0, e)) - snowDent(at - vec2(0.0, e));
        snowN = normalize(Ng + vec3(dx, 0.0, dz) / (2.0 * e) * near);
    }
    // Shallow snow pressed through shows the ground under it.
    float left = snowDepth * (1.0 - 0.9 * trail.r);
    share *= mix(1.0, smoothstep(0.004, 0.03, left), trail.r);
    float metallic = 0.0;
    applySnow(vec3(at.x, WorldPos.y, at.y), snowN, share, albedo, N, roughness, metallic);
    // Pressed snow is packed: smoother and a little grey; the walls of a
    // print are shaded off.
    albedo *= 1.0 - 0.25 * trail.r * share;
    roughness = mix(roughness, 0.4, trail.r * share);
    float depth = smoothstep(0.0, 0.25, snowDepth);
    ao *= 1.0 - 0.6 * trail.r * depth * share;
}

void main() {
    snowGradients(WorldPos);
    if (isShadowPass == 1) {
        FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    vec3 Ng = normalize(Normal);
    float h = WorldPos.y;
    float slope = 1.0 - abs(Ng.y);
    float n = valueNoise(WorldPos.xz * 0.08) * 2.0 - 1.0;

    vec3 albedo;
    vec3 N = Ng;
    float roughness = 0.9;
    float ao = 1.0;
    if (hasTerrainTextures == 1) {
        vec3 blend = pow(abs(Ng), vec3(4.0));
        blend /= blend.x + blend.y + blend.z;
        vec3 weights = layerWeights(h, slope, n);

        vec3 V = normalize(cameraPosition - WorldPos);
        float viewDistance = length(cameraPosition - WorldPos);
        float parallax = 1.0 - smoothstep(PARALLAX_FADE_START, PARALLAX_FADE_END, viewDistance);
        // Far away the coarse sampling takes over half of the colour.
        float far = smoothstep(15.0, 80.0, viewDistance) * 0.5;

        LayerSample samples[3];
        vec3 heights = vec3(0.0);
        for (int i = 0; i < 3; i++) {
            if (weights[i] > 0.001) {
                float s = 1.0 / tileSize[i];
                LayerSample fine = sampleLayer(i, WorldPos, Ng, blend, s, V, parallax);
                LayerSample macro = sampleLayer(i, WorldPos, Ng, blend, s / MACRO_SCALE, V, 0.0);
                samples[i].albedo = mix(fine.albedo, macro.albedo, far);
                // Add both layers' bumps to the surface normal.
                samples[i].normal = normalize(Ng + (fine.normal - Ng) + (macro.normal - Ng));
                samples[i].surface = fine.surface;
                heights[i] = fine.surface.b;
            } else {
                samples[i] = LayerSample(vec3(0.0), Ng, vec3(0.0));
            }
        }
        weights = heightBlend(weights, heights);

        albedo = vec3(0.0);
        vec3 normal = vec3(0.0);
        vec3 surface = vec3(0.0);
        for (int i = 0; i < 3; i++) {
            albedo += samples[i].albedo * weights[i];
            normal += samples[i].normal * weights[i];
            surface += samples[i].surface * weights[i];
        }
        N = normalize(normal);
        roughness = surface.r;
        // The scans' occlusion is for close-ups; at terrain distances its
        // full strength darkens whole fields.
        ao = mix(1.0, surface.g, 0.5);

        // Large-scale brightness variation hides the tiling from afar.
        albedo *= mix(0.8, 1.15, valueNoise(WorldPos.xz * 0.025));
    } else {
        albedo = pow(paletteColor(h, slope), vec3(2.2));
    }
    // Darker toward the deep sea floor.
    albedo *= mix(0.45, 1.0, smoothstep(15.0, 44.0, h));

    float snow = smoothstep(90.0, 100.0, h + n * 3.0);
    albedo = mix(albedo, pow(SNOW, vec3(2.2)), snow);
    roughness = mix(roughness, 0.6, snow);
    N = normalize(mix(N, Ng, snow));

    // Rain and snow, where the sky reaches and above the water.
    float metallic = 0.0;
    if (wetness + snowCover > 0.0) {
        float exposure = skyExposure(WorldPos, Ng) * smoothstep(WATER_LINE, WATER_LINE + 0.4, h);
        applyRain(WorldPos, Ng, exposure, albedo, N, roughness, metallic);
        terrainSnow(exposure, albedo, N, Ng, roughness, ao);
    }

    vec3 color = shadeSurface(WorldPos, N, Ng, albedo, roughness, metallic, ao, cloudShadow(WorldPos));
    FragColor = vec4(toDisplay(color), 1.0);
}
