// Shading of the water surface. `pbr/lighting.glsl` (sun, sky, shadows,
// `toDisplay`) and `water_waves.glsl` come before this.
//
// The surface is composed with the opaque scene under it (`sceneColor`,
// `sceneDepth`: a copy taken before the transparent pass): light from the
// ground is bent by the waves and absorbed on its way through the water,
// colour by colour (red first, so deep water turns blue-green), while the
// water itself scatters some daylight back. On top comes the reflection of
// the scene (marched in screen space) or the sky, weighted by the Fresnel
// term, the sun's highlight, light shining through the wave crests, and
// foam on the crests and along the shore.

in vec3 WorldPos;
in vec2 RestXZ;
in float Depth;
in float BodySize;
in float Crest;
in float WaveHeight;
// Metres per second the water flows (x, z).
in vec2 Flow;

out vec4 FragColor;

uniform sampler2D sceneColor;
uniform sampler2D sceneDepth;
// 0 when the opaque scene is not at hand (depth captures, previews): the
// water is then drawn see-through without refraction or shore blending.
uniform int hasScene;
uniform mat4 viewProjection;
uniform mat4 inverseViewProjection;
// The far water is left out where the chunks are loaded (`columnLoaded`),
// and within `farHole` metres of the camera at `farSink.xy` (under the
// ground, as the horizon is).
uniform int isFarWater;
uniform vec3 farSink;
uniform float farHole;
// Haze: as the terrain's (see its fragment shader), so far water fades into
// the sky like the land beside it.
const float HAZE_START = 600.0;
const float HAZE_DISTANCE = 9000.0;

// Absorption per unit of water, red, green and blue: red light is gone
// after a few units, blue carries furthest.
const vec3 ABSORPTION = vec3(0.45, 0.12, 0.085);
// Water up to `CLEAR_DEPTH` deep absorbs (and scatters) only
// `CLEAR_ABSORPTION` of that; from there to `BLUE_DEPTH` it turns to its
// full colour.
const float CLEAR_DEPTH = 1.0;
const float BLUE_DEPTH = 5.0;
const float CLEAR_ABSORPTION = 0.2;
// Colour of the light the water scatters back (silt and plankton: a little
// green in the blue).
const vec3 SCATTER_COLOR = vec3(0.003, 0.025, 0.045);
// Light shining through the thin top of a wave.
const vec3 CREST_COLOR = vec3(0.06, 0.40, 0.34);
// Reflectance of water head on (IOR 1.33).
const float WATER_F0 = 0.02;
const float REFRACTION = 0.035;
// Shore foam reaches this deep.
const float SHORE_FOAM_DEPTH = 1.0;
// Water fades into the ground over this depth, hiding the waterline's
// pixel steps.
const float SHORE_FADE_DEPTH = 0.12;
// The surface reaches this far under the ground at most (its grid runs on
// under the shore, see `water.rs`): beyond, behind a bank or a ridge, it
// is cut off rather than left for the depth test.
const float SURFACE_UNDER_GROUND = 1.0;
// Flowing water: its pattern moves with the flow for this long, then a
// second one half a period apart takes over (so it never stretches).
const float FLOW_PERIOD = 2.0;

// ── Noise ────────────────────────────────────────────────────────────────

// Value noise with its derivatives: (value, d/dx, d/dy).
vec3 noised(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    vec2 du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    float a = waveHash(i);
    float b = waveHash(i + vec2(1.0, 0.0));
    float c = waveHash(i + vec2(0.0, 1.0));
    float d = waveHash(i + vec2(1.0, 1.0));
    float k1 = b - a;
    float k2 = c - a;
    float k4 = a - b - c + d;
    return vec3(a + k1 * u.x + k2 * u.y + k4 * u.x * u.y,
                du * (vec2(k1, k2) + k4 * u.yx));
}

