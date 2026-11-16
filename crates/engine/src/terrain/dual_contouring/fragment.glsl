// Terrain surface: the biomes' ground colour (from the vertices), sand on
// the shore, rock on cliffs and snow where it is cold. With textures:
// grass, sand, rock, forest floor, dry grass, gravel, road, dirt, jungle
// floor, tundra moss, mud and cave rock blended
// by height, slope, climate, the biomes and what lies on the ground
// (roads, settlements, river banks), refined by the textures'
// own height maps, projected triplanar so cliffs do not
// stretch, and lit like models (`lighting.glsl` comes before this). Without
// textures the terrain falls back to plain colours.

in vec3 Normal;
in vec3 WorldPos;
// Display space.
in vec3 GroundColor;
// Temperature (°C at this height), humidity (0..1).
in vec2 Climate;
// Forest floor, dry grass, jungle floor, moss.
in vec4 Cover;
// Road, settlement, gravel, mud.
in vec4 Paint;
// How far under the surface (caves, under overhangs), 0..1.
in float Underground;

out vec4 FragColor;

layout(binding = 0) uniform sampler2DArray terrainAlbedo;
layout(binding = 1) uniform sampler2DArray terrainNormal;
// r: roughness, g: ambient occlusion, b: height
layout(binding = 2) uniform sampler2DArray terrainSurface;
uniform int hasTerrainTextures;
// The horizon (see `terrain::horizon` and the vertex shader) is left out
// within `horizonHole` metres of the camera at `horizonSink.xy` (x, z).
uniform int isHorizon;
uniform vec3 horizonSink;
uniform float horizonHole;
// Layers, in the order of `terrain::materials::LAYERS`.
const int GRASS = 0;
const int SAND = 1;
const int ROCK = 2;
const int FOREST = 3;
const int DRY = 4;
const int GRAVEL = 5;
const int ROAD = 6;
const int DIRT = 7;
const int JUNGLE = 8;
const int TUNDRA = 9;
const int MUD = 10;
const int CAVE_ROCK = 11;
const int LAYER_COUNT = 12;

// Metres one texture tile covers, per layer.
uniform float tileSize[LAYER_COUNT];
// Metres between the lowest and highest point of a layer's height map.
uniform float heightDepth[LAYER_COUNT];
// How far the layer's placements may be turned, as a share of a full turn
// either way (0 for the road, whose tracks follow it).
uniform float turn[LAYER_COUNT];
// 1 where the biomes' ground colour tints the layer (grass).
uniform float tinted[LAYER_COUNT];

// Colours of what lies on the ground, without textures (display space).
const vec3 ROAD_COLOR = vec3(0.50, 0.41, 0.26);
const vec3 TRODDEN_COLOR = vec3(0.56, 0.50, 0.38);
const vec3 GRAVEL_COLOR = vec3(0.47, 0.44, 0.40);
const vec3 MUD_COLOR = vec3(0.24, 0.19, 0.13);

const vec3 SNOW = vec3(0.86, 0.88, 0.92);

// The second, coarser sampling of each layer: this many tiles per tile.
// Its bumps stay visible from afar, where the fine normals are mipmapped
// flat, and it breaks up the tiling.
const float MACRO_SCALE = 6.0;
// Parallax depth relative to the scan's real height, and the distance
// over which it fades out (it is invisible further away).
const float PARALLAX_STRENGTH = 1.0;
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

// How hot and dry a spot is for a desert, 0..1.
float desertness(vec2 climate) {
    return (1.0 - smoothstep(0.12, 0.28, climate.y)) * smoothstep(16.0, 22.0, climate.x);
}

