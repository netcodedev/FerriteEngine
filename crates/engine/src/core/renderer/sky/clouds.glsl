// Cloud pass: the three cloud layers, at reduced resolution, into a
// texture the sky blends in (rgb: scattered light, premultiplied; a:
// opacity). `common.glsl` and `cloud_field.glsl` come before this.
//
//   low   350–1650 m  stratus .. cumulonimbus, raymarched (`cloudDensity`)
//   mid   3200 m      altocumulus: fields of small puffs, a thin sheet
//   high  7500 m      cirrus: wind-combed streaks of ice

in vec2 ndcPos;
out vec4 FragColor;

// Reprojection (`reproject` 1): this pass draws a quarter of the cloud
// texels, one of each 2x2 block (`phase` in it) into a target half as big
// each way; the others come from earlier frames (`reproject.glsl`).
uniform int reproject;
uniform ivec2 phase;
// The size of the full cloud texture.
uniform vec2 cloudSize;

// The cloud texel this fragment computes (its centre, in pixels of the
// full cloud texture). Set at the start of `main`.
vec2 cloudPixel;

// The opaque scene's depth at full size, when it is drawn before the sky
// (`useSceneDepth`): where geometry covers every pixel this cloud texel
// can blend into, the clouds are hidden and not marched.
uniform sampler2D sceneDepth;
uniform int useSceneDepth;

// Full-size pixels per cloud texel, along x and y.
uniform vec2 sceneDepthScale;

// Whether opaque geometry covers every full-size pixel whose sky samples
// this texel: the sky filters the cloud texture bilinearly, so the pixels
// of the texels around it count too.
bool hiddenByScene() {
    ivec2 last = textureSize(sceneDepth, 0) - 1;
    ivec2 lo = clamp(ivec2(floor((cloudPixel - 1.5) * sceneDepthScale)), ivec2(0), last);
    ivec2 hi = clamp(ivec2(ceil((cloudPixel + 1.5) * sceneDepthScale)), ivec2(0), last);
    for (int y = lo.y; y <= hi.y; ++y) {
        for (int x = lo.x; x <= hi.x; ++x) {
            if (texelFetch(sceneDepth, ivec2(x, y), 0).r >= 1.0) {
                return false;
            }
        }
    }
    return true;
}

