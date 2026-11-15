// The weather's maps (`renderer/weather`) and what is read from them:
// open sky, the ground's height and the trails in the snow. Shared by
// `weather.glsl` and the snow layer's vertex shader, so nothing here may
// need a fragment shader. Needs `isShadowPass` declared before it.

uniform float snowCover;     // how much of the open ground snow covers, 0..1
uniform float snowDepth;     // metres

// Depth of the scene seen from straight above: what is under a roof, a
// tree or an overhang stays dry.
uniform int useExposure;
uniform sampler2D exposureMap;
uniform mat4 exposureMatrix;
uniform float exposureRange; // metres the depth 0..1 spans
// The same view of the terrain alone: how high things are above the ground.
uniform sampler2D groundMap;

// Trails pressed into the snow: r how far pressed down (0..1), g the snow
// pushed up around a print. `trailArea` is (x, z) of its corner and size.
uniform int useSnowTrails;
uniform sampler2D snowTrailMap;
uniform vec3 trailArea;
// Metres around the camera (in x and in z) where the snow layer
// (`snow_layer.rs`) lies in full and shows the trails; 0 without it.
uniform float snowLayerReach;

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

// Bilinear blend of four texels from `textureGather` (its order: (0,1),
// (1,1), (1,0), (0,0)), `f` the spot's place between them.
float texelBlend(vec4 texels, vec2 f) {
    return mix(mix(texels.w, texels.z, f.x), mix(texels.x, texels.y, f.x), f.y);
}

// Share of the sky above `worldPos` that is open, 0..1, in any pass. The
// point is moved off the surface along its normal first, so walls and
// cliffs (which see their own top from above) count as open. Only what is well above the
// point shelters it: grass and bumps of the ground do not. The map is
// filtered (each texel's shelter blended bilinearly, over a few texels),
// so the edge of what is sheltered is soft instead of blocky.
float skyExposureMap(vec3 worldPos, vec3 Ng) {
    if (useExposure == 0) {
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
    // Ground on a slope is higher a texel or two uphill: allow for it.
    float slope = sqrt(max(1.0 - Ng.y * Ng.y, 0.0)) / max(abs(Ng.y), 0.2);
    float texelMetres = 1.0 / (size.x * exposureMatrix[0][0] * 0.5);
    // Metres something must be above the point to start and to fully
    // shelter it.
    float low = 0.5 + texelMetres * 3.0 * slope;
    float high = low + 1.0;
    float sheltered = 0.0;
    for (int j = -1; j <= 1; ++j) {
        for (int i = -1; i <= 1; ++i) {
            vec2 uv = q.xy + vec2(i, j) * 1.5 / size;
            vec2 f = fract(uv * size - 0.5);
            vec4 s = smoothstep(vec4(low), vec4(high),
                                (q.z - textureGather(exposureMap, uv, 0)) * exposureRange);
            sheltered += texelBlend(s, f);
        }
    }
    return mix(1.0 - sheltered / 9.0, 1.0, outside);
}

// `skyExposureMap`, but everything counts as open while drawing shadows
// or other depth: what is wet or snowed on does not change those.
float skyExposure(vec3 worldPos, vec3 Ng) {
    if (isShadowPass == 1) {
        return 1.0;
    }
    return skyExposureMap(worldPos, Ng);
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
    // The full map, never a mipmap picked from screen derivatives: those
    // are undefined after a discard or in a loop, and flicker.
    return textureLod(snowTrailMap, uv, 0.0).rg * fade;
}

// `snowTrail`, blurred over about `metres` (from the trail map's mipmaps):
// what a grid that coarse can show of it without sharp facets.
vec2 snowTrailBlurred(vec2 p, float metres) {
    if (useSnowTrails == 0) {
        return vec2(0.0);
    }
    vec2 uv = (p - trailArea.xy) / trailArea.z;
    vec2 edge = abs(uv * 2.0 - 1.0);
    float fade = 1.0 - smoothstep(0.9, 1.0, max(edge.x, edge.y));
    if (fade <= 0.0) {
        return vec2(0.0);
    }
    float texel = trailArea.z / float(textureSize(snowTrailMap, 0).x);
    float lod = max(log2(metres / texel), 0.0);
    return textureLod(snowTrailMap, uv, lod).rg * fade;
}

// `snowDent` for a grid with `metres` between its points.
float snowDentBlurred(vec2 p, float metres) {
    vec2 trail = snowTrailBlurred(p, metres);
    return snowDepth * (0.9 * trail.r - 0.35 * trail.g);
}

// How far the snow's surface at `p` lies below its untouched top, in
// metres: what feet pressed down, less what they pushed up around.
float snowDent(vec2 p) {
    vec2 trail = snowTrail(p);
    return snowDepth * (0.9 * trail.r - 0.35 * trail.g);
}

// Height of the ground (the terrain alone) under `p` (x, z), from the
// ground map. False where the map does not reach or has no terrain.
bool groundHeight(vec2 p, out float height) {
    height = 0.0;
    if (useExposure == 0) {
        return false;
    }
    vec4 clip = exposureMatrix * vec4(p.x, 0.0, p.y, 1.0);
    vec3 q = clip.xyz / clip.w * 0.5 + 0.5;
    if (any(lessThan(q.xy, vec2(0.0))) || any(greaterThan(q.xy, vec2(1.0)))) {
        return false;
    }
    vec2 size = vec2(textureSize(groundMap, 0));
    vec4 depth = textureGather(groundMap, q.xy, 0);
    // Cleared to the far end: no terrain there.
    if (max(max(depth.x, depth.y), max(depth.z, depth.w)) >= 1.0) {
        return false;
    }
    // `q.z` is the depth of height 0; depth grows downward, linearly.
    height = (q.z - texelBlend(depth, fract(q.xy * size - 0.5))) * exposureRange;
    return true;
}
