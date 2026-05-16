#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec3 color;

out vec3 Normal;
out vec3 Color;
out vec3 toLightVector;
out vec4 fragPosLightSpace;

uniform vec3 lightPosition;
uniform vec3 chunkWorldOffset;
uniform mat4 model;
uniform mat4 viewProjection;
uniform mat4 lightProjection;


void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;

    vec3 n = normalize(normals);
    Normal = n;

    float h = position.y;

    // slope: 0 = flat, 1 = vertical cliff (uses normalised normal)
    float slope = 1.0 - abs(n.y);

    // ── Biome palette ────────────────────────────────────────────────
    vec3 seabedDeep   = vec3(0.32, 0.22, 0.12); // dark muddy brown at depth
    vec3 sand         = vec3(0.76, 0.70, 0.50);
    vec3 grassLow     = vec3(0.22, 0.52, 0.12);
    vec3 grassHigh    = vec3(0.17, 0.40, 0.09);
    vec3 rock         = vec3(0.46, 0.43, 0.39);
    vec3 darkRock     = vec3(0.36, 0.34, 0.32);
    vec3 alpine       = vec3(0.50, 0.48, 0.46);
    vec3 snow         = vec3(0.92, 0.93, 0.96);

    // ── Height-based biome with smooth transitions ───────────────────
    // Breakpoints kept close to the original (water<50, grass 51-80)
    vec3 heightColor;
    if (h < 44.0) {
        // Underwater terrain: sandy at the waterline, muddy brown at depth
        heightColor = mix(seabedDeep, sand, smoothstep(15.0, 44.0, h));
    } else if (h < 52.0) {
        // narrow beach band right at the waterline
        heightColor = mix(sand, grassLow, smoothstep(48.0, 52.0, h));
    } else if (h < 78.0) {
        heightColor = mix(grassLow, grassHigh, smoothstep(52.0, 78.0, h));
    } else if (h < 90.0) {
        heightColor = mix(grassHigh, alpine, smoothstep(78.0, 90.0, h));
    } else if (h < 100.0) {
        heightColor = mix(alpine, snow, smoothstep(90.0, 100.0, h));
    } else {
        heightColor = snow;
    }

    // Smooth sine-based tint to break up flat colour.
    // Uses chunkWorldOffset + local position — stable world coords that are
    // independent of the camera, so the wave never flickers or has seams.
    vec3 stableWorldPos = chunkWorldOffset + position;
    float wave = sin(stableWorldPos.x * 0.29) * sin(stableWorldPos.z * 0.37) * 0.035;
    heightColor = clamp(heightColor + vec3(wave * 0.4, wave, wave * 0.2), 0.0, 1.0);

    // ── Slope-based rock ─────────────────────────────────────────────
    // Only kicks in on proper cliffs: blend starts at 55°, full rock at 72°
    // (slope 0.57 ≈ cos(55°) from horizontal, 0.80 ≈ cos(72°))
    float rockBlend = smoothstep(0.57, 0.80, slope);

    // Darker rock at elevation (mountain cliffs vs riverbank mud)
    vec3 slopeRock = mix(rock, darkRock, smoothstep(55.0, 90.0, h));

    // Suppress rocky look in water/beach zones
    float landFactor = smoothstep(44.0, 52.0, h);
    rockBlend *= landFactor;

    Color = mix(heightColor, slopeRock, rockBlend);

    // Use the stable world position for shadow lookup.
    // worldPosition above is actually camera-space (model = vp*chunk_T due to
    // parameter ordering), so it changes every frame and produces flickering
    // shadows. chunkWorldOffset + position is the true world-space position and
    // matches exactly what the shadow pass wrote into the depth map.
    fragPosLightSpace = lightProjection * vec4(chunkWorldOffset + position, 1.0);
    toLightVector = lightPosition;
}