// Raymarch steps through the low layer: more for a tall storm layer, so
// its steps stay short enough not to turn edges into grain.
const int   MARCH_STEPS_MIN = 56;
const int   MARCH_STEPS_MAX = 96;
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
    // Moonlit clouds stand out grey-blue against the night sky.
    night += vec3(0.10, 0.12, 0.17) * smoothstep(-0.05, 0.25, moonDirection.y)
        * clamp(moonlight, 0.0, 4.0);
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
    int steps = int(mix(float(MARCH_STEPS_MIN), float(MARCH_STEPS_MAX),
                        smoothstep(0.6, 1.0, cloudType)));
    float stepLen  = marchLen / float(steps);
    // Long steps can't resolve fine detail either: treat them as distance.
    float stepLod = clamp((stepLen - 40.0) / 160.0, 0.0, 0.6);

    // Per-pixel ray offset hides banding; stable over time (a changing
    // one would crawl without TAA), and weaker for grazing rays where it
    // would turn into density noise.
    float jitter = hash12(cloudPixel);
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

    for (int i = 0; i < steps; i++) {
        float lod = max(clamp(float(i) * stepLen * invFar, 0.0, 1.0), stepLod);
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
            float occ    = 0.55 * smoothstep(0.0, 2.0, dAbove);
            // ...but not by a storm's anvil: it is a thin sheet, and the
            // tower tops just under it would turn into dark streaks.
            occ *= 1.0 - smoothstep(0.70, 0.80, (pos.y + probeHeight - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM));

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

// Distance to the nearest of a jittered grid of points (cellular noise).
float cellDistance(vec2 x) {
    vec2 i = floor(x);
    vec2 f = fract(x);
    float best = 1e9;
    for (int y = -1; y <= 1; y++) {
        for (int xo = -1; xo <= 1; xo++) {
            vec2 o = vec2(float(xo), float(y));
            vec2 r = o + hash22(i + o) - f;
            best = min(best, dot(r, r));
        }
    }
    return sqrt(best);
}

// Altocumulus: clumps of rounded billows that merge in the middle of a
// patch and break up into small separate puffs at its edges, in loose
// rows across the wind.
float altoDensity(vec2 p, float lod) {
    // Higher layers ride faster winds.
    vec2 q = p - cloudWind().xz * 1.8;
    float a = clamp(altocumulus, 0.0, 1.0);
    float patches = cloudFbm(vec3(q.x * 0.00025, 5.0, q.y * 0.00025), 3);
    float lo = mix(0.78, 0.30, a);
    float cover = smoothstep(lo, lo + 0.18, patches);
    if (cover <= 0.0) return 0.0;

    // Billows ~350 m across, squashed across the wind into rows, with a
    // warp so the rows wander.
    vec2 w = normalize(vec2(0.9, 0.3));
    vec2 c = vec2(dot(q, w), dot(q, vec2(-w.y, w.x))) / vec2(450.0, 320.0);
    c += (vec2(cloudFbm(vec3(c.x * 0.2, 1.0, c.y * 0.2), 2),
               cloudFbm(vec3(c.x * 0.2, 7.0, c.y * 0.2), 2)) - 0.5) * 2.0;
    float billows = 1.0 - smoothstep(0.0, 0.85, cellDistance(c));
    float small   = 1.0 - smoothstep(0.0, 0.80, cellDistance(c * 2.3 + 5.1));
    float soft    = cloudFbm(vec3(c.x * 0.9, 3.0, c.y * 0.9), 3);
    float field = billows * 0.50 + small * 0.25 + soft * 0.40;
    // Far away the billows are smaller than a pixel: use their average.
    field = mix(field, 0.55, smoothstep(0.2, 0.8, lod));

    // The thicker the patch, the lower the threshold: billows merge.
    float cut = mix(0.80, 0.38, cover);
    return smoothstep(cut, cut + 0.22, field);
}

vec4 altoClouds(vec3 dir, float dayFactor) {
    if (altocumulus <= 0.0 || dir.y < 0.01 || camPos.y > ALTO_HEIGHT) return vec4(0.0);
    float t = (ALTO_HEIGHT - camPos.y) / dir.y;
    vec2 p = camPos.xz + dir.xz * t;
    float lod = clamp(t / 30000.0, 0.0, 1.0);
    float d = altoDensity(p, lod);
    if (d <= 0.001) return vec4(0.0);

    // Self-shadow: the sheet is thin, so a billow is lit on the side that
    // faces the sun and grey on the far side and in its thick middle.
    vec2 toSun = normalize(sunDirection.xz + vec2(1e-4));
    float dSun = altoDensity(p + toSun * 90.0, lod);
    float shade = 1.0 - 0.45 * clamp(dSun - d * 0.5, 0.0, 1.0) - 0.25 * d;

    vec3 sun = sunlightAt(ALTO_HEIGHT);
    vec3 lit = sun * shade * (0.45 + 0.6 * cloudPhase(dot(dir, sunDirection)))
             + cloudAmbient(dir, dayFactor) * 0.75;
    float alpha = clamp(d * 0.9, 0.0, 0.85) * smoothstep(0.01, 0.10, dir.y);
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
    vec2 ndc = ndcPos;
    cloudPixel = gl_FragCoord.xy;
    if (reproject == 1) {
        cloudPixel = floor(gl_FragCoord.xy) * 2.0 + vec2(phase) + 0.5;
        ndc = cloudPixel / cloudSize * 2.0 - 1.0;
    }
    if (useSceneDepth == 1 && hiddenByScene()) {
        FragColor = vec4(0.0);
        return;
    }
    vec3 dir = viewRay(ndc);
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
