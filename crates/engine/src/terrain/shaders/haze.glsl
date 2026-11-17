// The air between the camera and the far ground (see
// `terrain::shaders::HAZE_GLSL`): needs `cameraPosition` and `skyLight`
// (`pbr::LIGHTING_GLSL`).

// Haze: none up to `HAZE_START` metres, then thickening over
// `HAZE_DISTANCE`, toward the sky's light.
const float TERRAIN_HAZE_START = 600.0;
const float TERRAIN_HAZE_DISTANCE = 9000.0;

// `color` (linear) as the air between the camera and `p` hazes it.
vec3 terrainHaze(vec3 color, vec3 p) {
    vec3 toPoint = p - cameraPosition;
    float far = length(toPoint);
    vec3 along = normalize(vec3(toPoint.x, max(toPoint.y, 0.0) * 0.3, toPoint.z));
    float haze = 1.0 - exp(-max(far - TERRAIN_HAZE_START, 0.0) / TERRAIN_HAZE_DISTANCE);
    return mix(color, skyLight(along), haze);
}
