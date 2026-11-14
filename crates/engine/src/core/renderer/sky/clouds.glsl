// Cloud pass: the three cloud layers, at reduced resolution, into a
// texture the sky blends in (rgb: scattered light, premultiplied; a:
// opacity). `common.glsl` and `cloud_field.glsl` come before this.
//
//   low   350–1650 m  stratus .. cumulonimbus, raymarched (`cloudDensity`)
//   mid   3200 m      altocumulus: fields of small puffs, a thin sheet
//   high  7500 m      cirrus: wind-combed streaks of ice

in vec2 ndcPos;
out vec4 FragColor;

const int   MARCH_STEPS  = 56;
const int   SUN_STEPS    = 4;
const float SUN_STEP_LEN = 35.0;

const float ALTO_HEIGHT   = 3200.0;
const float CIRRUS_HEIGHT = 7500.0;

// ── Light ─────────────────────────────────────────────────────────────

// Colour of the sunlight that reaches `altitude`. Low sun shines through
// a lot of air, which scatters blue away first: white at noon, yellow,
// orange, then red at the horizon. Higher clouds see past the curve of
// the earth and stay lit after the sun has set for the ground, so cirrus
// glow pink after the low clouds have turned grey.
vec3 sunlightAt(float altitude) {
    // How far below the horizon the sun can be and still reach this
    // height. Twice the real dip so it shows in a short game day.
    float dip = sqrt(2.0 * max(altitude, 0.0) / 6.371e6) * 2.0;
    float elevation = sunDirection.y + dip;
    float visible = smoothstep(-0.012, 0.012, elevation);
    float e = clamp(elevation, 0.0, 1.0);

    // Air mass along the sun ray (Kasten & Young), ~38 at the horizon.
    float airmass = 1.0 / (e + 0.15 * pow(degrees(asin(e)) + 3.885, -1.253));
    // Optical depth straight up: Rayleigh (blue-heavy) thinning with
    // height, plus a little haze for every colour.
    vec3 tau = vec3(0.045, 0.105, 0.260) * exp(-altitude / 8000.0) + 0.02;
    vec3 noon = exp(-tau);
    vec3 light = exp(-tau * airmass) / noon;

    // Keep the colour, lose some of the darkening: the eye (and the auto
    // exposure) adapts, and a sunset sky is not nearly as dark as the
    // raw extinction says.
    float peak = max(light.r, max(light.g, light.b));
    light *= mix(1.0, 1.0 / max(peak, 1e-3), 0.7);
    // ...but the last light before the sun sets is still dimmer.
    light *= mix(0.35, 1.0, smoothstep(0.0, 0.12, e));
    return light * visible;
}

// Light from the sky dome on a cloud: blue-white by day, dim blue at
// night, and around sunrise and sunset warm on the side facing the sun
// and rosy-violet on the other.
vec3 cloudAmbient(vec3 dir, float dayFactor) {
    vec3 day   = vec3(0.78, 0.82, 0.90);
    vec3 night = vec3(0.060, 0.077, 0.126);
    vec3 ambient = mix(night, day, dayFactor);

    float twilight = 1.0 - smoothstep(0.0, 0.28, abs(sunDirection.y + 0.02));
    vec2 sunH = sunDirection.xz;
    float towardSun = length(sunH) > 1e-4 && length(dir.xz) > 1e-4
        ? dot(normalize(dir.xz), normalize(sunH)) * 0.5 + 0.5
        : 0.5;
    vec3 glow = mix(vec3(0.34, 0.24, 0.40), vec3(0.62, 0.34, 0.24), towardSun * towardSun);
    // The glow fades with the sky once the sun is down.
    glow *= 0.4 * smoothstep(-0.12, 0.04, sunDirection.y);
    return mix(ambient, ambient * 0.5 + glow, twilight);
}

// Mostly-forward scattering: the bright "silver lining" near the sun.
float cloudPhase(float vDotL) {
    float v = max(vDotL, 0.0);
    return 0.5 + 0.5 * v + 0.8 * pow(v, 8.0) + 0.4 * pow(v, 32.0);
}

// ── Low layer: raymarched volume ──────────────────────────────────────

