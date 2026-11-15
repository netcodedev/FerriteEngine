// The low cloud layer as a density field: stratus, stratocumulus, cumulus
// and cumulonimbus, chosen per region from the scene's cloud type. Shared
// by the sky (raymarches it) and the terrain (cloud shadows), so a shadow
// on the ground always belongs to a cloud in the sky.
//
// Whoever includes this declares the uniforms first:
//   uniform float uTime;         // seconds, for the wind
//   uniform float cloudCoverage; // 0..1
//   uniform float cloudType;     // 0 stratus .. 1 cumulonimbus

// The layer spans these heights (metres above the world origin). Only a
// cumulonimbus reaches the top; flatter types stay near the bottom.
const float CLOUD_BOTTOM = 350.0;
const float CLOUD_TOP    = 1650.0;

// How far the cloud type drifts from the scene's setting across the sky,
// so a cumulus sky still has a few flatter and taller clouds in it.
const float CLOUD_TYPE_SPREAD = 0.22;

// The value noise's lattice: one random value per point, repeating every
// CLOUD_NOISE_SIZE points (`sky::noise`). Bound by the engine.
const float CLOUD_NOISE_SIZE = 128.0;
layout(binding = 17) uniform sampler3D cloudNoise;

// Value noise in 3D, hermite-smoothed: the position in the cell is bent by
// the hermite curve, then the lattice is read with one trilinear lookup
// (which blends the cell's eight corners by it).
float cloudNoise3(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    vec3 u = f * f * (3.0 - 2.0 * f);
    return textureLod(cloudNoise, (i + u + 0.5) / CLOUD_NOISE_SIZE, 0.0).r;
}

// 3D fbm, normalised to 0..1. Rotates xz between octaves and offsets y so
// detail varies with altitude (towering shapes instead of an extruded 2D
// pattern).
float cloudFbm(vec3 p, int octaves) {
    float s = 0.0;
    float a = 0.5;
    float n = 0.0;
    mat2 R = mat2(0.80, 0.60, -0.60, 0.80);
    for (int i = 0; i < 6; i++) {
        if (i >= octaves) break;
        s += a * cloudNoise3(p);
        n += a;
        p.xz = R * p.xz * 2.03;
        p.y  = p.y * 2.03 + 11.0;
        a *= 0.5;
    }
    return s / max(n, 1e-4);
}

// Wind drift of the whole field. (No vertical part: a shift along y in
// noise space reads as clouds falling.)
vec3 cloudWind() {
    return vec3(0.9, 0.0, 0.3) * uTime * 0.7;
}

// Top of the layer, 0..1 of its height, for a cloud type: stratus is a
// thin sheet, cumulus a few hundred metres, cumulonimbus the whole layer.
float cloudTopFor(float type) {
    return 0.09 + 0.91 * type * type;
}

// Highest the layer reaches anywhere for the scene's cloud type, in world
// metres. The raymarch stops there.
float cloudLayerTop() {
    float type = clamp(cloudType + CLOUD_TYPE_SPREAD, 0.0, 1.0);
    return CLOUD_BOTTOM + (CLOUD_TOP - CLOUD_BOTTOM) * cloudTopFor(type);
}

