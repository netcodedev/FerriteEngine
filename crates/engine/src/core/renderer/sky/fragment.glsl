#version 460 core

in vec2 ndcPos;
out vec4 FragColor;

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

// Returns clouds layer color and alpha for the given view ray.
// Uses a flat plane at altitude H above the camera so clouds appear in a
// natural dome regardless of world scale. Two layers — dense low cumulus
// plus thin wispy cirrus — for a more believable sky.
vec4 sampleClouds(vec3 dir, float dayFactor, vec3 baseSky) {
    // Below horizon — no clouds.
    if (dir.y < 0.02) return vec4(0.0);

    vec3 wind = vec3(1.0, 0.0, 0.3);

    // Morph rates: how fast the noise pattern itself evolves (in addition
    // to translating with the wind). Slow enough that individual clouds
    // visibly persist for ~30s but clearly build and dissipate over time.
    float morphLow  = uTime * 0.030;
    float morphHigh = uTime * 0.018;

    // Slowly-oscillating coverage — weather "fronts" passing through.
    float coverageBase = 0.52;
    float coverage = coverageBase + 0.08 * sin(uTime * 0.015);

    // ── Low cumulus layer ─────────────────────────────────────────────
    float H1 = 380.0;
    float t1 = H1 / dir.y;
    vec3 p1 = camPos + dir * t1;
    vec2 uv1 = p1.xz * 0.0011 + wind.xz * uTime * 0.0009;
    float n1 = fbm(uv1, morphLow);
    // Detail break-up — subtract a higher-frequency layer so edges aren't
    // smooth blobs but have wispy detail.
    float d1 = fbm(uv1 * 3.7 + 5.2, morphLow * 2.5);
    n1 = n1 - 0.35 * d1;

    float c1 = smoothstep(coverage, coverage + 0.22, n1);
    // Distance fade — far clouds dissolve into haze near the horizon.
    float horiz1 = smoothstep(0.02, 0.18, dir.y);
    c1 *= horiz1;

    // ── High cirrus layer ─────────────────────────────────────────────
    float H2 = 900.0;
    float t2 = H2 / dir.y;
    vec3 p2 = camPos + dir * t2;
    vec2 uv2 = p2.xz * 0.0005 + wind.xz * uTime * 0.0004;
    // Cirrus: anisotropic stretched noise — sample with squashed Y to get
    // streak-like patterns instead of cotton balls.
    float n2 = fbm(vec2(uv2.x, uv2.y * 4.0) + 17.0, morphHigh);
    float c2 = smoothstep(0.55, 0.78, n2) * 0.45;
    c2 *= smoothstep(0.02, 0.25, dir.y);

    // ── Lighting ──────────────────────────────────────────────────────
    // Cheap self-shadow: sample density a step toward the sun and use Beer's
    // law for transmittance. Gives clouds a sun-facing bright side and a
    // shaded interior.
    vec2 sunUV = uv1 + vec2(sunDirection.x, sunDirection.z) * 0.03;
    float sn = fbm(sunUV, morphLow) - 0.35 * fbm(sunUV * 3.7 + 5.2, morphLow * 2.5);
    float sunDensity = max(0.0, sn - coverage);
    float transmit = exp(-3.5 * sunDensity);

    vec3 sunCol = mix(vec3(1.0, 0.55, 0.30), vec3(1.0, 0.97, 0.90),
                      smoothstep(0.0, 0.25, sunDirection.y));
    vec3 ambient = mix(vec3(0.20, 0.22, 0.28), vec3(0.78, 0.82, 0.90), dayFactor);

    // Forward-scatter silver lining when looking near the sun.
    float vDotL = max(dot(dir, sunDirection), 0.0);
    float forward = pow(vDotL, 8.0) * 0.6 + pow(vDotL, 32.0) * 0.4;

    vec3 lit = ambient * (0.5 + 0.5 * transmit)
             + sunCol * transmit * (0.6 + forward);

    // At night, clouds are dark silhouettes against the starfield.
    lit *= mix(vec3(0.08, 0.10, 0.16), vec3(1.0), dayFactor);

    // Compose: low layer over high layer.
    float a1 = clamp(c1, 0.0, 1.0);
    float a2 = clamp(c2 * (1.0 - a1), 0.0, 1.0);
    float alpha = a1 + a2;
    vec3 col = (lit * a1 + mix(baseSky, lit, 0.6) * a2);
    if (alpha > 0.0) col /= max(alpha, 1e-4);

    return vec4(col, alpha);
}

