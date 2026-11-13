// Shared by the cloud pass (`clouds.glsl`) and the sky (`fragment.glsl`):
// camera, sun and cloud uniforms, noise, and the volumetric cloud slab.
// Inserted after the `#version` line.

// Camera basis in world space — used to reconstruct the view ray for the
// current pixel without depending on a (potentially singular) inverse matrix.
uniform vec3 camForward;
uniform vec3 camRight;
uniform vec3 camUp;
uniform float tanHalfFovX;
uniform float tanHalfFovY;

uniform vec3 sunDirection; // normalized, world-space, points from origin toward the sun
uniform float timeOfDay;   // 0..1
uniform vec3 camPos;
uniform float uTime;       // monotonic seconds for wind drift
uniform float uCoverage;   // 0..1 — fraction of sky covered in clouds

float hash13(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return fract((p.x + p.y) * p.z);
}

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Value noise in 3D, hermite-smoothed. Using 3D noise with time on the z
// axis lets the cloud pattern smoothly *morph* (puffs build and dissipate)
// instead of just sliding rigidly with the wind.
float vnoise3(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    vec3 u = f * f * (3.0 - 2.0 * f);
    float n000 = hash13(i);
    float n100 = hash13(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash13(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash13(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash13(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash13(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash13(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash13(i + vec3(1.0, 1.0, 1.0));
    float nx00 = mix(n000, n100, u.x);
    float nx10 = mix(n010, n110, u.x);
    float nx01 = mix(n001, n101, u.x);
    float nx11 = mix(n011, n111, u.x);
    float nxy0 = mix(nx00, nx10, u.y);
    float nxy1 = mix(nx01, nx11, u.y);
    return mix(nxy0, nxy1, u.z);
}

// fBm where each octave evolves at a different temporal rate. Higher
// octaves (detail) churn faster than the large-scale shape, which mimics
// real clouds: overall structures persist for minutes while fine wisps
// shift in seconds.
float fbm(vec2 p, float t) {
    float s = 0.0;
    float a = 0.5;
    mat2 R = mat2(0.80, 0.60, -0.60, 0.80);
    float tz = t;
    for (int i = 0; i < 5; i++) {
        s += a * vnoise3(vec3(p, tz));
        p = R * p * 2.03;
        a *= 0.5;
        tz = tz * 1.7 + 13.0;  // decorrelate octaves and speed up detail
    }
    return s;
}

// ── Volumetric cloud slab ────────────────────────────────────────────
// Clouds live between CLOUD_BOTTOM and CLOUD_TOP (world-space metres above
// the camera). The renderer raymarches through this slab, accumulating
// density and lighting via Beer–Lambert transmittance.
const float CLOUD_BOTTOM = 350.0;
const float CLOUD_TOP    = 850.0;
const int   MARCH_STEPS  = 48;
const int   SUN_STEPS    = 4;
const float SUN_STEP_LEN = 35.0;

// Cheap 3D fbm specialised for cloud density. Built from the same 3D value
// noise as `fbm`, but uses true 3D rotation of the sample point so detail
// varies with altitude (giving towering cumulus shape instead of an
// extruded 2D pattern).
float cloudFbm(vec3 p, int octaves) {
    float s = 0.0;
    float a = 0.5;
    float n = 0.0;
    mat2 R = mat2(0.80, 0.60, -0.60, 0.80);
    for (int i = 0; i < 6; i++) {
        if (i >= octaves) break;
        s += a * vnoise3(p);
        n += a;
        // Rotate xz and scale; offset z so octaves don't stack at same plane.
        p.xz = R * p.xz * 2.03;
        p.y  = p.y * 2.03 + 11.0;
        a *= 0.5;
    }
    return s / max(n, 1e-4);
}

// Returns cloud density at world-space point `p` for the current time.
// `cheap=true` uses fewer octaves — used during sun-lighting sub-march
// where the inner detail doesn't visibly affect the result.
float cloudDensity(vec3 p, bool cheap, float lod) {
    float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
    if (h < 0.0 || h > 1.0) return 0.0;

    // Wind drift — slowly translate the whole field horizontally. (No
    // y-axis time offset: a monotonic y-shift in noise space reads as
    // clouds visibly falling, which is the wrong cue.)
    vec3 wind = vec3(0.9, 0.0, 0.3) * uTime * 0.7;
    vec3 q = (p - wind) * 0.0018;

    // ── Large-scale weather modulation ──────────────────────────────────
    // Very-low-freq noise that varies total density across the sky. Stays
    // present at full coverage so an overcast sky shows thicker and
    // thinner regions instead of one uniform plane.
    float weather   = cloudFbm(q * 0.30 + 3.1, 2);
    float weatherMod = mix(0.35, 1.0, smoothstep(0.30, 0.85, weather));

    // ── Vertical envelope with noisy bottom ────────────────────────────
    // Modulating the bottom edge by mid-freq noise creates dangling tufts
    // (mammatus-style) so the underside isn't a flat plane.
    float bottomNoise = cloudFbm(q * 1.6 + 31.0, 2);
    float bottomEdge  = smoothstep(0.0, mix(0.08, 0.32, bottomNoise), h);
    float topEdge     = smoothstep(1.0, 0.55, h);
    float vshape      = bottomEdge * topEdge;

    // Distance LOD — drop noise octaves hard once the sample is too far
    // away for the detail to span more than a pixel. Stops distant
    // clouds from aliasing into per-pixel sparkle.
    int baseOct  = cheap ? 3 : 5;
    int detailOct = cheap ? 2 : 3;
    baseOct   = max(1, baseOct   - int(lod * 4.0));
    detailOct = max(1, detailOct - int(lod * 3.0));

    float base = cloudFbm(q, baseOct);

    // Coverage threshold — 0 pushes cutoff above the noise max (clear),
    // 1 pushes it below the min (overcast).
    float cov = clamp(uCoverage, 0.0, 1.0);
    float cutoff = mix(1.05, -0.10, cov);
    float d = max(base - cutoff, 0.0);

    // ── Detail erosion ──────────────────────────────────────────────────
    // Multiplicative high-freq carving so even fully-overcast regions
    // show billowy texture instead of a smooth fill. Strength fades with
    // distance so we don't carve sub-pixel features.
    float erode = cloudFbm(q * 4.5 + 17.0, detailOct);
    float erodeAmt = mix(0.30, 0.90, lod);
    d *= mix(erodeAmt, 1.0, erode);

    return d * vshape * weatherMod * 5.0;
}

// Raymarch the cloud slab along view ray `rd` starting at camera `ro`.
// Returns scattered colour (rgb) and alpha (1 - transmittance).
vec4 sampleClouds(vec3 dir, float dayFactor, vec3 baseSky) {
    // Below horizon — slab is above camera, ray never enters.
    if (dir.y < 0.02) return vec4(0.0);

    // Slab entry/exit. Camera is assumed below the slab in this scene.
    float tEnter = (CLOUD_BOTTOM - camPos.y) / dir.y;
    float tExit  = (CLOUD_TOP    - camPos.y) / dir.y;
    if (camPos.y > CLOUD_BOTTOM) tEnter = max(0.0, tEnter);
    if (camPos.y > CLOUD_TOP)    return vec4(0.0);
    if (tExit  <= 0.0) return vec4(0.0);
    tEnter = max(tEnter, 0.0);

    // Cap total march distance so grazing rays don't blow out step count.
    float maxDist = 9000.0;
    float marchLen = min(tExit - tEnter, maxDist);
    float stepLen  = marchLen / float(MARCH_STEPS);

    // Per-pixel ray offset — dithers banding from low step count. The
    // hash is intentionally *not* time-varying: a frame-to-frame stable
    // dither hides ringing without flickering, while a time-varying one
    // requires TAA to average out and looks like crawling noise here.
    // Attenuate the jitter amplitude for grazing rays where each step
    // covers a huge swath of cloud and the offset creates per-pixel
    // density variance instead of just hiding banding.
    float jitter = hash12(gl_FragCoord.xy);
    float startLod = clamp(tEnter / 4000.0, 0.0, 1.0);
    float jitterScale = mix(1.0, 0.15, startLod);
    vec3 pos = camPos + dir * (tEnter + stepLen * jitter * jitterScale);

    // Pre-compute lighting parameters.
    vec3 sunCol = mix(vec3(1.0, 0.55, 0.30), vec3(1.0, 0.97, 0.90),
                      smoothstep(0.0, 0.25, sunDirection.y));
    vec3 ambient = mix(vec3(0.20, 0.22, 0.28), vec3(0.78, 0.82, 0.90), dayFactor);
    sunCol  *= mix(vec3(0.08, 0.10, 0.16), vec3(1.0), dayFactor);
    ambient *= mix(vec3(0.30, 0.35, 0.45), vec3(1.0), dayFactor);

    // Henyey-Greenstein-ish phase: strong forward scatter creates the
    // "silver lining" when looking near the sun.
    float vDotL = max(dot(dir, sunDirection), 0.0);
    float phase = 0.5 + 0.5 * vDotL
                + 0.8 * pow(vDotL, 8.0)
                + 0.4 * pow(vDotL, 32.0);

    float transmittance = 1.0;
    vec3  scatter = vec3(0.0);

    // Reciprocal of the far distance — used to compute per-sample LOD.
    // Smaller value pushes LOD into effect sooner, smoothing distant
    // samples by dropping fbm octaves earlier.
    float invFar = 1.0 / 4500.0;

    for (int i = 0; i < MARCH_STEPS; i++) {
        float distAlong = float(i) * stepLen;
        float lod = clamp(distAlong * invFar, 0.0, 1.0);
        float d = cloudDensity(pos, false, lod);
        if (d > 0.001) {
            // Sun visibility — short march toward the sun summing density.
            float sunOptical = 0.0;
            vec3  lp = pos;
            float lStep = SUN_STEP_LEN;
            for (int j = 0; j < SUN_STEPS; j++) {
                lp += sunDirection * lStep;
                sunOptical += cloudDensity(lp, true, lod) * lStep;
                lStep *= 1.6;  // exponentially-growing steps cover more sky cheaply
            }
            float sunT = exp(-sunOptical * 0.85);

            // Sky occlusion — a single probe well above the sample. If the
            // cloud is small the probe lands in clear air (no darkening);
            // if it's a tall body the probe is still inside cloud and the
            // underside darkens. Avoids the saturation problems of a
            // multi-step optical-depth integral with this density scale.
            float dAbove = cloudDensity(pos + vec3(0.0, 250.0, 0.0), true, lod);
            float occ    = clamp(dAbove * 0.30, 0.0, 0.55);
            vec3  ambHere = ambient * (1.0 - occ);

            // Mild powder term — only attenuates the very thinnest wisps,
            // leaving most of the cloud's sunlit response intact.
            float powder = mix(0.65, 1.0, 1.0 - exp(-d * 4.0));

            vec3 lit = sunCol * sunT * phase * powder + ambHere;

            // Beer–Lambert absorption for this segment.
            float segOptical = d * stepLen;
            float segAlpha   = 1.0 - exp(-segOptical);

            scatter      += transmittance * lit * segAlpha;
            transmittance *= 1.0 - segAlpha;

            if (transmittance < 0.01) break;
        }
        pos += dir * stepLen;
    }

    float alpha = 1.0 - transmittance;
    // Horizon fade — distant grazing rays dissolve into haze. A wide
    // fade range removes the band of per-pixel sparkle right above the
    // horizon where cloud detail is sub-pixel.
    alpha *= smoothstep(0.02, 0.35, dir.y);
    return vec4(scatter, alpha);
}


// View ray of the pixel at `ndc` (-1..1), from the camera basis.
vec3 viewRay(vec2 ndc) {
    return normalize(camForward + camRight * (ndc.x * tanHalfFovX) + camUp * (ndc.y * tanHalfFovY));
}

float dayFactorOf(vec3 sun) {
    return smoothstep(-0.10, 0.20, clamp(sun.y, -1.0, 1.0));
}