// How much of each layer a spot gets before the height maps refine it:
// sand on the shore, under water and in deserts, rock on cliffs and above
// the trees, dark rock underground; where plants grow, jungle floor in
// jungles, forest floor in forests (with grassy clearings), dry grass in
// savanna and steppe, moss in tundra and above the trees, grass elsewhere.
// Over that, mud in swamps, gravel on river banks and lake shores, dirt in
// settlements and the road, each leaving the rock of a cliff or a cave.
// `n` (-1..1) wobbles the borders.
void layerWeights(float h, float slope, float n, vec2 climate, out float w[LAYER_COUNT]) {
    float sand = max(1.0 - smoothstep(5.0, 8.5, h + n * 1.5), desertness(climate));
    float cliff = smoothstep(0.50, 0.72, slope + n * 0.05) * smoothstep(0.0, 8.0, h);
    float rock = max(cliff, 1.0 - smoothstep(-4.0, 0.0, climate.x + n * 1.5));
    float cave = smoothstep(0.1, 0.6, Underground);
    rock = max(rock, cave);
    sand *= 1.0 - rock;
    float plants = max(1.0 - sand - rock, 0.0);

    float patches = valueNoise(WorldPos.xz * 0.045);
    // Each takes what the ones before leave.
    float jungle = smoothstep(0.3, 0.6, Cover.z + n * 0.08);
    float forest = smoothstep(0.3, 0.65, Cover.x * mix(0.55, 1.15, patches) + n * 0.08) * (1.0 - jungle);
    float dry = smoothstep(0.25, 0.7, Cover.y + n * 0.12) * (1.0 - jungle - forest);
    float tundra = smoothstep(0.25, 0.65, Cover.w + n * 0.1) * (1.0 - jungle - forest - dry);
    for (int i = 0; i < LAYER_COUNT; i++) {
        w[i] = 0.0;
    }
    w[GRASS] = plants * max(1.0 - jungle - forest - dry - tundra, 0.0);
    w[SAND] = sand;
    w[ROCK] = rock * (1.0 - cave);
    w[CAVE_ROCK] = rock * cave;
    w[FOREST] = plants * forest;
    w[DRY] = plants * dry;
    w[JUNGLE] = plants * jungle;
    w[TUNDRA] = plants * tundra;

    // What lies on the ground, in the order it covers itself: a swamp's
    // mud under gravel under a settlement's dirt under the road.
    // A settlement's ground is trodden bare in patches, grass between;
    // a swamp's is mud in pools.
    float trodden = valueNoise(WorldPos.xz * 0.07) * 0.7 + valueNoise(WorldPos.xz * 0.23) * 0.3;
    float pools = valueNoise(WorldPos.xz * 0.05 + 17.0) * 0.6 + valueNoise(WorldPos.xz * 0.19) * 0.4;
    float cover[4] = float[4](
        smoothstep(0.3, 0.55, Paint.w * (pools + 0.2)),
        smoothstep(0.15, 0.6, Paint.z + n * 0.1),
        smoothstep(0.35, 0.6, Paint.y * (trodden + 0.15)),
        smoothstep(0.1, 0.5, Paint.x));
    int layers[4] = int[4](MUD, GRAVEL, DIRT, ROAD);
    for (int k = 0; k < 4; k++) {
        // Over everything but a cliff or a cave: also over the bare rock
        // above the trees, where a mountain road runs.
        float o = cover[k] * (1.0 - max(cliff, cave));
        if (o <= 0.0) {
            continue;
        }
        for (int i = 0; i < LAYER_COUNT; i++) {
            w[i] *= 1.0 - o;
        }
        w[layers[k]] += o;
    }

    // Too little to see is not sampled.
    float total = 0.0;
    for (int i = 0; i < LAYER_COUNT; i++) {
        w[i] = w[i] < 0.03 ? 0.0 : w[i];
        total += w[i];
    }
    for (int i = 0; i < LAYER_COUNT; i++) {
        w[i] /= max(total, 1e-4);
    }
}

// The strongest layers at a spot, at most `MAX_SAMPLED` (the rest are
// thin edges): their indices (-1: none) and weights, summing to 1.
const int MAX_SAMPLED = 4;
void strongest(float w[LAYER_COUNT], out ivec4 ids, out vec4 weights) {
    ids = ivec4(-1);
    weights = vec4(0.0);
    for (int i = 0; i < LAYER_COUNT; i++) {
        float v = w[i];
        int id = i;
        if (v <= 0.0) {
            continue;
        }
        // Insert into the list kept strongest first.
        for (int k = 0; k < MAX_SAMPLED; k++) {
            if (v > weights[k]) {
                float tv = weights[k];
                int ti = ids[k];
                weights[k] = v;
                ids[k] = id;
                v = tv;
                id = ti;
            }
        }
    }
    weights /= max(weights.x + weights.y + weights.z + weights.w, 1e-4);
}

