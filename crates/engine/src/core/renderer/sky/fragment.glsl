// The sky: gradient, sunset glow, sun, clouds (from the cloud pass) and
// stars. `common.glsl` comes before this.

in vec2 ndcPos;
out vec4 FragColor;

uniform sampler2D cloudTexture;

// Linear brightness of the sun disk, relative to a white sky.
const float SUN_DISK_RADIANCE = 40.0;

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
    vec4 clouds = texture(cloudTexture, ndcPos * 0.5 + 0.5);
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
    FragColor = vec4(sky, 1.0);
}
