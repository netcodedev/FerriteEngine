// ReSTIR for the lamps' shadows (`LampShadows` in `raytracing/lamps.rs`),
// after `surface.glsl` and `pbr/lights.glsl`: what a lamp would give a
// surface with nothing in the way, and the reservoirs that pick one lamp
// per pixel in proportion to it.
//
// A reservoir is kept in an RGBA16F texel: the lamp (index into `lights`,
// -1 for none), its weight W, how many candidates it stands for (M), and
// the surface's distance (clip w) it was made for.

// Light lamp `i` gives a surface at `P` (world) facing `N` with nothing in
// the way: its radiance faded by distance and cone, times N·L. Zero for
// lights whose shadows are not ray traced (no shadows, directional).
vec3 lampIrradiance(int i, vec3 P, vec3 N) {
    if (i < 0 || i >= lightCount) {
        return vec3(0.0);
    }
    Light l = lights[i];
    if (!castsShadows(l)) {
        return vec3(0.0);
    }
    int kind = int(l.directionKind.w + 0.5);
    if (kind == LIGHT_DIRECTIONAL) {
        return vec3(0.0);
    }
    vec3 toLight = l.positionRange.xyz - P;
    float d2 = dot(toLight, toLight);
    float range = l.positionRange.w;
    if (d2 >= range * range) {
        return vec3(0.0);
    }
    float d = sqrt(d2);
    vec3 L = toLight / max(d, 1e-4);
    float attenuation = rangeFade(d, range) / max(d2, 0.01);
    if (kind == LIGHT_SPOT) {
        attenuation *= spotCone(l, L);
    }
    return l.radianceBounce.rgb * attenuation * max(dot(N, L), 0.0);
}

// The target the reservoirs sample in proportion to.
float lampTarget(int i, vec3 P, vec3 N) {
    return dot(lampIrradiance(i, P, N), vec3(0.2126, 0.7152, 0.0722));
}

// Most a stored W may be (half floats end at 65504).
const float MAX_W = 60000.0;

struct Reservoir {
    int lamp;
    float target;   // p̂ of `lamp` at this pixel
    float weightSum;
    float count;    // M
};

Reservoir emptyReservoir() {
    return Reservoir(-1, 0.0, 0.0, 0.0);
}

// Take in a candidate `lamp` with target `target` and resampling weight
// `weight`, standing for `count` candidates.
void update(inout Reservoir r, int lamp, float target, float weight, float count, float u) {
    r.weightSum += weight;
    r.count += count;
    if (weight > 0.0 && u * r.weightSum < weight) {
        r.lamp = lamp;
        r.target = target;
    }
}

// The reservoir as stored, for a surface at clip w `distance`.
vec4 packReservoir(Reservoir r, float distance) {
    float W = r.target > 0.0 && r.count > 0.0 ? min(r.weightSum / (r.count * r.target), MAX_W) : 0.0;
    return vec4(W > 0.0 ? float(r.lamp) : -1.0, W, r.count, distance);
}

// Merge stored reservoir `s` (lamp, W, M) into `r`, its lamp's target
// looked at anew at this pixel's `P`, `N`; its M at most `maxCount`.
void merge(inout Reservoir r, vec4 s, vec3 P, vec3 N, float maxCount, float u) {
    float count = min(s.z, maxCount);
    if (!(count > 0.0)) {
        return;
    }
    int lamp = int(s.x + 0.5 * sign(s.x));
    float target = s.x >= 0.0 ? lampTarget(lamp, P, N) : 0.0;
    update(r, lamp, target, target * s.y * count, count, u);
}