// Height-based blend: where layers meet, the one whose height map is higher
// shows, so grass grows into the cracks of rock instead of fading over it.
vec4 heightBlend(vec4 weights, vec4 heights) {
    const float depth = 0.2;
    vec4 h = heights + weights;
    float top = max(max(h.x, h.y), max(h.z, h.w)) - depth;
    vec4 b = max(h - top, 0.0) * step(0.001, weights);
    return b / max(b.x + b.y + b.z + b.w, 1e-4);
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

// Tiling without repetition (after Inigo Quilez, "Texture Repetition",
// technique 3): a smooth noise picks one of many placements of the texture
// across the ground, each shifted and (for layers that allow it) turned by
// its own random angle, so neither a grid of copies nor the same strands
// in the same direction show. Where the noise moves from one placement to
// the next, both are read and the higher of the two by its height map
// shows. Gradients are given (`dx`, `dy`) so the jumps do not upset the
// mip level.
const float NO_TILE_SCALE = 0.5; // noise cells per tile
const float NO_TILE_STEPS = 12.0; // placements across the noise's range

struct Texels {
    vec3 albedo;
    vec3 surface;
    // Unpacked, in the frame of the unturned texture.
    vec3 normal;
};

// The texels at `uv` read through placement `i`: shifted and turned.
Texels fetchPlaced(int layer, vec2 uv, vec2 dx, vec2 dy, float i) {
    float h1 = hash12(vec2(i, float(layer) * 7.31 + 1.7));
    float h2 = hash12(vec2(i + 19.1, float(layer) * 3.17));
    float h3 = hash12(vec2(i * 1.37 + 5.3, 11.0));
    float angle = 6.2831853 * (h1 * 2.0 - 1.0) * turn[layer];
    float c = cos(angle), s = sin(angle);
    mat2 turn = mat2(c, s, -s, c);
    vec3 at = vec3(turn * uv + vec2(h2, h3) * 7.0, layer);
    vec2 tdx = turn * dx, tdy = turn * dy;
    vec3 n = unpackNormal(textureGrad(terrainNormal, at, tdx, tdy));
    // The normal's slope turned back into the ground's frame.
    n.xy = transpose(turn) * n.xy;
    return Texels(textureGrad(terrainAlbedo, at, tdx, tdy).rgb,
                  textureGrad(terrainSurface, at, tdx, tdy).rgb,
                  n);
}

Texels sampleNoTile(int layer, vec2 uv, vec2 dx, vec2 dy) {
    float l = valueNoise(uv * NO_TILE_SCALE + float(layer) * 13.7) * NO_TILE_STEPS;
    float i = floor(l);
    float f = fract(l);
    // Away from a change only one is needed.
    if (f < 0.35) {
        return fetchPlaced(layer, uv, dx, dy, i);
    }
    if (f > 0.65) {
        return fetchPlaced(layer, uv, dx, dy, i + 1.0);
    }
    Texels a = fetchPlaced(layer, uv, dx, dy, i);
    Texels b = fetchPlaced(layer, uv, dx, dy, i + 1.0);
    // The height maps move the change by at most 0.07: it stays within
    // the band where both are read.
    float w = smoothstep(0.42, 0.58, f + 0.07 * (b.surface.b - a.surface.b));
    return Texels(mix(a.albedo, b.albedo, w), mix(a.surface, b.surface, w), mix(a.normal, b.normal, w));
}

// Triplanar sample of one layer with whiteout-blended normals
// (Ben Golus, "Normal Mapping for a Triplanar Shader"), `s` tiles per metre,
// each projection without repetition (`sampleNoTile`). `dpdx`, `dpdy`: how
// the world position changes across the pixel. `parallax` (0..1) shifts the
// top projection by the height map toward the viewer `V`, so the ground
// looks deeper close up.
LayerSample sampleLayer(int layer, vec3 p, vec3 Ng, vec3 blend, float s, vec3 V, float parallax,
                        vec3 dpdx, vec3 dpdy) {
    vec3 axisSign = vec3(Ng.x < 0.0 ? -1.0 : 1.0, Ng.y < 0.0 ? -1.0 : 1.0, Ng.z < 0.0 ? -1.0 : 1.0);
    vec3 albedo = vec3(0.0);
    vec3 surface = vec3(0.0);
    vec3 nX = vec3(0.0), nY = vec3(0.0), nZ = vec3(0.0);
    if (blend.x > 0.01) {
        vec2 uv = vec2(p.z * axisSign.x, p.y) * s;
        vec2 dx = vec2(dpdx.z * axisSign.x, dpdx.y) * s;
        vec2 dy = vec2(dpdy.z * axisSign.x, dpdy.y) * s;
        Texels t = sampleNoTile(layer, uv, dx, dy);
        albedo += t.albedo * blend.x;
        surface += t.surface * blend.x;
        vec3 n = t.normal;
        n.x *= axisSign.x;
        n.z *= axisSign.x;
        nX = vec3(n.xy + Ng.zy, abs(n.z) * Ng.x);
    }
    if (blend.y > 0.01) {
        vec2 uv = vec2(p.x * axisSign.y, p.z) * s;
        vec2 dx = vec2(dpdx.x * axisSign.y, dpdx.z) * s;
        vec2 dy = vec2(dpdy.x * axisSign.y, dpdy.z) * s;
        if (parallax > 0.0 && Ng.y > 0.0) {
            float height = sampleNoTile(layer, uv, dx, dy).surface.b;
            // Offset limiting: not divided by the view's height, so a
            // grazing view does not drag the texture into streaks.
            vec2 toEye = vec2(V.x * axisSign.y, V.z);
            uv += toEye * (height - 0.5) * heightDepth[layer] * PARALLAX_STRENGTH * s * parallax;
        }
        Texels t = sampleNoTile(layer, uv, dx, dy);
        albedo += t.albedo * blend.y;
        surface += t.surface * blend.y;
        vec3 n = t.normal;
        n.x *= axisSign.y;
        n.z *= axisSign.y;
        nY = vec3(n.xy + Ng.xz, abs(n.z) * Ng.y);
    }
    if (blend.z > 0.01) {
        vec2 uv = vec2(-p.x * axisSign.z, p.y) * s;
        vec2 dx = vec2(-dpdx.x * axisSign.z, dpdx.y) * s;
        vec2 dy = vec2(-dpdy.x * axisSign.z, dpdy.y) * s;
        Texels t = sampleNoTile(layer, uv, dx, dy);
        albedo += t.albedo * blend.z;
        surface += t.surface * blend.z;
        vec3 n = t.normal;
        n.x *= -axisSign.z;
        n.z *= axisSign.z;
        nZ = vec3(n.xy + Ng.xy, abs(n.z) * Ng.z);
    }
    float total = max(blend.x * step(0.01, blend.x) + blend.y * step(0.01, blend.y)
                      + blend.z * step(0.01, blend.z), 1e-4);
    vec3 n = normalize(nX.zyx * blend.x + nY.xzy * blend.y + nZ.xyz * blend.z);
    return LayerSample(albedo / total, n, surface / total);
}

// The ground without textures (display space): the biomes' colour, the
// sea floor darkening with depth, rock on cliffs.
vec3 paletteColor(float h, float slope, float n) {
    vec3 seabedDeep = vec3(0.32, 0.22, 0.12);
    vec3 rock       = vec3(0.46, 0.43, 0.39);
    vec3 darkRock   = vec3(0.36, 0.34, 0.32);
    vec3 color = mix(seabedDeep, GroundColor, smoothstep(-40.0, -3.0, h));
    color = mix(color, MUD_COLOR, Paint.w * 0.5);
    color = mix(color, GRAVEL_COLOR, Paint.z * 0.7);
    color = mix(color, TRODDEN_COLOR, Paint.y * 0.55);
    color = mix(color, ROAD_COLOR, Paint.x);
    // Patches, so a field is not one flat colour.
    color *= mix(0.9, 1.08, valueNoise(WorldPos.xz * 0.035) * 0.6 + n * 0.2 + 0.2);
    float rockBlend = smoothstep(0.57, 0.80, slope) * smoothstep(0.0, 8.0, h);
    // Rock is darker where it is cold (and high).
    vec3 rockColor = mix(darkRock, rock, smoothstep(-6.0, 12.0, Climate.x));
    return mix(color, rockColor, rockBlend);
}

// Haze: none up to `HAZE_START` metres, then thickening over
// `HAZE_DISTANCE`, toward the sky's light times `HAZE_BRIGHTNESS`.
const float HAZE_START = 600.0;
const float HAZE_DISTANCE = 9000.0;
const float HAZE_BRIGHTNESS = 1.0;

// Height of the water's surface (`dual_contouring::WATER_LEVEL`).
const float WATER_LINE = 0.0;
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
    if (isHorizon == 1 && length(WorldPos.xz - horizonSink.xy) < horizonHole) {
        discard;
    }
    snowGradients(WorldPos);
    // Here, where every pixel of a quad still runs: the layers are read
    // with these gradients further down, in branches.
    vec3 dpdx = dFdx(WorldPos);
    vec3 dpdy = dFdy(WorldPos);
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
        float weights[LAYER_COUNT];
        layerWeights(h, slope, n, Climate, weights);

        vec3 V = normalize(cameraPosition - WorldPos);
        float viewDistance = length(cameraPosition - WorldPos);
        float parallax = 1.0 - smoothstep(PARALLAX_FADE_START, PARALLAX_FADE_END, viewDistance);
        // Far away the coarse sampling takes over half of the colour.
        float far = smoothstep(15.0, 80.0, viewDistance) * 0.5;
        float macroBumps = mix(0.15, 1.0, smoothstep(8.0, 60.0, viewDistance));

        ivec4 ids;
        vec4 layerWeight;
        strongest(weights, ids, layerWeight);
        LayerSample samples[MAX_SAMPLED];
        vec4 heights = vec4(0.0);
        for (int k = 0; k < MAX_SAMPLED; k++) {
            int i = ids[k];
            if (i < 0) {
                samples[k] = LayerSample(vec3(0.0), Ng, vec3(0.0));
                continue;
            }
            float s = 1.0 / tileSize[i];
            LayerSample fine = sampleLayer(i, WorldPos, Ng, blend, s, V, parallax, dpdx, dpdy);
            LayerSample macro = sampleLayer(i, WorldPos, Ng, blend, s / MACRO_SCALE, V, 0.0, dpdx, dpdy);
            samples[k].albedo = mix(fine.albedo, macro.albedo, far);
            // Add both samplings' bumps to the surface normal. The coarse
            // one is the same scan six times larger: up close its bumps are
            // wide, soft waves, so they come in with distance.
            samples[k].normal = normalize(Ng + (fine.normal - Ng) + (macro.normal - Ng) * macroBumps);
            samples[k].surface = fine.surface;
            heights[k] = fine.surface.b;
        }
        layerWeight = heightBlend(layerWeight, heights);
        // The grass takes on the biomes' colour (relative to a mid green,
        // so the texture's own detail stays). Red and blue never outgrow
        // green by much: a brown ground colour would turn grass purple.
        vec3 tint = clamp(GroundColor / vec3(0.24, 0.50, 0.13), vec3(0.4), vec3(2.5));
        tint.rb = min(tint.rb, vec2(tint.g * 1.4));
        vec3 grassTint = pow(mix(vec3(1.0), tint, 0.7), vec3(2.2));

        albedo = vec3(0.0);
        vec3 normal = vec3(0.0);
        vec3 surface = vec3(0.0);
        for (int k = 0; k < MAX_SAMPLED; k++) {
            if (ids[k] < 0) {
                continue;
            }
            albedo += samples[k].albedo * mix(vec3(1.0), grassTint, tinted[ids[k]]) * layerWeight[k];
            normal += samples[k].normal * layerWeight[k];
            surface += samples[k].surface * layerWeight[k];
        }
        N = normalize(normal);
        roughness = surface.r;
        // The scans' occlusion is for close-ups; at terrain distances its
        // full strength darkens whole fields.
        ao = mix(1.0, surface.g, 0.5);

        // Large-scale brightness variation hides the tiling from afar.
        albedo *= mix(0.8, 1.15, valueNoise(WorldPos.xz * 0.025));
    } else {
        albedo = pow(paletteColor(h, slope, n), vec3(2.2));
    }
    // Darker toward the deep sea floor.
    albedo *= mix(0.45, 1.0, smoothstep(-30.0, 0.0, h));
    // Open sea on the horizon (see `terrain::horizon::SEA_MARK`).
    bool sea = Climate.y > 1.5;
    if (sea) {
        // Close to the colour the near water shows from afar.
        albedo = pow(vec3(0.07, 0.25, 0.45), vec3(2.2));
        roughness = 0.35;
        N = Ng;
        ao = 1.0;
    }

    // Lasting snow where the mean temperature is well below freezing; it
    // slides off steep rock.
    float snow = sea ? 0.0 : 1.0 - smoothstep(-6.0, -3.5, Climate.x + n * 1.5);
    snow *= 1.0 - 0.75 * smoothstep(0.62, 0.85, slope);
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
    // Far away the air between hazes the ground into the sky's colour.
    vec3 toPoint = WorldPos - cameraPosition;
    float far = length(toPoint);
    vec3 along = normalize(vec3(toPoint.x, max(toPoint.y, 0.0) * 0.3, toPoint.z));
    float haze = 1.0 - exp(-max(far - HAZE_START, 0.0) / HAZE_DISTANCE);
    color = mix(color, skyLight(along) * HAZE_BRIGHTNESS, haze);
    FragColor = vec4(toDisplay(color), 1.0);
}
