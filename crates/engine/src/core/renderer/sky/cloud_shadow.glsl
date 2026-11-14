// Cloud shadow on the ground (`sky::CLOUD_SHADOW_GLSL`). Needs
// `cloud_field.glsl` and the lighting uniforms before it.
// Samples the sky's own cloud field (`sky/cloud_field.glsl`, included
// before this) along the sun ray, so the shadow on the ground matches
// whatever cloud sits between this point and the sun.
float cloudShadow(vec3 worldPos) {
    // Sun below the horizon — no point sampling.
    if (sunDirection.y < 0.05) return 1.0;

    // March from cloud base to cloud top along the sun direction starting
    // above this terrain point. Few steps, large jumps — we just need an
    // optical-depth estimate, not detailed scattering.
    float tEnter = (CLOUD_BOTTOM  - worldPos.y) / sunDirection.y;
    float tExit  = (cloudLayerTop() - worldPos.y) / sunDirection.y;
    if (tExit <= 0.0) return 1.0;
    tEnter = max(tEnter, 0.0);

    const int   STEPS = 4;
    float stepLen = (tExit - tEnter) / float(STEPS);
    vec3  pos = worldPos + sunDirection * (tEnter + 0.5 * stepLen);

    float optical = 0.0;
    for (int i = 0; i < STEPS; i++) {
        optical += cloudDensity(pos, true, 0.0) * stepLen;
        pos += sunDirection * stepLen;
    }

    // Beer–Lambert through the cloud column toward the sun. Coefficient
    // is gentler than the sky's self-shadow (0.85) so terrain stays
    // readable under thick cover.
    float sunT = exp(-optical * 0.35);
    return mix(0.20, 1.0, sunT);
}