void main() {
    // Build the view ray straight from camera basis vectors. ndcPos is the
    // screen-space NDC coordinate of this fragment in [-1, 1].
    vec3 dir = normalize(
        camForward
        + camRight * (ndcPos.x * tanHalfFovX)
        + camUp    * (ndcPos.y * tanHalfFovY)
    );

    float sunAlt = clamp(sunDirection.y, -1.0, 1.0);
    float dayFactor = smoothstep(-0.10, 0.20, sunAlt);

    // ── Sky gradient (day vs night) ────────────────────────────────────
    vec3 zenithDay    = vec3(0.18, 0.42, 0.85);
    vec3 horizonDay   = vec3(0.72, 0.85, 1.00);
    vec3 zenithNight  = vec3(0.005, 0.010, 0.030);
    vec3 horizonNight = vec3(0.020, 0.030, 0.080);

    float up = clamp(dir.y, 0.0, 1.0);
    float vBlend = sqrt(up);
    vec3 dayCol   = mix(horizonDay,   zenithDay,   vBlend);
    vec3 nightCol = mix(horizonNight, zenithNight, vBlend);
    vec3 sky = mix(nightCol, dayCol, dayFactor);

    // ── Sunset/sunrise glow (only when sun is near horizon) ────────────
    float sunHorizonProx = 1.0 - smoothstep(0.0, 0.30, abs(sunAlt));
    if (sunHorizonProx > 0.001) {
        float horizonProx = exp(-dir.y * dir.y * 18.0);
        float horizLen = length(vec2(sunDirection.x, sunDirection.z));
        float hemi = 0.0;
        if (horizLen > 1e-4) {
            vec2 sunH = vec2(sunDirection.x, sunDirection.z) / horizLen;
            vec2 dirH = vec2(dir.x, dir.z);
            float dirLen = length(dirH);
            if (dirLen > 1e-4) {
                hemi = max(dot(dirH / dirLen, sunH), 0.0);
            }
        }
        float glow = pow(hemi, 4.0) * horizonProx * sunHorizonProx;
        sky = mix(sky, vec3(1.00, 0.45, 0.18), clamp(glow * 0.85, 0.0, 0.85));
    }

    // ── Sun disk + soft halo ───────────────────────────────────────────
    float sunDot = clamp(dot(dir, sunDirection), -1.0, 1.0);
    float sunVisible = smoothstep(-0.04, 0.05, sunAlt);
    float sunDisk = smoothstep(0.9994, 0.9998, sunDot);
    float sunHalo = pow(max(sunDot, 0.0), 180.0) * 0.55;
    sky += vec3(1.00, 0.95, 0.85) * sunDisk * sunVisible;
    sky += vec3(1.00, 0.80, 0.55) * sunHalo * sunVisible;

    // ── Clouds ─────────────────────────────────────────────────────────
    vec4 clouds = sampleClouds(dir, dayFactor, sky);
    sky = mix(sky, clouds.rgb, clouds.a);

    // ── Stars (no seams; 3D-hash over unit sphere) ─────────────────────
    // Use a separate, tighter envelope than dayFactor so stars only appear
    // once the sun is clearly below the horizon (no faint stars in dusk).
    float starNight = 1.0 - smoothstep(-0.18, -0.05, sunAlt);
    if (starNight > 0.001 && dir.y > 0.0) {
        // Lock stars to the celestial sphere that co-rotates with the sun:
        // sample in a frame rotated by the negative of the daily angle so a
        // given star sits at a fixed point on the rotating sky dome.
        float angle = timeOfDay * 6.2831853 - 1.5707963;
        float c = cos(-angle);
        float s = sin(-angle);
        vec3 starDir = vec3(c * dir.x - s * dir.y, s * dir.x + c * dir.y, dir.z);

        // Star grid sized so each star covers ~2–3 pixels (with a soft AA
        // edge) instead of being subpixel — anything sub-pixel flickers as
        // the camera moves because sample points slide across cells.
        vec3 scaled = starDir * 120.0;
        vec3 cell = floor(scaled);
        vec3 sub  = fract(scaled) - 0.5;
        float r = hash13(cell);
        float starMask = step(0.9832, r);
        float bright = 0.5 + 0.5 * fract(r * 91.7);
        float dist = length(sub);
        // Inner radius full-bright, outer radius zero — soft circular falloff.
        // ~2px stars: small enough to look like pin-pricks, large enough to
        // be sampled consistently every frame.
        float shape = 1.0 - smoothstep(0.06, 0.16, dist);
        float horizonFade = smoothstep(0.0, 0.12, dir.y);
        sky += vec3(1.0, 1.0, 0.95) * starMask * shape * bright * starNight * horizonFade;
    }

    FragColor = vec4(sky, 1.0);
}
