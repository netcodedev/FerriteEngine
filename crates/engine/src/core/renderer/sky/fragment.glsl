// The sky: gradient, sunset glow, sun, clouds (from the cloud pass) and
// stars. `common.glsl` comes before this.

in vec2 ndcPos;
out vec4 FragColor;

uniform sampler2D cloudTexture;
// How hard it rains or snows, 0..1: the distance fades into grey murk.
uniform float precipitation;
// Must match `MURK` in `weather/weather.glsl`.
const vec3 MURK = vec3(0.50, 0.53, 0.57);

// Linear brightness of the sun disk, relative to a white sky.
const float SUN_DISK_RADIANCE = 40.0;
// The moon disk and the glow of the sky around and under it (display
// colours, at `moonlight` 1).
const float MOON_DISK_RADIANCE = 3.0;
const vec3 MOON_SKY_ZENITH = vec3(0.020, 0.032, 0.070);
const vec3 MOON_SKY_HORIZON = vec3(0.035, 0.050, 0.095);

void main() {
    vec3 dir = viewRay(ndcPos);

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
    // A moonlit night sky is a deep blue, not black.
    float moonUp = smoothstep(-0.05, 0.25, moonDirection.y) * clamp(moonlight, 0.0, 4.0);
    nightCol += mix(MOON_SKY_HORIZON, MOON_SKY_ZENITH, vBlend) * moonUp;
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
    sky += vec3(1.00, 0.80, 0.55) * sunHalo * sunVisible;

    // ── Clouds ─────────────────────────────────────────────────────────
    // Raymarched at reduced resolution by the cloud pass.
    // Premultiplied: rgb is the light the clouds scatter, a how much of
    // the sky behind them they hide.
    vec4 clouds = texture(cloudTexture, ndcPos * 0.5 + 0.5);
    sky = sky * (1.0 - clouds.a) + clouds.rgb;

    // ── Rain and snow ──────────────────────────────────────────────────
    // What falls hides the distance: the horizon goes the grey of the
    // murk the ground fades into, and some of it reaches up the sky.
    if (precipitation > 0.0) {
        vec3 murk = MURK * mix(0.04, 1.0 - 0.3 * precipitation, dayFactor);
        float low = 1.0 - smoothstep(-0.05, 0.5, dir.y);
        sky = mix(sky, murk, precipitation * mix(0.3, 0.95, low));
    }

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
        // Attenuate by cloud opacity along this ray — opaque cloud cover
        // hides stars behind it.
        float cloudHide   = 1.0 - clouds.a;
        sky += vec3(1.0, 1.0, 0.95) * starMask * shape * bright
             * starNight * horizonFade * cloudHide;
    }

    // The colours above are display colours; the HDR target holds linear
    // light. The sun is far brighter than the sky around it: past what a
    // display shows, so it rolls off to white and blooms.
    sky = pow(max(sky, vec3(0.0)), vec3(2.2));
    float sunCover = 1.0 - clouds.a;
    sky += vec3(1.00, 0.95, 0.85) * SUN_DISK_RADIANCE * sunDisk * sunVisible * sunCover;

    // ── Moon disk and halo, at night ───────────────────────────────────
    float moonDot = clamp(dot(dir, moonDirection), -1.0, 1.0);
    float moonVisible = smoothstep(-0.03, 0.05, moonDirection.y)
        * (1.0 - dayFactor) * min(moonlight, 1.0);
    if (moonVisible > 0.0) {
        float moonDisk = smoothstep(0.99955, 0.99975, moonDot);
        float moonHalo = pow(max(moonDot, 0.0), 400.0) * 0.08;
        // Darker patches on the disk ("seas"), fixed to the moon.
        vec3 across = dir - moonDirection * moonDot;
        float seas = 0.82 + 0.18 * hash13(floor(across * 900.0));
        vec3 moonColor = vec3(0.80, 0.86, 1.00);
        sky += moonColor * (MOON_DISK_RADIANCE * moonDisk * seas + moonHalo)
            * moonVisible * sunCover;
    }
    FragColor = vec4(sky, 1.0);
}