// Small ripples the grid is too coarse for, as the slope of a height
// field (d/dx, d/dz): wind-driven noise drifting different ways. Octaves
// finer than a pixel fade out (`footprint`: world units per pixel), so
// distant water does not sparkle.
vec2 rippleSlope(vec2 p, float t, float footprint) {
    const mat2 turn = mat2(0.8, 0.6, -0.6, 0.8);
    vec2 slope = vec2(0.0);
    float amplitude = 0.22;
    float frequency = 0.22;
    vec2 drift = WIND * 0.9;
    for (int i = 0; i < 4; i++) {
        float resolved = 1.0 - smoothstep(0.15, 0.4, footprint * frequency);
        if (resolved <= 0.0) {
            break;
        }
        vec3 n = noised(p * frequency + drift * t * frequency);
        slope += n.yz * amplitude * frequency * resolved;
        p = turn * p;
        drift = turn * drift * 1.15;
        amplitude *= 0.6;
        frequency *= 2.0;
    }
    return slope;
}

float foamPattern(vec2 p, float t) {
    float a = noised(p * 0.55 + vec2(t * 0.11, t * 0.07)).x;
    float b = noised(p * 1.7 - vec2(t * 0.05, t * 0.13)).x;
    float c = noised(p * 4.3 + vec2(t * 0.17, -t * 0.09)).x;
    return a * 0.5 + b * 0.32 + c * 0.18;
}

// ── Sky ──────────────────────────────────────────────────────────────────

// The sky in direction `dir`, without the sun disk (linear). Follows the
// sky pass (`renderer/sky/fragment.glsl`), with the clouds as an overcast
// veil since the cloud texture only covers the screen.
vec3 skyColor(vec3 dir) {
    float sunAlt = clamp(sunDirection.y, -1.0, 1.0);
    float dayFactor = smoothstep(-0.10, 0.20, sunAlt);
    float up = clamp(dir.y, 0.0, 1.0);
    float vBlend = sqrt(up);
    vec3 dayCol = mix(vec3(0.72, 0.85, 1.00), vec3(0.18, 0.42, 0.85), vBlend);
    vec3 nightCol = mix(vec3(0.020, 0.030, 0.080), vec3(0.005, 0.010, 0.030), vBlend);
    vec3 sky = mix(nightCol, dayCol, dayFactor);

    float sunHorizonProx = 1.0 - smoothstep(0.0, 0.30, abs(sunAlt));
    float horizLen = length(sunDirection.xz);
    float dirLen = length(dir.xz);
    if (sunHorizonProx > 0.001 && horizLen > 1e-4 && dirLen > 1e-4) {
        float hemi = max(dot(dir.xz / dirLen, sunDirection.xz / horizLen), 0.0);
        float glow = pow(hemi, 4.0) * exp(-dir.y * dir.y * 18.0) * sunHorizonProx;
        sky = mix(sky, vec3(1.00, 0.45, 0.18), clamp(glow * 0.85, 0.0, 0.85));
    }
    float sunDot = max(dot(dir, normalize(sunDirection)), 0.0);
    sky += vec3(1.00, 0.80, 0.55) * pow(sunDot, 180.0) * 0.55 * smoothstep(-0.04, 0.05, sunAlt);

    sky = pow(max(sky, vec3(0.0)), vec3(2.2));
    float cover = clamp(cloudCoverage, 0.0, 1.0);
    float grey = dot(sky, vec3(0.3, 0.5, 0.2));
    return mix(sky, vec3(grey * 1.1), cover * 0.6);
}

// ── Screen space ─────────────────────────────────────────────────────────

vec3 worldAt(vec2 uv, float depth) {
    vec4 p = inverseViewProjection * vec4(vec3(uv, depth) * 2.0 - 1.0, 1.0);
    return p.xyz / p.w;
}

vec3 project(vec3 world) {
    vec4 clip = viewProjection * vec4(world, 1.0);
    return vec3(clip.xy / clip.w * 0.5 + 0.5, clip.w);
}