vec4 lowClouds(vec3 dir, float dayFactor) {
    if (dir.y < 0.02) return vec4(0.0);
    float layerTop = cloudLayerTop();
    if (camPos.y > layerTop) return vec4(0.0);

    float tEnter = max((CLOUD_BOTTOM - camPos.y) / dir.y, 0.0);
    float tExit  = (layerTop - camPos.y) / dir.y;
    if (tExit <= 0.0) return vec4(0.0);

    // Cap total march distance so grazing rays don't blow out step count.
    float marchLen = min(tExit - tEnter, 9000.0);
    float stepLen  = marchLen / float(MARCH_STEPS);

    // Per-pixel ray offset hides banding; stable over time (a changing
    // one would crawl without TAA), and weaker for grazing rays where it
    // would turn into density noise.
    float jitter = hash12(gl_FragCoord.xy);
    float startLod = clamp(tEnter / 4000.0, 0.0, 1.0);
    vec3 pos = camPos + dir * (tEnter + stepLen * jitter * mix(1.0, 0.15, startLod));

    float phase = cloudPhase(dot(dir, sunDirection));
    vec3 ambient = cloudAmbient(dir, dayFactor);
    // The sun's colour hardly changes across the layer; two heights are
    // enough to let the tops of tall clouds keep their light a bit longer.
    vec3 sunBase = sunlightAt(CLOUD_BOTTOM);
    vec3 sunTop  = sunlightAt(layerTop);
    float probeHeight = clamp((layerTop - CLOUD_BOTTOM) * 0.35, 40.0, 250.0);

    float transmittance = 1.0;
    vec3  scatter = vec3(0.0);
    const float invFar = 1.0 / 4500.0;

    for (int i = 0; i < MARCH_STEPS; i++) {
        float lod = clamp(float(i) * stepLen * invFar, 0.0, 1.0);
        float d = cloudDensity(pos, false, lod);
        if (d > 0.001) {
            // Sun visibility: a short march toward the sun.
            float sunOptical = 0.0;
            vec3  lp = pos;
            float lStep = SUN_STEP_LEN;
            for (int j = 0; j < SUN_STEPS; j++) {
                lp += sunDirection * lStep;
                sunOptical += cloudDensity(lp, true, lod) * lStep;
                lStep *= 1.6;
            }
            float sunT = exp(-sunOptical * 0.85);

            // Sky occlusion: a probe above the sample darkens the bases
            // of tall clouds but not small ones (and, in a flat sheet,
            // its thicker parts).
            float dAbove = cloudDensity(pos + vec3(0.0, probeHeight, 0.0), true, lod);
            float occ    = clamp(dAbove * 0.30, 0.0, 0.55);

            // Mild powder term — dims only the thinnest wisps.
            float powder = mix(0.65, 1.0, 1.0 - exp(-d * 4.0));

            float hLayer = clamp((pos.y - CLOUD_BOTTOM) / (layerTop - CLOUD_BOTTOM), 0.0, 1.0);
            vec3 sun = mix(sunBase, sunTop, hLayer);
            vec3 lit = sun * sunT * phase * powder + ambient * (1.0 - occ);

            float segAlpha = 1.0 - exp(-d * stepLen);
            scatter       += transmittance * lit * segAlpha;
            transmittance *= 1.0 - segAlpha;
            if (transmittance < 0.01) break;
        }
        pos += dir * stepLen;
    }

    // Distant grazing rays dissolve into the haze above the horizon,
    // where cloud detail is sub-pixel and would sparkle.
    float fade = smoothstep(0.02, 0.22, dir.y);
    return vec4(scatter, 1.0 - transmittance) * fade;
}

// ── Mid layer: altocumulus ────────────────────────────────────────────

// Rounded puffs, one per jittered grid cell (1 in the middle of a puff,
// 0 between them).
float puffs(vec2 x) {
    vec2 i = floor(x);
    vec2 f = fract(x);
    float best = 1e9;
    for (int y = -1; y <= 1; y++) {
        for (int xo = -1; xo <= 1; xo++) {
            vec2 o = vec2(float(xo), float(y));
            vec2 h = hash22(i + o);
            vec2 r = o + 0.15 + 0.7 * h - f;
            // Puffs of different sizes.
            best = min(best, dot(r, r) / mix(0.45, 1.0, h.x));
        }
    }
    return 1.0 - smoothstep(0.03, 0.42, best);
}

float altoDensity(vec2 p, float lod) {
    // Higher layers ride faster winds.
    vec2 q = p - cloudWind().xz * 1.8;
    float patches = cloudFbm(vec3(q.x * 0.00028, 5.0, q.y * 0.00028), 3);
    float a = clamp(altocumulus, 0.0, 1.0);
    float edge = mix(0.78, 0.30, a);
    float cover = smoothstep(edge, edge + 0.16, patches);
    if (cover <= 0.0) return 0.0;

    // Warped cells, roughly 160 m across, so the puffs line up in loose
    // waves instead of a grid.
    vec2 c = q / 160.0;
    c += (vec2(cloudFbm(vec3(c.x * 0.15, 1.0, c.y * 0.15), 2),
               cloudFbm(vec3(c.x * 0.15, 7.0, c.y * 0.15), 2)) - 0.5) * 3.0;
    // Ragged puffs: noise eats into their edges.
    float cells = puffs(c) * mix(0.45, 1.0, cloudFbm(vec3(c.x * 1.7, 3.0, c.y * 1.7), 3));
    // Far away the puffs are smaller than a pixel: use their average.
    cells = mix(cells, 0.32, smoothstep(0.15, 0.8, lod));
    return cells * cover;
}