// Cloud density at world point `p`. `cheap` uses fewer octaves (light
// marches, shadows), `lod` (0 near .. 1 far) drops detail with distance so
// far clouds do not alias.
float cloudDensity(vec3 p, bool cheap, float lod) {
    float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
    if (h < 0.0 || h > 1.0) return 0.0;

    vec3 q = (p - cloudWind()) * 0.0018;

    // ── Weather: how much cloud and which type, per region ──────────────
    // Sampled at the layer's base, so a cloud keeps one type all the way up.
    vec3 qFlat = vec3(q.x, CLOUD_BOTTOM * 0.0018, q.z);
    float weather    = cloudFbm(qFlat * 0.30 + 3.1, 2);
    float weatherMod = mix(0.35, 1.0, smoothstep(0.30, 0.85, weather));
    float typeNoise  = cloudFbm(qFlat * 0.21 + 47.0, 2);
    float type = clamp(cloudType + (typeNoise - 0.5) * 2.0 * CLOUD_TYPE_SPREAD, 0.0, 1.0);

    // ── Storm cells ─────────────────────────────────────────────────────
    // A cumulonimbus sky is mostly cumulus with a few big storm cells, kilometres
    // apart. Inside a cell the clouds reach higher the closer they are to its
    // core, which gives one tall dome instead of a field of equal towers.
    float storm  = smoothstep(0.75, 1.0, type);
    float cellN  = cloudFbm(qFlat * 0.11 + 91.0, 3);
    float cell   = clamp((cellN - 0.50) / 0.28, 0.0, 1.0) * storm;
    float heaped = min(type, 0.70);

    float top = mix(cloudTopFor(heaped), 1.0, cell * (2.0 - cell));
    float hn = h / top; // 0 at the base, 1 at this cloud's top

    // The anvil: a flat, smooth sheet at the top of the layer, wider than
    // the tower under it, stretched downwind.
    float anvilD = 0.0;
    if (storm > 0.0) {
        vec3 qa = qFlat + vec3(-0.9, 0.0, -0.3) * smoothstep(0.80, 1.0, h) * 1.2;
        float wide = clamp((cloudFbm(qa * 0.11 + 91.0, 3) - 0.47) / 0.22, 0.0, 1.0) * storm;
        float sheet = smoothstep(0.80, 0.88, h) * (1.0 - smoothstep(0.95, 0.99, h));
        float edge = cloudFbm(q * 1.3 + 61.0, cheap ? 2 : 3);
        anvilD = max(wide * (0.55 + 0.45 * edge) - 0.35, 0.0) * sheet * 1.6;
    }
    if (hn > 1.0) return anvilD * weatherMod * 5.0;

    // Flat clouds spread wide: stretch the noise horizontally for stratus.
    // Storm towers stretch it vertically, so a puff carries on upward
    // instead of breaking into stacked blobs.
    float spread = mix(0.45, 1.0, smoothstep(0.0, 0.5, type));
    float stretch = mix(1.0, 0.55, cell);
    vec3 qs = vec3(q.x * spread, q.y * stretch, q.z * spread);

    // ── Vertical profile ────────────────────────────────────────────────
    // Cumulus have flat bases and round tops; stratus soft edges on both
    // sides. The noisy base gives dangling tufts instead of a flat plane.
    float bottomNoise = cloudFbm(q * 1.6 + 31.0, 2);
    float baseSoft    = mix(0.40, 0.10, smoothstep(0.0, 0.5, type)) * mix(1.0, 0.4, cell);
    float bottomEdge  = smoothstep(0.0, baseSoft * mix(0.5, 1.4, bottomNoise), hn);
    // Rounded tops: the threshold rises towards the top so a cloud narrows
    // into a dome.
    float narrowing = smoothstep(mix(0.55, 0.30, heaped), 1.0, hn);
    float topEdge = smoothstep(1.0, 0.90, hn);

    int baseOct   = cheap ? 3 : 5;
    int detailOct = cheap ? 2 : 3;
    baseOct   = max(1, baseOct   - int(lod * 4.0));
    detailOct = max(1, detailOct - int(lod * 3.0));

    float base = cloudFbm(qs, baseOct);

    // Coverage threshold: 0 pushes the cutoff above the noise (clear), 1
    // below it (overcast). A storm cell's core is solid whatever the
    // coverage.
    float cov = clamp(cloudCoverage, 0.0, 1.0);
    float cutoff = mix(1.05, -0.10, cov);
    cutoff += narrowing * mix(0.25, 0.55, heaped) * (1.0 - 0.5 * cell);
    cutoff -= 0.45 * cell;
    float d = max(base - cutoff, 0.0);

    // ── Detail erosion ──────────────────────────────────────────────────
    // Billowy carving for heaped clouds, much less of it for stratus, and
    // less with distance so sub-pixel features are not carved.
    float erode = cloudFbm(q * 4.5 + 17.0, detailOct);
    float erodeAmt = mix(mix(0.60, 0.25, smoothstep(0.1, 0.6, type)), 0.90, lod);
    d *= mix(erodeAmt, 1.0, erode);

    return max(d * bottomEdge * topEdge, anvilD) * weatherMod * 5.0;
}