// 0 at the screen's edge, 1 a bit inside.
float edgeFade(vec2 uv) {
    vec2 d = min(uv, 1.0 - uv);
    return clamp(min(d.x, d.y) / 0.08, 0.0, 1.0);
}

// The scene reflected along `R` from `origin`: marched through the depth
// copy. rgb is the reflection, a how sure the march is (0: use the sky).
vec4 screenReflection(vec3 origin, vec3 R) {
    vec3 eye = cameraPosition;
    float stepLength = 1.5;
    vec3 p = origin;
    for (int i = 0; i < 28; i++) {
        p += R * stepLength;
        vec3 s = project(p);
        if (s.z <= 0.0 || s.x < 0.0 || s.x > 1.0 || s.y < 0.0 || s.y > 1.0) {
            break;
        }
        float d = texture(sceneDepth, s.xy).r;
        if (d < 1.0) {
            float sceneDistance = distance(eye, worldAt(s.xy, d));
            float rayDistance = distance(eye, p);
            float behind = rayDistance - sceneDistance;
            if (behind > 0.0 && behind < stepLength * 2.5 + 1.0) {
                // Refine between the last two points.
                vec3 lo = p - R * stepLength;
                vec3 hi = p;
                for (int j = 0; j < 5; j++) {
                    vec3 mid = (lo + hi) * 0.5;
                    vec3 ms = project(mid);
                    float md = texture(sceneDepth, ms.xy).r;
                    if (distance(eye, mid) > distance(eye, worldAt(ms.xy, md))) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                vec3 hs = project(hi);
                float fade = edgeFade(hs.xy) * (1.0 - float(i) / 28.0);
                return vec4(texture(sceneColor, hs.xy).rgb, fade);
            }
        }
        stepLength *= 1.22;
    }
    return vec4(0.0);
}

// The sky where the reflected direction is on screen, with its clouds;
// a = 0 where it is not.
vec4 screenSky(vec3 R) {
    vec4 clip = viewProjection * vec4(R, 0.0);
    if (clip.w <= 0.0) {
        return vec4(0.0);
    }
    vec2 uv = clip.xy / clip.w * 0.5 + 0.5;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || texture(sceneDepth, uv).r < 1.0) {
        return vec4(0.0);
    }
    // Leave the sun to the highlight below.
    return vec4(min(texture(sceneColor, uv).rgb, vec3(4.0)), edgeFade(uv));
}

// The scene's lamps mirrored in the surface: a glint for each, drawn out
// into a streak by the waves (GGX, like the sun), behind their shadows.
vec3 lampReflections(vec3 N, vec3 V, float NdotV, float roughness) {
    vec3 sum = vec3(0.0);
    for (int i = 0; i < lightCount; ++i) {
        Light l = lights[i];
        int kind = int(l.directionKind.w + 0.5);
        vec3 L;
        float attenuation = 1.0;
        if (kind == LIGHT_DIRECTIONAL) {
            L = -l.directionKind.xyz;
        } else {
            vec3 toLight = l.positionRange.xyz - WorldPos;
            float d2 = dot(toLight, toLight);
            float range = l.positionRange.w;
            if (d2 >= range * range) {
                continue;
            }
            float d = sqrt(d2);
            L = toLight / max(d, 1e-4);
            attenuation = rangeFade(d, range) / max(d2, 0.01);
            if (kind == LIGHT_SPOT) {
                attenuation *= spotCone(l, L);
            }
        }
        float NdotL = dot(N, L);
        if (attenuation <= 0.0 || NdotL <= 0.0) {
            continue;
        }
        attenuation *= lightShadow(l, WorldPos, vec3(0.0, 1.0, 0.0), L);
        vec3 H = normalize(V + L);
        float NdotH = max(dot(N, H), 0.0);
        float F = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - max(dot(H, V), 0.0), 5.0);
        float specular = distributionGGX(NdotH, roughness * roughness)
            * geometrySmith(NdotV, NdotL, roughness) * F / max(4.0 * NdotV * NdotL, 1e-4);
        sum += l.radianceBounce.rgb * attenuation * specular * NdotL;
    }
    return sum;
}

