// Shared by the cloud pass (`clouds.glsl`) and the sky (`fragment.glsl`):
// camera, sun and cloud uniforms, sunlight colour, and the three cloud
// layers. Inserted after the `#version` line, before `cloud_field.glsl`.

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
uniform float uTime;         // monotonic seconds for wind drift
uniform float cloudCoverage; // 0..1 — fraction of sky the low layer covers
uniform float cloudType;     // 0 stratus .. 1 cumulonimbus (see `CloudType`)
uniform float altocumulus;   // 0..1 — amount of the mid layer
uniform float cirrus;        // 0..1 — amount of the high layer
uniform vec3 moonDirection;  // normalized, toward the moon
uniform float moonlight;     // 0 = moonless (see `GlobalIllumination`)

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

vec2 hash22(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// View ray of the pixel at `ndc` (-1..1), from the camera basis.
vec3 viewRay(vec2 ndc) {
    return normalize(camForward + camRight * (ndc.x * tanHalfFovX) + camUp * (ndc.y * tanHalfFovY));
}

float dayFactorOf(vec3 sun) {
    return smoothstep(-0.10, 0.20, clamp(sun.y, -1.0, 1.0));
}