vec4 altoClouds(vec3 dir, float dayFactor) {
    if (altocumulus <= 0.0 || dir.y < 0.01 || camPos.y > ALTO_HEIGHT) return vec4(0.0);
    float t = (ALTO_HEIGHT - camPos.y) / dir.y;
    vec2 p = camPos.xz + dir.xz * t;
    float lod = clamp(t / 30000.0, 0.0, 1.0);
    float d = altoDensity(p, lod);
    if (d <= 0.001) return vec4(0.0);

    // Self-shadow: the sheet is thin, so the light a puff gets depends on
    // how much cloud lies toward the sun just next to it.
    vec2 toSun = normalize(sunDirection.xz + vec2(1e-4));
    float dSun = altoDensity(p + toSun * 60.0, lod);
    float shade = 1.0 - 0.55 * clamp(dSun - d * 0.3, 0.0, 1.0);

    vec3 sun = sunlightAt(ALTO_HEIGHT);
    vec3 lit = sun * shade * (0.55 + 0.6 * cloudPhase(dot(dir, sunDirection)))
             + cloudAmbient(dir, dayFactor) * 0.85;
    float alpha = clamp(d * 0.85, 0.0, 0.8) * smoothstep(0.01, 0.10, dir.y);
    return vec4(lit * alpha, alpha);
}

// ── High layer: cirrus ────────────────────────────────────────────────

float cirrusDensity(vec2 p, float lod) {
    vec2 q = p - cloudWind().xz * 3.0;
    // Streaks along the wind: stretch the noise across it.
    vec2 w = normalize(vec2(0.9, 0.3));
    vec2 s = vec2(dot(q, w), dot(q, vec2(-w.y, w.x)));
    vec2 st = s * vec2(0.00011, 0.00075);
    // Bend the streaks into hooks and sweeps.
    st.y += (cloudFbm(vec3(st.x * 0.5, 3.0, st.y * 0.12), 3) - 0.5) * 2.5;

    float patches = cloudFbm(vec3(q.x * 0.00006, 9.0, q.y * 0.00006), 2);
    float a = clamp(cirrus, 0.0, 1.0);
    float edge = mix(0.74, 0.28, a);
    float cover = smoothstep(edge, edge + 0.2, patches);
    if (cover <= 0.0) return 0.0;

    float n = cloudFbm(vec3(st.x, 1.0, st.y), lod > 0.5 ? 3 : 5);
    // Fine fibres across the streaks.
    float fibres = cloudFbm(vec3(st.x * 0.4, 5.0, st.y * 7.0), lod > 0.5 ? 1 : 3);
    float streak = smoothstep(0.45, 0.80, n) * mix(0.35, 1.0, fibres);
    // Thin out along the streaks into wisps.
    streak *= smoothstep(0.25, 0.65, cloudFbm(vec3(st.x * 3.0, 7.0, st.y * 0.6), 2));
    return streak * cover;
}

vec4 cirrusClouds(vec3 dir, float dayFactor) {
    if (cirrus <= 0.0 || dir.y < 0.005) return vec4(0.0);
    float t = (CIRRUS_HEIGHT - camPos.y) / dir.y;
    vec2 p = camPos.xz + dir.xz * t;
    float lod = clamp(t / 60000.0, 0.0, 1.0);
    float d = cirrusDensity(p, lod);
    if (d <= 0.001) return vec4(0.0);

    // Thin ice: lit right through, very bright toward the sun.
    float v = max(dot(dir, sunDirection), 0.0);
    vec3 sun = sunlightAt(CIRRUS_HEIGHT);
    vec3 lit = sun * (0.85 + 1.2 * pow(v, 6.0)) + cloudAmbient(dir, dayFactor) * 0.6;
    float alpha = clamp(d * 0.6, 0.0, 0.55) * smoothstep(0.005, 0.08, dir.y);
    return vec4(lit * alpha, alpha);
}

void main() {
    vec3 dir = viewRay(ndcPos);
    float dayFactor = dayFactorOf(sunDirection);

    // Front to back: the low layer covers the mid one, which covers the
    // high one.
    vec4 low = lowClouds(dir, dayFactor);
    vec4 result = low;
    if (result.a < 0.99) {
        vec4 mid = altoClouds(dir, dayFactor);
        result += (1.0 - result.a) * mid;
    }
    if (result.a < 0.99) {
        vec4 high = cirrusClouds(dir, dayFactor);
        result += (1.0 - result.a) * high;
    }
    FragColor = result;
}