// Where the two flow phases have carried the pattern by now (`p` moved
// back along the flow), and how much the first counts.
void flowPhases(vec2 p, out vec2 p0, out vec2 p1, out float w0) {
    float t0 = fract(uTime / FLOW_PERIOD);
    float t1 = fract(uTime / FLOW_PERIOD + 0.5);
    p0 = p - Flow * t0 * FLOW_PERIOD;
    p1 = p - Flow * t1 * FLOW_PERIOD + vec2(7.3, 3.1);
    w0 = 1.0 - abs(1.0 - 2.0 * t0);
}

void main()
{
    if (Depth < -SURFACE_UNDER_GROUND) {
        discard;
    }
    if (isFarWater == 1 && (columnLoaded(WorldPos.xz) || length(WorldPos.xz - farSink.xy) < farHole)) {
        discard;
    }
    // Seen from under its own level, a water surface only shows its
    // underside, and the far water's lies a little under the chunks' (see
    // the vertex shader): from under the near water's surface (diving) it
    // would show as a second surface below the first.
    if (isFarWater == 1) {
        if (cameraPosition.y < WorldPos.y + 0.15) {
            discard;
        }
    }
    if (isShadowPass == 1) {
        FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    vec3 toEye = cameraPosition - WorldPos;
    float viewDistance = length(toEye);
    vec3 V = toEye / viewDistance;
    vec3 L = normalize(sunDirection);

    // ── Normal: the waves plus ripples, calmer with distance ──────────
    WaveSample waves = sampleWaves(RestXZ, BodySize, Depth);
    float far = smoothstep(60.0, 400.0, viewDistance);
    float rippleStrength = mix(0.55, 1.0, smoothstep(10.0, 200.0, BodySize))
        * smoothstep(-0.5, 0.5, Depth);
    float footprint = length(fwidth(RestXZ));
    float speed = length(Flow);
    vec2 slope;
    if (speed > 0.05) {
        // Carried downstream.
        vec2 p0, p1;
        float w0;
        flowPhases(RestXZ, p0, p1, w0);
        slope = (rippleSlope(p0, uTime, footprint) * w0 + rippleSlope(p1, uTime, footprint) * (1.0 - w0))
            * rippleStrength * (1.0 + 0.25 * min(speed, 4.0));
    } else {
        slope = rippleSlope(RestXZ, uTime, footprint) * rippleStrength;
    }
    // Rings where raindrops hit, too fine to see from afar.
    float rainNear = 1.0 - smoothstep(15.0, 50.0, viewDistance);
    if (rainAmount > 0.0 && rainNear > 0.0) {
        float open = skyExposure(WorldPos, vec3(0.0, 1.0, 0.0));
        slope += rainRipples(RestXZ, weatherTime, rainAmount, footprint) * 0.7 * rainNear * open;
    }
    // Waves shorter than a few pixels would only shimmer (a moiré from
    // afar or high up): their slope fades as a pixel grows.
    vec3 waveNormal = normalize(mix(waves.normal, vec3(0.0, 1.0, 0.0), 0.8 * smoothstep(0.6, 4.0, footprint)));
    vec3 N = normalize(waveNormal + vec3(-slope.x, 0.0, -slope.y));
    bool below = cameraPosition.y < WorldPos.y;
    if (below) {
        N = -N;
    }
    float NdotV = max(dot(N, V), 1e-3);

    // ── What lies under the surface ───────────────────────────────────
    vec3 sunLight = sunRadiance();
    float shadow = sunShadow(WorldPos, vec3(0.0, 1.0, 0.0), L);
    // Daylight inside the water, which it scatters back.
    vec3 ambient = skyLight(vec3(0.0, 1.0, 0.0)) + sunLight * max(L.y, 0.0) * (0.35 + 0.65 * shadow)
        + moonRadiance() * max(moonDirection.y, 0.0) * 0.5
        // Lamp light the water scatters back around the lamp.
        + localLightAt(WorldPos) * 0.35;
    vec3 scattered = SCATTER_COLOR * ambient;

    vec3 underwater;
    float waterDepth;  // how deep the water is in front of the ground here
    float alpha = 1.0;
    if (hasScene == 1) {
        vec2 size = vec2(textureSize(sceneDepth, 0));
        vec2 uv = gl_FragCoord.xy / size;
        float groundDepth = texture(sceneDepth, uv).r;
        vec3 ground = worldAt(uv, groundDepth);
        waterDepth = groundDepth >= 1.0 ? 1e4 : WorldPos.y - ground.y;
        float through = groundDepth >= 1.0 ? 1e4 : distance(WorldPos, ground);

        // Bend the view by the waves, less where the water is shallow or
        // far away; but never onto something in front of the water.
        vec2 bend = N.xz * REFRACTION * clamp(through / 3.0, 0.0, 1.0) / (1.0 + viewDistance * 0.03);
        vec2 refractedUv = clamp(uv + bend, vec2(0.0), vec2(1.0));
        float refractedDepth = texture(sceneDepth, refractedUv).r;
        vec3 refractedGround = worldAt(refractedUv, refractedDepth);
        if (refractedDepth < 1.0 && distance(cameraPosition, refractedGround) < viewDistance) {
            refractedUv = uv;
            refractedDepth = groundDepth;
            refractedGround = ground;
        }
        float path = refractedDepth >= 1.0 ? 1e4 : distance(WorldPos, refractedGround);
        // The light also came down through the water to reach the ground.
        float column = refractedDepth >= 1.0 ? 1e4 : max(WorldPos.y - refractedGround.y, 0.0);
        // Shallow water (a stream, the shallows of a lake) is clear: the
        // ground shows through it as it is, and the water only turns blue
        // where it gets deep.
        float clear = 1.0 - smoothstep(CLEAR_DEPTH, BLUE_DEPTH, waterDepth);
        float thickness = (path + column * 0.6) * mix(1.0, CLEAR_ABSORPTION, clear);
        vec3 transmittance = exp(-ABSORPTION * thickness);
        vec3 behind = texture(sceneColor, refractedUv).rgb;
        underwater = behind * transmittance
            + scattered * (1.0 - exp(-ABSORPTION.g * 2.5 * path * mix(1.0, CLEAR_ABSORPTION, clear)));
        alpha = smoothstep(0.0, SHORE_FADE_DEPTH, waterDepth);
    } else {
        waterDepth = max(Depth, 0.0);
        underwater = scattered;
        alpha = 0.85;
    }

    // ── Reflection ────────────────────────────────────────────────────
    vec3 R = reflect(-V, N);
    R.y = abs(R.y);  // ripples tilting the reflection into the water
    R = normalize(R);
    vec3 reflection = skyColor(R);
    if (hasScene == 1 && !below) {
        vec4 skyOnScreen = screenSky(R);
        reflection = mix(reflection, skyOnScreen.rgb, skyOnScreen.a);
        // Marched along a calmer normal: the ripples would scatter the
        // reflection of the shore into specks.
        vec3 Rs = reflect(-V, normalize(mix(waves.normal, N, 0.3)));
        Rs.y = abs(Rs.y);
        vec4 scene = screenReflection(WorldPos, normalize(Rs));
        reflection = mix(reflection, scene.rgb, scene.a);
    }
    float fresnel = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - NdotV, 5.0);
    // Distant water seen at a grazing angle is a mirror, but not a perfect
    // one: the waves hide part of it.
    fresnel = min(fresnel, 0.9);

    vec3 color = mix(underwater, reflection, fresnel);
    if (below) {
        // From under the water the sky shows through a bright window
        // overhead; beyond about 49 degrees the surface mirrors the dark
        // water instead (total internal reflection).
        float window = smoothstep(0.62, 0.7, NdotV);
        vec3 deep = SCATTER_COLOR * ambient;
        color = mix(deep, skyColor(vec3(0.0, 1.0, 0.0)) * exp(-ABSORPTION * 2.0), window);
        alpha = 1.0;
    }

    // ── Sun ───────────────────────────────────────────────────────────
    float NdotL = max(dot(N, L), 0.0);
    if (NdotL > 0.0 && !below) {
        float roughness = mix(0.07, 0.16, far);
        vec3 H = normalize(V + L);
        float NdotH = max(dot(N, H), 0.0);
        float F = WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - max(dot(H, V), 0.0), 5.0);
        float specular = distributionGGX(NdotH, roughness * roughness)
            * geometrySmith(NdotV, NdotL, roughness) * F / max(4.0 * NdotV * NdotL, 1e-4);
        color += sunLight * specular * NdotL * shadow;
    }

    // ── Lamps ─────────────────────────────────────────────────────────
    if (!below && lightCount > 0) {
        // A little rougher than for the sun: a lamp is a point, and its
        // reflection would otherwise be a single sparkling pixel.
        color += lampReflections(N, V, NdotV, mix(0.12, 0.2, far));
    }

    // Light through the crests, toward a camera looking at the sun.
    float facing = pow(clamp(dot(-V, L) * 0.5 + 0.5, 0.0, 1.0), 3.0);
    float crestLight = max(WaveHeight, 0.0) * facing * (1.0 - far);
    color += CREST_COLOR * sunLight * shadow * crestLight * 0.12;

    // ── Foam ──────────────────────────────────────────────────────────
    float pattern;
    float rapids = 0.0;
    if (speed > 0.05) {
        vec2 p0, p1;
        float w0;
        flowPhases(RestXZ, p0, p1, w0);
        pattern = foamPattern(p0, uTime) * w0 + foamPattern(p1, uTime) * (1.0 - w0);
        // White water where it runs fast.
        rapids = smoothstep(0.55, 0.85, pattern) * smoothstep(1.5, 4.0, speed) * 0.8;
    } else {
        pattern = foamPattern(RestXZ, uTime);
    }
    float shore = 1.0 - smoothstep(0.0, SHORE_FOAM_DEPTH, waterDepth);
    // Bands washing up the beach.
    float surge = 0.5 + 0.5 * sin(waterDepth * 5.0 - uTime * 1.8 + pattern * 6.0);
    // Surf needs waves: a pond only gets the wet line below.
    float surf = smoothstep(15.0, 150.0, BodySize);
    float shoreFoam = smoothstep(0.62, 0.92, pattern * 1.1 + shore * (0.15 + 0.35 * surge)) * shore * surf;
    // A thin wet line where the water touches the ground.
    shoreFoam = max(shoreFoam, (1.0 - smoothstep(0.0, 0.12, waterDepth)) * 0.7);
    float crestFoam = smoothstep(0.5, 0.85, pattern + Crest * 0.6) * Crest;
    float foam = clamp(shoreFoam + crestFoam + rapids, 0.0, 1.0) * (1.0 - far * 0.7);
    vec3 foamLight = (sunLight * max(dot(normalize(N + vec3(0.0, 1.0, 0.0)), L), 0.0) * shadow
        + skyLight(vec3(0.0, 1.0, 0.0))) * 0.8;
    color = mix(color, foamLight, foam);
    alpha = max(alpha, foam * smoothstep(0.0, SHORE_FADE_DEPTH * 0.5, waterDepth));

    vec3 toPoint = WorldPos - cameraPosition;
    vec3 along = normalize(vec3(toPoint.x, max(toPoint.y, 0.0) * 0.3, toPoint.z));
    float haze = 1.0 - exp(-max(length(toPoint) - HAZE_START, 0.0) / HAZE_DISTANCE);
    color = mix(color, skyLight(along), haze);
    FragColor = vec4(toDisplay(weatherFog(color, WorldPos)), alpha);
}
