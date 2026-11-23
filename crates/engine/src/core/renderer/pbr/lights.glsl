// The scene's lights as the shaders see them (`light/light.rs`), the
// clusters that list which of them reach where (`light/clusters.rs`), and
// how their light falls off. Shared by the lit shaders (before
// `lighting.glsl`) and the ray traced lamp shadows
// (`raytracing/restir_*.glsl`).

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
                         // z shadow map layer (-1: none), w the lamp's
                         // radius when it casts shadows (-1: it does not)
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


// Whether light `l` casts shadows (a point or spot lamp with "shadows" on).
bool castsShadows(Light l) {
    return l.cone.w >= 0.0;
}
