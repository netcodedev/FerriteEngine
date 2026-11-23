// Ray traced reflections in the lit shaders (graphics setting
// "Reflexionen" RT): after `raytracing/rays.glsl` and `lighting.glsl`,
// only where the device traces rays (`RAY_QUERIES`, set by
// `gfx::Program`).

// Farthest a reflection looks (metres); past it, the sky.
const float REFLECTION_DISTANCE = 300.0;

// The air between a mirror and what it shows, `t` metres away: as the
// terrain's haze thickens with the distance from the camera
// (`terrain/shaders/haze.glsl`, same numbers), toward the sky's light.
const float REFLECTION_HAZE_START = 600.0;
const float REFLECTION_HAZE_DISTANCE = 9000.0;

vec3 reflectionHaze(vec3 color, vec3 R, float t) {
    vec3 along = normalize(vec3(R.x, max(R.y, 0.0) * 0.3, R.z));
    float haze = 1.0 - exp(-max(t - REFLECTION_HAZE_START, 0.0) / REFLECTION_HAZE_DISTANCE);
    return mix(color, skyLight(along), haze);
}

// Light leaving the hit toward the ray's origin: its emission, the sun (or
// the moon) past what is in the way (a second ray), the sky and the
// scene's lamps (without shadows), all on a rough, non-metal surface.
vec3 shadeHit(Hit hit, vec3 toViewer) {
    vec3 N = hit.normal;
    vec3 albedo = hit.albedo;
    vec3 F0 = vec3(0.04);
    vec3 color = hit.emissive;
    vec3 origin = hit.position + N * 0.03;
    vec3 L = normalize(sunDirection);
    vec3 sun = sunRadiance();
    if (moonShadows == 0 && dot(sun, sun) > 0.0 && dot(N, L) > 0.0) {
        float blocker;
        float lit = traceShadow(origin, L, 4000.0, blocker);
        color += brdf(N, toViewer, L, albedo, 1.0, 0.0, F0) * sun * lit;
    }
    vec3 moon = moonRadiance();
    vec3 M = normalize(moonDirection);
    if (dot(moon, moon) > 0.0 && dot(N, M) > 0.0) {
        float lit = 1.0;
        if (moonShadows == 1) {
            float blocker;
            lit = traceShadow(origin, M, 4000.0, blocker);
        }
        color += brdf(N, toViewer, M, albedo, 1.0, 0.0, F0) * moon * lit;
    }
    // The scene's lamps, diffuse only.
    ivec2 list = lightsAt(hit.position);
    for (int k = 0; k < list.y; ++k) {
        int i = lightAt(list, k);
        if (i >= lightCount) {
            continue;
        }
        Light l = lights[i];
        int kind = int(l.directionKind.w + 0.5);
        vec3 toLight;
        float attenuation = 1.0;
        if (kind == LIGHT_DIRECTIONAL) {
            toLight = -l.directionKind.xyz;
        } else {
            vec3 d = l.positionRange.xyz - hit.position;
            float d2 = dot(d, d);
            float range = l.positionRange.w;
            if (d2 >= range * range) {
                continue;
            }
            float dist = sqrt(d2);
            toLight = d / max(dist, 1e-4);
            attenuation = rangeFade(dist, range) / max(d2, 0.01);
            if (kind == LIGHT_SPOT) {
                attenuation *= spotCone(l, toLight);
            }
        }
        color += albedo / PI * max(dot(N, toLight), 0.0) * l.radianceBounce.rgb * attenuation;
    }
    color += skyLight(N) * albedo;
    return color;
}

// What a mirror at `origin` sees along `R`: rgb the light, a 1 where the
// ray met something (0: the caller's sky).
vec4 traceReflection(vec3 origin, vec3 R, float maxDistance) {
    Hit hit;
    if (!traceClosest(origin, R, maxDistance, hit)) {
        return vec4(0.0);
    }
    return vec4(reflectionHaze(shadeHit(hit, -R), R, length(hit.position - origin)), 1.0);
}
